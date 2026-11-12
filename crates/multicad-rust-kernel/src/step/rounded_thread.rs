//! Certified five-band spline exchange for the literal double-rounded law.
use super::helical::{DEGREE, JETS, Spline, ThreadSplines, product};
use super::*;
use crate::{
    rounded_thread::{Circle, Thread},
    step_read::MAX_STEP_ENTITIES,
};
mod receipt;
const FACTORIALS: [f64; JETS] = [1., 1., 2., 6., 24., 120.];
const MAX_ERROR: f64 = 1e-7;

pub fn write_rounded_thread(
    thread: &Thread,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_rounded_thread(thread, origin, u, v, tolerance))
}
pub fn supports_rounded_thread(
    thread: &Thread,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
) -> bool {
    write_rounded_thread(thread, origin, u, v, tolerance).is_ok()
}
pub(super) fn encode(
    file: &mut StepFile,
    thread: &Thread,
    placement: Placement,
    tolerance: f64,
) -> Result<Entity, StepError> {
    let mut candidate = StepFile::new();
    let solid = encode_shell(&mut candidate, thread, &placement, tolerance)?;
    candidate.finish(vec![solid]);
    crate::step_read::verify_rounded_thread(
        &candidate.data,
        thread,
        placement.origin,
        placement.u,
        placement.v,
        tolerance,
    )
    .map_err(read_error)?;
    crate::step_read::read_rounded_thread(&candidate.data, tolerance).map_err(read_error)?;
    encode_shell(file, thread, &placement, tolerance)
}

fn read_error(error: crate::step_read::ReadError) -> StepError {
    match error {
        crate::step_read::ReadError::Limit => StepError::Limit,
        _ => StepError::InvalidGeometry,
    }
}

pub(crate) fn prepare(
    thread: &Thread,
    origin: Point,
    tolerance: f64,
) -> Result<ThreadSplines, StepError> {
    if !tolerance.is_finite() || tolerance <= 0. {
        return Err(StepError::InvalidGeometry);
    }
    let tolerance = tolerance.min(MAX_ERROR);
    let s = thread.spec();
    let circle_extent = (0..5)
        .filter_map(|i| thread.profile_circle(i))
        .map(|c| c.center_radius_mm.abs() + c.radius_mm)
        .fold(0., f64::max);
    let center_error = (0..5)
        .filter_map(|i| thread.profile_circle(i))
        .map(|c| c.center_error_mm)
        .fold(0., f64::max);
    let scale = origin.into_iter().map(f64::abs).fold(0., f64::max)
        + circle_extent
        + s.major_radius_mm * (s.length_mm / thread.helical_rate().abs())
        + s.major_radius_mm
        + s.length_mm
        + s.pitch_mm
        + 1.;
    if 256. * f64::EPSILON * scale + center_error > tolerance / 8.
        || thread.root_radius_mm() <= tolerance
    {
        return Err(StepError::Limit);
    }
    let rate = thread.helical_rate();
    let hand = rate.signum();
    let spin = s.length_mm / rate.abs();
    let axial_budget = tolerance / (32. * s.major_radius_mm);
    let axial = checked_affine([0., spin], [1.; 2], hand, axial_budget, None)?;
    let breaks = thread.profile_breaks();
    let mut bands = Vec::with_capacity(5);
    for (band, pair) in breaks.windows(2).enumerate() {
        let phases = [pair[0], pair[1]];
        let radii = phases.map(|p| thread.radius_at_phase(p));
        let first = bands
            .last()
            .map(|b: &Spline| *b.poles.last().expect("spline"));
        let curve = if let Some(circle) = thread.profile_circle(band) {
            let mut curve =
                circle_spline(circle, phases, radii, rate.abs(), -hand, tolerance / 128.)?;
            if let Some(first) = first {
                curve.poles[0] = first;
            }
            curve
        } else {
            let domain = phases.map(|p| p / rate.abs());
            checked_affine(domain, radii, -hand, tolerance / 32., first)?
        };
        bands.push(curve);
    }
    let seam = bands[0].poles[0];
    *bands
        .last_mut()
        .expect("five bands")
        .poles
        .last_mut()
        .expect("spline") = seam;
    if bands.iter().any(|b| !b.advances_angle(-hand)) {
        return Err(StepError::Limit);
    }
    let count = bands.iter().map(|b| b.poles.len()).sum::<usize>();
    if count > (MAX_STEP_ENTITIES / 2) / axial.poles.len() {
        return Err(StepError::Limit);
    }
    // Affine charts were receipted after sharing their incoming endpoint.
    // Circle charts are checked now, including the final periodic wrap pole.
    for (band, curve) in bands.iter().enumerate() {
        if let Some(circle) = thread.profile_circle(band) {
            let phases = [breaks[band], breaks[band + 1]];
            receipt::circle(curve, circle, phases, rate.abs(), -hand, tolerance / 32.)?;
        }
    }
    let heights = axial.linear_poles(s.length_mm);
    Ok(ThreadSplines {
        bands,
        axial,
        heights,
    })
}
/// Construct and certify one affine-radius rotation chart as one operation.
/// The optional incoming endpoint is an actual shared finite pole and is
/// included in the coefficient receipt before the curve can be published.
fn checked_affine(
    domain: [f64; 2],
    radii: [f64; 2],
    sense: f64,
    budget: f64,
    first: Option<[f64; 2]>,
) -> Result<Spline, StepError> {
    let mut curve = Spline::new(domain[0], domain[1], radii, sense, budget / 4.)?;
    if let Some(first) = first {
        curve.poles[0] = first;
    }
    receipt::affine(&curve, domain, radii, sense, budget)?;
    Ok(curve)
}

fn circle_spline(
    c: Circle,
    phases: [f64; 2],
    radii: [f64; 2],
    rate: f64,
    sense: f64,
    error: f64,
) -> Result<Spline, StepError> {
    let angles = phases.map(|p| ((p - c.center_phase_mm) / c.radius_mm).asin());
    let amplitude = c.radius_mm / rate;
    let span_limit = [1.5_f64, 2., 2.5, 3.]
        .into_iter()
        .map(|rho| {
            // Entire signed circle chart times exp(i*s/rate), bounded on a
            // complex disk; Cauchy's estimate cancels 12! in Hermite's remainder.
            let log_bound =
                (c.center_radius_mm.abs() + c.radius_mm * rho.cosh()).ln() + amplitude * rho.sinh();
            2. * rho * ((error.ln() - log_bound) / (DEGREE + 1) as f64).exp()
        })
        .fold(0., f64::max);
    let mut spline = Spline::with_jets(angles[0], angles[1], span_limit, |t| {
        circle_jet(c, t, rate, sense)
    })?;
    spline.poles[0] = helical::affine_jet(phases[0] / rate, radii[0], 0., sense)[0];
    *spline.poles.last_mut().expect("spline") =
        helical::affine_jet(phases[1] / rate, radii[1], 0., sense)[0];
    Ok(spline)
}
fn circle_jet(c: Circle, t: f64, rate: f64, sense: f64) -> [[f64; 2]; JETS] {
    let (s, co) = t.sin_cos();
    let trig = [[s, co], [co, -s], [-s, -co], [-co, s]];
    let amplitude = sense * c.radius_mm / rate;
    let (si, cs) = (sense * ((c.center_phase_mm + c.radius_mm * s) / rate)).sin_cos();
    let mut radial = std::array::from_fn::<_, JETS, _>(|i| {
        c.radial_sign * c.radius_mm * trig[i % 4][1] / FACTORIALS[i]
    });
    radial[0] += c.center_radius_mm;
    let mut spin = [[0.; 2]; JETS];
    spin[0] = [cs, si];
    for order in 1..JETS {
        let mut coeff = [0.; 2];
        for k in 1..=order {
            let im = amplitude * trig[k % 4][0] * k as f64 / FACTORIALS[k];
            let [x, y] = spin[order - k];
            coeff[0] -= im * y;
            coeff[1] += im * x;
        }
        spin[order] = coeff.map(|x| x / order as f64);
    }
    std::array::from_fn(|order| {
        let mut coeff = [0.; 2];
        for k in 0..=order {
            for (sum, term) in coeff.iter_mut().zip(spin[order - k]) {
                *sum += radial[k] * term;
            }
        }
        coeff.map(|x| x * FACTORIALS[order])
    })
}
fn encode_shell(
    file: &mut StepFile,
    thread: &Thread,
    placement: &Placement,
    tolerance: f64,
) -> Result<Entity, StepError> {
    encode_material(file, thread, placement, tolerance, None)
}
pub(super) fn encode_bore_shell(
    file: &mut StepFile,
    bore: &crate::rounded_thread::ThroughBore,
    placement: &Placement,
    tolerance: f64,
) -> Result<Entity, StepError> {
    if !tolerance.is_finite() || tolerance <= 0. {
        return Err(StepError::InvalidGeometry);
    }
    if bore.radius_mm() - bore.thread().spec().major_radius_mm <= 2. * tolerance.min(MAX_ERROR) {
        return Err(StepError::Limit);
    }
    encode_material(
        file,
        bore.thread(),
        placement,
        tolerance,
        Some(bore.radius_mm()),
    )
}
fn encode_material(
    file: &mut StepFile,
    thread: &Thread,
    placement: &Placement,
    tolerance: f64,
    stock: Option<f64>,
) -> Result<Entity, StepError> {
    let ThreadSplines {
        bands,
        axial,
        heights,
    } = prepare(thread, placement.origin, tolerance)?;
    let length = thread.spec().length_mm;
    let endpoints = [axial.poles[0], *axial.poles.last().expect("axial")];
    let rings: [Vec<Vertex>; 2] = std::array::from_fn(|end| {
        bands
            .iter()
            .map(|b| {
                file.vertex(
                    placement.point_f64(product(b.poles[0], endpoints[end]), [0., length][end]),
                )
            })
            .collect()
    });
    let rails = bands
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let mut points = axial
                .poles
                .iter()
                .zip(&heights)
                .map(|(&a, &z)| placement.point_f64(product(b.poles[0], a), z));
            helical::curve(file, &axial, [rings[0][i], rings[1][i]], &mut points)
        })
        .collect::<Vec<_>>();
    let forward = thread.spec().left_hand ^ stock.is_some();
    let mut caps: [Vec<Entity>; 2] = std::array::from_fn(|_| Vec::with_capacity(5));
    let mut faces = Vec::with_capacity(7 + usize::from(stock.is_some()));
    for (i, b) in bands.iter().enumerate() {
        let next = (i + 1) % 5;
        let rims = [0, 1].map(|end| {
            let mut points = b
                .poles
                .iter()
                .map(|&p| placement.point_f64(product(p, endpoints[end]), [0., length][end]));
            helical::curve(file, b, [rings[end][i], rings[end][next]], &mut points)
        });
        for (cap, rim) in caps.iter_mut().zip(rims) {
            cap.push(rim);
        }
        let uses = [
            (rims[0], true),
            (rails[next], true),
            (rims[1], false),
            (rails[i], false),
        ];
        let edges = if forward {
            uses.map(|(e, s)| file.oriented_edge(e, s))
        } else {
            [3, 2, 1, 0].map(|j| file.oriented_edge(uses[j].0, !uses[j].1))
        };
        let bound = file.bound(&edges, true);
        let support = helical::surface(file, b, &axial, &heights, placement);
        faces.push(file.face_on(&[bound], support, forward));
    }
    let cylinder = stock
        .map(|radius| {
            CylindricalTrim::new(
                file,
                [0., length].map(|z| placement.point_f64([0.; 2], z)),
                placement.normal,
                placement.u,
                radius,
            )
        })
        .transpose()?;
    for (end, rims) in caps.iter().enumerate() {
        let top = end == 1;
        let edges = if top == forward {
            rims.iter()
                .map(|&e| file.oriented_edge(e, true))
                .collect::<Vec<_>>()
        } else {
            rims.iter()
                .rev()
                .map(|&e| file.oriented_edge(e, false))
                .collect()
        };
        let bound = file.bound(&edges, stock.is_none());
        let mut bounds = Vec::with_capacity(1 + usize::from(stock.is_some()));
        if let Some(cylinder) = &cylinder {
            bounds.push(cylinder.cap_bound(file, top, true));
        }
        bounds.push(bound);
        let center = file.point(placement.point_f64([0.; 2], [0., length][end]));
        let normal = placement.normal.map(|n| if top { n } else { -n });
        faces.push(file.face(&bounds, center, normal, placement.u));
    }
    if let Some(cylinder) = cylinder {
        faces.push(cylinder.wall(file, true));
    }
    Ok(file.solid(&faces))
}
#[cfg(test)]
mod tests;

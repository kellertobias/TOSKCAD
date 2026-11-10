//! Bounded spline surfaces for a straight single-start helical thread.
use super::*;
use crate::{helical_thread::Thread, step_read::MAX_STEP_ENTITIES};

pub(crate) const DEGREE: usize = 11;
const JETS: usize = DEGREE.div_ceil(2);
const FACTORIAL: f64 = 479_001_600.;
const MAX_ERROR_MM: f64 = 1e-7;

type Complex = [f64; 2];

pub fn write_helical_thread(
    thread: &Thread,
    stock_radius_mm: Option<f64>,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> Result<String, StepError> {
    write_one(|writer| {
        writer.append_helical_thread(thread, stock_radius_mm, origin, u, v, tolerance_mm)
    })
}

/// Whether the requested straight thread fits the certified STEP budget.
pub fn supports_helical_thread(
    thread: &Thread,
    stock_radius_mm: Option<f64>,
    origin: Point,
    tolerance_mm: f64,
) -> bool {
    prepare(thread, stock_radius_mm, origin, tolerance_mm).is_ok()
}

pub(super) fn encode(
    file: &mut StepFile,
    thread: &Thread,
    stock_radius_mm: Option<f64>,
    placement: Placement,
    tolerance_mm: f64,
) -> Result<Entity, StepError> {
    let ThreadSplines {
        bands,
        axial,
        heights,
    } = prepare(thread, stock_radius_mm, placement.origin, tolerance_mm)?;
    let spec = thread.spec();
    let height = spec.length_mm;
    let [axial_start, axial_end] = [0, axial.poles.len() - 1];
    let endpoints = [axial.poles[axial_start], axial.poles[axial_end]];
    let rings = [0., height]
        .into_iter()
        .zip(endpoints)
        .map(|(z, spin)| {
            bands
                .iter()
                .map(|band| file.vertex(placement.point_f64(product(band.poles[0], spin), z)))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let rails = bands
        .iter()
        .enumerate()
        .map(|(i, band)| {
            let [bottom, top] = [rings[0][i], rings[1][i]];
            let mut poles = axial
                .poles
                .iter()
                .zip(&heights)
                .map(|(&spin, &z)| placement.point_f64(product(band.poles[0], spin), z));
            curve(file, &axial, [bottom, top], &mut poles)
        })
        .collect::<Vec<_>>();
    let mut faces = Vec::with_capacity(7);
    let mut cap_edges = [Vec::with_capacity(4), Vec::with_capacity(4)];
    let forward = spec.left_hand != stock_radius_mm.is_some();
    for (i, band) in bands.iter().enumerate() {
        let next = (i + 1) % bands.len();
        let rims = [0, 1].map(|end| {
            let z = [0., height][end];
            let mut poles = band
                .poles
                .iter()
                .map(|&radial| placement.point_f64(product(radial, endpoints[end]), z));
            curve(file, band, [rings[end][i], rings[end][next]], &mut poles)
        });
        for (cap, rim) in cap_edges.iter_mut().zip(rims) {
            cap.push(rim);
        }
        let surface = surface(file, band, &axial, &heights, &placement);
        let uses = [
            (rims[0], true),
            (rails[next], true),
            (rims[1], false),
            (rails[i], false),
        ];
        let edges = if forward {
            uses.map(|(edge, sense)| file.oriented_edge(edge, sense))
        } else {
            [3, 2, 1, 0].map(|i| file.oriented_edge(uses[i].0, !uses[i].1))
        };
        let bound = file.bound(&edges, true);
        faces.push(file.face_on(&[bound], surface, forward));
    }

    let centers = [0., height].map(|z| placement.point_f64([0.; 2], z));
    let outside = stock_radius_mm
        .map(|r| CylindricalTrim::new(file, centers, placement.normal, placement.u, r))
        .transpose()?;
    for (end, cap) in cap_edges.into_iter().enumerate() {
        let top = end == 1;
        let ring_forward = top == forward;
        let edges = if ring_forward {
            cap.into_iter()
                .map(|edge| file.oriented_edge(edge, true))
                .collect::<Vec<_>>()
        } else {
            cap.into_iter()
                .rev()
                .map(|edge| file.oriented_edge(edge, false))
                .collect::<Vec<_>>()
        };
        let trim = file.bound(&edges, outside.is_none());
        let mut bounds = Vec::with_capacity(2);
        if let Some(cylinder) = &outside {
            bounds.push(cylinder.cap_bound(file, top, true));
        }
        bounds.push(trim);
        let center = file.point(centers[end]);
        let normal = placement
            .normal
            .map(|value| if top { value } else { -value });
        faces.push(file.face(&bounds, center, normal, placement.u));
    }
    if let Some(cylinder) = &outside {
        faces.push(cylinder.wall(file, true));
    }
    Ok(file.solid(&faces))
}

pub(crate) struct ThreadSplines {
    pub(crate) bands: Vec<Spline>,
    pub(crate) axial: Spline,
    pub(crate) heights: Vec<f64>,
}

pub(crate) fn prepare(
    thread: &Thread,
    stock_radius_mm: Option<f64>,
    origin: Point,
    tolerance_mm: f64,
) -> Result<ThreadSplines, StepError> {
    let spec = thread.spec();
    if spec.starts != 1 || spec.root_radius_mm.is_some() {
        return Err(StepError::Unsupported);
    }
    if !tolerance_mm.is_finite()
        || tolerance_mm <= 0.
        || stock_radius_mm.is_some_and(|r| !r.is_finite() || r <= spec.major_radius_mm)
    {
        return Err(StepError::InvalidGeometry);
    }
    let tolerance = tolerance_mm.min(MAX_ERROR_MM);
    let height = spec.length_mm;
    let major = spec.major_radius_mm;
    let rate = thread.helical_rate();
    let spin = height / rate.abs();
    let scale = origin.into_iter().map(f64::abs).fold(0., f64::max)
        + stock_radius_mm.unwrap_or(major)
        + major * spin
        + height
        + 1.;
    // Reserve room for floating-point and decimal STEP representation after
    // rotating/translating the polynomial poles.
    if 128. * f64::EPSILON * scale > tolerance / 2.
        || spec.major_radius_mm - spec.depth_mm <= tolerance
        || stock_radius_mm.is_some_and(|radius| radius - spec.major_radius_mm <= tolerance)
    {
        return Err(StepError::Limit);
    }

    // Use dimensionless angular coordinates and a separable tensor product.
    // XY(p,z) = A(p) B(z), where A carries the radius and B is unit rotation.
    // Bound the factors separately. The product error is at most
    // e_A + R e_B + e_A e_B, leaving room for roundoff and the closure seam.
    let hand = rate.signum();
    let axial = Spline::new(0., spin, [1., 1.], hand, tolerance / (8. * major))?;
    let knots = thread.profile_breaks();
    let [_, start, upper, lower, crest, _] = knots;
    let phase = [start, upper, lower, crest, start + spec.pitch_mm];
    let root = thread.root_radius_mm();
    let radius = [major, root, root, major, major];
    let mut bands = phase
        .windows(2)
        .zip(radius.windows(2))
        .map(|(p, r)| {
            Spline::new(
                p[0] / rate.abs(),
                p[1] / rate.abs(),
                [r[0], r[1]],
                -hand,
                tolerance / 8.,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Both sides of the wrap use the same physical control point. The
    // difference between these periodic endpoints is only roundoff.
    let seam = bands[0].poles[0];
    let last_band = bands.last_mut().expect("four analytic bands");
    *last_band.poles.last_mut().expect("nonempty spline") = seam;
    if bands.iter().any(|band| !band.advances_angle(-hand)) {
        return Err(StepError::Limit);
    }
    let poles = bands.iter().map(|band| band.poles.len()).sum::<usize>();
    if poles > (MAX_STEP_ENTITIES / 2) / axial.poles.len() {
        return Err(StepError::Limit);
    }

    let heights = axial.linear_poles(height);
    Ok(ThreadSplines {
        bands,
        axial,
        heights,
    })
}

pub(crate) struct Spline {
    pub(crate) knots: Vec<f64>,
    pub(crate) poles: Vec<Complex>,
}

impl Spline {
    fn new(a: f64, b: f64, radii: [f64; 2], sense: f64, error: f64) -> Result<Self, StepError> {
        let span = b - a;
        let slope = (radii[1] - radii[0]) / span;
        let max_derivative = radii[0].max(radii[1]) + (DEGREE + 1) as f64 * slope.abs();
        // The two-end Hermite remainder is f^(12)(ξ)(t-a)^6(t-b)^6 / 12!.
        // Applying the scalar bound to a projection along the error vector
        // bounds the complex norm without a component-wise penalty.
        let segments = (span / (2. * (error * FACTORIAL / max_derivative).powf(1. / 12.))).ceil();
        if !(1. ..=1024.).contains(&segments) {
            return Err(StepError::Limit);
        }
        let segments = segments as usize;
        let knots = (0..=segments)
            .map(|i| {
                if i == segments {
                    b
                } else {
                    a + span * i as f64 / segments as f64
                }
            })
            .collect::<Vec<_>>();
        let mut poles = Vec::with_capacity(segments * DEGREE + 1);
        for pair in knots.windows(2) {
            let [start, end] = [pair[0], pair[1]];
            let length = end - start;
            let jets = |t, h| {
                let r = if t == a {
                    radii[0]
                } else if t == b {
                    radii[1]
                } else {
                    radii[0] + slope * (t - a)
                };
                control_half(t, r, slope, sense, h)
            };
            if poles.is_empty() {
                poles.extend(jets(start, length));
            } else {
                poles.extend(jets(start, length).into_iter().skip(1));
            }
            poles.extend(jets(end, -length).into_iter().rev());
        }
        Ok(Self { knots, poles })
    }

    fn linear_poles(&self, height: f64) -> Vec<f64> {
        // Bézier controls of an affine axial coordinate. This keeps caps
        // exactly planar and does not approximate the lead.
        let end = self.knots[self.knots.len() - 1];
        let mut result = vec![0.];
        for pair in self.knots.windows(2) {
            result.extend((1..=DEGREE).map(|i| {
                let z = if i == DEGREE {
                    pair[1]
                } else {
                    pair[0] + (pair[1] - pair[0]) * i as f64 / DEGREE as f64
                };
                z / end * height
            }));
        }
        result
    }

    fn advances_angle(&self, direction: f64) -> bool {
        self.poles.windows(DEGREE + 1).step_by(DEGREE).all(|poles| {
            // The numerator of d(arg(P))/dt is cross(P,P'). A Bernstein
            // polynomial stays in the hull of its coefficients. Multiplying
            // both factors gives a degree-21 Bernstein polynomial, whose
            // sign must agree throughout this span with the exact helix.
            (0..2 * DEGREE).all(|degree| {
                let first = degree.saturating_sub(DEGREE - 1);
                let last = degree.min(DEGREE);
                let coefficient = (first..=last)
                    .map(|i| {
                        let j = degree - i;
                        let a = poles[i];
                        let b = std::array::from_fn::<_, 2, _>(|k| poles[j + 1][k] - poles[j][k]);
                        let weight = binomial(DEGREE, i) * binomial(DEGREE - 1, j);
                        weight * (a[0] * b[1] - a[1] * b[0])
                    })
                    .fold((0., 0.), |(sum, absolute), term| {
                        (sum + term, absolute + term.abs())
                    });
                direction * coefficient.0 > 64. * f64::EPSILON * coefficient.1.max(1.)
            })
        })
    }

    fn multiplicities(&self) -> String {
        let mut values = vec![DEGREE.to_string(); self.knots.len()];
        values[0] = (DEGREE + 1).to_string();
        values[self.knots.len() - 1] = values[0].clone();
        values.join(",")
    }

    fn knot_values(&self) -> String {
        self.knots
            .iter()
            .map(|knot| format!("{knot:.16E}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn binomial(n: usize, k: usize) -> f64 {
    (1..=k.min(n - k)).fold(1., |value, i| value * (n + 1 - i) as f64 / i as f64)
}

pub(crate) fn product([ax, ay]: Complex, [bx, by]: Complex) -> Complex {
    [ax * bx - ay * by, ax * by + ay * bx]
}

fn control_half(t: f64, radius: f64, slope: f64, sense: f64, length: f64) -> [Complex; JETS] {
    let (sin, cos) = (sense * t).sin_cos();
    let rotation = [cos, sin];
    let imaginary = [0., sense];
    let mut power = [1., 0.];
    let mut jet = [[0.; 2]; JETS];
    for (k, derivative) in jet.iter_mut().enumerate() {
        let radial = [radius, -sense * k as f64 * slope];
        *derivative = product(product(rotation, power), radial);
        power = product(power, imaginary);
    }
    std::array::from_fn(|i| {
        let mut point = jet[0];
        let mut weight = 1.;
        for (k, derivative) in jet.iter().enumerate().take(i + 1).skip(1) {
            weight *= length * (i - k + 1) as f64 / ((DEGREE - k + 1) * k) as f64;
            point[0] += derivative[0] * weight;
            point[1] += derivative[1] * weight;
        }
        point
    })
}

fn curve(
    file: &mut StepFile,
    spline: &Spline,
    [from, to]: [Vertex; 2],
    points: &mut dyn Iterator<Item = Point>,
) -> Entity {
    let poles = points
        .enumerate()
        .map(|(i, point)| {
            if i == 0 {
                from.cartesian
            } else if i + 1 == spline.poles.len() {
                to.cartesian
            } else {
                file.point(point)
            }
        })
        .collect::<Vec<_>>();
    let multiplicities = spline.multiplicities();
    let knots = spline.knot_values();
    let curve = file.entity(format_args!(
        "B_SPLINE_CURVE_WITH_KNOTS('',{DEGREE},{},.UNSPECIFIED.,.F.,.F.,({multiplicities}),({knots}),.UNSPECIFIED.)",
        Entities(&poles)
    ));
    let (start, end) = (from.vertex, to.vertex);
    file.entity(format_args!("EDGE_CURVE('',{start},{end},{curve},.T.)"))
}

fn surface(
    file: &mut StepFile,
    radial: &Spline,
    axial: &Spline,
    heights: &[f64],
    placement: &Placement,
) -> Entity {
    let rows = radial
        .poles
        .iter()
        .map(|&radial| {
            axial
                .poles
                .iter()
                .zip(heights)
                .map(|(&spin, &z)| file.point(placement.point_f64(product(radial, spin), z)))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let net = rows
        .iter()
        .map(|row| Entities(row).to_string())
        .collect::<Vec<_>>()
        .join(",");
    let u_mult = radial.multiplicities();
    let v_mult = axial.multiplicities();
    let u_knots = radial.knot_values();
    let v_knots = axial.knot_values();
    file.entity(format_args!(
        "B_SPLINE_SURFACE_WITH_KNOTS('',{DEGREE},{DEGREE},({net}),.UNSPECIFIED.,.F.,.F.,.F.,({u_mult}),({v_mult}),({u_knots}),({v_knots}),.UNSPECIFIED.)"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helical_thread::Spec;

    fn thread(starts: u32, left_hand: bool) -> Thread {
        Thread::new(Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 3.7,
            depth_mm: 5. * 3_f64.sqrt() / 16.,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts,
            left_hand,
            root_radius_mm: None,
        })
        .unwrap()
    }

    #[test]
    fn bounded_spline_matches_rotation_and_affine_radius_between_knots() {
        for sense in [-1., 1.] {
            for radii in [[1., 1.], [3., 2.4], [2.4, 3.]] {
                let error = 1e-8;
                let [a, b] = [0., 7.4];
                let spline = Spline::new(a, b, radii, sense, error).unwrap();
                let linear = spline.linear_poles(5.);
                for (i, pair) in spline.knots.windows(2).enumerate() {
                    for j in 0..=17 {
                        let t = j as f64 / 17.;
                        let x = pair[0] + t * (pair[1] - pair[0]);
                        let r = radii[0] + (radii[1] - radii[0]) * (x - a) / (b - a);
                        let (sin, cos) = (sense * x).sin_cos();
                        let idx = i * DEGREE;
                        let mut controls: [[f64; 3]; DEGREE + 1] = std::array::from_fn(|j| {
                            let [a, b] = spline.poles[idx + j];
                            [a, b, linear[idx + j]]
                        });
                        for n in (1..=DEGREE).rev() {
                            for k in 0..n {
                                controls[k] = std::array::from_fn(|l| {
                                    (1. - t) * controls[k][l] + t * controls[k + 1][l]
                                });
                            }
                        }
                        assert!((controls[0][0] - r * cos).hypot(controls[0][1] - r * sin) < error);
                        assert!((controls[0][2] - x / b * 5.).abs() < 1e-13);
                    }
                }
            }
        }
    }

    #[test]
    fn threaded_exchange_bounds_its_domain_and_retains_both_handings() {
        let origin = [0.; 3];
        let u = [1., 0., 0.];
        let v = [0., 1., 0.];
        let write = |thread: &Thread, radius, tolerance| {
            write_helical_thread(thread, radius, origin, u, v, tolerance)
        };
        for (left, radius) in [
            (false, None),
            (true, Some(4.5)),
            (true, None),
            (false, Some(4.5)),
        ] {
            let step = write(&thread(1, left), radius, 1e-7).unwrap();
            assert_eq!(step.matches("B_SPLINE_SURFACE_WITH_KNOTS").count(), 4);
            assert_eq!(
                step.matches("CYLINDRICAL_SURFACE").count(),
                usize::from(radius.is_some())
            );
            assert_eq!(step.matches("MANIFOLD_SOLID_BREP").count(), 1);
            assert_eq!(
                step.matches("ADVANCED_FACE").count(),
                6 + usize::from(radius.is_some())
            );
            assert!(!step.contains("FACETED_BREP"));
            let verify = |input: &str| {
                crate::step_read::verify_helical_thread(
                    input,
                    &thread(1, left),
                    radius,
                    origin,
                    u,
                    v,
                    1e-7,
                )
            };
            let volume = verify(&step).unwrap();
            let source_volume = thread(1, left).volume_mm3();
            let expected = radius.map_or(source_volume, |radius| {
                std::f64::consts::PI * radius * radius * 3.7 - source_volume
            });
            assert!((volume - expected).abs() < 1e-6);
            assert_ne!(volume, expected);

            let changed_degree = step.replacen(
                "B_SPLINE_SURFACE_WITH_KNOTS('',11,11",
                "B_SPLINE_SURFACE_WITH_KNOTS('',11,10",
                1,
            );
            let changed_units = step.replacen(".MILLI.,.METRE.", "$,.METRE.", 1);
            let changed_control =
                step.replacen("CARTESIAN_POINT('',(", "CARTESIAN_POINT('',(1000.,", 1);
            for changed in [changed_degree, changed_units, changed_control] {
                assert_ne!(changed, step);
                assert!(verify(&changed).is_err());
            }
        }
        assert_eq!(
            write(&thread(2, false), None, 1e-7),
            Err(StepError::Unsupported)
        );
        assert_eq!(
            write(&thread(1, false), Some(3.), 1e-7),
            Err(StepError::InvalidGeometry)
        );
        assert_eq!(
            write(&thread(1, false), None, f64::NAN),
            Err(StepError::InvalidGeometry)
        );
        assert_eq!(write(&thread(1, false), None, 1e-16), Err(StepError::Limit));
        assert!(matches!(
            Spline::new(0., 1e12, [1., 1.], 1., 1e-8),
            Err(StepError::Limit)
        ));
        assert!(supports_helical_thread(
            &thread(1, false),
            None,
            origin,
            1e-7
        ));
        let narrow_crest = Thread::new(Spec {
            major_radius_mm: 0.001,
            pitch_mm: 0.001,
            length_mm: 0.0037,
            depth_mm: 0.0005,
            crest_width_mm: 1e-12,
            ..thread(1, false).spec()
        })
        .unwrap();
        assert_eq!(write(&narrow_crest, None, 1e-7), Err(StepError::Limit));
        let long = Thread::new(Spec {
            length_mm: 124.,
            ..thread(1, false).spec()
        })
        .unwrap();
        assert_eq!(write(&long, None, 1e-7), Err(StepError::Limit));
    }
}

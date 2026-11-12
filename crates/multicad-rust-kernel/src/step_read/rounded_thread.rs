//! Full finite graph proof of double-rounded thread tensor BREP members.
use super::helical::{Proof, face, orient, require, ring, spline_edge};
use super::*;
use crate::{
    rounded_thread::Thread,
    step::{
        helical::{DEGREE, product},
        rounded_thread::prepare,
    },
};

mod import;
mod matcher;
pub use matcher::rounded_thread_import_matches;
#[derive(Clone, Debug)]
pub struct ImportedRoundedThread {
    /// Certified physical candidate; no original family, fit or units inferred.
    pub thread: Thread,
    pub origin_mm: Point,
    pub u: Point,
    pub v: Point,
}
pub fn read_rounded_thread(
    input: &str,
    tolerance: f64,
) -> Result<(Readback, ImportedRoundedThread)> {
    let mut members = compound::read_members_by(input, tolerance, true, true, |shell, id, _| {
        import::read_member(shell, id)
    })?;
    members.pop().ok_or(ReadError::Invalid)
}
pub fn read_rounded_thread_compound(
    input: &str,
    tolerance: f64,
) -> Result<Vec<(Readback, ImportedRoundedThread)>> {
    compound::read_members_by(input, tolerance, true, false, |shell, id, _| {
        import::read_member(shell, id)
    })
}

pub fn verify_rounded_thread(
    input: &str,
    thread: &Thread,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
) -> Result<f64> {
    let mut members = compound::read_members_by(input, tolerance, true, true, |shell, id, _| {
        verify_member(shell, id, thread, [origin, u, v], None)
    })?;
    members.pop().ok_or(ReadError::Invalid)
}
pub(super) fn verify_member(
    shell: &mut Shell<'_, '_>,
    id: u32,
    thread: &Thread,
    [origin, u, v]: [Point; 3],
    error: Option<f64>,
) -> Result<f64> {
    require(
        origin.into_iter().chain(u).chain(v).all(f64::is_finite)
            && (dot(u, u) - 1.).abs() <= 1e-10
            && (dot(v, v) - 1.).abs() <= 1e-10
            && dot(u, v).abs() <= 1e-10,
    )?;
    let normal = cross(u, v);
    let axis = unit(normal)?;
    let splines = prepare(thread, origin, shell.tolerance).map_err(helical::step_error)?;
    let proof = Proof {
        error,
        extent: thread.spec().major_radius_mm.max(thread.spec().length_mm),
    };
    let world = |[x, y]: Point2, z: f64| {
        std::array::from_fn(|i| origin[i] + u[i] * x + v[i] * y + normal[i] * z)
    };
    let ids = shell.member_faces(id)?;
    require(ids.len() == 7)?;
    let mut available = ids.into_iter().collect::<BTreeSet<_>>();
    require(available.len() == 7)?;
    let mut selected = Vec::with_capacity(7);
    // A finite endpoint proposes a unique band, but cannot qualify it: the
    // complete net, knot structure, curve controls and incidence follow below.
    for b in &splines.bands {
        let expected = world(product(b.poles[0], splines.axial.poles[0]), 0.);
        let mut candidates = Vec::new();
        for &id in &available {
            let (surface, _, _) = face(shell, id)?;
            let e = shell.simple(surface)?;
            if e.name != "B_SPLINE_SURFACE_WITH_KNOTS" {
                continue;
            }
            let Some(Value::List(rows)) = e.args.get(3) else {
                return Err(ReadError::Invalid);
            };
            let first = refs(rows.first().ok_or(ReadError::Invalid)?)?;
            let point = shell.point(*first.first().ok_or(ReadError::Invalid)?)?;
            if proof.equal(point, expected) {
                candidates.push(id);
            }
        }
        let [id] = candidates[..] else {
            return Err(ReadError::Invalid);
        };
        available.remove(&id);
        selected.push(id);
    }
    let length = thread.spec().length_mm;
    let mut cap_axes = Vec::with_capacity(2);
    for end in 0..2 {
        let expected = world([0.; 2], [0., length][end]);
        let mut candidates = Vec::new();
        for &id in &available {
            let (surface, _, _) = face(shell, id)?;
            let [Value::String(_), a] = shell.args(surface, "PLANE")? else {
                return Err(ReadError::Invalid);
            };
            if proof.equal(shell.placement(reference(a)?)?.origin, expected) {
                candidates.push((id, reference(a)?));
            }
        }
        let [(id, axis)] = candidates[..] else {
            return Err(ReadError::Invalid);
        };
        available.remove(&id);
        selected.push(id);
        cap_axes.push(axis);
    }
    require(available.is_empty())?;
    let forward = thread.spec().left_hand;
    let quadrature = helical::volume::Quadrature::new();
    let mut flux = 0.;
    let mut rims = [[0_u32; 5]; 2];
    let mut rails = [[0_u32; 2]; 5];
    let mut corners = [[[0_u32; 2]; 2]; 5];
    for (i, radial) in splines.bands.iter().enumerate() {
        let (surface, outward, bounds) = face(shell, selected[i])?;
        let [bound] = bounds[..] else {
            return Err(ReadError::Invalid);
        };
        require(outward == forward)?;
        let [
            Value::String(_),
            Value::Number(d1),
            Value::Number(d2),
            Value::List(rows),
            Value::Enum("UNSPECIFIED"),
            Value::Enum("F"),
            Value::Enum("F"),
            Value::Enum("F"),
            rm,
            am,
            rk,
            ak,
            Value::Enum("UNSPECIFIED"),
        ] = shell.args(surface, "B_SPLINE_SURFACE_WITH_KNOTS")?
        else {
            return Err(ReadError::Invalid);
        };
        require(*d1 == DEGREE as f64 && *d2 == DEGREE as f64 && rows.len() == radial.poles.len())?;
        proof.knots(rm, rk, radial)?;
        proof.knots(am, ak, &splines.axial)?;
        let controls = rows
            .iter()
            .zip(&radial.poles)
            .map(|(row, &r)| {
                proof.poles(
                    shell,
                    row,
                    splines
                        .axial
                        .poles
                        .iter()
                        .zip(&splines.heights)
                        .map(|(&a, &z)| world(product(r, a), z)),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        flux += quadrature.spline_flux(&controls, origin, axis);
        let uses = orient(&ring(shell, bound, true)?, forward);
        let [(start, true), (next, true), (end, false), (previous, false)] = uses[..] else {
            return Err(ReadError::Invalid);
        };
        rims[0][i] = start;
        rims[1][i] = end;
        rails[i] = [previous, next];
        for (j, (edge, k)) in [(previous, 0), (next, radial.poles.len() - 1)]
            .into_iter()
            .enumerate()
        {
            corners[i][j] = spline_edge(
                proof,
                shell,
                edge,
                &splines.axial,
                &controls[k],
                splines
                    .axial
                    .poles
                    .iter()
                    .zip(&splines.heights)
                    .map(|(&a, &z)| world(product(radial.poles[k], a), z)),
            )?;
        }
        for (cap, edge) in [start, end].into_iter().enumerate() {
            let ai = if cap == 0 {
                0
            } else {
                splines.axial.poles.len() - 1
            };
            let a = splines.axial.poles[ai];
            let boundary = controls.iter().map(|row| row[ai]).collect::<Vec<_>>();
            let endpoints = spline_edge(
                proof,
                shell,
                edge,
                radial,
                &boundary,
                radial
                    .poles
                    .iter()
                    .map(|&r| world(product(r, a), [0., length][cap])),
            )?;
            require(endpoints == [corners[i][0][cap], corners[i][1][cap]])?;
        }
    }
    require((0..5).all(|i| rails[i][1] == rails[(i + 1) % 5][0]))?;
    for end in 0..2 {
        let (surface, outward, bounds) = face(shell, selected[5 + end])?;
        let [bound] = bounds[..] else {
            return Err(ReadError::Invalid);
        };
        require(outward)?;
        let _ = surface; // Its PLANE schema/axis was validated during selection.
        proof.axis(
            shell,
            &Value::Reference(cap_axes[end]),
            world([0.; 2], [0., length][end]),
            normal.map(|n| if end == 1 { n } else { -n }),
            u,
        )?;
        let actual = orient(&ring(shell, bound, true)?, (end == 1) == forward);
        let expected = rims[end].map(|edge| (edge, true));
        require(actual == expected)?;
    }
    let volume = if forward { flux } else { -flux };
    require(volume.is_finite() && volume > 0.)?;
    Ok(volume)
}

#[cfg(test)]
mod tests;

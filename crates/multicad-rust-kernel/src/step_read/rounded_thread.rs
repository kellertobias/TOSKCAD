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

pub(super) mod import;
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
    verify_material(shell, id, thread, [origin, u, v], error, None)
}
pub(super) fn verify_bore_member(
    shell: &mut Shell<'_, '_>,
    id: u32,
    bore: &crate::rounded_thread::ThroughBore,
    frame: [Point; 3],
    error: Option<f64>,
) -> Result<f64> {
    if bore.radius_mm() - bore.thread().spec().major_radius_mm <= 2. * shell.tolerance.min(1e-7) {
        return Err(ReadError::Limit);
    }
    verify_material(
        shell,
        id,
        bore.thread(),
        frame,
        error,
        Some(bore.radius_mm()),
    )
}
fn verify_material(
    shell: &mut Shell<'_, '_>,
    id: u32,
    thread: &Thread,
    [origin, u, v]: [Point; 3],
    error: Option<f64>,
    stock: Option<f64>,
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
        extent: stock
            .unwrap_or(thread.spec().major_radius_mm)
            .max(thread.spec().length_mm),
    };
    let world = |[x, y]: Point2, z: f64| {
        std::array::from_fn(|i| origin[i] + u[i] * x + v[i] * y + normal[i] * z)
    };
    let ids = shell.member_faces(id)?;
    let count = 7 + usize::from(stock.is_some());
    require(ids.len() == count)?;
    let mut available = ids.into_iter().collect::<BTreeSet<_>>();
    require(available.len() == count)?;
    let mut selected = Vec::with_capacity(count);
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
            if stock.is_some() && shell.simple(surface)?.name == "CYLINDRICAL_SURFACE" {
                continue;
            }
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
    let cylinder = if stock.is_some() {
        Some(remaining_cylinder(&mut available)?)
    } else {
        None
    };
    require(available.is_empty())?;
    let forward = thread.spec().left_hand;
    let wall_forward = forward ^ stock.is_some();
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
        require(outward == wall_forward)?;
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
        let uses = orient(&ring(shell, bound, true)?, wall_forward);
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
    let mut cylinder_rims = [0; 2];
    let mut cap_origins = [[0.; 3]; 2];
    for end in 0..2 {
        let (surface, outward, bounds) = face(shell, selected[5 + end])?;
        require(outward)?;
        let _ = surface;
        let (bound, outer) = if stock.is_some() {
            let [a, b] = bounds[..] else {
                return Err(ReadError::Invalid);
            };
            match (shell.simple(a)?.name, shell.simple(b)?.name) {
                ("FACE_OUTER_BOUND", "FACE_BOUND") => (b, Some(a)),
                ("FACE_BOUND", "FACE_OUTER_BOUND") => (a, Some(b)),
                _ => return Err(ReadError::Invalid),
            }
        } else {
            let [bound] = bounds[..] else {
                return Err(ReadError::Invalid);
            };
            (bound, None)
        };
        let cap = proof.axis(
            shell,
            &Value::Reference(cap_axes[end]),
            world([0.; 2], [0., length][end]),
            normal.map(|n| if end == 1 { n } else { -n }),
            u,
        )?;
        cap_origins[end] = cap.origin;
        super::rounded_thread_bore::verify_direction(
            shell,
            cap.normal,
            axis.map(|n| if end == 1 { n } else { -n }),
            stock.unwrap_or(thread.spec().major_radius_mm),
        )?;
        let actual = orient(
            &ring(shell, bound, stock.is_none())?,
            (end == 1) == wall_forward,
        );
        require(actual == rims[end].map(|edge| (edge, true)))?;
        if let Some(bound) = outer {
            let rim = ring(shell, bound, true)?;
            let [(edge, sense)] = rim[..] else {
                return Err(ReadError::Invalid);
            };
            require(sense == (end == 1))?;
            let circle = outer_circle(shell, edge)?;
            require(
                proof.equal(circle.center_mm, world([0.; 2], [0., length][end]))
                    && crate::step_read::length(sub(circle.normal, normal)) < proof.angular()
                    && Some(circle.radius_mm) == stock,
            )?;
            super::rounded_thread_bore::verify_direction(
                shell,
                circle.normal,
                axis,
                stock.expect("checked radius"),
            )?;
            cylinder_rims[end] = edge;
        }
    }
    let tool_volume = if forward { flux } else { -flux };
    require(tool_volume.is_finite() && tool_volume > 0.)?;
    let volume = if let Some(radius) = stock {
        let id = cylinder.expect("checked cylindrical closure");
        let (r, axis) = helical::check_cylinder_with_proof(
            proof,
            shell,
            id,
            cylinder_rims,
            (radius, true),
            [origin, normal, u],
        )?;
        super::rounded_thread_bore::verify_outer_frame(
            shell,
            id,
            cap_origins,
            [origin, u, v],
            r,
            length,
        )?;
        std::f64::consts::PI * r * r * dot(sub(cap_origins[1], cap_origins[0]), axis) - tool_volume
    } else {
        tool_volume
    };
    require(volume.is_finite() && volume > 0.)?;
    Ok(volume)
}

// The wall/cap selector consumes its proved faces before handing ownership of
// exactly one remaining cylindrical support to the closure proof.
/// A receiver's outer trim must be an actual full circle, not merely an
/// arbitrary successfully parsed analytic edge.
pub(super) fn outer_circle(shell: &mut Shell<'_, '_>, edge: u32) -> Result<CircleEdge> {
    shell.edge(edge)?;
    let Curve::Circle(circle) = shell.edges[&edge].curve else {
        return Err(ReadError::Invalid);
    };
    Ok(circle)
}

pub(super) fn remaining_cylinder(available: &mut BTreeSet<u32>) -> Result<u32> {
    if available.len() != 1 {
        return Err(ReadError::Invalid);
    }
    Ok(available.pop_first().expect("checked remainder"))
}

#[cfg(test)]
mod tests;

/// Narrow authored-class admission, after the existing legacy providers.
pub(super) fn recognize_member(
    shell: &mut Shell<'_, '_>,
    id: u32,
    faces: &[u32],
) -> Result<Option<(Readback, ImportSolid)>> {
    if faces.len() != 7 {
        return Ok(None);
    }
    let mut count = 0;
    for &id in faces {
        let (surface, _, _) = face(shell, id)?;
        let e = shell.simple(surface)?;
        if e.name == "B_SPLINE_SURFACE_WITH_KNOTS"
            && e.args.get(1..3)
                == Some(&[Value::Number(DEGREE as f64), Value::Number(DEGREE as f64)])
        {
            count += 1;
        }
    }
    if count != 5 {
        return Ok(None);
    }
    let (proof, body) = import::read_member(shell, id)?;
    let spec = body.thread.spec();
    Ok(Some((
        proof,
        ImportSolid {
            origin_mm: body.origin_mm,
            u: body.u,
            v: body.v,
            height_mm: spec.length_mm,
            profile: ImportProfile::RoundedThread { spec },
        },
    )))
}

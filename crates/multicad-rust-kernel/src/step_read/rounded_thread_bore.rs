//! Independent finite graph proof and physical import of rounded receivers.
use super::helical::{face, require};
use super::helical_import::cylindrical_closure::{self, Cylinder, expression_bound};
use super::*;
use crate::rounded_thread::ThroughBore;
#[derive(Clone, Debug)]
pub struct ImportedRoundedThreadBore {
    pub bore: ThroughBore,
    pub origin_mm: Point,
    pub u: Point,
    pub v: Point,
}
pub fn read_rounded_thread_bore(
    input: &str,
    tolerance: f64,
) -> Result<(Readback, ImportedRoundedThreadBore)> {
    let mut members = compound::read_members_by(input, tolerance, true, true, |shell, id, _| {
        read_member(shell, id)
    })?;
    members.pop().ok_or(ReadError::Invalid)
}
pub fn read_rounded_thread_bore_compound(
    input: &str,
    tolerance: f64,
) -> Result<Vec<(Readback, ImportedRoundedThreadBore)>> {
    compound::read_members_by(input, tolerance, true, false, |shell, id, _| {
        read_member(shell, id)
    })
}
pub(super) fn recognize_member(
    shell: &mut Shell<'_, '_>,
    id: u32,
    faces: &[u32],
) -> Result<Option<(Readback, ImportSolid)>> {
    if faces.len() != 8 {
        return Ok(None);
    }
    let mut count = 0;
    for &id in faces {
        let (surface, _, _) = face(shell, id)?;
        if shell.simple(surface)?.name == "B_SPLINE_SURFACE_WITH_KNOTS" {
            count += 1;
        }
    }
    if count != 5 {
        return Ok(None);
    }
    let (proof, body) = read_member(shell, id)?;
    Ok(Some((
        proof,
        ImportSolid {
            origin_mm: body.origin_mm,
            u: body.u,
            v: body.v,
            height_mm: body.bore.length_mm(),
            profile: ImportProfile::RoundedThreadBore {
                spec: body.bore.thread().spec(),
                stock_radius_mm: body.bore.radius_mm(),
            },
        },
    )))
}
fn read_member(
    shell: &mut Shell<'_, '_>,
    id: u32,
) -> Result<(Readback, ImportedRoundedThreadBore)> {
    let faces = shell.member_faces(id)?;
    if faces.len() != 8 {
        return Err(ReadError::Unsupported);
    }
    let mut inner = Vec::with_capacity(7);
    let mut radii = Vec::with_capacity(1);
    for face_id in faces {
        let (surface, _, _) = face(shell, face_id)?;
        match shell.simple(surface)?.name {
            "PLANE" | "B_SPLINE_SURFACE_WITH_KNOTS" => inner.push(face_id),
            "CYLINDRICAL_SURFACE" => {
                let [Value::String(_), _, Value::Number(r)] =
                    shell.args(surface, "CYLINDRICAL_SURFACE")?
                else {
                    return Err(ReadError::Invalid);
                };
                radii.push(*r);
            }
            _ => return Err(ReadError::Unsupported),
        }
    }
    let [radius] = radii[..] else {
        return Err(ReadError::Invalid);
    };
    require(inner.len() == 7)?;
    let candidate = super::rounded_thread::import::candidate(shell, &inner)?;
    // This constant-work constructor validates a finite containing radius;
    // its only failure is an invalid physical stock domain.
    let bore = ThroughBore::new(candidate.thread, radius).map_err(|_| ReadError::Invalid)?;
    let frame = [candidate.origin_mm, candidate.u, candidate.v];
    let tolerance = shell.tolerance.min(1e-7);
    let volume =
        super::rounded_thread::verify_bore_member(shell, id, &bore, frame, Some(tolerance / 16.))?;
    let normal = cross(frame[1], frame[2]);
    let mut bounds = [[0.; 3]; 2];
    for (end, sign) in [-1., 1.].into_iter().enumerate() {
        for axis in 0..3 {
            let direction = [
                frame[1][axis] * sign,
                frame[2][axis] * sign,
                normal[axis] * sign,
            ];
            let support = bore
                .support_bounds(direction, tolerance / 8.)
                .map_err(|_| ReadError::Limit)?[1];
            let value = frame[0][axis] + sign * (support + tolerance / 4.);
            bounds[end][axis] = if end == 0 {
                value.next_down()
            } else {
                value.next_up()
            };
        }
    }
    Ok((
        Readback {
            bounds_mm: bounds,
            volume_mm3: volume,
            face_count: 8,
            solid_count: 1,
            edges_mm: vec![],
            circles: vec![],
            semicircles: vec![],
            arcs: vec![],
            ellipses: vec![],
            splines: vec![],
        },
        ImportedRoundedThreadBore {
            bore,
            origin_mm: frame[0],
            u: frame[1],
            v: frame[2],
        },
    ))
}
pub fn verify_rounded_thread_bore(
    input: &str,
    bore: &ThroughBore,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
) -> Result<f64> {
    let mut members = compound::read_members_by(input, tolerance, true, true, |shell, id, _| {
        super::rounded_thread::verify_bore_member(shell, id, bore, [origin, u, v], None)
    })?;
    members.pop().ok_or(ReadError::Invalid)
}
pub(super) fn verify_outer_frame(
    shell: &Shell<'_, '_>,
    id: u32,
    caps: [Point; 2],
    frame: [Point; 3],
    radius: f64,
    height: f64,
) -> Result<()> {
    let (surface, _, _) = face(shell, id)?;
    let [Value::String(_), axis, Value::Number(_)] = shell.args(surface, "CYLINDRICAL_SURFACE")?
    else {
        return Err(ReadError::Invalid);
    };
    let p = shell.placement(reference(axis)?)?;
    let error = cylindrical_closure::error(
        Cylinder {
            frame,
            radius_mm: radius,
            axial_mm: [0., height],
        },
        Cylinder {
            frame: [p.origin, p.u, p.v],
            radius_mm: radius,
            axial_mm: [0., height],
        },
    );
    let normal = cross(frame[1], frame[2]);
    let endpoint = [0., height]
        .into_iter()
        .zip(caps)
        .map(|(z, p)| {
            (0..3).fold(0., |sum, i| {
                (sum + expression_bound([(frame[0][i], 1.), (normal[i], z), (p[i], -1.)])).next_up()
            })
        })
        .fold(0., f64::max);
    let allowed = shell.tolerance.min(1e-7) / 32.;
    require(error.is_finite() && endpoint.is_finite() && (error + endpoint).next_up() <= allowed)
}

pub(super) fn verify_direction(
    shell: &Shell<'_, '_>,
    actual: Point,
    expected: Point,
    extent: f64,
) -> Result<()> {
    let difference = (0..3).fold(0., |sum, i| {
        (sum + expression_bound([(actual[i], 1.), (expected[i], -1.)])).next_up()
    });
    // Gram admission allows 1e-10 in either diagonal and cross term. Its
    // two-dimensional operator norm is below 1+2e-10; include that finite
    // coordinate enclosure when bounding normal error over the complete cap.
    let extent = (extent * (1. + 2e-10)).next_up();
    require((extent * difference).next_up() <= shell.tolerance.min(1e-7) / 32.)
}

/// Compare all thread tensor controls and the complete finite outer closure.
/// The imported descriptor must first pass the independent member proof.
pub fn rounded_thread_bore_import_matches(
    imported: &ImportedRoundedThreadBore,
    expected: &ThroughBore,
    frame: [Point; 3],
    tolerance: f64,
) -> bool {
    if !super::rounded_thread::rounded_thread_import_matches(
        &super::rounded_thread::ImportedRoundedThread {
            thread: *imported.bore.thread(),
            origin_mm: imported.origin_mm,
            u: imported.u,
            v: imported.v,
        },
        expected.thread(),
        frame,
        tolerance,
    ) {
        return false;
    }
    let bound = cylindrical_closure::error(
        Cylinder {
            frame,
            radius_mm: expected.radius_mm(),
            axial_mm: [0., expected.length_mm()],
        },
        Cylinder {
            frame: [imported.origin_mm, imported.u, imported.v],
            radius_mm: imported.bore.radius_mm(),
            axial_mm: [0., imported.bore.length_mm()],
        },
    );
    bound.is_finite() && bound <= tolerance.min(1e-7) / 16.
}
#[cfg(test)]
mod tests;

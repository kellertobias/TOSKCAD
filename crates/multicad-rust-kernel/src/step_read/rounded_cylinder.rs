//! Independent recognition of closed rounded-cylinder analytic BREP members.
//! This explicit API does not broaden generic analytic/import admission.
use super::*;
use crate::rounded_cylinder::RoundedCylinder;
mod topology;
use topology::FaceUse;

#[derive(Clone, Debug)]
pub struct ImportedRoundedCylinder {
    pub geometry: RoundedCylinder,
    pub origin_mm: Point,
    pub u: Point,
    pub v: Point,
}

pub fn read_rounded_cylinder(
    input: &str,
    tolerance_mm: f64,
) -> Result<(Readback, ImportedRoundedCylinder)> {
    let mut members =
        compound::read_members_by(input, tolerance_mm, true, true, |shell, id, _| {
            read_member(shell, id)
        })?;
    members.pop().ok_or(ReadError::Invalid)
}

/// Every represented member must satisfy this specialized analytic contract.
pub fn read_rounded_cylinder_compound(
    input: &str,
    tolerance_mm: f64,
) -> Result<Vec<(Readback, ImportedRoundedCylinder)>> {
    compound::read_members_by(input, tolerance_mm, true, false, |shell, id, _| {
        read_member(shell, id)
    })
}

fn read_member(
    shell: &mut Shell<'_, '_>,
    solid: u32,
) -> Result<(Readback, ImportedRoundedCylinder)> {
    let ids = shell.member_faces(solid)?;
    if ids.len() > 5 {
        return Err(ReadError::Limit);
    }
    if !(4..=5).contains(&ids.len()) {
        return Err(ReadError::Unsupported);
    }
    if ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err(ReadError::Invalid);
    }
    let mut seen = BTreeSet::new();
    let faces = ids
        .into_iter()
        .map(|id| topology::face(shell, id, &mut seen))
        .collect::<Result<Vec<_>>>()?;
    let cylinders = faces
        .iter()
        .filter(|face| face.kind == "CYLINDRICAL_SURFACE")
        .collect::<Vec<_>>();
    let [wall] = cylinders.as_slice() else {
        return Err(ReadError::Unsupported);
    };
    let [Value::String(_), axis, Value::Number(radius)] =
        shell.args(wall.support, "CYLINDRICAL_SURFACE")?
    else {
        return Err(ReadError::Invalid);
    };
    let radius = *radius;
    if !radius.is_finite() || radius <= shell.tolerance || radius > 1e7 {
        return Err(ReadError::Invalid);
    }
    let frame = shell.placement(reference(axis)?)?;
    let mut rims = BTreeMap::new();
    for face in &faces {
        for usage in &face.wire {
            let (_, _, geometry) = topology::edge(shell, usage.edge)?;
            if shell.simple(geometry)?.name == "CIRCLE" && !rims.contains_key(&usage.edge) {
                rims.insert(usage.edge, topology::rim(shell, usage.edge)?);
            }
        }
    }
    let planes = faces
        .iter()
        .filter(|face| face.kind == "PLANE")
        .collect::<Vec<_>>();
    let [a, b] = planes.as_slice() else {
        return Err(ReadError::Invalid);
    };
    let cap = |face: &&FaceUse| -> Result<(u32, Plane)> {
        let [usage] = face.wire.as_slice() else {
            return Err(ReadError::Invalid);
        };
        let [Value::String(_), axis] = shell.args(face.support, "PLANE")? else {
            return Err(ReadError::Invalid);
        };
        let mut plane = shell.placement(reference(axis)?)?;
        if !face.forward {
            plane.normal = plane.normal.map(|x| -x);
            plane.v = plane.v.map(|x| -x);
        }
        let rim = rims.get(&usage.edge).ok_or(ReadError::Invalid)?;
        let delta = sub(rim.frame.origin, plane.origin);
        if dot(delta, plane.normal).abs() > shell.tolerance {
            return Err(ReadError::Invalid);
        }
        Ok((usage.edge, plane))
    };
    let a = cap(a)?;
    let b = cap(b)?;
    let [start, end] = if dot(a.1.normal, frame.normal) < 0. {
        [a, b]
    } else {
        [b, a]
    };
    let origin = rims[&start.0].frame.origin;
    let height = dot(sub(rims[&end.0].frame.origin, origin), frame.normal);
    let leverage = radius.abs().max(height.abs()).max(1.);
    topology::aligned(shell, start.1.normal, frame.normal.map(|x| -x), leverage)?;
    topology::aligned(shell, end.1.normal, frame.normal, leverage)?;
    if height <= shell.tolerance {
        return Err(ReadError::Invalid);
    }
    let magnitude = origin.into_iter().map(f64::abs).fold(0., f64::max) + radius + height;
    if 32. * f64::EPSILON * magnitude > shell.tolerance {
        return Err(ReadError::Unsupported);
    }
    let point = |z: f64| std::array::from_fn(|i| origin[i] + z * frame.normal[i]);
    topology::near(shell, rims[&end.0].frame.origin, point(height))?;
    topology::near(
        shell,
        cross(sub(frame.origin, origin), frame.normal),
        [0.; 3],
    )?;
    for rim in rims.values() {
        topology::aligned(shell, rim.frame.normal, frame.normal, leverage)?;
        topology::aligned(shell, rim.frame.u, frame.u, leverage)?;
        topology::near(
            shell,
            cross(sub(rim.frame.origin, origin), frame.normal),
            [0.; 3],
        )?;
    }
    let wall_rims = wall
        .wire
        .iter()
        .filter(|u| rims.contains_key(&u.edge))
        .map(|u| u.edge)
        .collect::<Vec<_>>();
    let [wa, wb] = wall_rims.as_slice() else {
        return Err(ReadError::Invalid);
    };
    let level = |edge: u32| dot(sub(rims[&edge].frame.origin, origin), frame.normal);
    let [low, high] = if level(*wa) < level(*wb) {
        [*wa, *wb]
    } else {
        [*wb, *wa]
    };
    for edge in [low, high] {
        if (rims[&edge].radius - radius).abs() > shell.tolerance {
            return Err(ReadError::Invalid);
        }
    }
    let wall_seam = topology::periodic(shell, wall, low, high, None)?;
    let mut fillets = [None; 2];
    let expected_rims = BTreeSet::from([start.0, end.0, low, high]);
    let mut seam_ids = BTreeMap::new();
    seam_ids.insert(wall.id, wall_seam);
    let mut consumed = BTreeSet::new();
    for (i, (cap, wall_rim)) in [(start.0, low), (end.0, high)].into_iter().enumerate() {
        if cap == wall_rim {
            continue;
        }
        let patches = faces
            .iter()
            .filter(|face| {
                face.kind == "TOROIDAL_SURFACE" && face.wire.iter().any(|u| u.edge == cap)
            })
            .collect::<Vec<_>>();
        let [patch] = patches.as_slice() else {
            return Err(ReadError::Invalid);
        };
        if !consumed.insert(patch.id) {
            return Err(ReadError::Invalid);
        }
        let [
            Value::String(_),
            axis,
            Value::Number(major),
            Value::Number(minor),
        ] = shell.args(patch.support, "TOROIDAL_SURFACE")?
        else {
            return Err(ReadError::Invalid);
        };
        let (major, minor) = (*major, *minor);
        let support = shell.placement(reference(axis)?)?;
        let center = if i == 0 { minor } else { height - minor };
        if !major.is_finite()
            || !minor.is_finite()
            || major <= shell.tolerance
            || minor <= shell.tolerance
            || (major - rims[&cap].radius).abs() > shell.tolerance
            || (major + minor - radius).abs() > shell.tolerance
            || (level(wall_rim) - center).abs() > shell.tolerance
        {
            return Err(ReadError::Invalid);
        }
        topology::near(shell, support.origin, point(center))?;
        topology::aligned(shell, support.normal, frame.normal, leverage)?;
        topology::aligned(shell, support.u, frame.u, leverage)?;
        let (lower, upper) = if i == 0 {
            (cap, wall_rim)
        } else {
            (wall_rim, cap)
        };
        let seam = topology::periodic(
            shell,
            patch,
            lower,
            upper,
            Some((&support, major, minor, i == 0)),
        )?;
        seam_ids.insert(patch.id, seam);
        fillets[i] = Some(minor);
    }
    if faces.len() != 3 + consumed.len() || rims.len() != expected_rims.len() {
        return Err(ReadError::Invalid);
    }
    // A cap's loop opposes the axis at the bottom and follows it at the top.
    for (edge, forward) in [(start.0, false), (end.0, true)] {
        let face = faces
            .iter()
            .find(|f| f.kind == "PLANE" && f.wire[0].edge == edge)
            .ok_or(ReadError::Invalid)?;
        if face.wire[0].forward != forward {
            return Err(ReadError::Invalid);
        }
    }
    topology::incidence(&faces, &expected_rims, &seam_ids)?;
    let geometry = RoundedCylinder::new(
        radius,
        height,
        fillets,
        (10. * shell.tolerance).max(1e-6),
        1e7,
    )
    .map_err(|_| ReadError::Invalid)?;
    let mut bounds = [[0.; 3]; 2];
    for axis in 0..3 {
        let direction = [frame.u[axis], frame.v[axis], frame.normal[axis]];
        bounds[0][axis] = origin[axis] - geometry.support(direction.map(|v| -v));
        bounds[1][axis] = origin[axis] + geometry.support(direction);
    }
    let readback = Readback {
        bounds_mm: bounds,
        volume_mm3: geometry.volume_mm3(),
        face_count: faces.len(),
        edges_mm: vec![],
        circles: expected_rims
            .iter()
            .map(|id| CircleEdge {
                center_mm: rims[id].frame.origin,
                normal: frame.normal,
                radius_mm: rims[id].radius,
            })
            .collect(),
        semicircles: vec![],
        arcs: vec![],
        ellipses: vec![],
        splines: vec![],
        solid_count: 1,
    };
    Ok((
        readback,
        ImportedRoundedCylinder {
            geometry,
            origin_mm: origin,
            u: frame.u,
            v: frame.v,
        },
    ))
}

#[cfg(test)]
mod tests;

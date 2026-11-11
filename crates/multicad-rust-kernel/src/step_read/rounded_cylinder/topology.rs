use super::*;
use std::f64::consts::{FRAC_PI_2, TAU};

pub(super) struct FaceUse {
    pub id: u32,
    pub support: u32,
    pub kind: &'static str,
    pub forward: bool,
    pub wire: Vec<WireUse>,
}
pub(super) struct WireUse {
    pub edge: u32,
    pub forward: bool,
}
pub(super) struct Rim {
    pub vertex: u32,
    pub frame: Plane,
    pub radius: f64,
}

pub(super) fn near(shell: &Shell<'_, '_>, a: Point, b: Point) -> Result<()> {
    let distance = length(sub(a, b));
    if !distance.is_finite() || distance > shell.tolerance {
        Err(ReadError::Invalid)
    } else {
        Ok(())
    }
}
pub(super) fn aligned(shell: &Shell<'_, '_>, a: Point, b: Point, leverage: f64) -> Result<()> {
    near(shell, a.map(|x| x * leverage), b.map(|x| x * leverage))
}
pub(super) fn edge(shell: &Shell<'_, '_>, id: u32) -> Result<(u32, u32, u32)> {
    let [Value::String(_), a, b, geometry, Value::Enum("T")] = shell.args(id, "EDGE_CURVE")? else {
        return Err(ReadError::Invalid);
    };
    Ok((reference(a)?, reference(b)?, reference(geometry)?))
}

pub(super) fn face(shell: &Shell<'_, '_>, id: u32, seen: &mut BTreeSet<u32>) -> Result<FaceUse> {
    let [Value::String(_), bounds, support, forward] = shell.args(id, "ADVANCED_FACE")? else {
        return Err(ReadError::Invalid);
    };
    let support = reference(support)?;
    let forward = sense(forward)?;
    let kind = match shell.simple(support)?.name {
        "PLANE" => "PLANE",
        "CYLINDRICAL_SURFACE" => "CYLINDRICAL_SURFACE",
        "TOROIDAL_SURFACE" => "TOROIDAL_SURFACE",
        _ => return Err(ReadError::Unsupported),
    };
    if kind != "PLANE" && !forward {
        return Err(ReadError::Invalid);
    }
    let [bound] = refs(bounds)?.try_into().map_err(|_| ReadError::Invalid)?;
    let (_, wire, bound_forward) = shell.bound(bound, true)?;
    let [Value::String(_), oriented] = shell.args(wire, "EDGE_LOOP")? else {
        return Err(ReadError::Invalid);
    };
    let ids = refs(oriented)?;
    if ids.len() > 4 {
        return Err(ReadError::Limit);
    }
    let mut wire = Vec::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(ReadError::Invalid);
        }
        let [
            Value::String(_),
            Value::Derived,
            Value::Derived,
            edge,
            forward,
        ] = shell.args(id, "ORIENTED_EDGE")?
        else {
            return Err(ReadError::Invalid);
        };
        wire.push(WireUse {
            edge: reference(edge)?,
            forward: sense(forward)?,
        });
    }
    if !bound_forward {
        wire.reverse();
        for usage in &mut wire {
            usage.forward = !usage.forward;
        }
    }
    let mut first = None;
    let mut previous = None;
    for usage in &wire {
        let (a, b, _) = edge(shell, usage.edge)?;
        let (start, end) = if usage.forward { (a, b) } else { (b, a) };
        if previous.is_some_and(|last| last != start) {
            return Err(ReadError::Invalid);
        }
        first.get_or_insert(start);
        previous = Some(end);
    }
    if first.is_none() || first != previous {
        return Err(ReadError::Invalid);
    }
    Ok(FaceUse {
        id,
        support,
        kind,
        forward,
        wire,
    })
}

pub(super) fn rim(shell: &mut Shell<'_, '_>, id: u32) -> Result<Rim> {
    let (start, end, geometry) = edge(shell, id)?;
    if start != end {
        return Err(ReadError::Invalid);
    }
    let [Value::String(_), axis, Value::Number(radius)] = shell.args(geometry, "CIRCLE")? else {
        return Err(ReadError::Invalid);
    };
    let radius = *radius;
    let frame = shell.placement(reference(axis)?)?;
    if !radius.is_finite() || radius <= shell.tolerance || radius > 1e7 {
        return Err(ReadError::Invalid);
    }
    let actual = shell.vertex(start)?;
    near(
        shell,
        actual,
        std::array::from_fn(|i| frame.origin[i] + radius * frame.u[i]),
    )?;
    Ok(Rim {
        vertex: start,
        frame,
        radius,
    })
}

/// Match a cyclic rotation of the outward fundamental rectangle.
pub(super) fn periodic(
    shell: &mut Shell<'_, '_>,
    face: &FaceUse,
    low: u32,
    high: u32,
    toroid: Option<(&Plane, f64, f64, bool)>,
) -> Result<u32> {
    if face.wire.len() != 4 || low == high {
        return Err(ReadError::Invalid);
    }
    let start = face
        .wire
        .iter()
        .position(|u| u.edge == low && u.forward)
        .ok_or(ReadError::Invalid)?;
    let at = |i: usize| &face.wire[(start + i) % 4];
    let seam = at(1).edge;
    if !at(1).forward
        || at(2).edge != high
        || at(2).forward
        || at(3).edge != seam
        || at(3).forward
        || seam == low
        || seam == high
    {
        return Err(ReadError::Invalid);
    }
    let (a, b, curve) = edge(shell, seam)?;
    let low_rim = rim(shell, low)?;
    let high_rim = rim(shell, high)?;
    if a != low_rim.vertex || b != high_rim.vertex || a == b {
        return Err(ReadError::Invalid);
    }
    if let Some((support, major, minor, lower)) = toroid {
        toroid_seam(
            shell,
            curve,
            face.support,
            [a, b],
            (support, major, minor, lower),
        )?;
    } else {
        shell.line_curve(curve, [a, b], true)?;
    }
    Ok(seam)
}

fn toroid_seam(
    shell: &mut Shell<'_, '_>,
    curve: u32,
    surface: u32,
    vertices: [u32; 2],
    patch: (&Plane, f64, f64, bool),
) -> Result<()> {
    let (support, major, minor, lower) = patch;
    let [
        Value::String(_),
        circle,
        pcurves,
        Value::Enum("PCURVE_S1" | "PCURVE_S2"),
    ] = shell.args(curve, "SEAM_CURVE")?
    else {
        return Err(ReadError::Invalid);
    };
    let circle = reference(circle)?;
    let [a, b] = refs(pcurves)?.try_into().map_err(|_| ReadError::Invalid)?;
    if a == b {
        return Err(ReadError::Invalid);
    }
    let decode = |id| -> Result<(Point, Point, f64)> {
        let (owner, line) = shell.pcurve_definition(id)?;
        if owner != surface {
            return Err(ReadError::Invalid);
        }
        shell.line(line, true)
    };
    let mut lines = [decode(a)?, decode(b)?];
    lines.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
    let angular = (shell.tolerance / (major + minor)).min(1e-8);
    for (line, expected) in lines.into_iter().zip([0., TAU]) {
        if length(sub(line.0, [expected, 0., 0.])) > angular
            || length(sub(line.1, [0., 1., 0.])) > angular
            || (line.2 - 1.).abs() > angular
        {
            return Err(ReadError::Invalid);
        }
    }
    let [Value::String(_), axis, Value::Number(radius)] = shell.args(circle, "CIRCLE")? else {
        return Err(ReadError::Invalid);
    };
    if (*radius - minor).abs() > shell.tolerance {
        return Err(ReadError::Invalid);
    }
    let frame = shell.placement(reference(axis)?)?;
    let leverage = (major + minor).max(1.);
    near(
        shell,
        frame.origin,
        std::array::from_fn(|i| support.origin[i] + major * support.u[i]),
    )?;
    aligned(shell, frame.normal, support.v.map(|x| -x), leverage)?;
    aligned(shell, frame.u, support.u, leverage)?;
    let angles = if lower {
        [-FRAC_PI_2, 0.]
    } else {
        [0., FRAC_PI_2]
    };
    for (id, angle) in vertices.into_iter().zip(angles) {
        let expected = std::array::from_fn(|i| {
            frame.origin[i] + minor * (angle.cos() * frame.u[i] + angle.sin() * support.normal[i])
        });
        let actual = shell.vertex(id)?;
        near(shell, actual, expected)?;
    }
    Ok(())
}

pub(super) fn incidence(
    faces: &[FaceUse],
    rims: &BTreeSet<u32>,
    seams: &BTreeMap<u32, u32>,
) -> Result<()> {
    let mut uses = BTreeMap::<u32, Vec<(u32, bool)>>::new();
    for face in faces {
        for usage in &face.wire {
            uses.entry(usage.edge)
                .or_default()
                .push((face.id, usage.forward));
        }
    }
    if uses.len() != rims.len() + seams.len() {
        return Err(ReadError::Invalid);
    }
    for (id, uses) in uses {
        let [a, b] = uses.as_slice() else {
            return Err(ReadError::Invalid);
        };
        if a.1 == b.1
            || if rims.contains(&id) {
                a.0 == b.0
            } else {
                a.0 != b.0 || seams.get(&a.0) != Some(&id)
            }
        {
            return Err(ReadError::Invalid);
        }
    }
    Ok(())
}

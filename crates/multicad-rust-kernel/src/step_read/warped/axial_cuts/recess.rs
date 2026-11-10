use super::*;

pub(super) struct Extension {
    pub(super) mouth: Point,
    pub(super) host: Use,
    pub(super) walls: BTreeSet<usize>,
    pub(super) recess: ConvexRecess,
}

pub(super) fn read(
    reader: &Shell<'_, '_>,
    source: &Boundary,
    incidence: &Incidence,
    end: End,
    axis: Point,
    direction: f64,
) -> Result<Option<Extension>> {
    let shoulder = &source.faces[end.at.face];
    if shoulder.holes.len() != 1 || !(3..=64).contains(&shoulder.loop_.len()) {
        return Ok(None);
    }
    let tol = reader.tolerance;
    let mut ring = Vec::with_capacity(shoulder.loop_.len());
    let mut walls = BTreeSet::new();
    let mut exit: Option<(Use, f64)> = None;
    let height = |point| dot(sub(point, end.center), axis);
    for &(edge, forward) in &shoulder.loop_ {
        let (vertices, curve) = source.curves[&edge];
        if !matches!(curve, TrimCurve::Line) {
            return Ok(None);
        }
        ring.push(reader.points[&vertices[usize::from(!forward)]]);
        let side = incidence.adjacent(edge, end.at.face)?;
        let face = &source.faces[side.face];
        let Surface::Plane { origin, normal } = face.surface else {
            return Ok(None);
        };
        if face.loop_.len() != 4
            || !face.holes.is_empty()
            || side.hole.is_some()
            || dot(normal, axis).abs() > 1e-10
            || dot(normal, sub(end.center, origin)) <= tol
            || !walls.insert(side.face)
        {
            return Ok(None);
        }
        let mut far = None;
        for &(candidate, _) in &face.loop_ {
            let (ids, curve) = source.curves[&candidate];
            if !matches!(curve, TrimCurve::Line) {
                return Ok(None);
            }
            let [a, b] = ids.map(|id| height(reader.points[&id]));
            if (a - b).abs() <= tol && a * direction > tol && far.replace((candidate, a)).is_some()
            {
                return Ok(None);
            }
        }
        let Some((far_edge, travel)) = far else {
            return Ok(None);
        };
        let host = incidence.adjacent(far_edge, side.face)?;
        if host.hole.is_none() || host.face == end.at.face {
            return Ok(None);
        }
        if let Some((old_host, old_travel)) = exit {
            if old_host != host || (old_travel - travel).abs() > tol {
                return Ok(None);
            }
        } else {
            exit = Some((host, travel));
        }
    }
    let (host, travel) = exit.ok_or(ReadError::Invalid)?;
    let mouth = std::array::from_fn(|i| end.center[i] + axis[i] * travel);
    Ok(Some(Extension {
        mouth,
        host,
        walls,
        recess: ConvexRecess {
            wall_source_faces: vec![0; ring.len()],
            rim_mm: ring,
            travel_mm: axis.map(|v| v * (travel + direction * 4. * tol)),
            source: 0,
            cap_source_faces: [1, 2],
        },
    }))
}

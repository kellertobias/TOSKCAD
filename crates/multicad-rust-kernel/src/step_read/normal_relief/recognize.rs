use super::*;
use crate::normal_relief::Kind;
use crate::normal_sweep::Sweep;

fn planar(face: &Face) -> Option<(Point, Point)> {
    if let Face::Planar { origin, normal, .. } = face {
        Some((*origin, *normal))
    } else {
        None
    }
}
fn outer_loops(shell: &Shell<'_, '_>, id: u32, work: &mut Work) -> Result<Vec<Wire>> {
    let mut wires = loops(shell, id, work)?;
    let [Value::String(_), bounds, _, _] = shell.args(id, "ADVANCED_FACE")? else {
        return Err(ReadError::Invalid);
    };
    let bounds = refs(bounds)?;
    let mut outer = None;
    for (i, id) in bounds.into_iter().enumerate() {
        if shell.bound(id, wires.len() == 1)?.0 {
            require(outer.replace(i).is_none())?;
        }
    }
    wires.swap(0, outer.ok_or(ReadError::Unsupported)?);
    Ok(wires)
}
fn profile(
    shell: &Shell<'_, '_>,
    wire: &Wire,
    frame: Frame,
    reverse: bool,
    level: f64,
    epsilon: f64,
    work: &mut Work,
) -> Result<Vec<ProfileEdge>> {
    work.charge(wire.len())?;
    let mut edges = wire
        .iter()
        .map(|&(id, forward)| {
            let edge = &shell.edges[&id];
            let ends = if forward {
                edge.vertices
            } else {
                [edge.vertices[1], edge.vertices[0]]
            };
            let [a, b] = ends.map(|id| frame.local(shell.points[&id]));
            require((a[2] - level).abs() <= epsilon && (b[2] - level).abs() <= epsilon)?;
            let a = [a[0], a[1]];
            let b = [b[0], b[1]];
            match edge.curve {
                Curve::Line => Ok(ProfileEdge::Line { a, b }),
                curve => {
                    let (center, axis, radius, _) =
                        super::proof::circle(curve).ok_or(ReadError::Invalid)?;
                    let center = frame.local(center);
                    require((center[2] - level).abs() <= epsilon)?;
                    let center = [center[0], center[1]];
                    let clockwise = (dot(axis, frame.axes[2]) < 0.) == forward;
                    Ok(if matches!(curve, Curve::Circle(_)) {
                        ProfileEdge::Circle {
                            center,
                            radius_mm: radius,
                            clockwise,
                        }
                    } else {
                        ProfileEdge::Arc {
                            center,
                            a,
                            b,
                            clockwise,
                        }
                    })
                }
            }
        })
        .collect::<Result<Vec<_>>>()?;
    if reverse {
        edges.reverse();
        for edge in &mut edges {
            *edge = edge.reversed();
        }
    }
    Ok(edges)
}
fn adjacent_cap(
    shell: &Shell<'_, '_>,
    faces: &[Face],
    wire: &Wire,
    face_edges: &[Vec<u32>],
    top: usize,
    axis: Point,
    work: &mut Work,
) -> Result<usize> {
    let mut far = None;
    for &(id, _) in wire {
        let walls = shell.edges[&id]
            .uses
            .iter()
            .filter(|e| e.face != top)
            .map(|e| e.face)
            .collect::<Vec<_>>();
        let [wall] = walls[..] else {
            return Err(ReadError::Invalid);
        };
        let mut candidates = BTreeSet::new();
        for id in &face_edges[wall] {
            let edge = &shell.edges[id];
            work.charge(1)?;
            for use_ in &edge.uses {
                if use_.face != top
                    && use_.face != wall
                    && planar(&faces[use_.face])
                        .is_some_and(|(_, n)| dot(n, axis).abs() > 1. - 1e-10)
                {
                    candidates.insert(use_.face);
                }
            }
        }
        let [candidate] = candidates.into_iter().collect::<Vec<_>>()[..] else {
            return Err(ReadError::Invalid);
        };
        require(far.is_none_or(|previous| previous == candidate))?;
        far = Some(candidate);
    }
    far.ok_or(ReadError::Invalid)
}
fn infer(
    shell: &Shell<'_, '_>,
    ids: &[u32],
    faces: &[Face],
    readback: &Readback,
    work: &mut Work,
) -> Result<ImportSolid> {
    let mut planar_faces = Vec::new();
    for (index, face) in faces.iter().enumerate() {
        if planar(face).is_some() {
            let [Value::String(_), bounds, _, _] = shell.args(ids[index], "ADVANCED_FACE")? else {
                return Err(ReadError::Invalid);
            };
            let count = refs(bounds)?.len();
            work.charge(count)?;
            planar_faces.push((count, index));
        }
    }
    planar_faces.sort_unstable();
    let &(count, top) = planar_faces.last().ok_or(ReadError::Unsupported)?;
    require(
        count >= 2
            && planar_faces
                .iter()
                .rev()
                .nth(1)
                .is_none_or(|(n, _)| *n < count),
    )?;
    let (top_origin, axis) = planar(&faces[top]).expect("selected plane");
    let mut bottom = faces
        .iter()
        .enumerate()
        .filter(|(_, f)| planar(f).is_some_and(|(_, n)| dot(n, axis) < -1. + 1e-10));
    let (bottom_index, bottom_face) = bottom.next().ok_or(ReadError::Unsupported)?;
    require(bottom.next().is_none())?;
    let (origin, _) = planar(bottom_face).expect("selected opposite plane");
    let height = dot(sub(top_origin, origin), axis);
    let [Value::String(_), _, surface, _] = shell.args(ids[top], "ADVANCED_FACE")? else {
        return Err(ReadError::Invalid);
    };
    let [Value::String(_), placement] = shell.args(reference(surface)?, "PLANE")? else {
        return Err(ReadError::Invalid);
    };
    let plane = shell.placement(reference(placement)?)?;
    let frame = Frame::new(origin, plane.u, cross(axis, plane.u))?;
    let epsilon = shell.tolerance / 16.;
    let base_rings = outer_loops(shell, ids[bottom_index], work)?
        .iter()
        .map(|wire| profile(shell, wire, frame, true, 0., epsilon, work))
        .collect::<Result<Vec<_>>>()?;
    let mut levels = faces
        .iter()
        .filter_map(planar)
        .filter(|(_, n)| dot(*n, axis).abs() > 1. - 1e-10)
        .map(|(p, _)| frame.local(p)[2])
        .collect::<Vec<_>>();
    levels.sort_unstable_by(f64::total_cmp);
    require(
        levels
            .windows(2)
            .all(|p| p[0] == p[1] || p[1] - p[0] > shell.tolerance),
    )?;
    let minimum = shell.tolerance.next_up();
    let base =
        Sweep::new(base_rings, height, shell.tolerance, minimum).map_err(|_| ReadError::Invalid)?;
    let top_wires = outer_loops(shell, ids[top], work)?;
    let mut face_edges = vec![Vec::new(); faces.len()];
    for (&id, edge) in &shell.edges {
        work.charge(edge.uses.len())?;
        for use_ in &edge.uses {
            face_edges[use_.face].push(id);
        }
    }
    let mut motifs = Vec::new();
    let mut kind = None;
    let mut cap_ids = BTreeSet::new();
    for wire in &top_wires[1..] {
        let far = adjacent_cap(shell, faces, wire, &face_edges, top, axis, work)?;
        if far == bottom_index {
            continue;
        }
        require(cap_ids.insert(far))?;
        let (far_origin, far_normal) = planar(&faces[far]).ok_or(ReadError::Invalid)?;
        require(dot(far_normal, axis) > 0.)?;
        let delta = dot(sub(far_origin, top_origin), axis);
        let next = if delta > 0. {
            Kind::Raised
        } else {
            Kind::Recessed
        };
        require(kind.is_none_or(|old| old == next))?;
        kind = Some(next);
        let ring = profile(shell, wire, frame, true, height, epsilon, work)?;
        motifs.push(
            Sweep::new(vec![ring], delta.abs(), shell.tolerance, minimum)
                .map_err(|_| ReadError::Invalid)?,
        );
    }
    let relief = Relief::new(
        base,
        motifs,
        kind.ok_or(ReadError::Unsupported)?,
        shell.tolerance,
        minimum,
    )
    .map_err(|e| {
        if e == crate::normal_relief::Error::Limit {
            ReadError::Limit
        } else {
            ReadError::Invalid
        }
    })?;
    proof::verify(shell, ids, faces, readback, &relief, frame, work)?;
    Ok(ImportSolid {
        origin_mm: origin,
        u: frame.axes[0],
        v: frame.axes[1],
        height_mm: height,
        profile: ImportProfile::NormalRelief { relief },
    })
}
pub(super) fn recognize(
    shell: &Shell<'_, '_>,
    ids: &[u32],
    faces: &[Face],
    readback: &Readback,
) -> Result<Option<ImportSolid>> {
    match infer(shell, ids, faces, readback, &mut Work(MAX_WORK)) {
        Ok(body) => Ok(Some(body)),
        Err(ReadError::Limit) => Err(ReadError::Limit),
        Err(_) => Ok(None),
    }
}

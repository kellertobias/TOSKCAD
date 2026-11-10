//! Match every parsed STEP identity to one replayed geometric identity.
use super::*;
use crate::warped_union::OrientedEdge;

pub(in crate::step_read::warped) fn replayed(
    reader: &Shell<'_, '_>,
    source: &Boundary,
    chart: Chart,
    shell: &warped_union::Shell,
) -> Result<()> {
    if shell.vertices().len() != reader.points.len()
        || shell.edges().len() != source.curves.len()
        || shell.faces().len() != source.faces.len()
    {
        return Err(ReadError::Invalid);
    }
    point_budget(reader.points.len())?;
    let tol = reader.tolerance;
    let mut vertices = BTreeMap::new();
    let mut claimed = BTreeSet::new();
    for (&id, &world) in &reader.points {
        let point = chart.local(world);
        let mut matches = shell
            .vertices()
            .iter()
            .enumerate()
            .filter(|&(_, &vertex)| length(sub(vertex, point)) <= tol);
        let Some((model, _)) = matches.next() else {
            return Err(ReadError::Invalid);
        };
        if matches.next().is_some() || !claimed.insert(model) {
            return Err(ReadError::Invalid);
        }
        vertices.insert(id, model);
    }
    let key = |[a, b]: [usize; 2]| [a.min(b), a.max(b)];
    let mut by_vertices = BTreeMap::<_, Vec<_>>::new();
    for (index, edge) in shell.edges().iter().enumerate() {
        by_vertices
            .entry(key(edge.vertices))
            .or_default()
            .push(index);
    }
    let mut edges = BTreeMap::new();
    claimed.clear();
    for (&id, &(ends, curve)) in &source.curves {
        let ends = ends.map(|vertex| vertices[&vertex]);
        let mut candidates = by_vertices
            .get(&key(ends))
            .into_iter()
            .flatten()
            .filter_map(|&index| {
                let model = shell.edges()[index];
                let forward = ends == model.vertices;
                curve_matches(curve, true, model.curve, forward, chart, tol).then_some(
                    OrientedEdge {
                        edge: index,
                        forward,
                    },
                )
            });
        let mapped = candidates.next().ok_or(ReadError::Invalid)?;
        if candidates.next().is_some() || !claimed.insert(mapped.edge) {
            return Err(ReadError::Invalid);
        }
        edges.insert(id, mapped);
    }
    let by_use = face_uses(shell.faces())?;
    let map_wire = |wire: &[(u32, bool)]| {
        wire.iter()
            .map(|&(id, forward)| {
                let edge = edges[&id];
                OrientedEdge {
                    forward: forward == edge.forward,
                    ..edge
                }
            })
            .collect::<Vec<_>>()
    };
    claimed.clear();
    for face in &source.faces {
        let outer = map_wire(&face.loop_);
        let &index = by_use
            .get(&(outer[0].edge, outer[0].forward))
            .ok_or(ReadError::Invalid)?;
        let model = &shell.faces()[index];
        if !claimed.insert(index)
            || !support_matches(face.surface, model, chart, tol)
            || !same_loop(&outer, &model.boundary)
            || face.holes.len() != model.holes.len()
            || face.holes.iter().any(|wire| {
                let inner = map_wire(wire);
                !model
                    .holes
                    .iter()
                    .any(|expected| same_loop(&inner, expected))
            })
        {
            return Err(ReadError::Invalid);
        }
    }
    Ok(())
}

fn same_loop(a: &[OrientedEdge], b: &[OrientedEdge]) -> bool {
    a.len() == b.len()
        && b.iter()
            .position(|edge| edge == &a[0])
            .is_some_and(|start| {
                a.iter()
                    .zip(b.iter().cycle().skip(start))
                    .all(|(a, b)| a == b)
            })
}

pub(super) fn point_budget(points: usize) -> Result<()> {
    if points.saturating_mul(points) > 24_000_000 {
        return Err(ReadError::Limit);
    }
    Ok(())
}

pub(super) fn face_uses(faces: &[warped_union::Face]) -> Result<BTreeMap<(usize, bool), usize>> {
    let mut by_use = BTreeMap::new();
    for (index, face) in faces.iter().enumerate() {
        for usage in face.boundary.iter().chain(face.holes.iter().flatten()) {
            if by_use.insert((usage.edge, usage.forward), index).is_some() {
                return Err(ReadError::Invalid);
            }
        }
    }
    Ok(by_use)
}

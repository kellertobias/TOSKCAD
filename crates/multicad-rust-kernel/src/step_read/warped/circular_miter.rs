//! Recover a circular sweep's ordered centerline from shared rim incidence.
use super::{axial_cuts::verify, *};

pub(super) fn reconstruct(
    reader: &Shell<'_, '_>,
    boundary: &Boundary,
) -> Result<Option<ImportedWarped>> {
    let face_count = boundary.faces.len();
    if face_count < 10
        || face_count % 4 != 2
        || boundary.curves.len() != 2 * face_count
        || boundary
            .faces
            .iter()
            .any(|face| face.loop_.len() != 4 || !face.holes.is_empty())
    {
        return Ok(None);
    }

    let leg_count = (face_count - 2) / 4;
    let Some((rims, edge_rim)) = rims(boundary, leg_count + 1) else {
        return Ok(None);
    };
    let Some((chain, cap_vertex)) = centerline(boundary, &rims, &edge_rim) else {
        return Ok(None);
    };

    // Incidence proposes a path. The replay matches every oriented STEP
    // edge and face against the shell derived from that path and phase.
    let start = rims[chain[0]];
    let origin_mm = start.center_mm();
    let u = [1., 0., 0.];
    let v = [0., 1., 0.];
    let chart = Chart {
        origin: origin_mm,
        u,
        v,
        up: [0., 0., 1.],
        band: Band {
            center_z_mm: 0.,
            radius_mm: 1.,
        },
    };
    let path: Vec<_> = chain
        .into_iter()
        .map(|index| chart.local(rims[index].center_mm()))
        .collect();
    let radial = chart.local(reader.points[&cap_vertex]);
    let shell = warped_union::circular_miter_chain_from_radial(
        &path,
        start.semiaxes_mm()[0],
        reader.tolerance,
        radial,
    )
    .map_err(axial_cuts::replay_error)?;
    verify::replayed(reader, boundary, chart, &shell)?;
    Ok(Some(ImportedWarped {
        shell,
        origin_mm,
        u,
        v,
    }))
}

type RimIds = BTreeMap<u32, usize>;

fn rims(boundary: &Boundary, expected: usize) -> Option<(Vec<Ellipse>, RimIds)> {
    let mut incident = BTreeMap::<u32, Vec<u32>>::new();
    let mut arcs = BTreeMap::new();
    for (&id, &(vertices, curve)) in &boundary.curves {
        if let TrimCurve::Arc(arc) = curve {
            arcs.insert(id, (vertices, arc.ellipse()));
            for vertex in vertices {
                incident.entry(vertex).or_default().push(id);
            }
        }
    }
    if arcs.len() != 4 * expected || incident.values().any(|edges| edges.len() != 2) {
        return None;
    }

    let mut remaining: BTreeSet<_> = arcs.keys().copied().collect();
    let mut rims = Vec::with_capacity(expected);
    let mut edge_rim = BTreeMap::new();
    while let Some(&seed) = remaining.first() {
        let (_, ellipse) = arcs[&seed];
        let mut pending = vec![seed];
        let mut size = 0;
        while let Some(edge) = pending.pop() {
            if remaining.remove(&edge) {
                let (vertices, support) = arcs[&edge];
                size += 1;
                if size > 4 || is_circle(support) != is_circle(ellipse) {
                    return None;
                }
                edge_rim.insert(edge, rims.len());
                for vertex in vertices {
                    pending.extend(&incident[&vertex]);
                }
            }
        }
        if size != 4 {
            return None;
        }
        rims.push(ellipse);
    }
    // A straight continuation may contribute a circular interior rim. A
    // completely straight shell belongs to the ordinary axial reader.
    rims.iter()
        .any(|&rim| !is_circle(rim))
        .then_some((rims, edge_rim))
}

fn is_circle(ellipse: Ellipse) -> bool {
    let [major, minor] = ellipse.semiaxes_mm();
    major == minor
}

fn centerline(
    boundary: &Boundary,
    rims: &[Ellipse],
    edge_rim: &RimIds,
) -> Option<(Vec<usize>, u32)> {
    let mut caps = Vec::new();
    let mut legs = BTreeMap::<[usize; 2], usize>::new();
    for face in &boundary.faces {
        let uses: &[(u32, bool); 4] = face
            .loop_
            .as_slice()
            .try_into()
            .expect("checked four-edge miter face");
        let [a, b, c, d] = uses.map(|(id, _)| edge_rim.get(&id).copied());
        match face.surface {
            Surface::Plane { .. } => {
                let rim = a?;
                if [b, c, d] != [a; 3] || !is_circle(rims[rim]) {
                    return None;
                }
                let cap_vertex = boundary.curves[&face.loop_[0].0].0[0];
                caps.push((rim, cap_vertex));
            }
            Surface::Cylinder { .. } => {
                let [first, second] = match [a, b, c, d] {
                    [Some(a), None, Some(c), None] => [a, c],
                    [None, Some(b), None, Some(d)] => [b, d],
                    _ => return None,
                };
                if first == second {
                    return None;
                }
                let key = [first.min(second), first.max(second)];
                *legs.entry(key).or_default() += 1;
            }
        }
    }

    let [(start, cap_vertex), (end, _)] = caps.try_into().ok()?;
    if start == end || legs.len() + 1 != rims.len() || legs.values().any(|&sectors| sectors != 4) {
        return None;
    }
    let mut links = vec![Vec::new(); rims.len()];
    for [a, b] in legs.into_keys() {
        links[a].push(b);
        links[b].push(a);
    }
    for (index, neighbors) in links.iter().enumerate() {
        let cap = index == start || index == end;
        if neighbors.len() != if cap { 1 } else { 2 } {
            return None;
        }
    }
    let chain = walk_rims(&links, start, end);
    (chain.len() == rims.len()).then_some((chain, cap_vertex))
}

fn walk_rims(links: &[Vec<usize>], start: usize, end: usize) -> Vec<usize> {
    // Every rim has degree two except the two caps. The component reached
    // from a cap is a path, with the opposite cap as its only other end.
    let mut chain = vec![start];
    let mut current = start;
    let mut previous = usize::MAX;
    while current != end {
        let next = *links[current]
            .iter()
            .find(|&&rim| rim != previous)
            .expect("a validated rim has its next neighbor");
        chain.push(next);
        (previous, current) = (current, next);
    }
    chain
}

#[cfg(test)]
mod tests;

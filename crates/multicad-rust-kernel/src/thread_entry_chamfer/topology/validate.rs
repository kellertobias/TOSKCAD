//! Cold source incidence proof, including periodic seams and vertex links.
use super::*;
fn connected(
    adjacency: &BTreeMap<usize, Vec<usize>>,
    start: usize,
    work: &mut Work<'_>,
) -> Result<usize, Error> {
    let mut seen = BTreeSet::new();
    let mut pending = vec![start];
    while let Some(id) = pending.pop() {
        work.step()?;
        if seen.insert(id) {
            pending.extend(adjacency.get(&id).ok_or(Error::Unresolved)?);
        }
    }
    Ok(seen.len())
}
pub(super) fn check(body: &Candidate, work: &mut Work<'_>) -> Result<(), Error> {
    if body.faces.is_empty() || body.edges.is_empty() || body.vertices.is_empty() {
        return Err(Error::Unresolved);
    }
    if body.faces.len() != body.source.face_count() as usize {
        return Err(Error::Unresolved);
    }
    let mut uses: Vec<Vec<(usize, bool)>> = vec![Vec::new(); body.edges.len()];
    let mut used_vertices = BTreeSet::new();
    for (face, f) in body.faces.iter().enumerate() {
        work.step()?;
        if f.loops.is_empty() {
            return Err(Error::Unresolved);
        }
        for wire in &f.loops {
            if wire.is_empty() {
                return Err(Error::Unresolved);
            }
            let mut ends = Vec::with_capacity(wire.len());
            for u in wire {
                work.step()?;
                let edge = body.edges.get(u.edge).ok_or(Error::Unresolved)?;
                let mut vertices = edge.vertices;
                for &v in &vertices {
                    if v >= body.vertices.len() {
                        return Err(Error::Unresolved);
                    }
                    used_vertices.insert(v);
                }
                if !u.forward {
                    vertices.reverse();
                }
                ends.push(vertices);
                uses[u.edge].push((face, u.forward));
            }
            if (0..ends.len()).any(|i| ends[i][1] != ends[(i + 1) % ends.len()][0]) {
                return Err(Error::Unresolved);
            }
        }
    }
    if used_vertices.len() != body.vertices.len() {
        return Err(Error::Unresolved);
    }
    let mut adjacency = (0..body.faces.len())
        .map(|i| (i, Vec::new()))
        .collect::<BTreeMap<_, _>>();
    let mut fans: BTreeMap<usize, Vec<[usize; 2]>> = BTreeMap::new();
    for (index, uses) in uses.into_iter().enumerate() {
        work.step()?;
        if uses.len() != 2 || uses[0].1 == uses[1].1 {
            return Err(Error::Unresolved);
        }
        let [a, b] = [uses[0].0, uses[1].0];
        if a == b {
            if !body.edges[index].periodic_seam {
                return Err(Error::Unresolved);
            }
        } else {
            adjacency.get_mut(&a).ok_or(Error::Unresolved)?.push(b);
            adjacency.get_mut(&b).ok_or(Error::Unresolved)?.push(a);
            for v in body.edges[index].vertices {
                fans.entry(v).or_default().push([a, b]);
            }
        }
    }
    if connected(&adjacency, 0, work)? != body.faces.len() {
        return Err(Error::Unresolved);
    }
    for pairs in fans.values() {
        let mut fan: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for &[a, b] in pairs {
            fan.entry(a).or_default().push(b);
            fan.entry(b).or_default().push(a);
        }
        if fan.values().any(|v| v.len() != 2) || connected(&fan, pairs[0][0], work)? != fan.len() {
            return Err(Error::Unresolved);
        }
    }
    for edge in &body.edges {
        coupling::source(&body.source, edge.source, work)?;
    }
    for edge in &body.edges {
        for (end, id) in edge.vertices.into_iter().enumerate() {
            work.step()?;
            let vertex = body.vertices.get(id).ok_or(Error::Unresolved)?;
            let expected = curve::bounds(&body.source, edge.source, [end as f64; 2])?;
            let mut error = I::point(0.);
            for (i, value) in expected.into_iter().enumerate() {
                let stored = vertex.coordinates_mm[i];
                I::new(stored.lower, stored.upper).checked()?;
                if value.lo < stored.lower || value.hi > stored.upper {
                    return Err(Error::Unresolved);
                }
                error = error + I::point((value - I::point(vertex.nominal_mm[i])).magnitude());
            }
            if error.checked()?.hi > vertex.maximum_error_mm
                || !vertex.maximum_error_mm.is_finite()
                || vertex.maximum_error_mm > body.maximum_vertex_error_mm
            {
                return Err(Error::Unresolved);
            }
        }
    }

    for (index, face) in body.faces.iter().enumerate() {
        let loops = if index == 1 && body.source.terminal.is_some() {
            2
        } else {
            1
        };
        if body.source.face(index as u32) != Some(face.role) || face.loops.len() != loops {
            return Err(Error::Unresolved);
        }
        for wire in &face.loops {
            for &usage in wire {
                work.step()?;
                coupling::check(&body.source, face.role, &body.edges[usage.edge], usage)?;
            }
        }
    }
    Ok(())
}

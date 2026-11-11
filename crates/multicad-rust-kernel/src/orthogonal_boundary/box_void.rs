//! Exact rectangular material with strictly enclosed, mutually separated rectangular voids.
use super::*;
use crate::orthogonal::{BooleanError, OrientedFace, OrthogonalSolid, solid_from_oriented_faces};

pub(crate) struct Certificate {
    pub groups: Vec<Vec<usize>>,
    pub origin: Point,
    pub axes: [Point; 3],
    pub solid: OrthogonalSolid,
}

type ProofResult<T> = std::result::Result<T, BooleanError>;

pub(crate) fn boundary_certificate(boundary: &Boundary) -> ProofResult<Certificate> {
    budget(
        boundary.faces.len(),
        boundary
            .faces
            .iter()
            .flat_map(|face| face.rings.iter().map(Vec::len)),
    )?;
    if boundary.shell_count < 2 || boundary.faces.iter().any(|face| face.rings.len() != 1) {
        return Err(BooleanError::Invalid);
    }
    let faces = boundary
        .faces
        .iter()
        .map(|face| OrientedFace {
            normal: face.normal,
            rings: face.rings.clone(),
        })
        .collect::<Vec<_>>();
    let certificate = certify(&faces, 0.)?;
    if certificate.groups.len() != boundary.shell_count {
        return Err(BooleanError::Invalid);
    }
    Ok(certificate)
}

fn dot(a: Point, b: Point) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}
fn cross([a, b, c]: Point, [d, e, f]: Point) -> Point {
    [b * f - c * e, c * d - a * f, a * e - b * d]
}

// Bound the same boundary work domain as the consolidated quad provider.
pub(crate) const MAX_WORK: usize = 4 * MAX_QUADS;

struct Prepared {
    groups: Vec<Vec<usize>>,
    origin: Point,
    axes: [Point; 3],
    local: Vec<OrientedFace>,
    voids: Vec<[Point; 2]>,
}

fn components<'a>(
    faces: impl ExactSizeIterator<Item = (u32, &'a [Vec<Point>])>,
) -> Option<Vec<Vec<usize>>> {
    let count = faces.len();
    if count == 0 {
        return None;
    }
    let mut edges = BTreeMap::<_, (usize, bool, Option<usize>)>::new();
    let mut adjacent = vec![Vec::new(); count];
    for (face, (component, rings)) in faces.enumerate() {
        if rings.is_empty() {
            return None;
        }
        for ring in rings {
            if ring.len() < 4 || ring.iter().flatten().any(|value| !value.is_finite()) {
                return None;
            }
            let mut unique = BTreeSet::new();
            if ring.iter().any(|&point| !unique.insert(point_key(point))) {
                return None;
            }
            for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                let [a, b] = [a, b].map(|point| (component, point_key(point)));
                let forward = a < b;
                let key = if forward { [a, b] } else { [b, a] };
                match edges.entry(key) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert((face, forward, None));
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        let (previous_face, previous, paired) = entry.get_mut();
                        if paired.is_some() || *previous == forward || *previous_face == face {
                            return None;
                        }
                        adjacent[*previous_face].push(face);
                        adjacent[face].push(*previous_face);
                        *paired = Some(face);
                    }
                }
            }
        }
    }
    let mut fans = BTreeMap::<_, BTreeMap<usize, Vec<usize>>>::new();
    for (edge, (a, _, b)) in edges {
        let b = b?;
        for vertex in edge {
            let fan = fans.entry(vertex).or_default();
            fan.entry(a).or_default().push(b);
            fan.entry(b).or_default().push(a);
        }
    }
    for fan in fans.values() {
        if fan.values().any(|neighbors| neighbors.len() != 2) {
            return None;
        }
        let mut visited = BTreeSet::new();
        let mut pending = vec![*fan.keys().next().expect("incident faces")];
        while let Some(face) = pending.pop() {
            if visited.insert(face) {
                pending.extend(&fan[&face]);
            }
        }
        if visited.len() != fan.len() {
            return None;
        }
    }
    let mut visited = vec![false; count];
    let mut groups = Vec::new();
    for first in 0..count {
        if visited[first] {
            continue;
        }
        let mut pending = vec![first];
        let mut group = Vec::new();
        visited[first] = true;
        while let Some(face) = pending.pop() {
            group.push(face);
            for &other in &adjacent[face] {
                if !visited[other] {
                    visited[other] = true;
                    pending.push(other);
                }
            }
        }
        group.sort_unstable();
        groups.push(group);
    }
    Some(groups)
}

pub(crate) fn validate_boundary_graph(boundary: &Boundary) -> ProofResult<()> {
    budget(
        boundary.faces.len(),
        boundary
            .faces
            .iter()
            .flat_map(|face| face.rings.iter().map(Vec::len)),
    )?;
    let groups = components(
        boundary
            .faces
            .iter()
            .map(|face| (face.component, face.rings.as_slice())),
    )
    .ok_or(BooleanError::Invalid)?;
    if groups.len() != boundary.shell_count {
        return Err(BooleanError::Invalid);
    }
    Ok(())
}

pub(crate) fn recognize(faces: &[OrientedFace], tolerance: f64) -> Option<Certificate> {
    certify(faces, tolerance).ok()
}

fn budget(faces: usize, mut counts: impl Iterator<Item = usize>) -> ProofResult<usize> {
    if faces > MAX_QUADS {
        return Err(BooleanError::Limit);
    }
    counts
        .try_fold(0usize, |count, next| {
            count.checked_add(next).filter(|&count| count <= MAX_WORK)
        })
        .ok_or(BooleanError::Limit)
}

pub(crate) fn certify(faces: &[OrientedFace], tolerance: f64) -> ProofResult<Certificate> {
    let vertices = budget(
        faces.len(),
        faces
            .iter()
            .flat_map(|face| face.rings.iter().map(Vec::len)),
    )?;
    let prepared = prepare(faces, tolerance).ok_or(BooleanError::Invalid)?;
    let count = prepared.voids.len();
    let pairs = count
        .checked_mul(count.saturating_sub(1))
        .and_then(|count| count.checked_div(2))
        .ok_or(BooleanError::Limit)?;
    if pairs > MAX_WORK - vertices {
        return Err(BooleanError::Limit);
    }
    for (i, a) in prepared.voids.iter().enumerate() {
        for b in &prepared.voids[..i] {
            if !(0..3).any(|axis| {
                b[0][axis] - a[1][axis] > tolerance || a[0][axis] - b[1][axis] > tolerance
            }) {
                return Err(BooleanError::Invalid);
            }
        }
    }
    let solid = solid_from_oriented_faces(&prepared.local, tolerance)?;
    Ok(Certificate {
        groups: prepared.groups,
        origin: prepared.origin,
        axes: prepared.axes,
        solid,
    })
}

fn prepare(faces: &[OrientedFace], tolerance: f64) -> Option<Prepared> {
    let first = faces.first()?;
    let mut origin = *first.rings.first()?.first()?;
    let u = first.normal;
    let v = faces
        .iter()
        .map(|face| face.normal)
        .find(|&normal| dot(u, normal).abs() < 1e-10)?;
    let axes = [u, v, cross(u, v)];
    if axes
        .iter()
        .any(|&axis| (dot(axis, axis) - 1.).abs() > 1e-10)
    {
        return None;
    }
    let cardinal = axes.map(|axis| {
        axis.into_iter().filter(|&value| value != 0.).count() == 1
            && axis.into_iter().any(|value| value.abs() == 1.)
    });
    // Exact cardinal coordinates must not lose a real gap by subtracting a
    // distant anchor or by merging independent nearby support planes.
    for (i, axis) in axes.iter().enumerate() {
        if cardinal[i] {
            let coordinate = axis.iter().position(|&value| value != 0.)?;
            origin[coordinate] = 0.;
        }
    }
    let mut local = Vec::with_capacity(faces.len());
    for face in faces {
        if face.rings.len() != 1 || face.rings[0].len() < 4 {
            return None;
        }
        let components = axes.map(|axis| dot(face.normal, axis));
        let axis = components.iter().position(|value| value.abs() > 0.5)?;
        if components.iter().enumerate().any(|(i, &value)| {
            (value
                - if i == axis {
                    components[axis].signum()
                } else {
                    0.
                })
            .abs()
                > 1e-10
        }) {
            return None;
        }
        let normal = std::array::from_fn(|i| {
            if i == axis {
                components[axis].signum()
            } else {
                0.
            }
        });
        let rings = face
            .rings
            .iter()
            .map(|ring| {
                ring.iter()
                    .map(|&point| axes.map(|axis| dot(sub(point, origin), axis)))
                    .collect()
            })
            .collect();
        local.push(OrientedFace { normal, rings });
    }
    let magnitude = faces
        .iter()
        .flat_map(|face| face.rings.iter().flatten())
        .flatten()
        .copied()
        .map(f64::abs)
        .fold(1., f64::max);
    let roundoff = 128. * f64::EPSILON * magnitude;
    let roundoff = if tolerance > 0. {
        roundoff.min(tolerance / 16.)
    } else {
        roundoff
    };
    // Restore common plane coordinates after a rigid chart round trip. Never
    // merge a chain wider than the roundoff budget.
    for axis in 0..3 {
        let budget = if cardinal[axis] { 0. } else { roundoff };
        let mut coordinates = local
            .iter()
            .flat_map(|face| face.rings.iter().flatten())
            .map(|point| point[axis])
            .collect::<Vec<_>>();
        if coordinates.iter().any(|value| !value.is_finite()) {
            return None;
        }
        coordinates.sort_by(f64::total_cmp);
        coordinates.dedup();
        let mut representatives = BTreeMap::new();
        let mut start = 0;
        while start < coordinates.len() {
            let mut end = start + 1;
            while end < coordinates.len() && coordinates[end] - coordinates[start] <= budget {
                end += 1;
            }
            let value = coordinates[start].midpoint(coordinates[end - 1]);
            for &coordinate in &coordinates[start..end] {
                representatives.insert(
                    if coordinate == 0. {
                        0
                    } else {
                        coordinate.to_bits()
                    },
                    value,
                );
            }
            start = end;
        }
        for point in local
            .iter_mut()
            .flat_map(|face| face.rings.iter_mut().flatten())
        {
            point[axis] = representatives[&if point[axis] == 0. {
                0
            } else {
                point[axis].to_bits()
            }];
        }
    }
    let extent = |indices: &[usize]| {
        let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
        for &i in indices {
            for point in local[i].rings.iter().flatten() {
                for axis in 0..3 {
                    bounds[0][axis] = bounds[0][axis].min(point[axis]);
                    bounds[1][axis] = bounds[1][axis].max(point[axis]);
                }
            }
        }
        bounds
    };
    let outer = extent(&(0..local.len()).collect::<Vec<_>>());
    let mut groups = components(faces.iter().map(|face| (0, face.rings.as_slice())))?;
    if groups.len() < 2 {
        return None;
    }
    let supported = |indices: &[usize], bounds: [Point; 2], inward: bool| {
        indices.iter().all(|&i| {
            let face = &local[i];
            let axis = face
                .normal
                .iter()
                .position(|&n| n != 0.)
                .expect("normalized support");
            let side = usize::from((face.normal[axis] > 0.) != inward);
            face.rings[0]
                .iter()
                .all(|point| point[axis] == bounds[side][axis])
        })
    };
    let exterior = groups
        .iter()
        .position(|indices| supported(indices, outer, false))?;
    let exterior = groups.remove(exterior);
    groups.insert(0, exterior);
    let mut voids = Vec::new();
    for indices in &groups[1..] {
        let inner = extent(indices);
        if (0..3).any(|axis| {
            inner[0][axis] - outer[0][axis] <= tolerance
                || outer[1][axis] - inner[1][axis] <= tolerance
                || inner[1][axis] - inner[0][axis] <= tolerance
        }) || !supported(indices, inner, true)
        {
            return None;
        }
        voids.push(inner);
    }
    Some(Prepared {
        groups,
        origin,
        axes,
        local,
        voids,
    })
}

#[cfg(test)]
mod tests;

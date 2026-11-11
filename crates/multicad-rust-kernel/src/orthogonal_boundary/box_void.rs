//! Exact rectangular material with one strictly enclosed rectangular void.
use super::*;
use crate::orthogonal::{OrientedFace, OrthogonalSolid, solid_from_oriented_faces};

pub(crate) struct Certificate {
    pub groups: [Vec<usize>; 2],
    pub origin: Point,
    pub axes: [Point; 3],
    pub solid: OrthogonalSolid,
}

pub(crate) fn boundary(boundary: &Boundary) -> Option<Certificate> {
    if boundary.shell_count != 2 || boundary.faces.iter().any(|face| face.rings.len() != 1) {
        return None;
    }
    let faces = boundary
        .faces
        .iter()
        .map(|face| OrientedFace {
            normal: face.normal,
            rings: face.rings.clone(),
        })
        .collect::<Vec<_>>();
    recognize(&faces, 0.)
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

fn closed(faces: &[OrientedFace]) -> bool {
    let mut edges = BTreeMap::<_, (bool, u8)>::new();
    for ring in faces.iter().flat_map(|face| &face.rings) {
        for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            let [a, b] = [a, b].map(point_key);
            if a == b {
                return false;
            }
            let forward = a < b;
            let key = if forward { [a, b] } else { [b, a] };
            match edges.entry(key) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert((forward, 1));
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let (previous, count) = entry.get_mut();
                    if *count != 1 || *previous == forward {
                        return false;
                    }
                    *count = 2;
                }
            }
        }
    }
    edges.values().all(|&(_, count)| count == 2)
}

pub(crate) fn recognize(faces: &[OrientedFace], tolerance: f64) -> Option<Certificate> {
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
    let mut groups = [Vec::new(), Vec::new()];
    for (i, face) in local.iter().enumerate() {
        let axis = face.normal.iter().position(|&n| n != 0.)?;
        let exterior = usize::from(face.normal[axis] > 0.);
        let coordinate = face.rings[0][0][axis];
        groups[usize::from(coordinate != outer[exterior][axis])].push(i);
    }
    if groups.iter().any(Vec::is_empty) {
        return None;
    }
    let inner = extent(&groups[1]);
    if (0..3).any(|axis| {
        inner[0][axis] - outer[0][axis] <= tolerance
            || outer[1][axis] - inner[1][axis] <= tolerance
            || inner[1][axis] - inner[0][axis] <= tolerance
    }) {
        return None;
    }
    for (group, indices) in groups.iter().enumerate() {
        for &i in indices {
            let face = &local[i];
            let axis = face.normal.iter().position(|&n| n != 0.)?;
            let side = usize::from((face.normal[axis] > 0.) != (group == 1));
            let bounds = if group == 0 { outer } else { inner };
            if face.rings[0]
                .iter()
                .any(|point| point[axis] != bounds[side][axis])
            {
                return None;
            }
        }
    }
    // The material reconstruction permits collinear face subdivisions. STEP
    // also requires the original exact edge pairs to close without T junctions.
    if !closed(faces) {
        return None;
    }
    let solid = solid_from_oriented_faces(&local, tolerance).ok()?;
    Some(Certificate {
        groups,
        origin,
        axes,
        solid,
    })
}

#[cfg(test)]
mod tests;

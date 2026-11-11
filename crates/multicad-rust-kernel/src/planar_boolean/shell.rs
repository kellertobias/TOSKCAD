use super::*;
use crate::display::triangulate_region;
use spade::Point2;

pub(super) struct Boundary {
    pub(super) vertices: Vec<Point>,
    pub(super) faces: Vec<Face>,
    tolerance: f64,
    remaining_incidences: usize,
    budget: usize,
}

impl Boundary {
    pub(super) fn new(tolerance: f64) -> Self {
        Self {
            vertices: Vec::new(),
            faces: Vec::new(),
            tolerance,
            remaining_incidences: MAX_INCIDENCES,
            budget: MAX_WORK,
        }
    }

    pub(super) fn add(
        &mut self,
        polygon: &[Point],
        normal: Point,
        source: usize,
        source_face: usize,
    ) -> Result<(), Error> {
        charge(&mut self.remaining_incidences, polygon.len())?;
        let mut vertices = Vec::with_capacity(polygon.len());
        for &point in polygon {
            charge(&mut self.budget, self.vertices.len())?;
            let mut matches = self.vertices.iter().enumerate().filter_map(|(id, &other)| {
                let distance = norm(sub(point, other));
                (distance <= self.tolerance).then_some((id, distance))
            });
            let vertex = if let Some((id, distance)) = matches.next() {
                let magnitude = point.into_iter().map(f64::abs).fold(1., f64::max);
                let numeric_tolerance = (8. * f64::EPSILON * magnitude).max(self.tolerance * 1e-3);
                if distance > numeric_tolerance || matches.next().is_some() {
                    return Err(Error::Unresolved);
                }
                id
            } else {
                self.vertices.push(point);
                self.vertices.len() - 1
            };
            if vertices.contains(&vertex) {
                return Err(Error::Unresolved);
            }
            vertices.push(vertex);
        }
        self.faces.push(Face {
            vertices,
            normal,
            source,
            source_face,
        });
        Ok(())
    }

    /// Clipping one face may introduce vertices into an adjacent unsplit edge.
    /// Install each of these once in both loops before testing shell incidence.
    pub(super) fn split_edges(&mut self, budget: &mut usize) -> Result<(), Error> {
        let incidences = MAX_INCIDENCES - self.remaining_incidences;
        charge(budget, incidences.saturating_mul(self.vertices.len()))?;
        let index = (self.vertices.len() >= 32).then(|| {
            std::array::from_fn::<_, 3, _>(|axis| {
                let mut ids: Vec<_> = (0..self.vertices.len()).collect();
                ids.sort_unstable_by(|&a, &b| {
                    self.vertices[a][axis].total_cmp(&self.vertices[b][axis])
                });
                ids
            })
        });
        let magnitude = self
            .vertices
            .iter()
            .flatten()
            .map(|value| value.abs())
            .fold(1., f64::max);
        // Any accepted point projects between the endpoints and lies within
        // the edge tolerance. Leave room for rounding in both computations.
        let padding = self.tolerance + 8. * f64::EPSILON * magnitude;
        let mut remaining_incidences = MAX_INCIDENCES;
        for face in &mut self.faces {
            let mut ring = Vec::new();
            for (&a, &b) in face
                .vertices
                .iter()
                .zip(face.vertices.iter().cycle().skip(1))
            {
                let start = self.vertices[a];
                let direction = sub(self.vertices[b], start);
                let length_squared = dot(direction, direction);
                let margin = self.tolerance / length_squared.sqrt();
                let mut splits = Vec::new();
                let mut test = |vertex| {
                    let relative = sub(self.vertices[vertex], start);
                    let fraction = dot(relative, direction) / length_squared;
                    if fraction > margin
                        && fraction < 1. - margin
                        && norm(sub(relative, scale(direction, fraction))) <= self.tolerance
                    {
                        splits.push((fraction, vertex));
                    }
                };
                if let Some(index) = &index {
                    let end = self.vertices[b];
                    let candidates = index
                        .iter()
                        .enumerate()
                        .map(|(axis, ids)| {
                            let low = start[axis].min(end[axis]) - padding;
                            let high = start[axis].max(end[axis]) + padding;
                            let first = ids.partition_point(|&i| self.vertices[i][axis] < low);
                            let last = ids.partition_point(|&i| self.vertices[i][axis] <= high);
                            &ids[first..last]
                        })
                        .min_by_key(|ids| ids.len())
                        .expect("three coordinate ranges");
                    for &vertex in candidates {
                        test(vertex);
                    }
                    // Preserve the full scan's input order for equal fractions.
                    splits.sort_unstable_by_key(|&(_, vertex)| vertex);
                } else {
                    for vertex in 0..self.vertices.len() {
                        test(vertex);
                    }
                }
                splits.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
                ring.push(a);
                ring.extend(splits.into_iter().map(|(_, vertex)| vertex));
            }
            charge(&mut remaining_incidences, ring.len())?;
            face.vertices = ring;
        }
        Ok(())
    }

    pub(super) fn prove(&self) -> Result<Proof, Error> {
        prove(&self.vertices, &self.faces)
    }
}

pub(super) struct Proof {
    pub(super) volume_mm3: f64,
    pub(super) solid_count: usize,
    pub(super) shells: Vec<Shell>,
}

pub(super) fn prove(vertices: &[Point], faces: &[Face]) -> Result<Proof, Error> {
    let mesh = mesh(vertices, faces, MAX_INCIDENCES).map_err(|error| match error {
        MeshError::BudgetExceeded => Error::Limit,
        _ => Error::Unresolved,
    })?;
    let proof = manufacturing::certify(&mesh).map_err(|_| Error::Unresolved)?;

    // Count outward shells only. A disconnected inward shell bounds a cavity
    // and contributes negative volume but does not create an additional part.
    let mut owner = BTreeMap::new();
    let mut adjacent = vec![Vec::new(); faces.len()];
    for (index, face) in faces.iter().enumerate() {
        for (&a, &b) in face
            .vertices
            .iter()
            .zip(face.vertices.iter().cycle().skip(1))
        {
            if let Some(other) = owner.insert([a.min(b), a.max(b)], index) {
                adjacent[index].push(other);
                adjacent[other].push(index);
            }
        }
    }
    let mut shell_ids = vec![usize::MAX; faces.len()];
    let mut solid_count = 0;
    let mut shells = Vec::new();
    for (index, face) in faces.iter().enumerate() {
        if shell_ids[index] != usize::MAX {
            continue;
        }
        let shell_id = shells.len();
        let origin = vertices[face.vertices[0]];
        let mut pending = vec![index];
        let mut six_volume = 0.;
        shell_ids[index] = shell_id;
        while let Some(current) = pending.pop() {
            let ring = &faces[current].vertices;
            let a = sub(vertices[ring[0]], origin);
            for edge in ring[1..].windows(2) {
                let b = sub(vertices[edge[0]], origin);
                let c = sub(vertices[edge[1]], origin);
                six_volume += dot(a, cross(b, c));
            }
            for &next in &adjacent[current] {
                if shell_ids[next] == usize::MAX {
                    shell_ids[next] = shell_id;
                    pending.push(next);
                }
            }
        }
        if six_volume > 0. {
            solid_count += 1;
        }
        shells.push(Shell {
            faces: Vec::new(),
            signed_volume_mm3: six_volume / 6.,
        });
    }
    for (face, shell_id) in shell_ids.into_iter().enumerate() {
        shells[shell_id].faces.push(face);
    }
    Ok(Proof {
        volume_mm3: proof.signed_volume_mm3,
        solid_count,
        shells,
    })
}

pub(super) fn mesh(
    vertices: &[Point],
    faces: &[Face],
    triangle_limit: usize,
) -> Result<DisplayMesh, MeshError> {
    let concave: Vec<_> = faces
        .iter()
        .map(|face| is_concave(vertices, face))
        .collect();
    let triangles = faces
        .iter()
        .zip(&concave)
        .map(|(face, &concave)| face.vertices.len() - 2 * usize::from(concave))
        .sum::<usize>();
    if triangles > triangle_limit {
        return Err(MeshError::BudgetExceeded);
    }
    let mut mesh = DisplayMesh {
        vertices: vertices.iter().copied().flatten().collect(),
        indices: Vec::with_capacity(triangles * 3),
        face_indices: Vec::with_capacity(triangles),
    };
    for (ordinal, (face, concave)) in faces.iter().zip(concave).enumerate() {
        if concave {
            for triangle in triangulate_face(vertices, face)? {
                mesh.triangle(triangle, ordinal as u32);
            }
            continue;
        }
        let origin = vertices[face.vertices[0]];
        let center = add(
            origin,
            scale(
                face.vertices
                    .iter()
                    .fold([0.; 3], |sum, &i| add(sum, sub(vertices[i], origin))),
                1. / face.vertices.len() as f64,
            ),
        );
        let center_index = mesh.vertices.len() / 3;
        mesh.vertices.extend(center);
        for (&a, &b) in face
            .vertices
            .iter()
            .zip(face.vertices.iter().cycle().skip(1))
        {
            mesh.triangle([a, b, center_index], ordinal as u32);
        }
    }
    Ok(mesh)
}

pub(super) fn is_concave(vertices: &[Point], face: &Face) -> bool {
    let points = || face.vertices.iter().map(|&id| vertices[id]);
    points()
        .zip(points().cycle().skip(1))
        .zip(points().cycle().skip(2))
        .any(|((a, b), c)| {
            let first = sub(b, a);
            let second = sub(c, b);
            let turn = dot(cross(first, second), face.normal);
            turn < 0. && -turn > 64. * f64::EPSILON * norm(first) * norm(second)
        })
}

/// Triangulate the physical boundary without inserting boundary samples.
/// Return indices in the original shell chart so adjacent faces share edges.
pub(super) fn triangulate_face(
    vertices: &[Point],
    face: &Face,
) -> Result<Vec<[usize; 3]>, MeshError> {
    let axis = (0..3)
        .max_by(|&a, &b| face.normal[a].abs().total_cmp(&face.normal[b].abs()))
        .expect("three coordinates");
    // Drop the dominant axis, preserving orientation with respect to the
    // outward normal. Projection is one-to-one in this planar chart.
    let mut chart = [(axis + 1) % 3, (axis + 2) % 3];
    if face.normal[axis] < 0. {
        chart.swap(0, 1);
    }
    let [u, v] = chart;
    let origin = vertices[face.vertices[0]];
    let points = face
        .vertices
        .iter()
        .map(|&index| {
            let point = sub(vertices[index], origin);
            Point2::new(point[u], point[v])
        })
        .collect();
    let count = face.vertices.len();
    let edges: Vec<_> = (0..count).map(|i| [i, (i + 1) % count]).collect();
    triangulate_region(points, &edges, 0).map(|region| {
        region
            .triangles
            .into_iter()
            .map(|triangle| triangle.map(|index| face.vertices[index]))
            .collect()
    })
}

#[cfg(test)]
mod tests;

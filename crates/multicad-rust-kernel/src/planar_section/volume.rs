//! Clip a polyhedral drawing body to the material retained behind a section.
use super::*;
use crate::planar_projection::Face;

type Point3 = [f64; 3];
type Edge3 = [Point3; 2];
type EdgeKey = [[u64; 3]; 2];

pub struct Retained {
    /// Same ordering as the source bodies. Empty bodies were clipped away.
    pub bodies: Vec<Body>,
    pub trimmed: Vec<bool>,
}

pub fn retain(
    bodies: Vec<Body>,
    options: Options,
    far: Option<f64>,
) -> Result<Retained, ProjectionError> {
    validate_options(options)?;
    if far.is_some_and(|far| !far.is_finite() || options.depth - far <= options.tolerance) {
        return Err(ProjectionError::Invalid);
    }
    let mut work = MAX_INCIDENCES;
    let mut output = Retained {
        bodies: Vec::with_capacity(bodies.len()),
        trimmed: Vec::with_capacity(bodies.len()),
    };
    let mut planes = vec![Plane {
        axis: 2,
        offset: options.depth,
        sign: -1.,
    }];
    if let Some(offset) = far {
        planes.push(Plane {
            axis: 2,
            offset,
            sign: 1.,
        });
    }
    for (axis, window) in options.windows.into_iter().enumerate() {
        if let Some([min, max]) = window {
            planes.extend([
                Plane {
                    axis,
                    offset: min,
                    sign: 1.,
                },
                Plane {
                    axis,
                    offset: max,
                    sign: -1.,
                },
            ]);
        }
    }
    for mut body in bodies {
        let mut trimmed = false;
        for &plane in &planes {
            let (clipped, changed) = plane.clip(body, options.tolerance, &mut work)?;
            body = clipped;
            trimmed |= changed;
            if body.faces.is_empty() {
                break;
            }
        }
        output.bodies.push(body);
        output.trimmed.push(trimmed);
    }
    Ok(output)
}

#[derive(Clone, Copy)]
struct Plane {
    axis: usize,
    offset: f64,
    sign: f64,
}

impl Plane {
    fn clip(
        self,
        body: Body,
        tolerance: f64,
        work: &mut usize,
    ) -> Result<(Body, bool), ProjectionError> {
        charge(work, body.edges.len())?;
        let distance =
            |p: Point3| signed_distance(p[self.axis], self.offset, tolerance) * self.sign;
        let mut has_inside = false;
        let mut has_outside = false;
        for &vertex in body.edges.iter().flatten() {
            if vertex.iter().any(|v| !v.is_finite() || v.abs() > 1e8) {
                return Err(ProjectionError::Invalid);
            }
            let d = distance(vertex);
            has_inside |= d > 0.;
            has_outside |= d < 0.;
        }
        if !has_outside {
            return Ok((body, false));
        }
        if !has_inside {
            return Ok((
                Body {
                    edges: vec![],
                    faces: vec![],
                },
                true,
            ));
        }
        let edges: Vec<_> = body
            .edges
            .into_iter()
            .map(|edge| self.edge(edge, distance))
            .collect();
        let mut result = Boundary::default();
        for face in body.faces {
            charge(work, face.edges.len())?;
            let edges = face
                .edges
                .iter()
                .map(|&i| edges.get(i).ok_or(ProjectionError::Invalid))
                .collect::<Result<Vec<_>, _>>()?;
            let kept = self.face(&face, &edges)?;
            if kept.len() >= 3 {
                result.push(face, kept)?;
            }
        }
        let mut cap = Vec::new();
        for (index, (&count, edge)) in result.incidence.iter().zip(&result.edges).enumerate() {
            if count == 1 {
                if edge.iter().any(|p| p[self.axis] != self.offset) {
                    return Err(ProjectionError::Invalid);
                }
                // Each retained face already has closed boundaries. Edges
                // with one use form the cap boundary by parity.
                cap.push(index);
            }
        }
        if !cap.is_empty() {
            let mut point = [0.; 3];
            let mut normal = [0.; 3];
            point[self.axis] = self.offset;
            normal[self.axis] = -self.sign;
            result.faces.push(Face {
                point,
                normal,
                edges: cap,
            });
        }
        Ok((
            Body {
                edges: result.edges,
                faces: result.faces,
            },
            true,
        ))
    }

    fn edge(self, [mut a, mut b]: Edge3, distance: impl Fn(Point3) -> f64) -> Edge {
        let da = distance(a);
        let db = distance(b);
        if da == 0. {
            a[self.axis] = self.offset;
        }
        if db == 0. {
            b[self.axis] = self.offset;
        }
        if crosses(da, db) {
            let mut hit = intersection(a, b, da, db);
            hit[self.axis] = self.offset;
            Edge::Crossing {
                kept: if da > 0. { [a, hit] } else { [hit, b] },
                hit,
            }
        } else if da > 0. {
            Edge::Inside([a, b])
        } else if da == 0. && db == 0. {
            Edge::Coplanar([a, b])
        } else {
            Edge::Outside
        }
    }

    fn face(self, face: &Face, edges: &[&Edge]) -> Result<Vec<Edge3>, ProjectionError> {
        if edges.iter().all(|e| matches!(e, Edge::Coplanar(_))) {
            return Ok(edges
                .iter()
                .filter_map(|e| match e {
                    Edge::Coplanar(ends) if self.sign * face.normal[self.axis] < 0. => Some(*ends),
                    _ => None,
                })
                .collect());
        }
        let mut kept = Vec::new();
        let mut cut = Vec::new();
        for edge in edges {
            match edge {
                Edge::Crossing { kept: ends, hit } => {
                    kept.push(*ends);
                    cut.push(*hit);
                }
                Edge::Inside(ends) => kept.push(*ends),
                Edge::Outside | Edge::Coplanar(_) => {}
            }
        }
        let u = (self.axis + 1) % 3;
        let v = (self.axis + 2) % 3;
        let along = if face.normal[u].abs() > face.normal[v].abs() {
            v
        } else {
            u
        };
        kept.extend(pairs(cut, along)?);
        Ok(kept)
    }
}

enum Edge {
    Crossing { kept: Edge3, hit: Point3 },
    Inside(Edge3),
    Coplanar(Edge3),
    Outside,
}

#[derive(Default)]
struct Boundary {
    edges: Vec<Edge3>,
    faces: Vec<Face>,
    indices: BTreeMap<EdgeKey, usize>,
    incidence: Vec<u8>,
}

impl Boundary {
    fn push(&mut self, mut face: Face, edges: Vec<Edge3>) -> Result<(), ProjectionError> {
        let mut unique = BTreeSet::new();
        for edge in edges {
            let mut key = edge.map(|point| point.map(|v| if v == 0. { 0 } else { v.to_bits() }));
            key.sort_unstable();
            if key[0] != key[1] {
                unique.insert(key);
            }
        }
        if unique.len() < 3 {
            return Ok(());
        }
        let mut vertices = BTreeMap::new();
        for key in &unique {
            for &point in key {
                *vertices.entry(point).or_insert(0usize) += 1;
            }
        }
        if vertices.values().any(|count| count % 2 != 0) {
            return Err(ProjectionError::Invalid);
        }
        face.edges.clear();
        for key in unique {
            let index = *self.indices.entry(key).or_insert_with(|| {
                let index = self.edges.len();
                self.edges.push(key.map(|p| p.map(f64::from_bits)));
                self.incidence.push(0);
                index
            });
            let count = &mut self.incidence[index];
            if *count == 2 {
                return Err(ProjectionError::Invalid);
            }
            *count += 1;
            face.edges.push(index);
        }
        self.faces.push(face);
        Ok(())
    }
}

#[cfg(test)]
mod tests;

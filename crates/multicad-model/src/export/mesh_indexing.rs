//! Exact-coordinate indexing for stored 3MF objects only. No vertex moves,
//! tolerance welding, native geometry changes or normal interpolation.
use super::mesh::MeshObject;
use std::collections::{HashMap, HashSet};

const MAX_TRIANGLES: usize = 2_000_000;
const MAX_VERTICES: usize = MAX_TRIANGLES * 3;

struct Corners(Vec<u32>);
impl Corners {
    fn root(&mut self, corner: u32) -> u32 {
        let mut current = corner;
        while self.0[current as usize] != current {
            current = self.0[current as usize];
        }
        let mut next = corner;
        while self.0[next as usize] != next {
            let parent = self.0[next as usize];
            self.0[next as usize] = current;
            next = parent;
        }
        current
    }
    fn join(&mut self, a: u32, b: u32) {
        let a = self.root(a);
        let b = self.root(b);
        self.0[a.max(b) as usize] = a.min(b);
    }
}
#[derive(Clone, Copy)]
struct Edge {
    corners: [u32; 2],
    ascending: bool,
    paired: bool,
}

// An already closed index graph is affirmative component ownership evidence.
// Keep it rather than forcing coincident closed shells into shared edges.
fn originally_closed(object: &MeshObject) -> bool {
    let mut edges = HashMap::new();
    for triangle in &object.triangles {
        for (i, j) in [(0, 1), (1, 2), (2, 0)] {
            let (a, b) = (triangle[i], triangle[j]);
            if a == b {
                return false;
            }
            let key = (a.min(b), a.max(b));
            let value = edges.entry(key).or_insert((0_u8, a < b));
            if value.0 >= 2 || (value.0 == 1 && value.1 == (a < b)) {
                return false;
            }
            value.0 += 1;
        }
    }
    edges.values().all(|value| value.0 == 2)
}

pub(super) fn closed_indexed(object: &MeshObject) -> Result<MeshObject, String> {
    let fail = |reason: &str| format!("3MF object {:?}: {reason}", object.name);
    if object.triangles.is_empty()
        || object.triangles.len() > MAX_TRIANGLES
        || object.vertices.len() > MAX_VERTICES
    {
        return Err(fail(
            "mesh exceeds the bounded nonempty 2,000,000-triangle / 6,000,000-vertex domain",
        ));
    }
    let preserve_indices = originally_closed(object);
    let mut exact = HashMap::new();
    let mut vertices = Vec::new();
    let mut remap = Vec::with_capacity(object.vertices.len());
    for point in &object.vertices {
        if point.iter().any(|c| !c.is_finite()) {
            return Err(fail("vertex coordinates must be finite"));
        }
        // The existing number writer also serializes both signed zeros as0.
        let key = point.map(|c| if c == 0. { 0 } else { c.to_bits() });
        let index = if preserve_indices {
            let index = vertices.len() as u32;
            vertices.push(*point);
            index
        } else {
            *exact.entry(key).or_insert_with(|| {
                let index = vertices.len() as u32;
                vertices.push(*point);
                index
            })
        };
        remap.push(index);
    }
    let mut triangles = Vec::with_capacity(object.triangles.len());
    let mut faces = HashSet::new();
    for source in &object.triangles {
        let mut triangle = [0; 3];
        for i in 0..3 {
            triangle[i] = *remap
                .get(source[i] as usize)
                .ok_or_else(|| fail("triangle references an absent vertex"))?;
        }
        if triangle[0] == triangle[1] || triangle[1] == triangle[2] || triangle[2] == triangle[0] {
            return Err(fail("exact indexing collapses a triangle"));
        }
        let mut unordered = triangle;
        unordered.sort_unstable();
        if !faces.insert(unordered) {
            return Err(fail(
                "coincident duplicate triangles have ambiguous shell ownership",
            ));
        }
        let [a, b, c] = triangle.map(|i| vertices[i as usize]);
        let u = std::array::from_fn::<_, 3, _>(|i| b[i] - a[i]);
        let v = std::array::from_fn::<_, 3, _>(|i| c[i] - a[i]);
        let cross = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        if cross.iter().any(|c| !c.is_finite()) || cross.iter().all(|c| *c == 0.) {
            return Err(fail("triangle has zero or nonfinite area"));
        }
        triangles.push(triangle);
    }
    drop(remap);
    drop(exact);
    drop(faces);
    let mut corners = Corners((0..triangles.len() as u32 * 3).collect());
    let mut edges: HashMap<(u32, u32), Edge> = HashMap::new();
    for (face, triangle) in triangles.iter().enumerate() {
        for (i, j) in [(0, 1), (1, 2), (2, 0)] {
            let (a, b) = (triangle[i], triangle[j]);
            let ascending = a < b;
            let key = (a.min(b), a.max(b));
            let current = [face as u32 * 3 + i as u32, face as u32 * 3 + j as u32];
            if let Some(first) = edges.get_mut(&key) {
                if first.paired {
                    return Err(fail(
                        "coincident/shared edge has more than two incident triangles; shell ownership is ambiguous",
                    ));
                }
                if first.ascending == ascending {
                    return Err(fail("edge triangles do not have opposite winding"));
                }
                corners.join(first.corners[0], current[1]);
                corners.join(first.corners[1], current[0]);
                first.paired = true;
            } else {
                edges.insert(
                    key,
                    Edge {
                        corners: current,
                        ascending,
                        paired: false,
                    },
                );
            }
        }
    }
    if edges.values().any(|edge| !edge.paired) {
        return Err(fail(
            "mesh has an open indexed boundary; exact coordinates cannot close it",
        ));
    }
    drop(edges);
    // A geometric point can belong to disconnected closed fans (for example
    // two shells touching at one vertex). Preserve separate index identities
    // for those fans instead of making a nonmanifold shared vertex. Edge-only
    // corner adjacency defines the fans without a spatial tolerance or guess.
    let mut first_fan = vec![None; vertices.len()];
    let mut extra_fans = HashMap::new();
    for (face, triangle) in triangles.iter_mut().enumerate() {
        for (corner, index) in triangle.iter_mut().enumerate() {
            let original = *index;
            let fan = corners.root(face as u32 * 3 + corner as u32);
            match first_fan[original as usize] {
                None => first_fan[original as usize] = Some(fan),
                Some(first) if first == fan => {}
                Some(_) => {
                    *index = *extra_fans.entry((original, fan)).or_insert_with(|| {
                        let index = vertices.len() as u32;
                        vertices.push(vertices[original as usize]);
                        index
                    });
                }
            }
        }
    }
    if vertices.len() > MAX_VERTICES {
        return Err(fail("split vertex fans exceed the bounded vertex domain"));
    }
    Ok(MeshObject {
        name: object.name.clone(),
        vertices,
        triangles,
    })
}

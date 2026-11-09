//! Certify the exact indexed connectivity of a fresh analytic tessellation.
//!
//! This check preserves vertex identities and the input winding. It proves
//! closed, oriented manifold connectivity and measures signed material
//! volume. The analytic tessellator establishes the geometric embedding and
//! chord tolerance before certification.

use crate::display::{DisplayMesh, MeshError};

const MAX_TRIANGLES: usize = 5_000_000;
const MAX_VERTICES: usize = MAX_TRIANGLES * 3;
const INVALID: MeshError = MeshError::UnresolvedBoundary;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshProof {
    pub edge_pairs: u32,
    pub vertex_links: u32,
    pub signed_volume_mm3: f64,
}

// Path compression keeps the corner and shell unions compact. Numbering
// roots by the lowest input index makes local volume origins deterministic.
struct Sets(Vec<u32>);

impl Sets {
    fn new(count: usize) -> Self {
        Self((0..count as u32).collect())
    }

    fn root(&mut self, node: u32) -> u32 {
        let mut root = node;
        while self.0[root as usize] != root {
            root = self.0[root as usize];
        }
        let mut node = node;
        while node != root {
            let parent = self.0[node as usize];
            self.0[node as usize] = root;
            node = parent;
        }
        root
    }

    fn join(&mut self, a: u32, b: u32) {
        let (a, b) = (self.root(a), self.root(b));
        self.0[a.max(b) as usize] = a.min(b);
    }
}

fn shape_budget(vertices: usize, triangles: usize) -> Result<(), MeshError> {
    if vertices > MAX_VERTICES || triangles > MAX_TRIANGLES {
        Err(MeshError::BudgetExceeded)
    } else {
        Ok(())
    }
}

fn next(corner: u32) -> u32 {
    corner / 3 * 3 + (corner % 3 + 1) % 3
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Check indexed triangles without changing their coordinates or topology.
/// Distinct closed shells may have separate index identities at coincident
/// points. Inward cavity shells contribute negative signed volume.
pub fn certify(mesh: &DisplayMesh) -> Result<MeshProof, MeshError> {
    let (vertices, vertex_tail) = mesh.vertices.as_chunks::<3>();
    let (triangles, index_tail) = mesh.indices.as_chunks::<3>();
    if !vertex_tail.is_empty()
        || !index_tail.is_empty()
        || triangles.is_empty()
        || mesh.face_indices.len() != triangles.len()
    {
        return Err(INVALID);
    }
    shape_budget(vertices.len(), triangles.len())?;
    if mesh
        .vertices
        .iter()
        .any(|coordinate| !coordinate.is_finite())
    {
        return Err(INVALID);
    }

    let mut edges = Vec::with_capacity(mesh.indices.len());
    for (face, &triangle) in triangles.iter().enumerate() {
        let [Some(a), Some(b), Some(c)] = triangle.map(|i| vertices.get(i as usize)) else {
            return Err(INVALID);
        };
        let area = cross(sub(*b, *a), sub(*c, *a));
        if area.iter().any(|c| !c.is_finite()) || area == [0.; 3] {
            return Err(INVALID);
        }
        for (i, a) in triangle.into_iter().enumerate() {
            let b = triangle[(i + 1) % 3];
            edges.push([a.min(b), a.max(b), (face * 3 + i) as u32]);
        }
    }
    edges.sort_unstable();

    let mut fans = Sets::new(mesh.indices.len());
    let mut shells = Sets::new(triangles.len());
    let mut uses = edges.iter().peekable();
    while let Some([a, b, start]) = uses.next().copied() {
        let [c, d, opposite] = uses.next().copied().ok_or(INVALID)?;
        if [a, b] != [c, d] || uses.peek().is_some_and(|e| e[..2] == [a, b]) {
            return Err(INVALID);
        }
        if mesh.indices[start as usize] == mesh.indices[opposite as usize]
            || mesh.indices[next(next(start)) as usize]
                == mesh.indices[next(next(opposite)) as usize]
        {
            return Err(INVALID);
        }
        fans.join(start, next(opposite));
        fans.join(next(start), opposite);
        shells.join(start / 3, opposite / 3);
    }
    let edge_pairs = (edges.len() / 2) as u32;
    drop(edges);

    let mut links = vec![u32::MAX; vertices.len()];
    let mut vertex_links = 0;
    for (corner, &vertex) in mesh.indices.iter().enumerate() {
        let root = fans.root(corner as u32);
        let link = &mut links[vertex as usize];
        if *link == u32::MAX {
            *link = root;
            vertex_links += 1;
        } else if *link != root {
            return Err(INVALID);
        }
    }

    // Recenter each shell before integration so distant placements do not
    // turn volume into a subtraction of large world-coordinate products.
    let mut volume = 0.;
    let mut correction = 0.;
    for (face, &triangle) in triangles.iter().enumerate() {
        let shell = shells.root(face as u32) as usize;
        let origin = vertices[triangles[shell][0] as usize];
        let [a, b, c] = triangle.map(|i| sub(vertices[i as usize], origin));
        let area = cross(b, c);
        let term = (a[0] * area[0] + a[1] * area[1] + a[2] * area[2]) / 6.;
        let adjusted = term - correction;
        let next = volume + adjusted;
        correction = (next - volume) - adjusted;
        volume = next;
    }
    if !volume.is_finite() || volume <= 0. {
        return Err(INVALID);
    }
    Ok(MeshProof {
        edge_pairs,
        vertex_links,
        signed_volume_mm3: volume,
    })
}

#[cfg(test)]
mod tests;

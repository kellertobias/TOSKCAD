//! Owned triangle data shared by the geometry backends.

#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<f64>,
    pub indices: Vec<u32>,
    /// One evaluation-local B-rep face index per triangle.
    pub face_indices: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ManufacturingMesh {
    pub mesh: Mesh,
    pub maximum_displacement_mm: f64,
    pub edge_pairs: u32,
    pub vertex_links: u32,
    pub removed_native_pole_triangles: u32,
    pub removed_exact_zero_triangles: u32,
    pub exact_vertex_pairs: u32,
    pub internal_constraint_edges: u32,
    pub removed_meshing_internal_edges: u32,
    pub removed_meshing_constraint_wire_containers: u32,
    pub removed_meshing_internal_vertices: u32,
    pub remeshed_without_internal_constraints: bool,
}

pub(crate) fn valid_parts(vertices: &[f64], indices: &[u32], face_indices: &[u32]) -> bool {
    !vertices.is_empty()
        && vertices.len().is_multiple_of(3)
        && indices.len().is_multiple_of(3)
        && face_indices.len() == indices.len() / 3
        && vertices.iter().all(|v| v.is_finite())
        && indices.iter().all(|&i| (i as usize) < vertices.len() / 3)
}

mod native;

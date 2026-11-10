use super::{ManufacturingMesh, Mesh};
use crate::ffi;

impl From<ffi::Mesh> for Mesh {
    fn from(value: ffi::Mesh) -> Self {
        let ffi::Mesh {
            vertices,
            indices,
            face_indices,
        } = value;
        Self {
            vertices,
            indices,
            face_indices,
        }
    }
}

impl From<ffi::ManufacturingMesh> for ManufacturingMesh {
    fn from(value: ffi::ManufacturingMesh) -> Self {
        let ffi::ManufacturingMesh {
            mesh,
            maximum_displacement_mm,
            edge_pairs,
            vertex_links,
            removed_native_pole_triangles,
            removed_exact_zero_triangles,
            exact_vertex_pairs,
            internal_constraint_edges,
            removed_meshing_internal_edges,
            removed_meshing_constraint_wire_containers,
            removed_meshing_internal_vertices,
            remeshed_without_internal_constraints,
        } = value;
        Self {
            mesh: mesh.into(),
            maximum_displacement_mm,
            edge_pairs,
            vertex_links,
            removed_native_pole_triangles,
            removed_exact_zero_triangles,
            exact_vertex_pairs,
            internal_constraint_edges,
            removed_meshing_internal_edges,
            removed_meshing_constraint_wire_containers,
            removed_meshing_internal_vertices,
            remeshed_without_internal_constraints,
        }
    }
}

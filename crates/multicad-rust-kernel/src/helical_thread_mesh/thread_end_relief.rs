//! Shared-ring manufacturing boundary of a retained thread prefix and neck.
use super::*;
use crate::thread_end_relief::Relief;

pub fn end_relief_volume_error_bound(relief: &Relief, options: Tessellation) -> f64 {
    volume_error_bound(relief.material_thread(), options)
        + TAU
            * relief.neck_radius_mm()
            * (relief.full_length_mm() - relief.material_length_mm())
            * options.deflection
}
pub fn tessellate_end_relief(
    relief: &Relief,
    options: Tessellation,
) -> Result<DisplayMesh, MeshError> {
    let mut builder = envelope_builder(relief.material_thread(), None, options)?;
    builder.build_wall()?;
    let [entry, mut shoulder] = std::mem::take(&mut builder.caps);
    builder.add_cap(0, entry)?;
    Builder::sort_ring(&mut shoulder)?;
    if builder.mesh.face_indices.len() + 5 * shoulder.len() > builder.limit {
        return Err(MeshError::BudgetExceeded);
    }
    let mut neck = Vec::with_capacity(shoulder.len());
    let mut far = Vec::with_capacity(shoulder.len());
    for &(angle, _) in &shoulder {
        let (sin, cos) = angle.sin_cos();
        let [x, y] = [relief.neck_radius_mm() * cos, relief.neck_radius_mm() * sin];
        neck.push(builder.mesh.vertices.len() as u32 / 3);
        builder
            .mesh
            .vertices
            .extend([x, y, relief.material_length_mm()]);
        far.push(builder.mesh.vertices.len() as u32 / 3);
        builder
            .mesh
            .vertices
            .extend([x, y, relief.full_length_mm()]);
    }
    let center = builder.mesh.vertices.len() as u32 / 3;
    builder
        .mesh
        .vertices
        .extend([0., 0., relief.full_length_mm()]);
    for i in 0..shoulder.len() {
        let next = (i + 1) % shoulder.len();
        let [a, b, c, d] = [shoulder[i].1, shoulder[next].1, neck[i], neck[next]];
        builder.triangle([a, d, c], 1)?;
        builder.triangle([a, b, d], 1)?;
        let [e, f] = [far[i], far[next]];
        builder.triangle([c, d, f], relief.wall_face())?;
        builder.triangle([c, f, e], relief.wall_face())?;
        builder.triangle([center, e, f], relief.far_cap_face())?;
    }
    Ok(builder.mesh)
}

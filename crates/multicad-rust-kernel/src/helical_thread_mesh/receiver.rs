//! Shared-ring closure of the actual blind bore, including its retained floor.
use super::*;
use crate::helical_receiver::Receiver;
pub fn receiver_volume_error_bound(receiver: &Receiver, options: Tessellation) -> f64 {
    volume_error_bound(receiver.thread(), options)
        + TAU * receiver.radius_mm() * receiver.stock_length_mm() * options.deflection
}
pub fn tessellate_receiver(
    receiver: &Receiver,
    options: Tessellation,
) -> Result<DisplayMesh, MeshError> {
    let exterior = BoreExterior::Cylinder(receiver.radius_mm());
    let mut builder = envelope_builder(receiver.thread(), Some(exterior), options)?;
    builder.build_wall()?;
    let [mouth, floor] = std::mem::take(&mut builder.caps);
    builder.add_cap(1, floor)?;
    if builder.mesh.face_indices.len() + 5 * mouth.len() > builder.limit {
        return Err(MeshError::BudgetExceeded);
    }
    for triangle in builder.mesh.indices.as_chunks_mut::<3>().0 {
        triangle.swap(1, 2);
    }
    let lower = builder.add_annular_cap(0, mouth, exterior)?;
    let mut upper = Vec::with_capacity(lower.vertices.len());
    for &vertex in &lower.vertices[..lower.vertices.len() - 1] {
        let p = vertex as usize * 3;
        upper.push(builder.mesh.vertices.len() as u32 / 3);
        builder.mesh.vertices.extend([
            builder.mesh.vertices[p],
            builder.mesh.vertices[p + 1],
            receiver.stock_length_mm(),
        ]);
    }
    upper.push(upper[0]);
    let center = builder.mesh.vertices.len() as u32 / 3;
    builder
        .mesh
        .vertices
        .extend([0., 0., receiver.stock_length_mm()]);
    let wall = 2 + 4 * receiver.thread().spec().starts;
    for (lower, upper) in lower
        .vertices
        .array_windows::<2>()
        .zip(upper.array_windows::<2>())
    {
        let [a, b] = *lower;
        let [c, d] = *upper;
        builder.triangle([a, b, d], wall)?;
        builder.triangle([a, d, c], wall)?;
        builder.triangle([center, c, d], wall + 1)?;
    }
    Ok(builder.mesh)
}

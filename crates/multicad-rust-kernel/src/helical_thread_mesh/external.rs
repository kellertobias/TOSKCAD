//! One shared boundary between the helical prefix and retained plain stock.
use super::*;
use crate::threaded_shaft::Shaft;

pub fn shaft_volume_error_bound(shaft: &Shaft, options: Tessellation) -> f64 {
    volume_error_bound(shaft.thread(), options)
        + TAU
            * shaft.radius_mm()
            * (shaft.stock_length_mm() - shaft.thread().spec().length_mm)
            * options.deflection
}

pub fn tessellate_shaft(shaft: &Shaft, options: Tessellation) -> Result<DisplayMesh, MeshError> {
    let exterior = (!shaft.tangent()).then_some(BoreExterior::Cylinder(shaft.radius_mm()));
    let mut builder = envelope_builder(shaft.thread(), exterior, options)?;
    builder.build_wall()?;
    let [entry, mut shoulder] = std::mem::take(&mut builder.caps);
    builder.add_cap(0, entry)?;
    Builder::sort_ring(&mut shoulder)?;
    if builder.mesh.face_indices.len() + 5 * shoulder.len() > builder.limit {
        return Err(MeshError::BudgetExceeded);
    }
    let thread = shaft.thread();
    let spec = thread.spec();
    let z = spec.length_mm;
    let mut join = Vec::with_capacity(shoulder.len());
    let mut far = Vec::with_capacity(shoulder.len());
    for &(angle, inner) in &shoulder {
        let [x, y, _] = builder.mesh.vertices.as_chunks::<3>().0[inner as usize];
        let radius = shaft.radius_mm();
        let (sin, cos) = angle.sin_cos();
        let outer = if shaft.tangent() && (radius - x.hypot(y)).abs() <= 64. * f64::EPSILON * radius
        {
            inner
        } else {
            let outer = builder.mesh.vertices.len() as u32 / 3;
            builder
                .mesh
                .vertices
                .extend([radius * cos, radius * sin, z]);
            outer
        };
        join.push(outer);
        far.push(builder.mesh.vertices.len() as u32 / 3);
        builder
            .mesh
            .vertices
            .extend([radius * cos, radius * sin, shaft.stock_length_mm()]);
    }
    let center = builder.mesh.vertices.len() as u32 / 3;
    builder
        .mesh
        .vertices
        .extend([0., 0., shaft.stock_length_mm()]);
    for i in 0..shoulder.len() {
        let next = (i + 1) % shoulder.len();
        let [a, b, c, d] = [shoulder[i].1, shoulder[next].1, join[i], join[next]];
        let lo = shoulder[i].0;
        let hi = if next == 0 { TAU } else { shoulder[next].0 };
        let phase = (z - thread.helical_rate() * ((lo + hi) * 0.5) - thread.profile_breaks()[1])
            .rem_euclid(spec.pitch_mm * spec.starts as f64);
        let period = (phase / spec.pitch_mm).floor() as u32;
        let face = shaft.shoulder_face(if shaft.tangent() { period } else { 0 });
        // The retained stock begins above this plane: the shoulder faces down.
        if a != c {
            builder.triangle([a, d, c], face)?;
        }
        if b != d {
            builder.triangle([a, b, d], face)?;
        }
        let [e, f] = [far[i], far[next]];
        builder.triangle([c, d, f], shaft.wall_face())?;
        builder.triangle([c, f, e], shaft.wall_face())?;
        builder.triangle([center, e, f], shaft.far_cap_face())?;
    }
    Ok(builder.mesh)
}

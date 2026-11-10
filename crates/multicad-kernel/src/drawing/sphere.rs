//! Exact silhouettes and section disks of one analytic sphere.
use super::*;
use multicad_rust_kernel::curved_projection::{Curve, Detail, Edge, Ellipse};
use std::f64::consts::TAU;

pub(super) fn project(
    input: &DrawingSolid<'_>,
    origin: [f64; 3],
    radius: f64,
    mut options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let offset: [f64; 3] = std::array::from_fn(|i| origin[i] - options.frame.origin[i]);
    let [x, y, z] = planar::view_axes(options.frame).map(|axis| {
        axis.into_iter()
            .zip(offset)
            .map(|(a, b)| a * b)
            .sum::<f64>()
    });
    let center = [x, y];
    let Some(depth) = options.section_offset_mm else {
        return silhouette(center, radius, options);
    };
    if options.detail.is_some()
        || options.section_depth_mm.is_some()
        || (!options.section_only
            && (options.section_u_mm.is_some() || options.section_v_mm.is_some()))
    {
        return Err(err(
            GeometryErrorKind::Unsupported,
            "Rust sphere sections with retained geometry require an unbounded view, and sphere detail crops require a non-section view",
        ));
    }

    let distance = depth - z;
    if distance <= -radius || (options.section_only && distance >= radius) {
        return Ok(Vec::new());
    }
    if distance >= radius {
        return silhouette(center, radius, options);
    }
    let absolute = distance.abs();
    let cut_radius = ((radius - absolute) * (radius + absolute)).sqrt();
    let mut output = section::project_disk(center, cut_radius, input, options)?;
    if !options.section_only && distance > 0. {
        // The retained sphere contains its equator behind the cut disk.
        options.maximum_curves -= output.len() as u32;
        options.maximum_points -= output
            .iter()
            .map(|curve| curve.preview_mm.len() as u32)
            .sum::<u32>();
        output.extend(silhouette(center, radius, options)?);
    }
    Ok(output)
}

fn silhouette(
    center: [f64; 2],
    radius: f64,
    options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let ellipse = Ellipse {
        center,
        u: [1., 0.],
        v: [0., 1.],
        major: radius,
        minor: radius,
        trim: [0., TAU],
    };
    let spans = options.detail.map_or_else(
        || vec![ellipse.trim],
        |(center, radius)| ellipse.in_detail(Detail { center, radius }),
    );
    curved::publish(
        spans
            .into_iter()
            .map(|trim| Edge {
                hidden: false,
                curve: Curve::Ellipse(Ellipse { trim, ..ellipse }),
            })
            .collect(),
        options,
        0,
    )
}

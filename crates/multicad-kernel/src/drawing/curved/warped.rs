//! Map the drawing view into a certified shell's local coordinate frame.
use super::*;
use multicad_rust_kernel::warped_union::{Elevation, Error, Shell};

pub(in crate::drawing) fn needs_sweep_shell(
    placement: PlaneFrame,
    options: DrawingProjection,
) -> bool {
    let axes = planar::view_axes(placement);
    options.detail.is_some()
        || planar::view_axes(options.frame)
            .iter()
            .any(|&view| !axes.iter().any(|&axis| dot(view, axis).abs() >= 1. - 1e-12))
}

pub(super) fn for_projection(
    solid: &Solid,
    source: usize,
    tolerance: f64,
    options: DrawingProjection,
    oblique_cuts: bool,
) -> Result<Option<Elevation>, GeometryError> {
    if let Some((shell, placement)) = solid.drawing_warped_shell() {
        return elevation(shell, placement, source, tolerance, options).map(Some);
    }
    let Some((body, placement)) = solid.normal_sweep_body() else {
        return Ok(None);
    };
    if !oblique_cuts || !needs_sweep_shell(placement, options) {
        return Ok(None);
    }
    let result = body
        .drawing_shell(tolerance)
        .map_err(drawing_error)
        .and_then(|shell| elevation(&shell, placement, source, tolerance, options));
    match result {
        Err(error) if error.kind == GeometryErrorKind::Unsupported => Ok(None),
        result => result.map(Some),
    }
}

pub(in crate::drawing) fn elevation(
    shell: &Shell,
    placement: PlaneFrame,
    source: usize,
    tolerance: f64,
    options: DrawingProjection,
) -> Result<Elevation, GeometryError> {
    let axes = planar::view_axes(placement);
    let local = |vector| axes.map(|axis| dot(axis, vector));
    let origin = local(std::array::from_fn(|i| {
        options.frame.origin[i] - placement.origin[i]
    }));
    let view = planar::view_axes(options.frame).map(local);
    shell
        .elevation(origin, view, source, tolerance, options.tangent_edges)
        .map_err(drawing_error)
}

fn drawing_error(cause: Error) -> GeometryError {
    match cause {
        Error::Invalid => planar::projection_error(planar_projection::ProjectionError::Invalid),
        Error::Limit => planar::projection_error(planar_projection::ProjectionError::Limit),
        Error::Unsupported => err(
            GeometryErrorKind::Unsupported,
            "Rust could not resolve the drawing against the trimmed shell boundary",
        ),
    }
}

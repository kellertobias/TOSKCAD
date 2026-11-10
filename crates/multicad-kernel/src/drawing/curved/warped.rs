//! Map the drawing view into a certified shell's local coordinate frame.
use super::*;
use multicad_rust_kernel::warped_union::{Elevation, Error, Shell};

pub(super) fn elevation(
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
        .map_err(|cause| match cause {
            Error::Invalid => planar::projection_error(planar_projection::ProjectionError::Invalid),
            Error::Limit => planar::projection_error(planar_projection::ProjectionError::Limit),
            Error::Unsupported => err(
                GeometryErrorKind::Unsupported,
                "Rust trimmed-shell drawing requires supported face trims and an axial or transverse cylinder view",
            ),
        })
}

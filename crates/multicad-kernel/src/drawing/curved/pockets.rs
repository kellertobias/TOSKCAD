//! Transfer exact face-on blind opening trims and floors into the drawing scene.
use super::*;
use multicad_rust_kernel::planar_projection::{FaceOpening, RimCandidate};
use std::f64::consts::TAU;

const DIRECTION_TOLERANCE: f64 = 32. * f64::EPSILON;

pub(super) struct Stock {
    pub solid: Solid,
    pub rims: Vec<RimCandidate>,
    pub openings: Vec<FaceOpening>,
}

pub(super) fn stock(
    solid: &Solid,
    source: usize,
    body: usize,
    options: DrawingProjection,
) -> Result<Option<Stock>, GeometryError> {
    let view = planar::view_axes(options.frame);
    if let Some(solid) = solid.stock_for_visible_projection(view[2]) {
        return Ok(Some(Stock {
            solid,
            rims: Vec::new(),
            openings: Vec::new(),
        }));
    }
    let Some((solid, openings)) = solid.drawing_circular_stock() else {
        return Ok(None);
    };
    let mut result = Stock {
        solid,
        rims: Vec::with_capacity(openings.len()),
        openings: Vec::with_capacity(openings.len()),
    };
    for opening in openings {
        let axis = view.map(|axis| dot(axis, opening.normal));
        if axis[2].abs() <= DIRECTION_TOLERANCE {
            // The blind cavity opens on a plane perpendicular to a view
            // ray's first-hit faces, which remain intact.
            continue;
        }
        if axis[0].hypot(axis[1]) > DIRECTION_TOLERANCE {
            return Err(err(
                GeometryErrorKind::Unsupported,
                "Rust circular drawing stock requires face-on or edge-on openings",
            ));
        }
        if axis[2] < 0. {
            // A back-facing blind opening cannot change the first material hit.
            continue;
        }
        let offset = std::array::from_fn(|i| opening.center[i] - options.frame.origin[i]);
        let [u, v, depth] = view.map(|axis| dot(axis, offset));
        let center = [u, v];
        result.openings.push(FaceOpening {
            body,
            face: opening.face,
            center,
            radius: opening.radius,
            floor_depth: depth - opening.depth,
        });
        result.rims.push(RimCandidate {
            source,
            center,
            depth,
            radius: opening.radius,
            trim: [0., TAU],
        });
    }
    Ok(Some(result))
}

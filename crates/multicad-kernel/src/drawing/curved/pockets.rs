//! Transfer exact circular blind opening trims and visible floors into the drawing scene.
use super::*;
use multicad_rust_kernel::planar_projection::{
    CircularOpeningEnd, FaceOpening, HiddenCandidate, RimCandidate, SpatialRimCandidate,
};
use std::f64::consts::TAU;

const DIRECTION_TOLERANCE: f64 = 32. * f64::EPSILON;

pub(super) struct Stock<T = Solid> {
    pub solid: T,
    pub rims: Vec<RimCandidate>,
    pub spatial_rims: Vec<SpatialRimCandidate>,
    pub openings: Vec<FaceOpening>,
    pub intrinsic_hidden: Vec<HiddenCandidate>,
}

pub(super) fn has_machined_stock(solid: &Solid) -> bool {
    solid.has_circular_drawing_stock()
        || solid
            .orthogonal_body()
            .is_some_and(|(body, _)| !body.drawing_stock().slots.is_empty())
}

pub(super) fn stock(
    solid: &Solid,
    source: usize,
    body: usize,
    options: DrawingProjection,
    mut covers: impl FnMut(shadows::Mouth) -> Result<bool, GeometryError>,
    mut oblique: impl FnMut(apertures::Cavity) -> Result<SpatialRimCandidate, GeometryError>,
) -> Result<Option<Stock>, GeometryError> {
    let view = planar::view_axes(options.frame);
    if let Some(solid) = solid.stock_for_visible_projection(view[2]) {
        return Ok(Some(Stock {
            solid,
            rims: Vec::new(),
            spatial_rims: Vec::new(),
            openings: Vec::new(),
            intrinsic_hidden: Vec::new(),
        }));
    }
    let Some((solid, openings, slots)) = solid.drawing_machined_stock() else {
        return Ok(None);
    };
    let mut result = Stock {
        solid,
        rims: Vec::with_capacity(openings.len()),
        spatial_rims: Vec::new(),
        openings: Vec::with_capacity(openings.len()),
        intrinsic_hidden: Vec::new(),
    };
    for slot in slots {
        if dot(view[2], slot.normal) > DIRECTION_TOLERANCE
            && !covers(shadows::Mouth {
                centers: slot.centers,
                normal: slot.normal,
                radius: slot.radius,
            })?
        {
            return Err(err(
                GeometryErrorKind::Unsupported,
                "Rust drawing projection cannot prove full shadow of a rounded mortise",
            ));
        }
    }
    for opening in openings {
        let axis = view.map(|axis| dot(axis, opening.normal));
        if axis[2] <= DIRECTION_TOLERANCE {
            // A blind opening facing away or edge-on leaves the first hit intact.
            continue;
        }
        if axis[0].hypot(axis[1]) > DIRECTION_TOLERANCE {
            let cavity = apertures::Cavity {
                mouth: opening.center,
                normal: opening.normal,
                view_normal: axis,
                radius: opening.radius,
                depth: opening.depth,
            };
            match oblique(cavity) {
                Ok(rim) => result.spatial_rims.push(rim),
                Err(error)
                    if error.kind == GeometryErrorKind::Unsupported
                        && covers(shadows::Mouth {
                            centers: [opening.center; 2],
                            normal: opening.normal,
                            radius: opening.radius,
                        })? => {}
                Err(error) => return Err(error),
            }
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
            end: CircularOpeningEnd::Floor(depth - opening.depth),
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

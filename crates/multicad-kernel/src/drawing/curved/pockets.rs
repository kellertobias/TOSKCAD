//! Certify visible circular openings when a planar stock proxy is sufficient.
use super::*;
use multicad_rust_kernel::curved_projection::{Edge, Ellipse};
use std::f64::consts::TAU;

type Point = [f64; 3];
const DIRECTION_TOLERANCE: f64 = 32. * f64::EPSILON;
const MAX_CHECKS: usize = 2_000_000;

pub(super) fn stock(
    index: usize,
    members: &[SceneMember<'_>],
    options: DrawingProjection,
    checks: &mut usize,
) -> Result<Option<(Solid, Vec<Edge>)>, GeometryError> {
    let member = &members[index];
    let view = planar::view_axes(options.frame);
    if let Some(stock) = member.solid.stock_for_visible_projection(view[2]) {
        return Ok(Some((stock, Vec::new())));
    }
    let Some((stock, openings)) = member.solid.drawing_circular_stock() else {
        return Ok(None);
    };
    if options.detail.is_some() {
        return Err(unsupported());
    }
    let tolerance = member.solid.geometry_policy().linear_tolerance_mm;
    let mut edges = Vec::with_capacity(openings.len());
    for opening in openings {
        let axis = view.map(|axis| dot(axis, opening.normal));
        if axis[2].abs() <= DIRECTION_TOLERANCE {
            continue;
        }
        if axis[0].hypot(axis[1]) > DIRECTION_TOLERANCE {
            return Err(unsupported());
        }
        if axis[2] < 0. {
            // A back-facing blind opening cannot change the first material hit.
            continue;
        }

        let offset = std::array::from_fn(|i| opening.center[i] - options.frame.origin[i]);
        let [u, v, depth] = view.map(|axis| dot(axis, offset));
        let center = [u, v];
        let radius = opening.radius;
        let floor = depth - opening.depth;
        for (other_index, candidate) in members.iter().enumerate() {
            if other_index == index {
                continue;
            }
            *checks = checks
                .checked_add(1)
                .filter(|count| *count <= MAX_CHECKS)
                .ok_or_else(|| {
                    err(
                        GeometryErrorKind::Limit,
                        "Rust circular opening proof exceeded its work budget",
                    )
                })?;
            let [lo, hi] = candidate.bounds;
            let [near, far] = distances(center, [lo, hi]);
            if near > radius + tolerance || hi[2] <= floor + tolerance {
                continue;
            }

            // Filling a void changes the occlusion only for geometry between
            // its floor and mouth, unless a real solid already covers the mouth.
            if lo[2] < depth - tolerance
                && !flush_circle(candidate, view[2], center, depth, radius, tolerance)
            {
                return Err(unsupported());
            }
            // The original visible rim must also be clear of every nearer
            // object. A bounding rectangle fully within the disk misses its rim.
            if hi[2] > depth + tolerance && far >= radius - tolerance {
                return Err(unsupported());
            }
        }
        edges.push(Edge {
            hidden: false,
            curve: Curve::Ellipse(Ellipse {
                center,
                u: [1., 0.],
                v: [0., 1.],
                major: radius,
                minor: radius,
                trim: [0., TAU],
            }),
        });
    }
    Ok(Some((stock, edges)))
}

fn unsupported() -> GeometryError {
    err(
        GeometryErrorKind::Unsupported,
        "Rust projection cannot yet resolve overlapping geometry in this circular opening",
    )
}

fn distances(center: [f64; 2], [lo, hi]: [Point; 2]) -> [f64; 2] {
    let nearest = |i: usize| (center[i] - center[i].clamp(lo[i], hi[i])).abs();
    let farthest = |i: usize| (center[i] - lo[i]).abs().max((center[i] - hi[i]).abs());
    [nearest(0).hypot(nearest(1)), farthest(0).hypot(farthest(1))]
}

fn flush_circle(
    candidate: &SceneMember<'_>,
    toward_viewer: Point,
    center: [f64; 2],
    depth: f64,
    radius: f64,
    tolerance: f64,
) -> bool {
    let Some((frame, r, _, None)) = circular_dimensions(candidate.solid) else {
        return false;
    };
    let [lo, hi] = candidate.bounds;
    (dot(unit(frame.normal()), toward_viewer).abs() - 1.).abs() <= DIRECTION_TOLERANCE
        && (r - radius).abs() <= tolerance
        && (depth - hi[2]).abs() <= tolerance
        && center
            .into_iter()
            .enumerate()
            .all(|(i, c)| (c - lo[i].midpoint(hi[i])).abs() <= tolerance)
}

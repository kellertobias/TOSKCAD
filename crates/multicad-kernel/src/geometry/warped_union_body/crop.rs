//! Crop an authored curved solid with an aligned box in its local chart.
use super::*;
use crate::geometry::planar_boolean_body::cell_faces;
use multicad_rust_kernel::orthogonal::Box3;

impl Solid {
    /// Intersect a retained quarter-round chart with one exact aligned box.
    pub fn intersect_warped_box(
        operands: &[&Solid],
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Option<Result<Solid, GeometryError>> {
        let &[first, second] = operands else {
            return None;
        };
        if policy.boolean_fuzzy_mm != 0. || first.policy != policy || second.policy != policy {
            return None;
        }
        let (base, crop, body, frame) = if let Some((body, frame)) = first.warped_union_body() {
            (first, second, body, frame)
        } else {
            let (body, frame) = second.warped_union_body()?;
            (second, first, body, frame)
        };
        let chart = body.chart.as_ref()?;
        let axes = [frame.u, frame.v, frame.normal()];
        let inverse = RigidTransform {
            rotation: axes,
            translation_mm: axes.map(|axis| -dot(axis, frame.origin)),
        };
        let crop = match crop.transformed(inverse) {
            Ok(solid) => solid,
            Err(cause) => return Some(Err(cause)),
        };
        let cells = match crop.orthogonal_cells() {
            Ok(cells) => cells?,
            Err(cause) => return Some(Err(cause)),
        };
        let &[limit] = cells.as_slice() else {
            return None;
        };
        let [min, max] = body.geometry.bounds_mm();
        let stock = Box3 { min, max };
        let tolerance = policy.linear_tolerance_mm;
        if (0..3).any(|i| limit.min[i] >= stock.max[i] || limit.max[i] <= stock.min[i]) {
            return Some(Err(empty_crop()));
        }
        if contains(limit, stock) {
            return Some(base.share());
        }
        if chart.bosses.iter().map(AuthoredBoss::bounds).any(|bounds| {
            !contains(
                limit,
                Box3 {
                    min: bounds.min,
                    max: bounds.max,
                },
            )
        }) {
            return Some(Err(error(
                "quarter-round crop",
                GeometryErrorKind::Unsupported,
                "The crop could change an attached boss",
            )));
        }
        let forward = RigidTransform {
            rotation: std::array::from_fn(|i| axes.map(|axis| axis[i])),
            translation_mm: frame.origin,
        };
        Some(
            crop_chart(chart, &crop, stock, limit, feature, policy, tolerance)
                .and_then(|local| local.transformed(forward)),
        )
    }
}

fn contains(outer: Box3, inner: Box3) -> bool {
    (0..3).all(|i| outer.min[i] <= inner.min[i] && outer.max[i] >= inner.max[i])
}

fn empty_crop() -> GeometryError {
    error(
        "quarter-round crop",
        GeometryErrorKind::InvalidResult,
        "Boolean removed all material; no output body can be committed",
    )
}

fn unsupported() -> GeometryError {
    error(
        "quarter-round crop",
        GeometryErrorKind::Unsupported,
        "The crop is too close to an existing boundary for an exact result",
    )
}

fn crop_axial(
    chart: &mut Chart,
    stock: Box3,
    limit: Box3,
    tolerance: f64,
) -> Result<(), GeometryError> {
    let low: [bool; 3] = std::array::from_fn(|i| limit.min[i] > stock.min[i]);
    let high: [bool; 3] = std::array::from_fn(|i| limit.max[i] < stock.max[i]);
    let mut kept = Vec::with_capacity(chart.axial.len());
    for mut tool in std::mem::take(&mut chart.axial) {
        let bounds = tool.bounds(&chart.axial_bounds);
        if (0..3).any(|i| {
            (low[i] && bounds.max[i] <= limit.min[i]) || (high[i] && bounds.min[i] >= limit.max[i])
        }) {
            continue;
        }
        let axis = subtract(tool.cut.end_mm, tool.cut.start_mm);
        for i in 0..3 {
            let crosses_low = low[i] && bounds.min[i] < limit.min[i];
            let crosses_high = high[i] && bounds.max[i] > limit.max[i];
            if !crosses_low && !crosses_high {
                continue;
            }
            if chart.band.is_some()
                || tool.cut.recess.is_some()
                || tool.open_side.is_some()
                || (crosses_low && crosses_high)
                || axis[i].abs() > tolerance
            {
                return Err(error(
                    "planar crop",
                    GeometryErrorKind::Unsupported,
                    "The crop crosses an axial cut beyond the supported single side",
                ));
            }
            let mut point_mm = [0.; 3];
            let mut normal = [0.; 3];
            point_mm[i] = if crosses_high {
                limit.max[i]
            } else {
                limit.min[i]
            };
            normal[i] = if crosses_high { 1. } else { -1. };
            tool.open_side = Some(OpenBoreSide { point_mm, normal });
        }
        kept.push(tool);
    }
    chart.axial = kept;
    Ok(())
}

fn crop_chart(
    original: &Chart,
    crop: &Solid,
    stock: Box3,
    limit: Box3,
    feature: EntityId,
    policy: GeometryPolicy,
    tolerance: f64,
) -> Result<Solid, GeometryError> {
    let mut chart = original.clone();
    crop_axial(&mut chart, stock, limit, tolerance)?;
    let topology = crop.rust_topology().ok_or_else(unsupported)??;
    let crop_faces = crop.faces()?;
    let operand = chart.topologies.len();
    let padding = (16. * tolerance).max(1e-3);
    let limit_mm = policy.maximum_coordinate_mm;
    let mut remaining = Box3 {
        min: stock.min.map(|x| (x - padding).max(-limit_mm)),
        max: stock.max.map(|x| (x + padding).min(limit_mm)),
    };
    for axis in 0..3 {
        for high in [false, true] {
            let level = if high {
                limit.max[axis]
            } else {
                limit.min[axis]
            };
            let old = if high {
                stock.max[axis]
            } else {
                stock.min[axis]
            };
            let sign = if high { 1. } else { -1. };
            let overlap = sign * (old - level);
            if overlap <= 0. {
                continue;
            }
            if overlap <= tolerance {
                return Err(unsupported());
            }
            let mut outside = remaining;
            if high {
                outside.min[axis] = level;
                remaining.max[axis] = level;
            } else {
                outside.max[axis] = level;
                remaining.min[axis] = level;
            }
            let face = crop_faces
                .iter()
                .find(|face| {
                    face.plane.is_some_and(|(point, normal)| {
                        normal[axis] == sign && (point[axis] - level).abs() <= tolerance
                    })
                })
                .ok_or_else(unsupported)?;
            let source = topology
                .elements
                .iter()
                .position(|element| {
                    element.key.kind == TopologyKind::Face && element.key.ordinal == face.index
                })
                .ok_or_else(unsupported)?;
            let mut ancestry = [None; 6];
            // The complement has the opposite orientation at the crop plane.
            ancestry[2 * axis + usize::from(!high)] = Some(source);
            let piece = ConvexPiece::new(cell_faces(outside, PlaneFrame::default()), tolerance)
                .map_err(source_error)?;
            chart
                .append_piece(piece, &ancestry, operand, policy)
                .ok_or_else(unsupported)??;
        }
    }
    chart.topologies.push(Rc::new(topology));
    let [stock, cutters] = [
        &chart.pieces[..chart.stock_count],
        &chart.pieces[chart.stock_count..],
    ];
    let union = Union::subtract(stock, cutters, tolerance)
        .map_err(source_error)?
        .ok_or_else(empty_crop)?;
    chart.finish(union, feature, policy)
}

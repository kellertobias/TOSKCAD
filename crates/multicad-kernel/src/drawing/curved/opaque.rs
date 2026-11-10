//! Prove when an unsupported leaf cannot contribute to a visible drawing.
use super::*;
use multicad_rust_kernel::orthogonal::Box3;
use std::borrow::Cow;

type Point = [f64; 3];
const ANGULAR_TOLERANCE: f64 = 32. * f64::EPSILON;
const WORK_LIMIT: usize = 2_000_000;

pub(super) fn visible_members<'a>(
    members: &[(usize, &'a Solid)],
    view: PlaneFrame,
) -> Result<Vec<(usize, &'a Solid)>, GeometryError> {
    let mut covers = Vec::new();
    let screen = planar::view_axes(view);
    for (index, &(_, solid)) in members.iter().enumerate() {
        let detached = solid.stock_for_visible_projection(screen[2]);
        let stock = detached.as_ref().unwrap_or(solid);
        let (cells, frame) = if let Some((body, frame)) = stock.orthogonal_body() {
            if body.has_curved_cuts() {
                continue;
            }
            (Cow::Borrowed(body.drawing_stock().cells), frame)
        } else if let Some((cells, frame)) = stock.drawing_rectilinear_cells()? {
            (Cow::Owned(cells), frame)
        } else {
            continue;
        };
        let axes =
            planar::view_axes(frame).map(|axis| screen.map(|direction| dot(axis, direction)));
        if axes.iter().any(|axis| {
            axis.iter()
                .filter(|component| component.abs() > ANGULAR_TOLERANCE)
                .count()
                != 1
        }) {
            continue;
        }
        if cells.len() > WORK_LIMIT.saturating_sub(covers.len()) {
            return Err(limit());
        }
        let origin = offset(frame.origin, view.origin, screen);
        covers.extend(
            cells
                .iter()
                .map(|&cell| (index, projected_bounds(cell, axes, origin))),
        );
    }
    let identity = PlaneFrame::default();
    let axes = [identity.u, identity.v, identity.normal()]
        .map(|axis| screen.map(|direction| dot(axis, direction)));
    let origin = offset(identity.origin, view.origin, screen);
    let mut remaining = WORK_LIMIT;
    let mut visible = Vec::with_capacity(members.len());
    'member: for (index, &(source, solid)) in members.iter().enumerate() {
        let bounds = solid.bounds()?;
        let target = projected_bounds(
            Box3 {
                min: bounds.min,
                max: bounds.max,
            },
            axes,
            origin,
        );
        let tolerance = solid.geometry_policy().linear_tolerance_mm;
        for &(owner, cover) in &covers {
            remaining = remaining.checked_sub(1).ok_or_else(limit)?;
            if owner != index
                && cover.max[2] > target.max[2] + tolerance
                && (0..2).all(|i| cover.min[i] <= target.min[i] && cover.max[i] >= target.max[i])
            {
                continue 'member;
            }
        }
        visible.push((source, solid));
    }
    Ok(visible)
}

fn limit() -> GeometryError {
    err(
        GeometryErrorKind::Limit,
        "Rust drawing visibility proof exceeded its work budget",
    )
}

fn offset(origin: Point, view: Point, axes: [Point; 3]) -> Point {
    let delta = std::array::from_fn(|i| origin[i] - view[i]);
    axes.map(|axis| dot(delta, axis))
}

pub(super) fn projected_bounds(cell: Box3, axes: [Point; 3], origin: Point) -> Box3 {
    let [min, max] = [false, true].map(|upper| {
        std::array::from_fn(|i| {
            axes.iter().enumerate().fold(origin[i], |sum, (j, axis)| {
                let coordinate = if upper == (axis[i] >= 0.) {
                    cell.max[j]
                } else {
                    cell.min[j]
                };
                coordinate.mul_add(axis[i], sum)
            })
        })
    });
    Box3 { min, max }
}

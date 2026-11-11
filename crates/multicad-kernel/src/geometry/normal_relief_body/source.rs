//! Reconstruct an unchanged column in its retained intrinsic source chart.
use super::*;
use crate::planar_contours::SourcedPlanarContours;

pub(super) struct Source {
    pub(super) depth_mm: f64,
    pub(super) opposite: u32,
    pub(super) base_frame: PlaneFrame,
    pub(super) contours: SourcedPlanarContours,
}
fn add_up(a: f64, b: f64) -> f64 {
    let sum = a + b;
    let part = sum - a;
    if (a - (sum - part)) + (b - part) > 0. {
        sum.next_up()
    } else {
        sum
    }
}
fn mul_up(a: f64, b: f64) -> f64 {
    let p = a * b;
    if a != 0. && b != 0. && p.abs() < 1e-280 || a.mul_add(b, -p) > 0. {
        p.next_up()
    } else {
        p
    }
}
fn sum_error(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let p = s - a;
    (s, (a - (s - p)) + (b - p))
}
fn direction(frame: PlaneFrame, p: [f64; 3]) -> [f64; 3] {
    let axes = [frame.u, frame.v, frame.normal()];
    std::array::from_fn(|i| p.into_iter().zip(axes).map(|(p, a)| p * a[i]).sum())
}
fn evaluation_error(frame: PlaneFrame, p: [f64; 3]) -> f64 {
    let axes = [frame.u, frame.v, frame.normal()];
    let mut total = 0.;
    for i in 0..3 {
        let mut value = 0.;
        let mut error = 0.;
        for (axis, coordinate) in axes.into_iter().zip(p) {
            let product = axis[i] * coordinate;
            let residual = axis[i].mul_add(coordinate, -product).abs();
            error = add_up(
                error,
                if axis[i] != 0. && coordinate != 0. && product.abs() < 1e-280 {
                    residual.next_up()
                } else {
                    residual
                },
            );
            let (sum, residual) = sum_error(value, product);
            value = sum;
            error = add_up(error, residual.abs());
        }
        let (_, residual) = sum_error(frame.origin[i], value);
        total = add_up(total, add_up(error, residual.abs()));
    }
    total
}
fn placement_receipt(
    original: PlaneFrame,
    base: PlaneFrame,
    local: PlaneFrame,
    bounds: Bounds,
) -> f64 {
    let axes = [local.u, local.v, local.normal()];
    let actual = [base.u, base.v, base.normal()];
    let mut error = evaluation_error(original, local.origin);
    for (axis, actual) in axes.into_iter().zip(actual) {
        let expected = direction(original, axis);
        let mut difference = 0.;
        for (a, b) in actual.into_iter().zip(expected) {
            let (delta, residual) = sum_error(a, -b);
            difference = add_up(difference, add_up(delta.abs(), residual.abs()));
        }
        let extent = axis.into_iter().enumerate().fold(0., |sum, (i, a)| {
            let size = add_up(
                bounds.min[i].abs().max(bounds.max[i].abs()).next_up(),
                local.origin[i].abs(),
            );
            add_up(sum, mul_up(a.abs(), size))
        });
        error = add_up(error, mul_up(difference, extent));
    }
    error
}

pub(super) fn retained_source(
    stock: &Solid,
    selected: u32,
    requested: PlaneFrame,
) -> Result<Option<Source>, GeometryError> {
    let Some(source) = stock.retained_cap_source(selected)? else {
        return Ok(None);
    };
    let canonical = place_frame(source.placement, source.frame);
    if requested
        != (PlaneFrame {
            origin: canonical.origin_mm,
            u: canonical.u,
            v: canonical.v,
        })
    {
        return Ok(None);
    }
    let cap = PlaneFrame {
        origin: source.frame.origin_mm,
        u: source.frame.u,
        v: source.frame.v,
    };
    let mut contours = source
        .local
        .planar_face_contours_with_sources(selected, cap)?;
    contours.contours.frame = ReferenceFrame {
        origin_mm: requested.origin,
        u: requested.u,
        v: requested.v,
    };
    let normal = cap.normal();
    let Some(axis) = normal.iter().position(|v| v.abs() == 1.) else {
        return Ok(None);
    };
    if normal
        .iter()
        .enumerate()
        .any(|(i, v)| i != axis && *v != 0.)
    {
        return Ok(None);
    }
    let mut intrinsic = cap;
    intrinsic.origin[axis] = source.opposite_origin[axis];
    let base_frame = PlaneFrame {
        origin: position(source.placement, intrinsic.origin),
        u: direction(source.placement, intrinsic.u),
        v: direction(source.placement, intrinsic.v),
    };
    if placement_receipt(
        source.placement,
        base_frame,
        intrinsic,
        source.local.bounds()?,
    ) > stock.policy.linear_tolerance_mm / 32.
    {
        return Err(error(
            "surface relief",
            GeometryErrorKind::Unsupported,
            "The retained stock chart cannot be reconstructed within its source precision budget",
        ));
    }
    Ok(Some(Source {
        depth_mm: source.depth_mm,
        opposite: source.opposite,
        base_frame,
        contours,
    }))
}

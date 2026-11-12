//! Conservative identity of two complete finite circular-cylinder closures.
use super::{Point, cross};

fn up_sum(a: f64, b: f64) -> f64 {
    (a + b).next_up()
}
fn up_product(a: f64, b: f64) -> f64 {
    (a * b).next_up()
}
fn difference(a: f64, b: f64) -> [f64; 2] {
    let s = a - b;
    let part = s - a;
    [s, (a - (s - part)) + (-b - part)]
}
/// Bound the exact real sum of products of the supplied finite f64 values.
/// Subnormal product residuals get one outward ulp even when FMA rounds zero.
pub(in crate::step_read) fn expression_bound(terms: impl IntoIterator<Item = (f64, f64)>) -> f64 {
    let mut value = 0.;
    let mut error = 0.;
    for (a, b) in terms {
        let product = a * b;
        let sum = value + product;
        if !sum.is_finite() {
            return f64::INFINITY;
        }
        let part = sum - value;
        let residual = (value - (sum - part)) + (product - part);
        error = up_sum(
            error,
            up_sum(a.mul_add(b, -product).abs().next_up(), residual.abs()),
        );
        value = sum;
    }
    up_sum(value.abs(), error)
}
fn vector_difference(a: Point, b: Point) -> f64 {
    (0..3).fold(0., |bound, i| {
        up_sum(bound, expression_bound([(a[i], 1.), (b[i], -1.)]))
    })
}
fn endpoint_error(a: [Point; 2], b: [Point; 2], la: f64, lb: f64) -> f64 {
    let [length, low_length] = difference(la, lb);
    (0..3).fold(0., |bound, i| {
        let [origin, low_origin] = difference(a[0][i], b[0][i]);
        let [axis, low_axis] = difference(a[1][i], b[1][i]);
        up_sum(
            bound,
            expression_bound([
                (origin, 1.),
                (low_origin, 1.),
                (axis, la),
                (low_axis, la),
                (b[1][i], length),
                (b[1][i], low_length),
            ]),
        )
    })
}

pub(in crate::step_read) struct Cylinder {
    pub(in crate::step_read) frame: [Point; 3],
    pub(in crate::step_read) radius_mm: f64,
    pub(in crate::step_read) axial_mm: [f64; 2],
}

pub(in crate::step_read) fn error(a: Cylinder, b: Cylinder) -> f64 {
    let [origin, u, v] = a.frame;
    let na = cross(u, v);
    let nb = cross(b.frame[1], b.frame[2]);
    let ends = [a.axial_mm[0], a.axial_mm[1]];
    let candidate_ends = [b.axial_mm[0], b.axial_mm[1]];
    let center_error = ends
        .into_iter()
        .zip(candidate_ends)
        .map(|(lo, hi)| endpoint_error([origin, na], [b.frame[0], nb], lo, hi))
        .fold(0., f64::max);
    let radius = a.radius_mm;
    let stock_radius_mm = b.radius_mm;
    let frame_error = up_product(
        radius.max(stock_radius_mm),
        up_sum(
            vector_difference(u, b.frame[1]),
            vector_difference(v, b.frame[2]),
        ),
    );
    let radius_error = expression_bound([(radius, 1.), (stock_radius_mm, -1.)]);
    let radius_scale = up_sum(
        vector_difference(b.frame[1], [0.; 3]),
        vector_difference(b.frame[2], [0.; 3]),
    );
    up_sum(
        center_error,
        up_sum(frame_error, up_product(radius_error, radius_scale)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn product_sum_receipts_enclose_cancellation_underflow_and_overflow() {
        assert!(expression_bound([(1e16, 1.), (1., 1.), (-1e16, 1.)]) >= 1.);
        assert!(expression_bound([(f64::from_bits(1), 0.5)]) > 0.);
        assert_eq!(expression_bound([(f64::MAX, 2.)]), f64::INFINITY);
    }
}

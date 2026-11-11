#[derive(Debug)]
pub(super) enum Refusal {
    Precision,
    Angle,
}
type Point = [f64; 3];
fn add_up(a: f64, b: f64) -> f64 {
    (a + b).next_up()
}
fn mul_up(a: f64, b: f64) -> f64 {
    (a * b).next_up()
}
fn difference(a: f64, b: f64) -> (f64, f64) {
    let value = a - b;
    let z = value - a;
    let residual = (a - (value - z)) - (b + z);
    (value, residual.abs())
}
/// Certify the normal of the actual emitted f64 triangle. FMA product tails
/// and TwoSum subtraction tails bound its exact finite-vertex cross product;
/// a separately budgeted analytic-normal error is charged in the same dot test.
pub(super) fn facet(
    points: [Point; 3],
    normal: Point,
    angle: f64,
    normal_error: f64,
    wall: bool,
) -> Result<(), Refusal> {
    let edges: [[(f64, f64); 3]; 2] =
        [1, 2].map(|j| std::array::from_fn(|i| difference(points[j][i], points[0][i])));
    let mut cross = [0.; 3];
    let mut error = [0.; 3];
    for (i, (j, k)) in [(1, 2), (2, 0), (0, 1)].into_iter().enumerate() {
        let (a, ea) = edges[0][j];
        let (b, eb) = edges[1][k];
        let (c, ec) = edges[0][k];
        let (d, ed) = edges[1][j];
        let p = a * b;
        let q = c * d;
        let (value, subtract_error) = difference(p, q);
        cross[i] = value;
        let product_error = add_up(a.mul_add(b, -p).abs(), c.mul_add(d, -q).abs());
        let edge_error = add_up(
            add_up(mul_up(a.abs(), eb), mul_up(b.abs(), ea)),
            mul_up(ea, eb),
        );
        let other_error = add_up(
            add_up(mul_up(c.abs(), ed), mul_up(d.abs(), ec)),
            mul_up(ec, ed),
        );
        error[i] = add_up(
            add_up(product_error, subtract_error),
            add_up(edge_error, other_error),
        );
    }
    let length = cross[0].hypot(cross[1]).hypot(cross[2]);
    let error_l1 = error.into_iter().fold(0., add_up);
    let length_upper = add_up(mul_up(length, 1. + 8. * f64::EPSILON), error_l1);
    let dot = cross
        .into_iter()
        .zip(normal)
        .map(|(a, b)| a * b)
        .sum::<f64>();
    let dot_error = mul_up(
        cross
            .into_iter()
            .zip(normal)
            .map(|(a, b)| mul_up(a.abs(), b.abs()))
            .fold(0., add_up),
        8. * f64::EPSILON,
    );
    let total_error = add_up(
        dot_error,
        add_up(
            mul_up(error_l1, 1. + normal_error),
            mul_up(length_upper, normal_error),
        ),
    );
    let lower = (dot - total_error).next_down();
    if !length_upper.is_finite() || length <= error_l1 || !lower.is_finite() || lower <= 0. {
        return Err(Refusal::Precision);
    }
    if wall {
        // Positive radial projection on all three finite vertices proves it
        // throughout the triangle. Together with the shared closed chart this
        // preserves a one-valued radial graph, needed by the volume bound.
        for point in points {
            let radial = cross[0] * point[0] + cross[1] * point[1];
            let arithmetic = mul_up(
                add_up(
                    mul_up(cross[0].abs(), point[0].abs()),
                    mul_up(cross[1].abs(), point[1].abs()),
                ),
                8. * f64::EPSILON,
            );
            let uncertainty = add_up(
                arithmetic,
                add_up(
                    mul_up(error[0], point[0].abs()),
                    mul_up(error[1], point[1].abs()),
                ),
            );
            if (radial - uncertainty).next_down() <= 0. {
                return Err(Refusal::Angle);
            }
        }
    }
    if lower < mul_up(length_upper, angle.cos().next_up()) {
        return Err(Refusal::Angle);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finite_triangle_winding_angle_and_radial_receipts_are_independent() {
        let triangle = [[0., 0., 0.], [2., 0., 0.], [0., 3., 0.]];
        assert!(facet(triangle, [0., 0., 1.], 0.1, 0., false).is_ok());
        assert!(matches!(
            facet(triangle, [0., 0., -1.], 0.1, 0., false),
            Err(Refusal::Precision)
        ));
        assert!(matches!(
            facet([[0.; 3]; 3], [0., 0., 1.], 0.1, 0., false),
            Err(Refusal::Precision)
        ));
        assert!(matches!(
            facet([[f64::NAN; 3]; 3], [0., 0., 1.], 0.1, 0., false),
            Err(Refusal::Precision)
        ));
        assert!(matches!(
            facet(triangle, [0.6, 0., 0.8], 0.1, 0., false),
            Err(Refusal::Angle)
        ));
        let wall = [[1., -1., 0.], [1., 1., 0.], [1., -1., 1.]];
        assert!(facet(wall, [1., 0., 0.], 0.1, 0., true).is_ok());
        let inward = wall.map(|p| [-p[0], p[1], p[2]]);
        assert!(matches!(
            facet(inward, [1., 0., 0.], 0.1, 0., true),
            Err(Refusal::Angle)
        ));
        let moved = triangle.map(|p| [1e7 + p[0] * 1e-7, -1e7 + p[1] * 1e-7, 1e7]);
        assert!(facet(moved, [0., 0., 1.], 0.1, 0., false).is_ok());
    }
}

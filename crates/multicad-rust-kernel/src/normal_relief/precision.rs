//! Forward error bounds of retained floating-point charts, without fitting.
use super::*;
pub(crate) fn add_up(a: f64, b: f64) -> f64 {
    let value = a + b;
    let part = value - a;
    let error = (a - (value - part)) + (b - part);
    if error > 0. { value.next_up() } else { value }
}
pub(crate) fn mul_up(a: f64, b: f64) -> f64 {
    let value = a * b;
    if a != 0. && b != 0. && value.abs() < 1e-280 || a.mul_add(b, -value) > 0. {
        value.next_up()
    } else {
        value
    }
}
pub(crate) fn product_error(a: f64, b: f64, p: f64) -> f64 {
    let error = a.mul_add(b, -p).abs();
    if a != 0. && b != 0. && p.abs() < 1e-280 {
        error.next_up()
    } else {
        error
    }
}
pub(crate) fn dot_error(a: Point, b: Point, target: f64) -> f64 {
    let mut value = -target;
    let mut error = 0.;
    for i in 0..3 {
        let p = a[i] * b[i];
        error = add_up(error, product_error(a[i], b[i], p));
        let sum = value + p;
        let part = sum - value;
        error = add_up(error, ((value - (sum - part)) + (p - part)).abs());
        value = sum;
    }
    add_up(value.abs(), error)
}
pub(super) fn line_error(a: [f64; 2], b: [f64; 2]) -> Result<f64, Error> {
    let delta: [f64; 2] = std::array::from_fn(|i| b[i] - a[i]);
    let length = delta[0].hypot(delta[1]);
    let u = [delta[0] / length, delta[1] / length, 0.];
    let normal = [-u[1], u[0], 0.];
    let mut residual = dot_error([delta[0], delta[1], 0.], normal, 0.);
    let mut reconstruction = 0.;
    for i in 0..2 {
        let (_, error) = sum_error(b[i], -a[i]);
        residual = add_up(residual, mul_up(normal[i].abs(), error.abs()));
        reconstruction = add_up(
            reconstruction,
            add_up(
                dot_error([u[i], 0., 0.], [length, 0., 0.], delta[i]),
                error.abs(),
            ),
        );
    }
    let error = residual
        .max(reconstruction)
        .max(mul_up(dot_error(u, u, 1.), length));
    if 8. * error / length > 1e-6 {
        return Err(Error::Unresolved);
    }
    Ok(error)
}

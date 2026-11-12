//! Outward arithmetic error bounds for finite geometry charts.
type Point = [f64; 3];

pub(super) fn difference_with_error(a: Point, b: Point) -> (Point, Point) {
    let values = std::array::from_fn(|i| a[i] - b[i]);
    let errors = std::array::from_fn(|i| {
        let split = values[i] - a[i];
        ((a[i] - (values[i] - split)) + (-b[i] - split)).abs()
    });
    (values, errors)
}

pub(super) fn dot_error(a: Point, b: Point) -> (f64, f64) {
    let mut sum = 0.;
    let mut error = 0.;
    for (a, b) in a.into_iter().zip(b) {
        let product = a * b;
        let next = sum + product;
        let split = next - sum;
        error = (error + a.mul_add(b, -product).abs().next_up()).next_up();
        error = (error + ((sum - (next - split)) + (product - split)).abs().next_up()).next_up();
        sum = next;
    }
    (sum, error)
}

pub(super) fn sum_error(a: f64, b: f64) -> (f64, f64) {
    let value = a + b;
    let split = value - a;
    (value, ((a - (value - split)) + (b - split)).abs().next_up())
}

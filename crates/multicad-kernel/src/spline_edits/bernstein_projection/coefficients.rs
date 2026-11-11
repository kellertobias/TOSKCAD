use super::interval::Interval;
use crate::BSpline2d;

pub(super) type Control = [Interval; 3];
pub(super) type Curve = [Control; 4];
pub(super) const MAX_COEFFICIENTS: usize = 9;
pub(super) type Polynomial = [Interval; MAX_COEFFICIENTS];

/// Local de Boor evaluation, retaining the interpolation factors as degree-one
/// Bernstein polynomials on the chosen interval rather than evaluating at u.
pub(super) fn extract(spline: &BSpline2d, knots: &[f64], span: usize, query: [f64; 2]) -> Curve {
    let degree = spline.degree as usize;
    let [start, end] = [knots[span], knots[span + 1]].map(Interval::point);
    let mut work = [[[Interval::ZERO; 3]; 4]; 4];
    for (i, controls) in work[..=degree].iter_mut().enumerate() {
        let pole = span - degree + i;
        let weight = Interval::point(spline.weights.as_ref().map_or(1., |w| w[pole]));
        controls[0] = [
            (Interval::point(spline.poles[pole][0]) - Interval::point(query[0])) * weight,
            (Interval::point(spline.poles[pole][1]) - Interval::point(query[1])) * weight,
            weight,
        ];
    }
    for k in 1..=degree {
        for j in (k..=degree).rev() {
            let [lo, hi] = [knots[span - degree + j], knots[span + j + 1 - k]].map(Interval::point);
            let width = hi - lo;
            let left = [(hi - start) / width, (hi - end) / width];
            let right = [(start - lo) / width, (end - lo) / width];
            let prior = work[j - 1];
            let next = work[j];
            for i in 0..=k {
                work[j][i] = std::array::from_fn(|axis| {
                    let mut value = Interval::ZERO;
                    if i < k {
                        let ratio = Interval::point((k - i) as f64) / Interval::point(k as f64);
                        value = ratio * (prior[i][axis] * left[0] + next[i][axis] * right[0]);
                    }
                    if i > 0 {
                        let ratio = Interval::point(i as f64) / Interval::point(k as f64);
                        value = value
                            + ratio * (prior[i - 1][axis] * left[1] + next[i - 1][axis] * right[1]);
                    }
                    value
                });
            }
        }
    }
    work[degree]
}

const BINOMIAL: [[u32; MAX_COEFFICIENTS]; MAX_COEFFICIENTS] = [
    [1, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 1, 0, 0, 0, 0, 0, 0, 0],
    [1, 2, 1, 0, 0, 0, 0, 0, 0],
    [1, 3, 3, 1, 0, 0, 0, 0, 0],
    [1, 4, 6, 4, 1, 0, 0, 0, 0],
    [1, 5, 10, 10, 5, 1, 0, 0, 0],
    [1, 6, 15, 20, 15, 6, 1, 0, 0],
    [1, 7, 21, 35, 35, 21, 7, 1, 0],
    [1, 8, 28, 56, 70, 56, 28, 8, 1],
];

fn derivative(source: &[Interval]) -> impl Iterator<Item = Interval> + '_ {
    let degree = Interval::point((source.len() - 1) as f64);
    source
        .windows(2)
        .map(move |pair| degree * (pair[1] - pair[0]))
}

pub(super) fn product(a: &[Interval], b: &[Interval], result: &mut [Interval], subtract: bool) {
    let m = a.len() - 1;
    let n = b.len() - 1;
    for (i, &a) in a.iter().enumerate() {
        for (j, &b) in b.iter().enumerate() {
            let k = i + j;
            let ratio = Interval::point(f64::from(BINOMIAL[m][i] * BINOMIAL[n][j]))
                / Interval::point(f64::from(BINOMIAL[m + n][k]));
            let term = ratio * (a * b);
            result[k] = if subtract {
                result[k] - term
            } else {
                result[k] + term
            };
        }
    }
}

/// The sign of d(distance²)/du equals the sign of A·(A'W−AW'), because
/// the omitted factors (knot width and positive W cubed) cannot change it.
pub(super) fn stationary(controls: &Curve, degree: usize) -> Polynomial {
    let mut result = [Interval::ZERO; MAX_COEFFICIENTS];
    let w = controls.map(|p| p[2]);
    let mut dw = [Interval::ZERO; 3];
    for (out, v) in dw.iter_mut().zip(derivative(&w[..=degree])) {
        *out = v;
    }
    for axis in 0..2 {
        let a = controls.map(|p| p[axis]);
        let mut da = [Interval::ZERO; 3];
        for (out, v) in da.iter_mut().zip(derivative(&a[..=degree])) {
            *out = v;
        }
        let mut tangent = [Interval::ZERO; 6];
        product(&da[..degree], &w[..=degree], &mut tangent, false);
        product(&a[..=degree], &dw[..degree], &mut tangent, true);
        product(&a[..=degree], &tangent[..2 * degree], &mut result, false);
    }
    result
}

//! Divergence and revolution Green forms over actual source faces.
use super::*;
pub(super) const QUANTITIES: usize = 14;
pub(super) type Values = [I; QUANTITIES];
const MONOMIALS: [(usize, usize, usize); 10] = [
    (0, 0, 0),
    (1, 0, 0),
    (0, 1, 0),
    (0, 0, 1),
    (2, 0, 0),
    (0, 2, 0),
    (0, 0, 2),
    (1, 1, 0),
    (1, 0, 1),
    (0, 1, 1),
];
#[derive(Clone, Copy)]
pub(super) enum Density {
    Tensor,
    Cone,
    Cap(I),
}
fn pow<const N: usize>(v: Series<N>, n: usize) -> Series<N> {
    (0..n).fold(Series::constant(1.), |a, _| a * v)
}
fn primitive<const N: usize>(
    z: Series<N>,
    n: usize,
    c: usize,
    chart: &axial::Chart,
    center: f64,
) -> Series<N> {
    let mid = chart.radius + chart.slope * I::point(center);
    let mut value = Series::constant(0.);
    for i in 0..=n {
        let coefficient =
            I::point(trims::binomial(n, i)) * power_i(mid, n - i) * power_i(chart.slope, i)
                / I::point((i + c + 1) as f64);
        value = value + Series::interval(coefficient) * pow(z, i + c + 1);
    }
    value
}
pub(super) fn power_i(v: I, n: usize) -> I {
    (0..n).fold(I::point(1.), |a, _| a * v)
}
pub(super) fn evaluate<const N: usize>(
    density: Density,
    jet: source::Jet<N>,
    chart: &axial::Chart,
    center: f64,
) -> Result<[Series<N>; QUANTITIES], Error> {
    let source::Jet {
        p,
        tangent: du,
        normal: q,
    } = jet;
    let [x, y, z] = p;
    let z = z - Series::constant(center);
    let mut values = [Series::constant(0.); QUANTITIES];
    match density {
        Density::Tensor => {
            let radial = x * q[0] + y * q[1];
            for (i, (a, b, c)) in MONOMIALS.into_iter().enumerate() {
                values[i] = pow(x, a) * pow(y, b) * pow(z, c) * radial
                    / Series::constant((a + b + 2) as f64);
            }
            values[10] = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt();
            values[11] = x * values[10];
            values[12] = y * values[10];
            values[13] = z * values[10];
        }
        Density::Cone => {
            let rho2 = x * x + y * y;
            let rho = rho2.sqrt();
            let cos = x / rho;
            let sin = y / rho;
            let angle_rate = (x * du[1] - y * du[0]) / rho2;
            for (i, (a, b, c)) in MONOMIALS.into_iter().enumerate() {
                values[i] = -primitive(z, a + b + 2, c, chart, center)
                    * pow(cos, a)
                    * pow(sin, b)
                    * angle_rate
                    / Series::constant((a + b + 2) as f64);
            }
            let scale = (I::point(1.) + chart.slope.square()).sqrt();
            values[10] = -Series::interval(scale) * primitive(z, 1, 0, chart, center) * angle_rate;
            values[11] =
                -Series::interval(scale) * primitive(z, 2, 0, chart, center) * cos * angle_rate;
            values[12] =
                -Series::interval(scale) * primitive(z, 2, 0, chart, center) * sin * angle_rate;
            values[13] = -Series::interval(scale) * primitive(z, 1, 1, chart, center) * angle_rate;
        }
        Density::Cap(station) => {
            let radial = x * du[1] - y * du[0];
            values[10] = radial / Series::constant(2.);
            values[11] = x * radial / Series::constant(3.);
            values[12] = y * radial / Series::constant(3.);
            values[13] = Series::interval(station - I::point(center)) * values[10];
        }
    }
    for value in values {
        value.checked()?;
    }
    Ok(values)
}

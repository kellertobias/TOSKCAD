use super::*;

// Eight point Gauss-Legendre quadrature integrates degree 15 exactly.
const NODES: [f64; 4] = [
    0.183_434_642_495_649_8,
    0.525_532_409_916_329,
    0.796_666_477_413_626_7,
    0.960_289_856_497_536_3,
];
const WEIGHTS: [f64; 4] = [
    0.362_683_783_378_362,
    0.313_706_645_877_887_3,
    0.222_381_034_453_374_5,
    0.101_228_536_290_376_3,
];

pub(super) fn integrate(constant: Point, du: Point, dv: Point) -> Result<f64, Error> {
    let center = add(constant, scale(add(du, dv), 0.5));
    patch(center, scale(du, 0.5), scale(dv, 0.5), 1., &mut 4096)
}

pub(super) fn patch(
    center: Point,
    u: Point,
    v: Point,
    weight: f64,
    remaining: &mut usize,
) -> Result<f64, Error> {
    charge(remaining, 1)?;
    let magnitude = norm(center);
    let [width_u, width_v] = [norm(u), norm(v)];
    let radius = (width_u + width_v) / magnitude;
    let q = radius * (2. + radius);
    // sqrt(1 + x) truncated at degree 7 has a tail bounded by
    // q^8/(1-q). Here x is quadratic in the two parameters, so the
    // quadrature integrates the truncation exactly. Both the integral and
    // the quadrature remainder are bounded by the tail times the patch area.
    if q >= 0.5 || 2. * q.powi(8) > 1e-9 * (1. - q) {
        let half_weight = weight * 0.5;
        let (half_u, half_v, shift) = if width_u >= width_v {
            let half = scale(u, 0.5);
            (half, v, half)
        } else {
            let half = scale(v, 0.5);
            (u, half, half)
        };
        return Ok(
            patch(sub(center, shift), half_u, half_v, half_weight, remaining)?
                + patch(add(center, shift), half_u, half_v, half_weight, remaining)?,
        );
    }
    let samples = || {
        NODES
            .into_iter()
            .zip(WEIGHTS)
            .flat_map(|(x, w)| [(x, w), (-x, w)])
    };
    let mut sum = 0.;
    for (s, w) in samples() {
        let along_u = add(center, scale(u, s));
        for (t, z) in samples() {
            sum += w * z * norm(add(along_u, scale(v, t)));
        }
    }
    Ok(weight * sum * 0.25)
}

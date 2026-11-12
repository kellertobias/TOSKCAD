//! Directed complete-parameter comparison of actual decoded polynomial data.
use super::*;
use crate::thread_entry_chamfer::trim::Jet;

pub(super) const FACTORIAL: [f64; 5] = [1., 1., 2., 6., 24.];

pub(super) fn evaluate<const N: usize>(
    points: &[[I; N]],
    parameter: I,
    work: &mut Work,
) -> Result<[I; N]> {
    work.charge(points.len() * points.len() * N)?;
    let mut values = points.to_vec();
    for n in (1..values.len()).rev() {
        for i in 0..n {
            let next = values[i + 1];
            for (value, next) in values[i].iter_mut().zip(next) {
                *value = finite((I::point(1.) - parameter) * *value + parameter * next)?;
            }
        }
    }
    Ok(values[0])
}

pub(super) fn derivative<const N: usize>(points: &[[I; N]]) -> Vec<[I; N]> {
    let n = I::point((points.len() - 1) as f64);
    points
        .windows(2)
        .map(|pair| std::array::from_fn(|axis| n * (pair[1][axis] - pair[0][axis])))
        .collect()
}

pub(super) fn jets<const N: usize>(
    points: &[[I; N]],
    parameter: I,
    work: &mut Work,
) -> Result<[Jet; N]> {
    let mut result = [Jet::constant(I::point(0.)); N];
    let mut derivatives = points.to_vec();
    for (n, factorial) in FACTORIAL.into_iter().enumerate() {
        if derivatives.is_empty() {
            break;
        }
        let value = evaluate(&derivatives, parameter, work)?;
        for axis in 0..N {
            result[axis].0[n] = finite(value[axis] / I::point(factorial))?;
        }
        derivatives = derivative(&derivatives);
    }
    Ok(result)
}

pub(super) fn jet_bounds<const N: usize>(points: &[[I; N]], work: &mut Work) -> Result<[Jet; N]> {
    let mut result = [Jet::constant(I::point(0.)); N];
    let mut derivatives = points.to_vec();
    for (n, factorial) in FACTORIAL.into_iter().enumerate() {
        if derivatives.is_empty() {
            break;
        }
        work.charge(derivatives.len() * N)?;
        for axis in 0..N {
            let range = derivatives
                .iter()
                .fold(derivatives[0][axis], |range, point| range.hull(point[axis]));
            result[axis].0[n] = finite(range / I::point(factorial))?;
        }
        derivatives = derivative(&derivatives);
    }
    Ok(result)
}

/// Taylor's theorem for the difference of two complete parameterized curves.
/// Coefficients are normalized derivatives at the shared midpoint; the fourth
/// coefficient bound applies throughout the whole interval.
pub(super) fn taylor_error<const N: usize>(
    actual: [Jet; N],
    expected: [Jet; N],
    actual_fourth: [I; N],
    expected_fourth: [I; N],
) -> Result<f64> {
    let mut total = I::point(0.);
    for axis in 0..N {
        let mut error = I::point(0.);
        for n in 0..4 {
            error = error
                + (actual[axis].0[n] - expected[axis].0[n]).abs()
                    * I::point(0.5_f64.powi(n as i32));
        }
        error = error + (actual_fourth[axis] - expected_fourth[axis]).abs() / I::point(16.);
        total = total + error;
    }
    Ok(finite(total)?.hi)
}

#[derive(Clone)]
pub(super) struct Curve<const N: usize> {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub poles: Vec<[I; N]>,
}
pub(super) fn finite(value: I) -> Result<I> {
    value.checked().map_err(|_| ReadError::Unsupported)
}
pub(super) fn split<const N: usize>(
    points: &[[I; N]],
    t: I,
    work: &mut Work,
) -> Result<[Vec<[I; N]>; 2]> {
    work.charge(points.len() * points.len() * N)?;
    let mut values = points.to_vec();
    let mut left = Vec::with_capacity(points.len());
    let mut right = Vec::with_capacity(points.len());
    for n in (1..=points.len()).rev() {
        left.push(values[0]);
        right.push(values[n - 1]);
        for i in 0..n - 1 {
            let next = values[i + 1];
            for (value, next) in values[i].iter_mut().zip(next) {
                *value = finite((I::point(1.) - t) * *value + t * next)?;
            }
        }
    }
    right.reverse();
    Ok([left, right])
}
pub(super) fn restrict<const N: usize>(
    points: &[[I; N]],
    domain: [I; 2],
    work: &mut Work,
) -> Result<Vec<[I; N]>> {
    let [a, b] = domain;
    if a.lo < 0. || b.hi > 1. || a.hi > b.lo {
        return Err(ReadError::Invalid);
    }
    if a.lo == 0. && a.hi == 0. && b.lo == 1. && b.hi == 1. {
        return Ok(points.to_vec());
    }
    let right = if a.lo == 0. && a.hi == 0. {
        points.to_vec()
    } else {
        split(points, a, work)?[1].clone()
    };
    let t = if b.lo == 1. && b.hi == 1. {
        I::point(1.)
    } else {
        finite((b - a) / (I::point(1.) - a))?
    };
    // Ordered closed-domain inputs prove the exact remapped parameter is in
    // [0,1]; intersect only the outward arithmetic excess with that fact.
    let t = I::new(t.lo.max(0.), t.hi.min(1.));
    finite(t)?;
    Ok(if t.lo == 1. && t.hi == 1. {
        right
    } else {
        split(&right, t, work)?[0].clone()
    })
}

impl<const N: usize> Curve<N> {
    pub fn read(shell: &Shell<'_, '_>, id: u32) -> Result<Self> {
        let raw = super::super::import::helical_cut::decode::curve::<N>(shell, id)?;
        Self::new(
            raw.axis.degree,
            raw.axis.knots,
            raw.axis.multiplicities,
            raw.points,
        )
    }
    pub fn new(
        degree: usize,
        knots: Vec<f64>,
        multiplicities: Vec<usize>,
        points: Vec<[f64; N]>,
    ) -> Result<Self> {
        require((1..=11).contains(&degree) && knots.len() >= 2 && knots.len() <= 1025)?;
        require(
            knots.len() == multiplicities.len() && points.len() == degree * (knots.len() - 1) + 1,
        )?;
        require(
            knots.iter().all(|x| x.is_finite()) && knots.windows(2).all(|pair| pair[0] < pair[1]),
        )?;
        require(
            multiplicities
                .iter()
                .enumerate()
                .all(|(i, &n)| n == degree + usize::from(i == 0 || i + 1 == knots.len())),
        )?;
        require(points.iter().flatten().all(|x| x.is_finite()))?;
        Ok(Self {
            degree,
            knots,
            poles: points
                .into_iter()
                .map(|point| point.map(I::point))
                .collect(),
        })
    }
    pub fn domain(&self) -> [f64; 2] {
        [self.knots[0], *self.knots.last().expect("validated axis")]
    }
    pub fn span(&self, parameter: f64) -> Result<usize> {
        let index = self
            .knots
            .partition_point(|&k| k <= parameter)
            .saturating_sub(1)
            .min(self.knots.len() - 2);
        require(parameter >= self.knots[index] && parameter <= self.knots[index + 1])?;
        Ok(index)
    }
    pub fn piece(&self, span: usize) -> &[[I; N]] {
        &self.poles[span * self.degree..=span * self.degree + self.degree]
    }
    pub fn restrict(&self, domain: [f64; 2], work: &mut Work) -> Result<Vec<[I; N]>> {
        let span = self.span(domain[0])?;
        let [a, b] = [self.knots[span], self.knots[span + 1]];
        require(domain[0] >= a && domain[1] <= b && domain[0] < domain[1])?;
        let t = domain.map(|x| {
            if x == a {
                I::point(0.)
            } else if x == b {
                I::point(1.)
            } else {
                (I::point(x) - I::point(a)) / (I::point(b) - I::point(a))
            }
        });
        let t = t.map(|x| I::new(x.lo.max(0.), x.hi.min(1.)));
        restrict(self.piece(span), t, work)
    }
}

/// Actual edge and surface-isoline use the same parameter. Comparing their
/// restricted Bernstein controls proves the entire common interval; endpoint
/// coincidence or Native sewing is not used as authority.
pub(super) fn identity_error<const N: usize>(
    a: &Curve<N>,
    b: &Curve<N>,
    work: &mut Work,
) -> Result<f64> {
    require(a.degree == b.degree)?;
    let domain = a.domain();
    let other = b.domain();
    require(domain[0] >= other[0] && domain[1] <= other[1])?;
    let mut knots = a.knots.clone();
    knots.extend(
        b.knots
            .iter()
            .copied()
            .filter(|&k| k > domain[0] && k < domain[1]),
    );
    knots.sort_by(f64::total_cmp);
    knots.dedup();
    let mut error = 0_f64;
    for span in knots.windows(2) {
        let a = a.restrict([span[0], span[1]], work)?;
        let b = b.restrict([span[0], span[1]], work)?;
        for (a, b) in a.into_iter().zip(b) {
            let value = a
                .into_iter()
                .zip(b)
                .fold(I::point(0.), |sum, (a, b)| sum + (a - b).abs());
            error = error.max(finite(value)?.hi);
        }
    }
    Ok(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn line(knots: Vec<f64>) -> Curve<3> {
        let mut points = vec![[1., 2., knots[0]]];
        for pair in knots.windows(2) {
            for i in 1..=11 {
                points.push([1., 2., pair[0] + (pair[1] - pair[0]) * i as f64 / 11.]);
            }
        }
        let mut m = vec![11; knots.len()];
        m[0] = 12;
        *m.last_mut().unwrap() = 12;
        Curve::new(11, knots, m, points).unwrap()
    }
    #[test]
    fn full_parameter_binding_handles_different_partitions_and_rejects_a_hidden_interior_error() {
        let a = line(vec![1., 2., 3.]);
        let b = line(vec![0., 1.5, 4.]);
        let mut work = Work {
            remaining: 1_000_000,
            interrupt: None,
            cancelled: false,
        };
        assert!(identity_error(&a, &b, &mut work).unwrap() < 1e-11);
        let mut wrong = a.clone();
        wrong.poles[5][2] = wrong.poles[5][2] + I::point(0.03);
        assert!(identity_error(&wrong, &b, &mut work).unwrap() > 0.001);
        assert!(matches!(
            identity_error(
                &a,
                &b,
                &mut Work {
                    remaining: 1,
                    interrupt: None,
                    cancelled: false
                }
            ),
            Err(ReadError::Limit)
        ));
        assert!(
            identity_error(
                &a,
                &b,
                &mut Work {
                    remaining: 1_000_000,
                    interrupt: None,
                    cancelled: false
                }
            )
            .unwrap()
                < 1e-11
        );
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    fn work() -> Work<'static> {
        Work {
            remaining: 1_000_000,
            interrupt: None,
            cancelled: false,
        }
    }
    #[test]
    fn raw_polynomial_dimensions_order_and_finite_interpolation_are_independently_checked() {
        assert!(Curve::new(0, vec![0., 1.], vec![1, 1], vec![[0.], [1.]]).is_err());
        assert!(Curve::new(1, vec![0., 1.], vec![2], vec![[0.], [1.]]).is_err());
        assert!(Curve::new(1, vec![0., f64::NAN], vec![2, 2], vec![[0.], [1.]]).is_err());
        assert!(Curve::new(1, vec![0., 1.], vec![1, 2], vec![[0.], [1.]]).is_err());
        assert!(Curve::new(1, vec![0., 1.], vec![2, 2], vec![[0.], [f64::NAN]]).is_err());
        assert!(
            restrict(
                &[[I::point(0.)], [I::point(1.)]],
                [I::point(0.7), I::point(0.6)],
                &mut work()
            )
            .is_err()
        );
        assert!(
            evaluate(
                &[[I::point(f64::MAX)], [I::point(-f64::MAX)]],
                I::point(2.),
                &mut work()
            )
            .is_err()
        );
        assert!(
            evaluate(
                &[[I::point(f64::MAX); 3], [I::point(-f64::MAX); 3]],
                I::point(2.),
                &mut work()
            )
            .is_err()
        );
        let value = evaluate(
            &[[I::point(0.)], [I::point(1.)]],
            I::point(0.5),
            &mut work(),
        )
        .unwrap()[0];
        assert!(value.lo <= 0.5 && value.hi >= 0.5);
        assert!(
            restrict(
                &[[I::point(0.); 3], [I::point(1.); 3]],
                [I::point(0.7), I::point(0.6)],
                &mut work()
            )
            .is_err()
        );
    }
}

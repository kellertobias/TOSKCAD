//! Actual surface/pcurve composition, with a complete fourth-order remainder.
use super::{polynomial::*, *};
use crate::thread_entry_chamfer::trim::Jet;

#[derive(Clone)]
struct Grid {
    width: usize,
    height: usize,
    points: Vec<[I; 3]>,
}
impl Grid {
    fn derivative(&self, u: bool, work: &mut Work) -> Result<Self> {
        let n = if u { self.width } else { self.height };
        if n == 1 {
            return Ok(Self {
                width: self.width,
                height: self.height,
                points: vec![[I::point(0.); 3]; self.points.len()],
            });
        }
        let width = self.width - usize::from(u);
        let height = self.height - usize::from(!u);
        work.charge(width * height * 3)?;
        let mut points = Vec::with_capacity(width * height);
        for i in 0..width {
            for j in 0..height {
                let a = self.points[i * self.height + j];
                let b = self.points[(i + usize::from(u)) * self.height + j + usize::from(!u)];
                let point =
                    std::array::from_fn(|axis| I::point((n - 1) as f64) * (b[axis] - a[axis]));
                for value in point {
                    finite(value)?;
                }
                points.push(point);
            }
        }
        Ok(Self {
            width,
            height,
            points,
        })
    }
    fn evaluate(&self, u: I, v: I, work: &mut Work) -> Result<[I; 3]> {
        let rows = self
            .points
            .chunks_exact(self.height)
            .map(|row| evaluate(row, v, work))
            .collect::<Result<Vec<_>>>()?;
        evaluate(&rows, u, work)
    }
    fn bounds(&self) -> [I; 3] {
        std::array::from_fn(|axis| {
            self.points
                .iter()
                .fold(self.points[0][axis], |range, p| range.hull(p[axis]))
        })
    }
}

pub(super) struct Cell {
    partials: Vec<Grid>,
}
impl Cell {
    pub fn new(width: usize, height: usize, points: Vec<[I; 3]>, work: &mut Work) -> Result<Self> {
        require(
            (2..=12).contains(&width)
                && (2..=12).contains(&height)
                && points.len() == width * height,
        )?;
        for point in &points {
            for &value in point {
                finite(value)?;
            }
        }
        let mut partials = Vec::with_capacity(15);
        let mut u = Grid {
            width,
            height,
            points,
        };
        for i in 0..5 {
            let mut v = u.clone();
            for j in 0..5 - i {
                partials.push(v.clone());
                if j < 4 - i {
                    v = v.derivative(false, work)?;
                }
            }
            if i < 4 {
                u = u.derivative(true, work)?;
            }
        }
        Ok(Self { partials })
    }
    fn partial(&self, i: usize, j: usize) -> &Grid {
        &self.partials[5 * i - i * (i.saturating_sub(1)) / 2 + j]
    }
    fn composition(&self, uv: [Jet; 2], midpoint: bool, work: &mut Work) -> Result<[Jet; 3]> {
        let position = [uv[0].0[0], uv[1].0[0]];
        let mut delta = uv;
        delta[0].0[0] = I::point(0.);
        delta[1].0[0] = I::point(0.);
        let mut powers = [[Jet::constant(I::point(1.)); 5]; 2];
        for axis in 0..2 {
            for n in 1..5 {
                powers[axis][n] = powers[axis][n - 1] * delta[axis];
            }
        }
        let mut result = [Jet::constant(I::point(0.)); 3];
        for i in 0..5 {
            for j in 0..5 - i {
                if midpoint && i + j > 3 {
                    continue;
                }
                work.charge(3 * 25)?;
                let partial = if midpoint {
                    self.partial(i, j)
                        .evaluate(position[0], position[1], work)?
                } else {
                    self.partial(i, j).bounds()
                };
                let monomial = powers[0][i] * powers[1][j];
                for axis in 0..3 {
                    result[axis] = result[axis]
                        + monomial.scale(partial[axis] / I::point(FACTORIAL[i] * FACTORIAL[j]));
                }
            }
        }
        for jet in result {
            jet.checked().map_err(|_| ReadError::Unsupported)?;
        }
        Ok(result)
    }
    /// UV controls are normalized to this actual tensor cell and stay in its
    /// closed unit square. The common curve interval is normalized to [0,1].
    pub fn curve_error(&self, curve: &[[I; 3]], uv: &[[I; 2]], work: &mut Work) -> Result<f64> {
        require(!curve.is_empty() && !uv.is_empty())?;
        require(uv.iter().flatten().all(|v| v.lo >= 0. && v.hi <= 1.))?;
        let actual = jets(curve, I::point(0.5), work)?;
        let actual_range = jet_bounds(curve, work)?;
        let mid = jets(uv, I::point(0.5), work)?;
        let range = jet_bounds(uv, work)?;
        let expected = self.composition(mid, true, work)?;
        let expected_range = self.composition(range, false, work)?;
        taylor_error(
            actual,
            expected,
            actual_range.map(|jet| jet.0[4]),
            expected_range.map(|jet| jet.0[4]),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_tensor_composition_proves_the_full_interval_and_detects_an_interior_departure() {
        let z = [0., 0., 1.];
        let points = (0..3)
            .flat_map(|i| {
                (0..3).map(move |j| [i as f64 / 2., j as f64 / 2., z[i] + z[j]].map(I::point))
            })
            .collect();
        let mut work = Work {
            remaining: 1_000_000,
            interrupt: None,
            cancelled: false,
        };
        let cell = Cell::new(3, 3, points, &mut work).unwrap();
        let uv = [[0., 0.], [1. / 3., 0.], [2. / 3., 1. / 3.], [1., 1.]].map(|p| p.map(I::point));
        let actual = [
            [0., 0., 0.],
            [0.25, 0., 0.],
            [0.5, 1. / 6., 1. / 6.],
            [0.75, 0.5, 0.5],
            [1., 1., 2.],
        ]
        .map(|p| p.map(I::point));
        assert!(cell.curve_error(&actual, &uv, &mut work).unwrap() < 1e-11);
        let mut wrong = actual;
        wrong[2][2] = wrong[2][2] + I::point(0.02);
        assert!(cell.curve_error(&wrong, &uv, &mut work).unwrap() > 0.001);
        assert!(matches!(
            cell.curve_error(
                &actual,
                &uv,
                &mut Work {
                    remaining: 1,
                    interrupt: None,
                    cancelled: false
                }
            ),
            Err(ReadError::Limit)
        ));
        assert!(
            cell.curve_error(
                &actual,
                &uv,
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
    #[test]
    fn actual_tensor_dimensions_and_nonfinite_control_derivatives_fail_closed() {
        let mut work = Work {
            remaining: 1_000_000,
            interrupt: None,
            cancelled: false,
        };
        assert!(Cell::new(1, 2, vec![], &mut work).is_err());
        assert!(Cell::new(2, 2, vec![[I::point(f64::INFINITY); 3]; 4], &mut work).is_err());
    }
}

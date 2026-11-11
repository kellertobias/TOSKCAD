//! Certified polynomial charts of the generating patches for STEP export.
//!
//! Each patch is interpolated at Chebyshev points of the second kind: one
//! span of degree `nu` along the profile and `spans` equal spans of degree
//! `V_DEGREE` across the width. The points include every span end, so the
//! chart passes through the exact patch corners, its boundary curves are
//! the one-dimensional interpolants of the exact boundary curves, and two
//! patches sharing a junction share that boundary curve. The interpolant is
//! stored in Bernstein form: one Bézier chart per span, joined with C⁰
//! continuity across the width.
//!
//! The certified position error is the tensor interpolation remainder of
//! `wheel_bounds.rs` plus the Lebesgue-amplified residual of the stored
//! poles at the interpolation points: with node residuals `δ`, the stored
//! polynomial differs from the exact interpolant by at most `Λ_u Λ_v max δ`.
use super::wheel::{PIECES, Wheel};
pub(crate) use super::wheel_bounds::V_DEGREE;
use super::wheel_bounds::{MAX_U_DEGREE, lebesgue, lobatto};
use super::*;

/// Largest number of spans across the face width.
pub(crate) const MAX_SPANS: usize = 64;

/// Poles of one patch in local mm of sector 0, indexed `[i][j]` along the
/// profile and across the width.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Chart {
    pub(crate) degree: usize,
    pub(crate) poles: Vec<Vec<Point>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Fit {
    pub(crate) spans: usize,
    pub(crate) charts: Vec<Chart>,
    /// Proven position error of every chart in mm.
    pub(crate) bound_mm: f64,
}

/// Point of a Bézier curve by de Casteljau's algorithm.
pub(crate) fn bezier(poles: &[Point], t: f64) -> Point {
    let mut work = poles.to_vec();
    for level in 1..work.len() {
        for i in 0..work.len() - level {
            work[i] = std::array::from_fn(|k| (1. - t) * work[i][k] + t * work[i + 1][k]);
        }
    }
    work[0]
}

/// Inverse of the Bernstein collocation matrix `B_i(t_k)` at `lobatto(n)`.
fn bernstein_inverse(n: usize) -> Vec<Vec<f64>> {
    let nodes = lobatto(n);
    let mut binomial = vec![1_f64; n + 1];
    for i in 1..=n {
        binomial[i] = binomial[i - 1] * (n + 1 - i) as f64 / i as f64;
    }
    let mut a: Vec<Vec<f64>> = nodes
        .iter()
        .map(|&t| {
            (0..=n)
                .map(|i| binomial[i] * t.powi(i as i32) * (1. - t).powi((n - i) as i32))
                .collect()
        })
        .collect();
    let mut inverse: Vec<Vec<f64>> = (0..=n)
        .map(|i| (0..=n).map(|j| f64::from(u8::from(i == j))).collect())
        .collect();
    for col in 0..=n {
        let pivot = (col..=n)
            .max_by(|&x, &y| a[x][col].abs().total_cmp(&a[y][col].abs()))
            .unwrap_or(col);
        a.swap(col, pivot);
        inverse.swap(col, pivot);
        let scale = a[col][col];
        for j in 0..=n {
            a[col][j] /= scale;
            inverse[col][j] /= scale;
        }
        for row in (0..=n).filter(|&row| row != col) {
            let factor = a[row][col];
            for j in 0..=n {
                a[row][j] -= factor * a[col][j];
                inverse[row][j] -= factor * inverse[col][j];
            }
        }
    }
    inverse
}

impl Wheel {
    /// Interpolation heights across the width in module units, shared span
    /// ends listed once.
    pub(crate) fn chart_heights(&self, spans: usize) -> Vec<f64> {
        let h = self.half_width();
        let nodes = lobatto(V_DEGREE);
        let mut heights = vec![-h];
        for s in 0..spans {
            for &t in &nodes[1..] {
                heights.push(-h + 2. * h * (s as f64 + t) / spans as f64);
            }
        }
        heights
    }

    /// Exact patch values at the interpolation points in local mm of
    /// sector 0, indexed like the poles.
    pub(crate) fn chart_nodes(
        &self,
        piece: usize,
        degree: usize,
        heights: &[f64],
    ) -> Vec<Vec<Point>> {
        let m = self.module_mm();
        let profile = lobatto(degree);
        let sections: Vec<_> = heights.iter().map(|&v| self.section(v)).collect();
        // The profile end joins the next patch's start, evaluated there so
        // both patches share bit-identical junction values.
        let at = |u: f64| {
            if u == 1. && piece < 4 {
                (PIECES[piece + 1], 0.)
            } else {
                (PIECES[piece], u)
            }
        };
        profile
            .iter()
            .map(|&u| {
                let (piece, u) = at(u);
                sections
                    .iter()
                    .map(|section| {
                        let [x, y] = self.piece_point(section, piece, u);
                        [x * m, y * m, section.v * m]
                    })
                    .collect()
            })
            .collect()
    }

    /// Proven interpolation remainder of one patch chart in mm.
    pub(crate) fn chart_remainder_mm(
        &self,
        piece: usize,
        degree: usize,
        spans: usize,
    ) -> Option<f64> {
        let remainders = self.remainders()?;
        Some(remainders[piece].bound(degree, spans) * self.module_mm())
    }

    /// Smallest certified charts whose position error is within half of
    /// `tolerance_mm`, or `None` beyond the degree and span limits.
    pub(crate) fn fit(&self, tolerance_mm: f64) -> Option<Fit> {
        let target = tolerance_mm / 4.;
        let degree = |piece: usize, spans: usize| {
            (3..=MAX_U_DEGREE).find(|&n| {
                self.chart_remainder_mm(piece, n, spans)
                    .is_some_and(|bound| bound <= target)
            })
        };
        let spans = (0..=MAX_SPANS.ilog2())
            .map(|k| 1 << k)
            .find(|&spans| (0..5).all(|piece| degree(piece, spans).is_some()))?;
        let heights = self.chart_heights(spans);
        let across = bernstein_inverse(V_DEGREE);
        let h = self.half_width();
        let m = self.module_mm();
        let mut charts = Vec::with_capacity(5);
        let mut bound_mm = 0_f64;
        for piece in 0..5 {
            let nu = degree(piece, spans)?;
            let nodes = self.chart_nodes(piece, nu, &heights);
            let along = bernstein_inverse(nu);
            // Interpolate along the profile at every height. The end rows
            // are exact boundary values.
            let rows: Vec<Vec<Point>> = (0..=nu)
                .map(|i| {
                    if i == 0 || i == nu {
                        return nodes[i].clone();
                    }
                    (0..heights.len())
                        .map(|j| {
                            std::array::from_fn(|k| {
                                (0..=nu).map(|l| along[i][l] * nodes[l][j][k]).sum()
                            })
                        })
                        .collect()
                })
                .collect();
            // Interpolate each width span. Span-end poles are the shared
            // interpolation values and the height is exactly linear.
            let poles: Vec<Vec<Point>> = rows
                .iter()
                .map(|row| {
                    (0..heights.len())
                        .map(|j| {
                            let (span, local) =
                                ((j.max(1) - 1) / V_DEGREE, (j.max(1) - 1) % V_DEGREE + 1);
                            if j == 0 || local == V_DEGREE {
                                return row[j];
                            }
                            let first = span * V_DEGREE;
                            let [x, y] = std::array::from_fn(|k| {
                                (0..=V_DEGREE)
                                    .map(|l| across[local][l] * row[first + l][k])
                                    .sum()
                            });
                            let z = -h
                                + 2. * h * (span as f64 + local as f64 / V_DEGREE as f64)
                                    / spans as f64;
                            [x, y, z * m]
                        })
                        .collect()
                })
                .collect();
            let chart = Chart { degree: nu, poles };
            let residual = chart_residual(&chart, spans, &nodes);
            let remainder = self.chart_remainder_mm(piece, nu, spans)?;
            bound_mm = bound_mm.max(remainder + lebesgue(nu) * lebesgue(V_DEGREE) * residual);
            charts.push(chart);
        }
        (bound_mm <= tolerance_mm / 2.).then_some(Fit {
            spans,
            charts,
            bound_mm,
        })
    }
}

/// Largest distance between a chart and values at its interpolation
/// points, including a bound on the de Casteljau rounding error.
pub(crate) fn chart_residual(chart: &Chart, spans: usize, nodes: &[Vec<Point>]) -> f64 {
    let profile = lobatto(chart.degree);
    let width = lobatto(V_DEGREE);
    let scale = chart
        .poles
        .iter()
        .flatten()
        .flatten()
        .fold(0_f64, |a, &b| a.max(b.abs()));
    let rounding = 4. * (chart.degree + V_DEGREE + 2) as f64 * f64::EPSILON * scale;
    let mut worst = 0_f64;
    for s in 0..spans {
        let first = s * V_DEGREE;
        // Evaluate each span column along the profile, then across.
        let columns: Vec<Vec<Point>> = profile
            .iter()
            .map(|&u| {
                (first..=first + V_DEGREE)
                    .map(|j| {
                        let column: Vec<Point> = chart.poles.iter().map(|row| row[j]).collect();
                        bezier(&column, u)
                    })
                    .collect()
            })
            .collect();
        for (i, column) in columns.iter().enumerate() {
            for (l, &t) in width.iter().enumerate() {
                let value = bezier(column, t);
                worst = worst.max(norm(sub(value, nodes[i][first + l])));
            }
        }
    }
    worst + rounding
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worm_gear::wheel::tests::{spec, wheel};

    #[test]
    fn charts_interpolate_patches_within_their_certified_bound() {
        let wheel = wheel(spec(1., 40, 1.5, 4., 0.05));
        let fit = wheel.fit(1e-7).unwrap();
        assert!(fit.bound_mm <= 0.5e-7, "{}", fit.bound_mm);
        let h = wheel.half_width();
        // Dense off-node samples stay within the proven bound.
        for (piece, chart) in fit.charts.iter().enumerate() {
            let n = chart.poles[0].len();
            assert_eq!(n, fit.spans * V_DEGREE + 1);
            for a in 0..=12 {
                let u = f64::from(a) / 12.;
                let column: Vec<Point> = (0..n)
                    .map(|j| bezier(&chart.poles.iter().map(|row| row[j]).collect::<Vec<_>>(), u))
                    .collect();
                for b in 0..=24 {
                    let t = f64::from(b) / 24.;
                    let span = ((t * fit.spans as f64) as usize).min(fit.spans - 1);
                    let local = t * fit.spans as f64 - span as f64;
                    let p = bezier(&column[span * V_DEGREE..=(span + 1) * V_DEGREE], local);
                    let v = -h + 2. * h * t;
                    let section = wheel.section(v);
                    let [x, y] = wheel.piece_point(&section, PIECES[piece], u);
                    let error = (p[0] - x).hypot(p[1] - y).hypot(p[2] - v);
                    assert!(error <= fit.bound_mm, "{piece} {u} {v} {error}");
                }
            }
            // Corner poles are the exact patch corners.
            let section = wheel.section(-h);
            let [x, y] = wheel.piece_point(&section, PIECES[piece], 0.);
            assert_eq!(chart.poles[0][0], [x, y, -h]);
        }
        // An unattainable tolerance exceeds the degree and span limits.
        assert!(wheel.fit(1e-300).is_none());
        let inverse = bernstein_inverse(1);
        assert_eq!(inverse, vec![vec![1., 0.], vec![0., 1.]]);
    }
}

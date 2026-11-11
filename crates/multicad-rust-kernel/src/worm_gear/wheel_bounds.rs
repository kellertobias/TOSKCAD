//! Rigorous derivative bounds of the generating patches.
//!
//! A patch point is `R(θ) Q` with `Q` and `θ` smooth in the patch
//! parameters `(u, v) ∈ [0, 1] × [-h, h]`. A rotation by a constant keeps
//! derivative lengths, so each bound evaluates `R(θ - θ₀) Q` with a fixed
//! `θ₀` per cell.
//!
//! The domain is covered by a grid of cells. Let `F_ab` denote the Taylor
//! coefficient `∂u^a ∂v^b f / (a! b!)`. On a cell with center `c` and
//! half-widths `(ρu, ρv)` the mean value theorem gives, per coordinate,
//!
//! `|∂u^a ∂v^b f| ≤ a! b! (|F_ab(c)| + (a+1) ρu |F_(a+1)b| + (b+1) ρv |F_a(b+1)|)`
//!
//! where the last two coefficients are enclosed over the whole cell by
//! interval Taylor arithmetic (`interval.rs`). The center term uses point
//! intervals, so overestimation only enters through the cell remainder,
//! which shrinks with the cell size. The largest value over all cells
//! bounds the derivative on the whole patch.
//!
//! The flank patches depend on the exit radius `c(v)` where the flank leaves
//! the blank, the root of `g(r, v) = |flank(r, v)|² - blank²`. Over a height
//! cell `V` the bound first proves `∂g/∂r < 0` on the whole cutter range, so
//! the root is unique, then finds an interval `C` with `g(C.lo, V) > 0` and
//! `g(C.hi, V) < 0`, so `c(V) ⊂ C`. Higher Taylor coefficients of `c` follow
//! from `g(c(v+t), v+t) = 0`: the coefficient `k` of `g` evaluated with
//! the first `k` coefficients of `c` equals `-∂g/∂r · c_k`.
use super::interval::{Interval, Series, factorial};
use super::jet::Real;
use super::wheel::{CUTTER_TIP, PIECES, Piece, WORM_ROOT, Wheel, turn};

/// Profile and height cells for the section motion and bending bounds.
const SECTION_CELLS: [usize; 2] = [32, 64];
/// Profile and height cells for coefficients along the profile and across
/// the width. Narrow height cells keep high-order enclosures tight.
const U_FIT_CELLS: [usize; 2] = [8, 16];
const V_FIT_CELLS: [usize; 2] = [8, 128];
/// Highest profile degree, below the STEP and OCCT B-spline limit of 25.
pub(crate) const MAX_U_DEGREE: usize = 24;
const U_ORDERS: usize = MAX_U_DEGREE + 2;
/// Degree of each span across the face width.
pub(crate) const V_DEGREE: usize = 8;
/// Cutter-range cells proving a unique exit radius.
const MONOTONE_CELLS: usize = 16;
const MONOTONE_DEPTH: u32 = 6;

/// Taylor coefficients in `u` (outer) and `v` (inner).
type Pair<const NU: usize, const NV: usize> = Series<Series<Interval, NV>, NU>;

pub(crate) fn cells(from: f64, to: f64, count: usize) -> impl Iterator<Item = Interval> {
    let at = move |i: usize| from + (to - from) * i as f64 / count as f64;
    (0..count).map(move |i| Interval::new(at(i), at(i + 1)))
}

fn side(piece: Piece) -> f64 {
    if piece == Piece::FlankPlus { -1. } else { 1. }
}

impl Wheel {
    /// `|flank(r, v)|² - blank²`, positive before the flank leaves the blank.
    fn exit_gap<T: Real>(&self, r: T, v: T, side: f64) -> T {
        let [x, y] = self.flank(r, v, side);
        x * x + y * y - T::constant(self.blank() * self.blank())
    }

    /// Exit radius and its height derivative at one section.
    pub(crate) fn crossing_slope(&self, v: f64, side: f64) -> Series<f64, 2> {
        type S = Series<f64, 2>;
        let c = self.crossing(v, side);
        let rate = self.exit_gap(S::variable(c), S::lift(v), side).0[1];
        let lean = self.exit_gap(S::lift(c), S::variable(v), side).0[1];
        Series([c, -lean / rate])
    }

    /// Taylor coefficients of the exit radius enclosed over a height cell.
    pub(crate) fn crossing_enclosure<const N: usize>(
        &self,
        cell: Interval,
        side: f64,
    ) -> Option<Series<Interval, N>> {
        type S = Series<Interval, 2>;
        let rate = |r: Interval| self.exit_gap(S::variable(r), S::lift(cell), side).0[1];
        // Bisect cutter-range cells whose enclosure does not yet prove the
        // sign, down to a bounded depth.
        let mut pending: Vec<(Interval, u32)> = cells(WORM_ROOT, CUTTER_TIP, MONOTONE_CELLS)
            .map(|r| (r, 0))
            .collect();
        while let Some((r, depth)) = pending.pop() {
            if rate(r).hi >= 0. {
                (depth < MONOTONE_DEPTH).then_some(())?;
                let mid = (r.lo + r.hi) / 2.;
                pending.extend([
                    (Interval::new(r.lo, mid), depth + 1),
                    (Interval::new(mid, r.hi), depth + 1),
                ]);
            }
        }
        let mid = (cell.lo + cell.hi) / 2.;
        let center = self.crossing(mid, side);
        let reach = self.crossing_slope(mid, side).0[1].abs() * (cell.hi - cell.lo) / 2. + 1e-12;
        let gap = |r: f64| self.exit_gap(Interval::point(r), cell, side);
        let enclosure = (0..48)
            .map(|k| 1.25 * reach * 2_f64.powi(k))
            .map(|d| Interval::new(center - d, center + d))
            .find(|c| gap(c.lo).lo > 0. && gap(c.hi).hi < 0.)?;
        let slope = rate(enclosure);
        let v = Series::<Interval, N>::variable(cell);
        let mut r = Series::<Interval, N>::lift(enclosure);
        for k in 1..N {
            let g = self.exit_gap(r, v, side);
            r.0[k] = -(g.0[k] / slope);
        }
        Some(r)
    }

    /// Patch coefficients over a cell, rotated back by the fixed angle `base`.
    /// `exit` encloses the exit-radius series over the cell's heights.
    fn coefficients<const NU: usize, const NV: usize>(
        &self,
        piece: Piece,
        [u, v]: [Interval; 2],
        exit: Series<Interval, NV>,
        base: f64,
    ) -> [Pair<NU, NV>; 2] {
        let (q, angle) = self.polar_patch(
            piece,
            Pair::variable(Series::lift(u)),
            Pair::lift(Series::variable(v)),
            Pair::lift(exit),
        );
        turn(q, angle - Pair::constant(base))
    }

    /// Exit-radius series over a height cell, or zero for pieces that do
    /// not depend on it.
    fn exit<const N: usize>(&self, piece: Piece, v: Interval) -> Option<Series<Interval, N>> {
        if piece.flank_side().is_some() {
            self.crossing_enclosure(v, side(piece))
        } else {
            Some(Series::constant(0.))
        }
    }

    /// Per-coordinate bounds of `|∂u^a ∂v^b P|` over one whole patch for each
    /// requested order. Every order needs `a + 1 < NU` and `b + 1 < NV`.
    fn patch_bounds<const NU: usize, const NV: usize, const K: usize>(
        &self,
        piece: Piece,
        orders: [[usize; 2]; K],
        [u_cells, v_cells]: [usize; 2],
    ) -> Option<[[f64; 2]; K]> {
        let h = self.half_width();
        let mut bounds = [[0_f64; 2]; K];
        for v in cells(-h, h, v_cells) {
            let middle = (v.lo + v.hi) / 2.;
            let exits = [
                self.exit::<NV>(piece, Interval::point(middle))?,
                self.exit(piece, v)?,
            ];
            let crossing = self.crossing(middle, side(piece));
            for u in cells(0., 1., u_cells) {
                let mid = [(u.lo + u.hi) / 2., middle];
                let radius = [u, v].map(|x| (x.hi - x.lo) / 2.);
                let base = self.polar_patch(piece, mid[0], mid[1], crossing).1;
                let center =
                    self.coefficients::<NU, NV>(piece, mid.map(Interval::point), exits[0], base);
                let cell = self.coefficients::<NU, NV>(piece, [u, v], exits[1], base);
                for (bound, [a, b]) in bounds.iter_mut().zip(orders) {
                    for k in 0..2 {
                        let value = center[k].0[a].0[b].magnitude()
                            + (a + 1) as f64 * cell[k].0[a + 1].0[b].magnitude() * radius[0]
                            + (b + 1) as f64 * cell[k].0[a].0[b + 1].magnitude() * radius[1];
                        bound[k] = bound[k].max(value * factorial(a) * factorial(b));
                    }
                }
            }
        }
        Some(bounds)
    }

    /// Upper bounds of `|∂P/∂v|` and `|∂²P/∂u²|` over every generating
    /// patch, in module units.
    pub(crate) fn section_bounds(&self) -> Option<[f64; 2]> {
        *self.section_bounds.get(|| self.compute_section_bounds())
    }

    fn compute_section_bounds(&self) -> Option<[f64; 2]> {
        let mut result = [0_f64; 2];
        for piece in PIECES {
            let bounds = self.patch_bounds::<4, 3, 2>(piece, [[0, 1], [2, 0]], SECTION_CELLS)?;
            for (slot, [x, y]) in result.iter_mut().zip(bounds) {
                *slot = slot.max(x.hypot(y));
            }
        }
        Some(result)
    }

    /// Largest Taylor coefficient magnitudes `|F_k|` of one patch along `u`
    /// (or along `v` when `along_v`), per coordinate in a frame rotated by a
    /// fixed angle, over the whole patch domain. A fixed rotation keeps the
    /// Euclidean length of every interpolation remainder.
    fn directional<const N: usize>(&self, piece: Piece, along_v: bool) -> Option<[[f64; 2]; N]> {
        type S<const N: usize> = Series<Interval, N>;
        let h = self.half_width();
        let base = self
            .polar_patch(piece, 0.5, 0., self.crossing(0., side(piece)))
            .1;
        let [u_cells, v_cells] = if along_v { V_FIT_CELLS } else { U_FIT_CELLS };
        let mut bounds = [[0_f64; 2]; N];
        for v in cells(-h, h, v_cells) {
            let exit = self.exit::<N>(piece, v)?;
            for u in cells(0., 1., u_cells) {
                let (q, angle) = if along_v {
                    self.polar_patch(piece, S::lift(u), S::variable(v), exit)
                } else {
                    self.polar_patch(piece, S::variable(u), S::lift(v), S::lift(exit.0[0]))
                };
                let point = turn(q, angle - S::constant(base));
                for (k, bound) in bounds.iter_mut().enumerate() {
                    for c in 0..2 {
                        bound[c] = bound[c].max(point[c].0[k].magnitude());
                    }
                }
            }
        }
        Some(bounds)
    }

    /// Taylor coefficient bounds of every patch for interpolation remainders.
    pub(crate) fn remainders(&self) -> Option<&[Remainder; 5]> {
        self.remainders
            .get(|| self.compute_remainders().map(Box::new))
            .as_deref()
    }

    fn compute_remainders(&self) -> Option<[Remainder; 5]> {
        let mut result = Vec::with_capacity(5);
        for piece in PIECES {
            result.push(Remainder {
                u: self.directional(piece, false)?,
                v: self.directional(piece, true)?,
                half_width: self.half_width(),
            });
        }
        result.try_into().ok()
    }
}

/// Upper bound of the Lebesgue constant of degree-`n` interpolation at the
/// Chebyshev points `cos(jπ/n)` (Trefethen, Approximation Theory and
/// Approximation Practice, Theorem 15.2).
pub(crate) fn lebesgue(n: usize) -> f64 {
    2. / std::f64::consts::PI * ((n + 1) as f64).ln() + 1.
}

/// Chebyshev points of the second kind on `[0, 1]`, endpoints included.
pub(crate) fn lobatto(n: usize) -> Vec<f64> {
    (0..=n)
        .map(|j| {
            // Reflect so both halves round identically and the ends are exact.
            let k = j.min(n - j);
            let x = (1. - (std::f64::consts::PI * k as f64 / n as f64).cos()) / 2.;
            if k == j { x } else { 1. - x }
        })
        .collect()
}

/// Remainder of degree-`n` interpolation at `lobatto(n)` mapped to an
/// interval of half-length `ℓ`, given `max |F_(n+1)|` in the same variable.
/// The node polynomial on `[-1, 1]` is `(x² - 1) U_(n-1)(x) / 2^(n-1)`,
/// whose magnitude is `|sin θ sin nθ| / 2^(n-1) ≤ 2^(1-n)` with `x = cos θ`,
/// so the Cauchy remainder is at most `ℓ^(n+1) 2^(1-n) max |F_(n+1)|`.
fn interpolation_error(n: usize, half_length: f64, coefficient: [f64; 2]) -> f64 {
    half_length.powi(n as i32 + 1) * 2_f64.powi(1 - n as i32) * coefficient[0].hypot(coefficient[1])
}

/// Proven Taylor coefficient bounds of one generating patch, module units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Remainder {
    u: [[f64; 2]; U_ORDERS],
    v: [[f64; 2]; V_DEGREE + 2],
    half_width: f64,
}

impl Remainder {
    /// Tensor interpolation error in module units for one span of degree
    /// `nu` along the profile and `spans` spans of degree `V_DEGREE` across
    /// the width. With `I = I_u I_v`, `f - I f = (f - I_u f) + I_u (f - I_v f)`
    /// and `|I_u g| ≤ Λ_u max |g|`.
    pub(crate) fn bound(&self, nu: usize, spans: usize) -> f64 {
        let across = interpolation_error(
            V_DEGREE,
            self.half_width / spans as f64,
            self.v[V_DEGREE + 1],
        );
        interpolation_error(nu, 0.5, self.u[nu + 1]) + lebesgue(nu) * across
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worm_gear::jet::Jet;
    use crate::worm_gear::wheel::tests::{spec, wheel};

    #[test]
    fn exit_radius_series_enclose_the_bisected_root_and_its_slope() {
        let wheel = wheel(spec(1., 40, 1.5, 4., 0.05));
        for cell in [Interval::new(0.2, 0.25), Interval::point(0.)] {
            for side in [1., -1.] {
                let series = wheel.crossing_enclosure::<3>(cell, side).unwrap();
                for v in [cell.lo, (cell.lo + cell.hi) / 2., cell.hi] {
                    let slope = wheel.crossing_slope(v, side);
                    assert!(series.0[0].contains(slope.0[0]));
                    assert!(series.0[1].contains(slope.0[1]));
                    // The slope matches a centered difference of the bisection.
                    let d = 1e-6;
                    let fd = (wheel.crossing(v + d, side) - wheel.crossing(v - d, side)) / (2. * d);
                    assert!((fd - slope.0[1]).abs() < 1e-6);
                }
            }
        }
        // Far outside the generating range no exit enclosure exists.
        assert!(
            wheel
                .crossing_enclosure::<2>(Interval::new(-9., 9.), 1.)
                .is_none()
        );
        assert!(
            wheel
                .crossing_enclosure::<2>(Interval::new(5.9, 6.), 1.)
                .is_none()
        );
    }

    #[test]
    fn derivative_bounds_dominate_dense_differences() {
        let wheel = wheel(spec(1., 40, 1.5, 4., 0.05));
        let [motion, bend] = wheel.section_bounds().unwrap();
        let h = wheel.half_width();
        let mut sampled = [0_f64; 2];
        for piece in PIECES {
            for i in 0..=40 {
                let v = -h + 2. * h * f64::from(i) / 40.;
                let section = wheel.section(v);
                let exit = wheel.crossing_slope(v, side(piece));
                for j in 0..=40 {
                    let u = f64::from(j) / 40.;
                    let p = wheel.patch(
                        piece,
                        Jet::constant(u),
                        Jet::variable(v, 0),
                        Jet {
                            value: exit.0[0],
                            d: [exit.0[1], 0.],
                        },
                    );
                    sampled[0] = sampled[0].max(p[0].d[0].hypot(p[1].d[0]));
                    let d = 1e-4;
                    let [a, b, c] =
                        [u - d, u, u + d].map(|t| wheel.piece_point(&section, piece, t));
                    let second = [0, 1].map(|k| (a[k] - 2. * b[k] + c[k]) / (d * d));
                    sampled[1] = sampled[1].max(second[0].hypot(second[1]));
                }
            }
        }
        for r in wheel.remainders().unwrap() {
            // More degree and more spans only tighten the remainder.
            assert!(r.bound(16, 2) < r.bound(12, 1) && r.bound(24, 8) < r.bound(16, 2));
            assert!(r.bound(24, 8) < 1e-9, "{}", r.bound(24, 8));
        }
        assert!(lebesgue(16) < 3.);
        let nodes = lobatto(2);
        assert!(nodes[0] == 0. && (nodes[1] - 0.5).abs() < 1e-15 && nodes[2] == 1.);
        assert!(
            motion >= sampled[0] && motion < 1.5 * sampled[0],
            "{motion} {sampled:?}"
        );
        assert!(
            bend >= sampled[1] && bend < 1.5 * sampled[1],
            "{bend} {sampled:?}"
        );
    }
}

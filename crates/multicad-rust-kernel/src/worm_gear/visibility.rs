//! Exact visibility of a generated wheel seen along its axis.
//!
//! Work in sector 0 and module units. At height `v` the space is star
//! shaped: its boundary angle increases strictly along the five pieces
//! (condition A), so inside the window `low(v) < θ < high(v)` the boundary
//! is a polar graph `r = ρ(θ, v)` and the space is `{r > ρ(θ, v)}`.
//!
//! Structure lemma. Assume, over the whole patch domain,
//!
//! - A: `P × P_u > 0` on every piece;
//! - T: `P × P_v > 0` on every piece, so every boundary point turns
//!   counterclockwise as the height grows and the window ends increase;
//! - B: the radius decreases along the minus flank and increases along the
//!   plus flank (`P · P_u` of fixed sign); on the edge transitions
//!   `r² = root(v)² + (travel)²` is monotone because the sweep length `w`
//!   never vanishes, and the torus has constant radius;
//! - N: the axial normal component `n = P_u × P_v` is negative on the minus
//!   flank and positive on the plus flank; on the torus `n = -φ_u root root'`
//!   has the sign of `-v`; on both edge transitions every cell where `n` may
//!   vanish has `∂n/∂v < 0` and `∂n/∂u > 0`.
//!
//! Then for every angle `θ`, `v ↦ ρ(θ, v)` is quasiconvex on its window.
//! Proof: by T a curve of constant `θ` moves to lower chain positions as `v`
//! grows. By N each edge carries one sign change of `n`, from positive below
//! to negative above, along a height `σ(u)` that increases with `u`; on the
//! edge ends that meet the torus `n = ±root'(v) w (1 - root/R)` vanishes
//! exactly at `v = 0`, matching the torus. So `σ` is a nondecreasing
//! function of the chain position (−∞ on the minus flank, +∞ on the plus
//! flank), and along the curve `σ - v` strictly decreases: `n` changes sign
//! at most once, from positive to negative. Since
//! `∂ρ/∂v|θ = -n r / (P × P_u)`, `ρ` first decreases and then increases.
//!
//! Hence every axial line meets the space of the slab in one open interval
//! of heights, and a boundary point below the near end `v_n` is visible from
//! that end exactly when its axial projection lies in the open space of the
//! near section. The same holds looking from either end. Lines are the near
//! face edges, the far face edges and junction curves, and the torus
//! silhouette at `v = 0`. Flanks and edge transitions carry no silhouette
//! because `n` vanishes there only on curves crossed transversally.
//!
//! Circles are classified in closed form: the open near space meets a circle
//! of radius `r` in the angles between the minus and plus boundary at that
//! radius. Other curves are classified by interval enclosures of their
//! polar coordinates against the near boundary radius, which is decreasing,
//! then constant on the torus, then increasing in angle (A, B). A cell is
//! decided when its enclosure lies strictly inside or outside; undecided
//! cells are halved until their image is smaller than the stated
//! resolution, which bounds where a visibility transition can lie.
use super::interval::{Interval, Series};
use super::jet::Real;
use super::wheel::{CUTTER_TIP, PIECES, Piece, Section, Wheel, turn};
use super::wheel_bounds::cells;
use super::*;

const STRUCTURE_CELLS: [usize; 2] = [8, 16];
const STRUCTURE_DEPTH: u32 = 7;
/// Initial cells per classified curve.
const CURVE_CELLS: usize = 16;

type Pair = Series<Series<Interval, 3>, 3>;

/// One projected line of an axial wheel view in local mm.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewCurve {
    /// Arc about the wheel axis at local height `z_mm`.
    Arc {
        z_mm: f64,
        radius_mm: f64,
        start_rad: f64,
        sweep_rad: f64,
        hidden: bool,
    },
    Polyline {
        points_mm: Vec<Point>,
        hidden: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewError {
    Invalid,
    /// The structure lemma could not be proven for this wheel.
    Unproven,
    Limit,
}

fn cross([a, b]: [Interval; 2], [c, d]: [Interval; 2]) -> Interval {
    a * d - b * c
}

fn dot([a, b]: [Interval; 2], [c, d]: [Interval; 2]) -> Interval {
    a * c + b * d
}

fn side(piece: Piece) -> f64 {
    if piece == Piece::FlankPlus { -1. } else { 1. }
}

/// Polar radius and angle of a polar patch point.
fn polar<T: Real>((q, angle): ([T; 2], T), atan: impl Fn(T) -> T) -> [T; 2] {
    [
        (q[0] * q[0] + q[1] * q[1]).sqrt(),
        angle + atan(q[1] / q[0]),
    ]
}

/// Runs `(from, to, visible)` of one classified curve parameter.
type Runs = Vec<(f64, f64, bool)>;

/// A curve of the view: its point map, its classified runs (all visible
/// when absent) and its chord count over the whole parameter range.
type Traced<'a> = (Box<dyn Fn(f64) -> Point + 'a>, Option<Runs>, f64);

impl Wheel {
    /// Whether the structure lemma holds for this wheel.
    pub(crate) fn view_structure(&self) -> bool {
        *self.view_structure.get(|| self.prove_view_structure())
    }

    fn prove_view_structure(&self) -> bool {
        self.prove_view_structure_to(STRUCTURE_DEPTH)
    }

    fn prove_view_structure_to(&self, limit: u32) -> bool {
        let h = self.half_width();
        let sweeps = cells(-h, h, 64).all(|v| {
            [1., -1.].into_iter().all(|side| {
                let w = self.contact(Interval::point(CUTTER_TIP), v, side)[2];
                w.lo > 0. || w.hi < 0.
            })
        });
        let [nu, nv] = STRUCTURE_CELLS;
        sweeps
            && PIECES.into_iter().all(|piece| {
                let mut pending: Vec<_> = cells(-h, h, nv)
                    .flat_map(|v| cells(0., 1., nu).map(move |u| ([u, v], 0)))
                    .collect();
                while let Some(([u, v], depth)) = pending.pop() {
                    if self.structure_holds(piece, [u, v]) {
                        continue;
                    }
                    if depth == limit {
                        return false;
                    }
                    let halves = |x: Interval| {
                        let mid = (x.lo + x.hi) / 2.;
                        [Interval::new(x.lo, mid), Interval::new(mid, x.hi)]
                    };
                    for a in halves(u) {
                        pending.extend(halves(v).map(|b| ([a, b], depth + 1)));
                    }
                }
                true
            })
    }

    /// Conditions A, T, B and N on one parameter cell.
    fn structure_holds(&self, piece: Piece, [u, v]: [Interval; 2]) -> bool {
        let exit = if piece.flank_side().is_some() {
            self.crossing_enclosure::<3>(v, side(piece))
        } else {
            Some(Series::constant(0.))
        };
        exit.is_some_and(|exit| {
            let middle = [(u.lo + u.hi) / 2., (v.lo + v.hi) / 2.];
            let crossing = self.crossing(middle[1], side(piece));
            let base = self.polar_patch(piece, middle[0], middle[1], crossing).1;
            let (q, angle) = self.polar_patch(
                piece,
                Pair::variable(Series::lift(u)),
                Pair::lift(Series::variable(v)),
                Pair::lift(exit),
            );
            let [x, y] = turn(q, angle - Pair::constant(base));
            let at = |a: usize, b: usize| [x.0[a].0[b], y.0[a].0[b]];
            let twice = |a: usize, b: usize| at(a, b).map(|c| c * Interval::point(2.));
            let [p, pu, pv, puv] = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)];
            let positive = |i: Interval| i.lo > 0.;
            let negative = |i: Interval| i.hi < 0.;
            let normal = cross(pu, pv);
            // The axial normal component has a fixed sign, or the cell holds
            // one transversal sign change moving up the chain.
            let along = || cross(twice(2, 0), pv) + cross(pu, puv);
            let up = || cross(puv, pv) + cross(pu, twice(0, 2));
            let shape = piece == Piece::Torus
                || positive(normal)
                || negative(normal)
                || (positive(along()) && negative(up()));
            let radial = match piece {
                Piece::FlankMinus => negative(dot(p, pu)),
                Piece::FlankPlus => positive(dot(p, pu)),
                _ => true,
            };
            positive(cross(p, pu)) && positive(cross(p, pv)) && radial && shape
        })
    }

    /// Axial normal component `P_u × P_v` of a patch point.
    fn axial_normal(&self, piece: Piece, u: f64, v: f64) -> f64 {
        use super::jet::Jet;
        let exit = self.crossing_slope(v, side(piece));
        let crossing = Jet {
            value: exit.0[0],
            d: [0., exit.0[1]],
        };
        let [x, y] = self.patch(piece, Jet::variable(u, 0), Jet::variable(v, 1), crossing);
        x.d[0] * y.d[1] - y.d[0] * x.d[1]
    }

    /// Contour generator `n = 0` of one side piece inside heights `[lo, hi]`
    /// as `(u, v)` samples. The structure lemma puts at most one zero on
    /// every profile line, positive below and negative above, at a height
    /// that increases with `u`, so the contour is one monotone arc.
    fn contour(&self, piece: Piece, [lo, hi]: [f64; 2], tolerance_mm: f64) -> Vec<[f64; 2]> {
        let bisect = |mut a: f64, mut b: f64, upper: &dyn Fn(f64) -> bool| {
            for _ in 0..60 {
                let mid = (a + b) / 2.;
                if upper(mid) {
                    b = mid;
                } else {
                    a = mid;
                }
            }
            (a + b) / 2.
        };
        // The zero lies above `lo` for u past `start` and below `hi` before `end`.
        let start = bisect(0., 1., &|u| self.axial_normal(piece, u, lo) > 0.);
        let end = bisect(0., 1., &|u| self.axial_normal(piece, u, hi) >= 0.);
        let height = |u: f64| bisect(lo, hi, &|v| self.axial_normal(piece, u, v) < 0.);
        if start >= end {
            return Vec::new();
        }
        let m = self.module_mm();
        let at = |t: f64| {
            let u = start + (end - start) * t;
            let v = height(u);
            let [x, y] = self.patch(piece, u, v, self.crossing(v, side(piece)));
            [x * m, y * m, v * m]
        };
        let count = chord_count(at, tolerance_mm, 0.3, 1 << 20).unwrap_or(1 << 20);
        (0..=count)
            .map(|i| {
                let u = start + (end - start) * i as f64 / count as f64;
                [u, height(u)]
            })
            .collect()
    }

    /// Boundary radius of sector 0 at an angle inside the window.
    pub(crate) fn boundary_radius(&self, section: &Section, theta: f64) -> f64 {
        let angle = |piece: Piece, u: f64| {
            let [x, y] = self.piece_point(section, piece, u);
            y.atan2(x)
        };
        let piece = PIECES
            .into_iter()
            .find(|&piece| angle(piece, 1.) >= theta)
            .unwrap_or(Piece::FlankPlus);
        let (mut lo, mut hi) = (0., 1.);
        for _ in 0..60 {
            let mid = (lo + hi) / 2.;
            if angle(piece, mid) < theta {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let [x, y] = self.piece_point(section, piece, (lo + hi) / 2.);
        x.hypot(y)
    }

    /// Angle of the minus or plus boundary at a radius between the root and
    /// the blank; the radius is monotone along each side (B).
    fn side_angle(&self, section: &Section, plus: bool, radius: f64) -> f64 {
        let pieces = if plus {
            [Piece::EdgePlus, Piece::FlankPlus]
        } else {
            [Piece::FlankMinus, Piece::EdgeMinus]
        };
        let point = |s: f64| {
            let (piece, u) = if s < 1. {
                (pieces[0], s)
            } else {
                (pieces[1], s - 1.)
            };
            self.piece_point(section, piece, u)
        };
        let (mut lo, mut hi) = (0., 2.);
        for _ in 0..64 {
            let mid = (lo + hi) / 2.;
            let [x, y] = point(mid);
            if (x.hypot(y) < radius) == plus {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let [x, y] = point((lo + hi) / 2.);
        y.atan2(x)
    }

    /// Angles of a circle of `radius` inside the open space of a section.
    fn open_arc(&self, section: &Section, radius: f64) -> Option<[f64; 2]> {
        if radius <= self.root_radius(section.v) {
            return None;
        }
        if radius >= self.blank() {
            return Some(section.exit);
        }
        Some([false, true].map(|plus| self.side_angle(section, plus, radius)))
    }

    /// Range of the boundary radius over an angle interval in the window.
    fn radius_range(
        &self,
        section: &Section,
        [torus_start, torus_end]: [f64; 2],
        th: Interval,
    ) -> [f64; 2] {
        let ends = [th.lo, th.hi].map(|t| self.boundary_radius(section, t));
        let floor = th.hi >= torus_start && th.lo <= torus_end;
        let low = if floor {
            self.root_radius(section.v)
        } else {
            ends[0].min(ends[1])
        };
        [low, ends[0].max(ends[1])]
    }

    fn in_space(&self, section: &Section, [r, theta]: [f64; 2]) -> bool {
        let [low, high] = section.exit;
        theta > low && theta < high && r > self.boundary_radius(section, theta)
    }

    /// Visible and hidden runs of a curve parameter in `[0, 1]`.
    fn classify_runs(
        &self,
        near: &Section,
        enclose: impl Fn(Interval) -> [Interval; 2],
        point: impl Fn(f64) -> [f64; 2],
        resolution: f64,
        budget: &mut usize,
    ) -> Result<Runs, ViewError> {
        let torus = [0., 1.].map(|u| {
            let [x, y] = self.piece_point(near, Piece::Torus, u);
            y.atan2(x)
        });
        let [low, high] = near.exit;
        let mut pending: Vec<_> = cells(0., 1., CURVE_CELLS).collect();
        let mut runs = Vec::new();
        while let Some(cell) = pending.pop() {
            *budget = budget.checked_sub(1).ok_or(ViewError::Limit)?;
            let [r, th] = enclose(cell);
            let inside = th.lo > low && th.hi < high;
            let state = if th.hi <= low || th.lo >= high {
                Some(false)
            } else if inside {
                let [least, most] = self.radius_range(near, torus, th);
                (r.lo > most || r.hi < least).then_some(r.lo > most)
            } else {
                None
            };
            let size = (r.hi - r.lo).hypot(r.hi * (th.hi - th.lo));
            let mid = (cell.lo + cell.hi) / 2.;
            match state {
                Some(visible) => runs.push((cell.lo, cell.hi, visible)),
                None if size <= resolution => {
                    runs.push((cell.lo, cell.hi, self.in_space(near, point(mid))));
                }
                None => pending.extend([Interval::new(cell.lo, mid), Interval::new(mid, cell.hi)]),
            }
        }
        runs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Runs = Vec::new();
        for run in runs {
            match merged.last_mut() {
                Some(last) if last.2 == run.2 => last.1 = run.1,
                _ => merged.push(run),
            }
        }
        Ok(merged)
    }

    /// Projected lines of the part between local heights `near_mm` (toward
    /// the viewer) and `far_mm`, viewed along the axis. The near face lines
    /// are included when `near_face` holds; hidden runs when `hidden`.
    /// Polylines stay within `tolerance_mm` of their exact curves and
    /// visibility transitions are located within `tolerance_mm / 64`.
    pub fn axial_view(
        &self,
        [near_mm, far_mm]: [f64; 2],
        near_face: bool,
        hidden: bool,
        tolerance_mm: f64,
        limit: usize,
    ) -> Result<Vec<ViewCurve>, ViewError> {
        let m = self.module_mm();
        let h = self.half_width();
        let [vn, vf] = [near_mm / m, far_mm / m];
        let valid = tolerance_mm > 0. && vn.abs() <= h && vf.abs() <= h && vn != vf;
        valid.then_some(()).ok_or(ViewError::Invalid)?;
        self.view_structure()
            .then_some(())
            .ok_or(ViewError::Unproven)?;
        let [_, bend, lean] = self.section_bounds().ok_or(ViewError::Unproven)?;
        let [near, far] = [vn, vf].map(|v| self.section(v));
        let mut budget = limit;
        let resolution = tolerance_mm / (64. * m);
        let mut sector = Vec::new();
        let blank = self.blank();
        let span = self.span();
        let arcs = |sector: &mut Vec<ViewCurve>,
                    v: f64,
                    radius: f64,
                    [a, b]: [f64; 2],
                    open: Option<[f64; 2]>| {
            let [lo, hi] = open.map_or([b, b], |[lo, hi]| [lo.clamp(a, b), hi.clamp(a, b)]);
            for (from, to, visible) in [(a, lo, false), (lo, hi, true), (hi, b, false)] {
                if to > from && (visible || hidden) {
                    sector.push(ViewCurve::Arc {
                        z_mm: v * m,
                        radius_mm: radius * m,
                        start_rad: from,
                        sweep_rad: to - from,
                        hidden: !visible,
                    });
                }
            }
        };
        let joint = |section: &Section, piece: Piece, u: f64| {
            let [x, y] = self.piece_point(section, piece, u);
            y.atan2(x)
        };
        let torus = |section: &Section| [0., 1.].map(|u| joint(section, Piece::Torus, u));
        // Near face: every edge lies on the nearest plane.
        if near_face {
            let [low, high] = near.exit;
            arcs(&mut sector, vn, blank, [-span, low], Some([-span, low]));
            arcs(&mut sector, vn, blank, [high, span], Some([high, span]));
            let floor = torus(&near);
            arcs(&mut sector, vn, self.root_radius(vn), floor, Some(floor));
            let inner = self.bore_radius_mm().map_or(0., |b| b / m);
            let (s, c) = (-span).sin_cos();
            sector.push(ViewCurve::Polyline {
                points_mm: [inner, blank]
                    .map(|r| [r * c * m, r * s * m, vn * m])
                    .to_vec(),
                hidden: false,
            });
        }
        // Far face circles and the junctions on the blank.
        for (radius, range) in [
            (blank, [-span, far.exit[0]]),
            (blank, [far.exit[1], span]),
            (self.root_radius(vf), torus(&far)),
        ] {
            arcs(&mut sector, vf, radius, range, self.open_arc(&near, radius));
        }
        for side in 0..2 {
            let [a, b] = [far.exit[side], near.exit[side]];
            arcs(
                &mut sector,
                vf,
                blank,
                [a.min(b), a.max(b)],
                Some(near.exit),
            );
        }
        if vn.min(vf) < 0. && vn.max(vf) > 0. {
            let middle = self.section(0.);
            let radius = self.root_radius(0.);
            arcs(
                &mut sector,
                0.,
                radius,
                torus(&middle),
                self.open_arc(&near, radius),
            );
        }
        // Profile pieces of both faces and the interior junctions.
        let count = (bend * m / (8. * tolerance_mm)).sqrt().ceil().max(1.);
        let steps = ((vn - vf).abs() * (lean * m / (8. * tolerance_mm)).sqrt())
            .ceil()
            .max(1.);
        let mut curves: Vec<Traced<'_>> = Vec::new();
        for piece in [
            Piece::FlankMinus,
            Piece::EdgeMinus,
            Piece::EdgePlus,
            Piece::FlankPlus,
        ] {
            if near_face {
                let section = near;
                curves.push((
                    Box::new(move |u| {
                        let [x, y] = self.piece_point(&section, piece, u);
                        [x, y, section.v]
                    }),
                    None,
                    count,
                ));
            }
            let crossing = Interval::point(far.crossing[usize::from(piece == Piece::FlankPlus)]);
            let runs = self.classify_runs(
                &near,
                |u| {
                    polar(
                        self.polar_patch(piece, u, Interval::point(vf), crossing),
                        Interval::atan,
                    )
                },
                |u| {
                    let [x, y] = self.piece_point(&far, piece, u);
                    [x.hypot(y), y.atan2(x)]
                },
                resolution,
                &mut budget,
            )?;
            curves.push((
                Box::new(move |u| {
                    let [x, y] = self.piece_point(&far, piece, u);
                    [x, y, vf]
                }),
                Some(runs),
                count,
            ));
        }
        for (piece, u) in [
            (Piece::EdgeMinus, 0.),
            (Piece::EdgeMinus, 1.),
            (Piece::EdgePlus, 0.),
            (Piece::EdgePlus, 1.),
        ] {
            let height = move |t: f64| vf + (vn - vf) * t;
            let at = move |t: f64| self.patch(piece, u, height(t), 0.);
            let runs = self.classify_runs(
                &near,
                |t| {
                    let [a, b] = [height(t.lo), height(t.hi)];
                    let v = Interval::new(a.min(b), a.max(b));
                    let point = Interval::point(u);
                    polar(
                        self.polar_patch(piece, point, v, Interval::point(0.)),
                        Interval::atan,
                    )
                },
                |t| {
                    let [x, y] = at(t);
                    [x.hypot(y), y.atan2(x)]
                },
                resolution,
                &mut budget,
            )?;
            curves.push((
                Box::new(move |t| {
                    let [x, y] = at(t);
                    [x, y, height(t)]
                }),
                Some(runs),
                steps,
            ));
        }
        for (point, runs, count) in curves {
            let runs = runs.unwrap_or_else(|| vec![(0., 1., true)]);
            for (from, to, visible) in runs {
                if !(visible || hidden) {
                    continue;
                }
                let first = (from * count).floor() as usize + 1;
                let last = (to * count).ceil() as usize;
                let samples = std::iter::once(from)
                    .chain((first..last).map(|i| i as f64 / count))
                    .chain([to]);
                let points: Vec<Point> = samples.map(|t| point(t).map(|x| x * m)).collect();
                budget = budget.checked_sub(points.len()).ok_or(ViewError::Limit)?;
                sector.push(ViewCurve::Polyline {
                    points_mm: points,
                    hidden: !visible,
                });
            }
        }
        // Contour generators lie at the minimum height-wise boundary radius of
        // their axial line, so material lies in front of them: always hidden.
        if hidden {
            let slab = [vn.min(vf), vn.max(vf)];
            for piece in [
                Piece::FlankMinus,
                Piece::EdgeMinus,
                Piece::EdgePlus,
                Piece::FlankPlus,
            ] {
                let samples = self.contour(piece, slab, tolerance_mm);
                if samples.len() > 1 {
                    budget = budget.checked_sub(samples.len()).ok_or(ViewError::Limit)?;
                    sector.push(ViewCurve::Polyline {
                        points_mm: samples
                            .into_iter()
                            .map(|[u, v]| {
                                let [x, y] = self.patch(piece, u, v, self.crossing(v, side(piece)));
                                [x * m, y * m, v * m]
                            })
                            .collect(),
                        hidden: true,
                    });
                }
            }
        }
        let mut view = Vec::with_capacity(sector.len() * self.teeth() as usize + 1);
        for k in 0..self.teeth() {
            let angle = self.sector_angle(k);
            view.extend(sector.iter().map(|curve| match curve {
                ViewCurve::Arc {
                    z_mm,
                    radius_mm,
                    start_rad,
                    sweep_rad,
                    hidden,
                } => ViewCurve::Arc {
                    z_mm: *z_mm,
                    radius_mm: *radius_mm,
                    start_rad: start_rad + angle,
                    sweep_rad: *sweep_rad,
                    hidden: *hidden,
                },
                ViewCurve::Polyline { points_mm, hidden } => ViewCurve::Polyline {
                    points_mm: points_mm.iter().map(|&p| rotate_z(p, angle)).collect(),
                    hidden: *hidden,
                },
            }));
        }
        if let (Some(bore), true) = (self.bore_radius_mm(), near_face) {
            view.push(ViewCurve::Arc {
                z_mm: near_mm,
                radius_mm: bore,
                start_rad: 0.,
                sweep_rad: std::f64::consts::TAU,
                hidden: false,
            });
        }
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worm_gear::wheel::tests::{spec, wheel};

    fn lines(view: &[ViewCurve], hidden: bool) -> Vec<&[Point]> {
        view.iter()
            .filter_map(|curve| match curve {
                ViewCurve::Polyline {
                    points_mm,
                    hidden: h,
                } if *h == hidden => Some(points_mm.as_slice()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_structure_lemma_is_proven_or_refused() {
        let wheel = wheel(spec(2., 64, 0.5, 6., 0.2));
        assert!(wheel.view_structure());
        // Without subdivision the coarse cells cannot prove it.
        assert!(!wheel.prove_view_structure_to(0));
        // Heights beyond the face have no proven exit radius.
        let outside = [Interval::new(0.2, 0.3), Interval::new(5., 6.)];
        assert!(!wheel.structure_holds(Piece::FlankMinus, outside));
    }

    #[test]
    fn view_lines_follow_the_open_near_space() {
        let wheel = wheel(spec(2., 64, 0.5, 6., 0.2));
        let h = wheel.spec().face_width_mm / 2.;
        let near = wheel.section(h / 2. / wheel.module_mm());
        let full = wheel
            .axial_view([h, -h], true, true, 1e-3, 10_000_000)
            .unwrap();
        let top = wheel.section(wheel.half_width());
        // Visible far-face and junction samples lie in the open near space,
        // hidden ones outside it, away from the located transitions.
        let check = |view: &[ViewCurve], section: &Section, near_z: f64| {
            let m = wheel.module_mm();
            for (hidden, polylines) in [(false, lines(view, false)), (true, lines(view, true))] {
                for points in polylines {
                    for pair in points.windows(2) {
                        let p: [f64; 3] = std::array::from_fn(|i| (pair[0][i] + pair[1][i]) / 2.);
                        let pitch = 2. * wheel.span();
                        let theta = p[1].atan2(p[0]);
                        let local = theta - (theta / pitch).round() * pitch;
                        let r = p[0].hypot(p[1]) / m;
                        if (p[2] - near_z).abs() < 1e-9 {
                            assert!(!hidden);
                            continue;
                        }
                        let margin = (r - wheel.boundary_radius(section, local).min(wheel.blank()))
                            .abs()
                            .min((local - section.exit[0]).abs())
                            .min((local - section.exit[1]).abs());
                        if margin > 1e-3 {
                            let inside = wheel.in_space(section, [r, local]);
                            assert_eq!(inside, !hidden, "{p:?}");
                        }
                    }
                }
            }
        };
        check(&full, &top, h);
        assert!(!lines(&full, true).is_empty() && !lines(&full, false).is_empty());
        let below = wheel
            .axial_view([-h, h], true, false, 1e-3, 10_000_000)
            .unwrap();
        assert!(lines(&below, true).is_empty());
        let retained = wheel
            .axial_view([h / 2., -h], false, true, 1e-3, 10_000_000)
            .unwrap();
        check(&retained, &near, h / 2.);
        let arcs = |view: &[ViewCurve], hidden: bool| {
            view.iter()
                .filter(|curve| matches!(curve, ViewCurve::Arc { hidden: h, .. } if *h == hidden))
                .count()
        };
        assert!(arcs(&retained, false) > 0 && arcs(&retained, true) > 0);
        assert_eq!(
            wheel.axial_view([h, h], true, true, 1e-3, 1000),
            Err(ViewError::Invalid)
        );
        assert_eq!(
            wheel.axial_view([h, -h], true, true, 1e-3, 100),
            Err(ViewError::Limit)
        );
        // The last budget charge fails one point short of the smallest
        // sufficient budget: contour samples with hidden lines, polyline
        // samples without.
        for hidden in [true, false] {
            let fits = |limit: usize| wheel.axial_view([h, -h], true, hidden, 1e-3, limit).is_ok();
            let (mut lo, mut hi) = (100, 10_000_000);
            while hi - lo > 1 {
                let mid = (lo + hi) / 2;
                if fits(mid) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            assert_eq!(
                wheel.axial_view([h, -h], true, hidden, 1e-3, lo),
                Err(ViewError::Limit)
            );
        }
        // A wheel whose structure fails is refused.
        let unproven = super::super::wheel::tests::wheel(spec(2., 64, 0.5, 6., 0.2));
        unproven.view_structure.get(|| false);
        assert_eq!(
            unproven.axial_view([h, -h], true, true, 1e-3, 1000),
            Err(ViewError::Unproven)
        );
    }
}

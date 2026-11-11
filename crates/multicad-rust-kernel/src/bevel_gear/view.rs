//! Exact axial drawing geometry: visibility, slab clipping and sections.
//!
//! Consider a line parallel to the axis at axial radius `ρ > 0` and azimuth
//! `φ`. Its points have polar angle `γ = atan2(ρ, z)`, which decreases as `z`
//! grows. The tooth domain at a fixed azimuth is `γ ≤ γ_hi(φ)` because the
//! half tooth width never increases with γ (the fillet meets its flank
//! before its widest azimuth and the spherical involute narrows outward).
//! Both back cones bound `z cos δ + ρ sin δ`, which is linear in `z`, and the
//! bore removes whole lines. So the solid meets every such line in one
//! interval whose upper end lies on the heel cap and whose lower end lies on
//! a side face or the toe cap.
//!
//! A viewer on the heel side therefore sees exactly the heel cap edges and
//! a viewer on the apex side sees every edge except those bounding only the
//! heel cap. A section plane perpendicular to the axis keeps one side of the
//! intervals and does not change which end of a retained interval an edge
//! point lies on, so retained views clip edges to the kept slab.
use super::*;

/// Clamped cubic B-spline in local coordinates with Bézier span joints.
#[derive(Clone, Debug, PartialEq)]
pub struct CubicSpline {
    pub poles_mm: Vec<Point>,
    pub knots: Vec<f64>,
    pub multiplicities: Vec<u8>,
    /// Certified bound of the distance to the exact piece.
    pub error_bound_mm: f64,
}

const MAX_SPANS: usize = 256;

/// Surface reached along each apex ray.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Level {
    Cap(End),
    /// Plane perpendicular to the axis at this local height.
    Plane(f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Visible,
    Hidden,
    Section,
}

/// One drawable curve in local coordinates.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewCurve {
    /// Segment along azimuth `phi` between two axial radii.
    Radial { phi: f64, radii: [f64; 2] },
    /// Circle about the axis starting at `start` with a positive sweep.
    Arc { radius: f64, start: f64, sweep: f64 },
    /// Cubic B-spline in local coordinates and a chord-bounded polyline.
    Spline {
        spline: CubicSpline,
        samples: Vec<Point>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewItem {
    pub role: Role,
    pub curve: ViewCurve,
}

/// Axial view request in the gear's local frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxialView {
    /// The viewer looks from beyond the heel toward the pitch apex.
    pub heel_side: bool,
    /// Retained interval of local axial heights.
    pub slab: [f64; 2],
    /// Height of a section plane perpendicular to the axis.
    pub section: Option<f64>,
    pub section_only: bool,
    pub hidden: bool,
    /// Also draw tangent-continuous ray edges between side faces.
    pub tangent: bool,
    pub tolerance: f64,
    pub max_points: usize,
}

struct Budget(usize);

impl Budget {
    fn take(&mut self, count: usize) -> Result<(), Error> {
        self.0 = self.0.checked_sub(count).ok_or(Error::Limit)?;
        Ok(())
    }
}

/// Parameter subrange of a monotone function on `[0, 1]` inside `[lo, hi]`.
fn clip(g: impl Fn(f64) -> f64, [lo, hi]: [f64; 2]) -> Option<[f64; 2]> {
    let [g0, g1] = [g(0.), g(1.)];
    let solve = |target: f64| {
        let (mut a, mut b) = (0_f64, 1_f64);
        for _ in 0..64 {
            let m = (a + b) / 2.;
            if (g(m) < target) == (g0 < g1) {
                a = m;
            } else {
                b = m;
            }
        }
        (a + b) / 2.
    };
    let below = |value: f64| value < lo;
    let above = |value: f64| value > hi;
    let low_end = |value: f64| if g0 < g1 { below(value) } else { above(value) };
    let high_end = |value: f64| if g0 < g1 { above(value) } else { below(value) };
    let start = if low_end(g0) {
        solve(if g0 < g1 { lo } else { hi })
    } else {
        0.
    };
    let stop = if high_end(g1) {
        solve(if g0 < g1 { hi } else { lo })
    } else {
        1.
    };
    let outside = (below(g0) && below(g1)) || (above(g0) && above(g1));
    (!outside && stop > start).then_some([start, stop])
}

/// Union of closed intervals, sorted and merged.
fn union(mut intervals: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    intervals.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut merged: Vec<[f64; 2]> = Vec::new();
    for [a, b] in intervals {
        match merged.last_mut() {
            Some(last) if a <= last[1] + 1e-12 => last[1] = last[1].max(b),
            _ => merged.push([a, b]),
        }
    }
    merged
}

/// Parts of `intervals` outside the sorted disjoint `cover`.
fn subtract(intervals: &[[f64; 2]], cover: &[[f64; 2]], minimum: f64) -> Vec<[f64; 2]> {
    let mut result = Vec::new();
    for &[a, b] in intervals {
        let mut start = a;
        for &[c, d] in cover {
            if c > start && c < b {
                result.push([start, c]);
            }
            if d > start {
                start = start.max(d);
            }
        }
        if b > start {
            result.push([start, b]);
        }
    }
    result.retain(|[a, b]| b - a > minimum);
    result
}

impl Bevel {
    /// Distance law `K / cos(γ − θ)` from the apex to one level.
    fn law(&self, level: Level) -> (f64, f64) {
        match level {
            Level::Cap(end) => (self.end_distance_mm(end), self.spec.pitch_cone_angle_rad),
            Level::Plane(z) => (z, 0.),
        }
    }

    fn level_point(&self, level: Level, piece: Piece, s: f64) -> (Point, Point) {
        let (k, theta) = self.law(level);
        let sample = self.sample(piece, s);
        let angle = sample.gamma - theta;
        let r = k / angle.cos();
        let dr = k * angle.sin() / angle.cos().powi(2);
        let dgamma = -sample.derivative[2] / sample.gamma.sin();
        (
            scale(sample.direction, r),
            std::array::from_fn(|i| r * sample.derivative[i] + dr * dgamma * sample.direction[i]),
        )
    }

    /// Bound of |d²p/ds²| for a fillet or involute piece on one level.
    ///
    /// With `p = r(γ) u`, `r = K / cos(γ − θ)`, `|u'| ≤ S`, `|u''| ≤ C` and
    /// `|γ'| ≤ |u'|`, differentiating `cos γ = u_z` twice gives
    /// `|γ''| ≤ (C + S²) / sin γ`. Then
    /// `p'' = (r''γ'² + r'γ'')u + 2r'γ'u' + r u''` and every γ lies in
    /// `[root, tip]` with `tip < π/2`, so `sin γ ≥ sin root`.
    fn second_bound(&self, piece: Piece, level: Level) -> f64 {
        let (k, theta) = self.law(level);
        let root = self.spec.root_cone_angle_rad;
        let tip = self.spec.tip_cone_angle_rad;
        let m = (theta - root).abs().max((tip - theta).abs());
        let (sm, cm) = m.sin_cos();
        let k = k.abs();
        let r0 = k / cm;
        let r1 = k * sm / (cm * cm);
        let r2 = k * (1. + sm * sm) / cm.powi(3);
        let (speed, curvature) = match piece {
            Piece::FilletLeft | Piece::FilletRight => (
                self.arc.abs() * self.rounding.sin(),
                self.curvature_bound(piece),
            ),
            _ => {
                // Before the half-tooth rotation the involute has
                // u' = c sin(bt) (c cos t, c sin t, −b) and
                // |u''|² = c⁴(sin²(bt) + b² cos²(bt)) + b⁴c² cos²(bt),
                // with b = sin β, c = cos β and 0 ≤ bt ≤ b·t_tip < π/2.
                let (b, c) = self.spec.base_cone_angle_rad.sin_cos();
                let rise = (b * self.tip_t).sin();
                let span = self.tip_t - self.touch_t;
                let second = (c.powi(4) * (rise * rise + b * b) + b.powi(4) * c * c).sqrt();
                (c * rise * span, second * span * span)
            }
        };
        let gamma2 = (curvature + speed * speed) / root.sin();
        r2 * speed * speed + r1 * gamma2 + 2. * r1 * speed * speed + r0 * curvature
    }

    /// Certified cubic B-spline of a fillet or involute piece of sector 0
    /// on one level, restricted to the piece parameter range `[s0, s1]`.
    pub fn profile_spline(
        &self,
        piece: Piece,
        level: Level,
        [s0, s1]: [f64; 2],
        tolerance: f64,
    ) -> Result<CubicSpline, Error> {
        let width = s1 - s0;
        let second = self.second_bound(piece, level) * width * width;
        let (k, _) = self.law(level);
        let scale_mm = k.abs() / self.spec.tip_cone_angle_rad.cos();
        let roundoff = 64. * f64::EPSILON * scale_mm;
        let at = |sigma: f64| {
            let (p, d) = self.level_point(level, piece, s0 + width * sigma);
            (p, d.map(|v| v * width))
        };
        let mut spans = 1;
        while spans <= MAX_SPANS {
            let h = 1. / spans as f64;
            let mut poles = vec![at(0.).0];
            let mut error = 0_f64;
            for i in 0..spans {
                let (a, da) = at(i as f64 * h);
                let (b, db) = at((i + 1) as f64 * h);
                let p1: Point = std::array::from_fn(|j| a[j] + da[j] * h / 3.);
                let p2: Point = std::array::from_fn(|j| b[j] - db[j] * h / 3.);
                let bend = |x: Point, y: Point, z: Point| {
                    norm(std::array::from_fn(|j| x[j] - 2. * y[j] + z[j]))
                };
                let difference = bend(a, p1, p2).max(bend(p1, p2, b));
                // e = B − p vanishes with e' at both span ends, so
                // |e| ≤ h²/8 max|e''| ≤ (6·difference + h²·|p''|) / 8.
                error = error.max((6. * difference + h * h * second) / 8.);
                poles.extend([p1, p2, b]);
            }
            let error_bound_mm = error + roundoff;
            if error_bound_mm <= tolerance {
                let mut multiplicities = vec![3; spans + 1];
                multiplicities[0] = 4;
                multiplicities[spans] = 4;
                return Ok(CubicSpline {
                    poles_mm: poles,
                    knots: (0..=spans).map(|i| i as f64 * h).collect(),
                    multiplicities,
                    error_bound_mm,
                });
            }
            spans *= 2;
        }
        Err(Error::Limit)
    }

    /// Points of a piece on one level whose chords stay within `tolerance`.
    fn profile_samples(
        &self,
        piece: Piece,
        level: Level,
        [s0, s1]: [f64; 2],
        tolerance: f64,
        budget: &mut Budget,
    ) -> Result<Vec<Point>, Error> {
        let second = self.second_bound(piece, level) * (s1 - s0).powi(2);
        let count = (second / (8. * tolerance)).sqrt().ceil().max(1.);
        let count = count.min(1e7) as usize;
        budget.take(count + 1)?;
        Ok((0..=count)
            .map(|i| {
                let s = s0 + (s1 - s0) * i as f64 / count as f64;
                self.level_point(level, piece, s).0
            })
            .collect())
    }

    fn spline_item(
        &self,
        (sector, piece): (u32, Piece),
        level: Level,
        range: [f64; 2],
        role: Role,
        view: &AxialView,
        budget: &mut Budget,
    ) -> Result<ViewItem, Error> {
        let mut spline = self.profile_spline(piece, level, range, view.tolerance)?;
        let samples = self.profile_samples(piece, level, range, view.tolerance, budget)?;
        budget.take(spline.poles_mm.len())?;
        spline.poles_mm = spline
            .poles_mm
            .into_iter()
            .map(|p| self.rotate(sector, p))
            .collect();
        Ok(ViewItem {
            role,
            curve: ViewCurve::Spline {
                spline,
                samples: samples
                    .into_iter()
                    .map(|p| self.rotate(sector, p))
                    .collect(),
            },
        })
    }

    fn sector_angle(&self, sector: u32) -> f64 {
        self.sector_start(sector) + PI / f64::from(self.spec.teeth)
    }

    /// Radial segment between two local points, clipped to the slab.
    fn radial(
        &self,
        sector: u32,
        [a, b]: [Point; 2],
        slab: [f64; 2],
        role: Role,
    ) -> Option<ViewItem> {
        let [s, t] = clip(|s| a[2] + (b[2] - a[2]) * s, slab)?;
        let radius = |s: f64| (1. - s) * a[0].hypot(a[1]) + s * b[0].hypot(b[1]);
        Some(ViewItem {
            role,
            curve: ViewCurve::Radial {
                phi: a[1].atan2(a[0]) + self.sector_angle(sector),
                radii: [radius(s), radius(t)],
            },
        })
    }

    /// Visible, hidden and section curves of one axial view.
    pub fn axial_view(&self, view: &AxialView) -> Result<Vec<ViewItem>, Error> {
        if !(view.tolerance > 0. && view.tolerance.is_finite())
            || view.slab.iter().any(|z| z.is_nan())
            || view.slab[0] > view.slab[1]
            || view.section.is_some_and(|z| !z.is_finite())
        {
            return Err(Error::InvalidDimensions);
        }
        let mut budget = Budget(view.max_points);
        let mut items = Vec::new();
        if !view.section_only {
            self.edges(view, &mut items, &mut budget)?;
            if !view.hidden {
                items.retain(|item| item.role != Role::Hidden);
            }
            items = self.merge(items, view.tolerance);
        }
        if let Some(z) = view.section {
            self.section(z, None, view, &mut items, &mut budget)?;
            // A finite viewing depth cuts the retained body again. Its far
            // outline is visible where it bounds a face toward the viewer.
            let far = if view.heel_side {
                view.slab[0]
            } else {
                view.slab[1]
            };
            if !view.section_only && far.is_finite() {
                self.section(far, Some(view.heel_side), view, &mut items, &mut budget)?;
                if !view.hidden {
                    items.retain(|item| item.role != Role::Hidden);
                }
            }
        }
        Ok(items)
    }

    fn edges(
        &self,
        view: &AxialView,
        items: &mut Vec<ViewItem>,
        budget: &mut Budget,
    ) -> Result<(), Error> {
        let slab = view.slab;
        let inside = |z: f64| slab[0] <= z && z <= slab[1];
        let role = |heel_only: bool, heel: bool| {
            if (view.heel_side && heel) || (!view.heel_side && !heel_only) {
                Role::Visible
            } else {
                Role::Hidden
            }
        };
        let delta = self.spec.pitch_cone_angle_rad;
        for sector in 0..self.spec.teeth {
            for end in End::BOTH {
                let level = Level::Cap(end);
                let role = role(false, end == End::Heel);
                for &piece in &self.pieces {
                    match piece {
                        Piece::RootLeft | Piece::RootRight | Piece::Tip => {
                            let [a, b] = [0., 1.].map(|s| self.sample(piece, s));
                            let z = self.end_distance_along(end, a.gamma) * a.gamma.cos();
                            if inside(z) {
                                items.push(ViewItem {
                                    role,
                                    curve: ViewCurve::Arc {
                                        radius: self.end_radius(end, a.gamma),
                                        start: a.phi + self.sector_angle(sector),
                                        sweep: b.phi - a.phi,
                                    },
                                });
                            }
                        }
                        Piece::MeridianLeft | Piece::MeridianRight => {
                            let [a, b] = [0., 1.].map(|s| self.level_point(level, piece, s).0);
                            items.extend(self.radial(sector, [a, b], slab, role));
                        }
                        _ => {
                            let z = |s| self.level_point(level, piece, s).0[2];
                            if let Some(range) = clip(z, slab) {
                                items.push(self.spline_item(
                                    (sector, piece),
                                    level,
                                    range,
                                    role,
                                    view,
                                    budget,
                                )?);
                            }
                        }
                    }
                }
            }
            for pair in self.pieces.windows(2) {
                let sharp = pair.contains(&Piece::Tip);
                if !(sharp || view.tangent) {
                    continue;
                }
                let sample = self.sample(pair[0], 1.);
                let ends = [End::Toe, End::Heel].map(|end| {
                    let t = self.end_distance_mm(end) / (sample.gamma - delta).cos();
                    scale(sample.direction, t)
                });
                items.extend(self.radial(sector, ends, slab, role(false, false)));
            }
        }
        if let Some(bore) = self.bore_radius_mm() {
            for end in End::BOTH {
                if inside(self.back_cone_height_mm(end, bore)) {
                    items.push(ViewItem {
                        role: role(end == End::Heel, end == End::Heel),
                        curve: ViewCurve::Arc {
                            radius: bore,
                            start: 0.,
                            sweep: TAU,
                        },
                    });
                }
            }
        }
        Ok(())
    }

    /// Merge coincident radial segments and arcs, dropping hidden parts
    /// that coincide with visible ones.
    fn merge(&self, items: Vec<ViewItem>, tolerance: f64) -> Vec<ViewItem> {
        let mut radials: Vec<(f64, Role, [f64; 2])> = Vec::new();
        let mut arcs: Vec<(f64, Role, [f64; 2])> = Vec::new();
        let mut rest = Vec::new();
        for item in items {
            match item.curve {
                ViewCurve::Radial { phi, radii } => {
                    let [a, b] = radii;
                    radials.push((phi.rem_euclid(TAU), item.role, [a.min(b), a.max(b)]));
                }
                ViewCurve::Arc {
                    radius,
                    start,
                    sweep,
                } => {
                    let start = start.rem_euclid(TAU);
                    let stop = start + sweep;
                    arcs.push((radius, item.role, [start, stop.min(TAU)]));
                    if stop > TAU {
                        arcs.push((radius, item.role, [0., stop - TAU]));
                    }
                }
                ViewCurve::Spline { .. } => rest.push(item),
            }
        }
        let same = 1e-9;
        let mut output = Vec::new();
        let mut groups = |entries: Vec<(f64, Role, [f64; 2])>,
                          minimum: &dyn Fn(f64) -> f64,
                          make: &dyn Fn(f64, [f64; 2]) -> ViewCurve| {
            let mut keys: Vec<f64> = Vec::new();
            for &(key, ..) in &entries {
                if !keys
                    .iter()
                    .any(|k| (k - key).abs() <= same * key.abs().max(1.))
                {
                    keys.push(key);
                }
            }
            for key in keys {
                let pick = |role: Role| {
                    union(
                        entries
                            .iter()
                            .filter(|(k, r, _)| {
                                *r == role && (k - key).abs() <= same * key.abs().max(1.)
                            })
                            .map(|(_, _, interval)| *interval)
                            .collect(),
                    )
                };
                let visible = pick(Role::Visible);
                let hidden = subtract(&pick(Role::Hidden), &visible, minimum(key));
                for (role, list) in [(Role::Visible, visible), (Role::Hidden, hidden)] {
                    for interval in list {
                        output.push(ViewItem {
                            role,
                            curve: make(key, interval),
                        });
                    }
                }
            }
        };
        groups(radials, &|_| tolerance, &|phi, radii| ViewCurve::Radial {
            phi,
            radii,
        });
        groups(arcs, &|radius| tolerance / radius, &|radius, [a, b]| {
            ViewCurve::Arc {
                radius,
                start: a,
                sweep: b - a,
            }
        });
        output.extend(rest);
        output
    }

    /// Exact boundary of the plane section `z = height`.
    /// Boundary of the plane section `z = height`. `far` gives the viewer
    /// side for the far outline of a depth-limited view, whose pieces keep
    /// the visibility of the face they bound; the near cut is a section.
    fn section(
        &self,
        height: f64,
        far: Option<bool>,
        view: &AxialView,
        items: &mut Vec<ViewItem>,
        budget: &mut Budget,
    ) -> Result<(), Error> {
        let (s, c) = self.spec.pitch_cone_angle_rad.sin_cos();
        let outer = (self.end_distance_mm(End::Heel) - height * c) / s;
        let inner = ((self.end_distance_mm(End::Toe) - height * c) / s)
            .max(self.spec.bore_radius_mm)
            .max(0.);
        let root = self.spec.root_cone_angle_rad;
        let tip = self.spec.tip_cone_angle_rad;
        if height <= 0. || outer <= inner || inner >= height * tip.tan() {
            return Ok(());
        }
        // The heel cap faces the heel side; the side faces and toe cap face
        // the apex side; the bore circle lies under the near bore rim.
        let role = |heel_cap: bool, bore: bool| match far {
            None => Role::Section,
            Some(heel_side) if !bore && heel_side == heel_cap => Role::Visible,
            Some(_) => Role::Hidden,
        };
        let star = role(false, false);
        let level = Level::Plane(height);
        let gamma_of = |radius: f64| radius.atan2(height);
        let [low, high] = [inner, outer].map(gamma_of);
        let bored = inner == self.spec.bore_radius_mm;
        // Circles of the back cones and bore keep the azimuths where the
        // tooth star reaches beyond them.
        for (radius, gamma, present, role) in [
            (inner, low, inner > 0., role(false, bored)),
            (outer, high, true, role(true, false)),
        ] {
            let section = |curve| ViewItem { role, curve };
            if !present || gamma >= tip {
                continue;
            }
            if gamma < root {
                items.push(section(ViewCurve::Arc {
                    radius,
                    start: 0.,
                    sweep: TAU,
                }));
                continue;
            }
            let half = self.half_width(gamma);
            for sector in 0..self.spec.teeth {
                items.push(section(ViewCurve::Arc {
                    radius,
                    start: self.sector_angle(sector) - half,
                    sweep: 2. * half,
                }));
            }
        }
        let band = [low, high];
        let section = |curve| ViewItem { role: star, curve };
        let within = |gamma: f64| low <= gamma && gamma <= high;
        for sector in 0..self.spec.teeth {
            for &piece in &self.pieces {
                match piece {
                    Piece::RootLeft | Piece::RootRight | Piece::Tip => {
                        let gamma = if piece == Piece::Tip { tip } else { root };
                        if within(gamma) {
                            let [a, b] = [0., 1.].map(|s| self.sample(piece, s).phi);
                            items.push(section(ViewCurve::Arc {
                                radius: height * gamma.tan(),
                                start: a + self.sector_angle(sector),
                                sweep: b - a,
                            }));
                        }
                    }
                    _ => {
                        let Some(range) = clip(|s| self.sample(piece, s).gamma, band) else {
                            continue;
                        };
                        if matches!(piece, Piece::MeridianLeft | Piece::MeridianRight) {
                            let [a, b] = range.map(|s| self.level_point(level, piece, s).0);
                            let radius = |p: Point| p[0].hypot(p[1]);
                            items.push(section(ViewCurve::Radial {
                                phi: a[1].atan2(a[0]) + self.sector_angle(sector),
                                radii: [radius(a), radius(b)],
                            }));
                        } else {
                            items.push(self.spline_item(
                                (sector, piece),
                                level,
                                range,
                                star,
                                view,
                                budget,
                            )?);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

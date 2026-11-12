//! Smooth shared trim proposals, independently enclosed against the source.
//!
//! Cubic Hermite interpolation is used only after the complete fourth
//! derivative remainder has been bounded. These curves are not mesh edges.
pub(crate) use super::interval::I;
use super::*;
use std::ops::{Add, Mul, Neg, Sub};

/// A shared polynomial edge and its height pcurve use the same phase domain.
/// `source_error_mm` bounds the entire curve, not just its sampled vertices.
#[derive(Clone, Debug)]
pub(crate) struct Trim {
    pub start: u32,
    pub band: usize,
    pub active: bool,
    pub half: ConeHalf,
    pub phase: [f64; 2],
    pub poles: [Point; 4],
    pub height_poles: [f64; 4],
    pub angle_poles: [f64; 4],
    pub surface_u_poles: [f64; 4],
    pub source_error_mm: f64,
    pub height_error_mm: f64,
    pub angle_error_rad: f64,
    pub surface_u_error: f64,
    pub phase_forward: bool,
}

impl Trim {
    fn ordered(mut self, body: &Chamfer) -> Result<Self, Error> {
        self.phase_forward =
            advances_phase(self.poles.map(|p| p.map(I::point)), body.rate_interval()?)?
                && advances_pcurves(
                    self.height_poles,
                    self.angle_poles,
                    self.surface_u_poles,
                    body.rate_interval()?,
                )?;
        Ok(self)
    }
    /// Select the periodic branch of the existing source surface chart.
    /// Floating addition and any endpoint/domain adjustment remain explicit.
    pub(crate) fn surface_branch(
        mut self,
        body: &Chamfer,
        period: u32,
        domain: [f64; 2],
    ) -> Result<Self, Error> {
        if period > body.tool.spec().starts
            || domain.into_iter().any(|x| !x.is_finite())
            || domain[0] >= domain[1]
        {
            return Err(Error::Unresolved);
        }
        let offset = if self.band == 2 && body.tool.root_circle().is_some() {
            I::point(0.)
        } else {
            I::point(f64::from(period)) * I::point(body.tool.spec().pitch_mm)
                / body.rate_interval()?.abs()
        };
        let nominal_offset = offset.lo.midpoint(offset.hi);
        let mut error = 0_f64;
        for pole in &mut self.surface_u_poles {
            let exact = I::point(*pole) + offset;
            let rounded = (*pole + nominal_offset).clamp(domain[0], domain[1]);
            error = error.max((exact - I::point(rounded)).checked()?.magnitude());
            *pole = rounded;
        }
        self.surface_u_error = (I::point(self.surface_u_error) + I::point(error))
            .checked()?
            .hi;
        self.phase_forward = self.phase_forward
            && advances_pcurves(
                self.height_poles,
                self.angle_poles,
                self.surface_u_poles,
                body.rate_interval()?,
            )?;
        if !self.phase_forward {
            return Err(Error::Unresolved);
        }
        Ok(self)
    }

    pub(crate) fn error_bound(&self, body: &Chamfer) -> Result<f64, Error> {
        let k = body.cone_slope()?.abs();
        let radius =
            I::point(body.tool.spec().major_radius_mm) + k * I::point(self.height_error_mm);
        let speed = (radius / body.rate_interval()?.abs() + k + I::point(1.)).checked()?;
        let coupling = (speed * I::point(self.height_error_mm)).checked()?.hi;
        let rate = body.rate_interval()?.abs();
        let radial_speed = if let Some(root) = body.tool.root_circle().filter(|_| self.band == 2) {
            // The polynomial U approximation can lie just beyond the
            // trimmed root interval. Bound the whole circle, not only
            // the nominal source major radius, along that connection.
            let radial_extent = I::point(root.center_radius).abs() + I::point(root.radius);
            I::point(root.radius) * (I::point(1.) + radial_extent / rate)
        } else {
            I::point(body.tool.spec().major_radius_mm)
                + rate
                    * I::point(
                        body.profile(self.band, I::new(self.phase[0], self.phase[1]))?
                            .first
                            .magnitude(),
                    )
        };
        let pcurve_error = (I::point(self.angle_error_rad) * radius
            + I::point(self.surface_u_error) * radial_speed)
            .checked()?
            .hi;
        Ok(
            (I::point(self.source_error_mm) + I::point(coupling) + I::point(pcurve_error))
                .checked()?
                .hi,
        )
    }

    /// Share the actual topological vertex without hiding its displacement.
    /// All previous source and pcurve receipts remain and are enlarged by the
    /// complete coefficient change before the no-fold checks are repeated.
    pub(crate) fn with_endpoints(
        mut self,
        body: &Chamfer,
        endpoints: [Point; 2],
    ) -> Result<Self, Error> {
        let mut displacement = 0_f64;
        let mut height = 0_f64;
        for (index, point) in [0, 3].into_iter().zip(endpoints) {
            if point.into_iter().any(|v| !v.is_finite())
                || point[2] < 0.
                || point[2] > body.entry_length_mm
            {
                return Err(Error::Unresolved);
            }
            let delta = std::array::from_fn::<_, 3, _>(|axis| {
                (I::point(self.poles[index][axis]) - I::point(point[axis])).abs()
            });
            displacement = displacement.max(
                delta
                    .into_iter()
                    .fold(I::point(0.), |sum, d| sum + d)
                    .checked()?
                    .hi,
            );
            height = height.max(delta[2].hi);
            self.poles[index] = point;
        }
        self.source_error_mm = (I::point(self.source_error_mm) + I::point(displacement))
            .checked()?
            .hi;
        self.height_error_mm = (I::point(self.height_error_mm) + I::point(height))
            .checked()?
            .hi;
        self.height_poles = self.poles.map(|p| p[2]);
        self.phase_forward =
            advances_phase(self.poles.map(|p| p.map(I::point)), body.rate_interval()?)?
                && advances_pcurves(
                    self.height_poles,
                    self.angle_poles,
                    self.surface_u_poles,
                    body.rate_interval()?,
                )?;
        if !self.phase_forward {
            return Err(Error::Unresolved);
        }
        Ok(self)
    }
}

pub(crate) fn prepare(
    body: &Chamfer,
    options: QueryOptions,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Vec<Trim>, Error> {
    let mut work = Work::new(options, interrupt)?;
    work.step()?;
    let mut result = Vec::new();
    for start in 0..body.tool.spec().starts {
        for segment in body.phase_segments()? {
            let mut pending = vec![segment];
            while let Some(segment) = pending.pop() {
                work.step()?;
                let trim = propose(body, start, segment)?;
                if trim.phase_forward && trim.error_bound(body)? <= options.tolerance {
                    result.push(trim);
                    continue;
                }
                let [a, b] = segment.ends.map(|end| end.nominal);
                let mid = a.midpoint(b);
                if mid <= a || mid >= b {
                    return Err(Error::Unresolved);
                }
                let knot = super::phase::Knot {
                    phase: I::point(mid),
                    nominal: mid,
                    threshold: false,
                    glued: false,
                };
                let mut left = segment;
                let mut right = segment;
                left.ends[1] = knot;
                right.ends[0] = knot;
                pending.extend([right, left]);
            }
        }
    }
    Ok(result)
}

fn propose(body: &Chamfer, start: u32, segment: super::phase::Segment) -> Result<Trim, Error> {
    let ends = segment.ends;
    let phase = ends.map(|end| end.nominal);
    let width = I::point(phase[1]) - I::point(phase[0]);
    if width.lo <= 0. {
        return Err(Error::Unresolved);
    }
    let domain = ends[0].phase.hull(ends[1].phase);
    let whole = body.trim_jets(segment.band, segment.active, domain, start)?;
    let values = phase.map(|q| body.trim_jets(segment.band, segment.active, I::point(q), start));
    let [a, b] = [
        values[0].as_ref().map_err(|error| *error)?,
        values[1].as_ref().map_err(|error| *error)?,
    ];
    let control = |coordinate: usize| {
        [
            a[coordinate].0[0],
            a[coordinate].0[0] + width * a[coordinate].0[1] / I::point(3.),
            b[coordinate].0[0] - width * b[coordinate].0[1] / I::point(3.),
            b[coordinate].0[0],
        ]
    };
    let source_controls: [[I; 4]; 3] = std::array::from_fn(control);
    let angle_controls = control(3);
    let u_controls = control(4);
    let mut poles = std::array::from_fn(|i| {
        std::array::from_fn(|axis| {
            source_controls[axis][i]
                .lo
                .midpoint(source_controls[axis][i].hi)
        })
    });
    // Actual contact is on z=0 and the entry circle. Do not turn the rounded
    // nominal inverse phase into an invented positive-height boundary sliver.
    for (end, pole) in [0, 3].into_iter().enumerate() {
        if ends[end].threshold {
            let theta = (I::point(0.)
                - (I::point(phase[end])
                    + I::point(f64::from(start)) * I::point(body.tool.spec().pitch_mm)))
                / body.rate_interval()?;
            let xy = [theta.cos(), theta.sin()].map(|v| I::point(body.entry_radius_mm) * v);
            poles[pole] = [xy[0].lo.midpoint(xy[0].hi), xy[1].lo.midpoint(xy[1].hi), 0.];
        }
    }
    // Only the approximation controls are constrained to the exact material
    // chart. The source law is never changed; their complete displacement is
    // included below in the convex-hull coefficient error certificate.
    for pole in &mut poles {
        pole[2] = pole[2].clamp(0., body.entry_length_mm);
    }
    if segment.active && poles.iter().all(|pole| pole[2] == 0.) {
        return Err(Error::Unresolved);
    }
    let coefficient_error = std::array::from_fn::<_, 3, _>(|axis| {
        source_controls[axis]
            .iter()
            .zip(poles)
            .fold(0_f64, |bound, (&source, pole)| {
                bound.max((source - I::point(pole[axis])).magnitude())
            })
    });
    let width4 = width.square().square();
    let phase_error = ends.into_iter().fold(0_f64, |bound, end| {
        bound.max((end.phase - I::point(end.nominal)).magnitude())
    });
    let error = |axis: usize, coefficient_error: f64| {
        (I::point(coefficient_error)
            + I::point(whole[axis].0[4].magnitude()) * width4 / I::point(16.)
            + I::point(whole[axis].0[1].magnitude()) * I::point(phase_error))
        .checked()
    };
    let errors = [
        error(0, coefficient_error[0])?,
        error(1, coefficient_error[1])?,
        error(2, coefficient_error[2])?,
    ];
    let angle_poles = angle_controls.map(|value| value.lo.midpoint(value.hi));
    let mut surface_u_poles = u_controls.map(|value| value.lo.midpoint(value.hi));
    if let Some(root) = body.tool.root_circle().filter(|_| segment.band == 2) {
        let knots = body.tool.profile_breaks();
        let [a, b] = [knots[2], knots[3]].map(|q| ((q - root.phase) / root.radius).asin());
        for u in &mut surface_u_poles {
            *u = u.clamp(a, b);
        }
    }
    let scalar_error = |coordinate, controls: [I; 4], poles: [f64; 4]| {
        let residual = controls
            .into_iter()
            .zip(poles)
            .fold(0_f64, |bound, (value, pole)| {
                bound.max((value - I::point(pole)).magnitude())
            });
        error(coordinate, residual)
    };
    let source_error_mm = errors
        .into_iter()
        .fold(I::point(0.), |sum, value| sum + value)
        .checked()?
        .hi;
    Trim {
        start,
        band: segment.band,
        active: segment.active,
        half: segment.half,
        phase,
        height_poles: poles.map(|point| point[2]),
        angle_poles,
        surface_u_poles,
        poles,
        source_error_mm,
        height_error_mm: errors[2].hi,
        angle_error_rad: scalar_error(3, angle_controls, angle_poles)?.hi,
        surface_u_error: scalar_error(4, u_controls, surface_u_poles)?.hi,
        phase_forward: false,
    }
    .ordered(body)
}

fn advances_pcurves(
    height: [f64; 4],
    angle: [f64; 4],
    surface_u: [f64; 4],
    rate: I,
) -> Result<bool, Error> {
    for i in 0..3 {
        let u = I::point(surface_u[i + 1]) - I::point(surface_u[i]);
        let cone_phase = (I::point(height[i + 1]) - I::point(height[i]))
            - rate * (I::point(angle[i + 1]) - I::point(angle[i]));
        if u.checked()?.lo <= 0. || cone_phase.checked()?.lo <= 0. {
            return Ok(false);
        }
    }
    Ok(true)
}

/// A positive lifted `z-rate*atan2(y,x)` derivative proves the complete
/// retained polynomial edge cannot fold or reverse in its source chart.
fn advances_phase(poles: [[I; 3]; 4], rate: I) -> Result<bool, Error> {
    for component in poles.into_iter().flatten() {
        component.checked()?;
    }
    let range = |axis: usize| {
        poles
            .iter()
            .fold(poles[0][axis], |bound, point| bound.hull(point[axis]))
    };
    let derivative = |axis: usize| {
        poles
            .windows(2)
            .fold(None::<I>, |bound, pair| {
                let value = I::point(3.) * (pair[1][axis] - pair[0][axis]);
                Some(bound.map_or(value, |bound| bound.hull(value)))
            })
            .expect("fixed cubic")
    };
    let [x, y] = [range(0), range(1)];
    let radius2 = x.square() + y.square();
    if radius2.lo <= 0. {
        return Ok(false);
    }
    let phase_derivative = derivative(2) - rate * (x * derivative(1) - y * derivative(0)) / radius2;
    Ok(phase_derivative.checked()?.lo > 0.)
}

#[derive(Clone, Copy)]
pub(crate) struct Jet(pub(crate) [I; 5]);
impl Jet {
    pub(crate) fn constant(x: I) -> Self {
        let mut out = Self([I::point(0.); 5]);
        out.0[0] = x;
        out
    }
    pub(crate) fn variable(x: I) -> Self {
        let mut out = Self::constant(x);
        out.0[1] = I::point(1.);
        out
    }
    pub(crate) fn checked(self) -> Result<Self, Error> {
        for coefficient in self.0 {
            coefficient.checked()?;
        }
        Ok(self)
    }
    pub(crate) fn scale(self, value: I) -> Self {
        Self(self.0.map(|x| x * value))
    }
    pub(crate) fn sqrt(self) -> Result<Self, Error> {
        if self.0[0].lo <= 0. {
            return Err(Error::Unresolved);
        }
        let mut out = Self::constant(self.0[0].sqrt());
        for n in 1..5 {
            let mut value = self.0[n];
            for i in 1..n {
                value = value - out.0[i] * out.0[n - i];
            }
            out.0[n] = value / (I::point(2.) * out.0[0]);
        }
        out.checked()
    }
    pub(crate) fn reciprocal(self) -> Result<Self, Error> {
        let mut out = Self::constant(I::point(1.) / self.0[0]);
        for n in 1..5 {
            let value = (1..=n).fold(I::point(0.), |sum, i| sum + self.0[i] * out.0[n - i]);
            out.0[n] = -value / self.0[0];
        }
        out.checked()
    }
    pub(crate) fn sin_cos(self) -> Result<[Self; 2], Error> {
        let mut sin = Self::constant(self.0[0].sin());
        let mut cos = Self::constant(self.0[0].cos());
        for n in 1..5 {
            let mut s = I::point(0.);
            let mut c = I::point(0.);
            for i in 1..=n {
                let derivative = self.0[i] * I::point(i as f64);
                s = s + derivative * cos.0[n - i];
                c = c - derivative * sin.0[n - i];
            }
            sin.0[n] = s / I::point(n as f64);
            cos.0[n] = c / I::point(n as f64);
        }
        Ok([sin.checked()?, cos.checked()?])
    }
}
impl Add for Jet {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self(std::array::from_fn(|i| self.0[i] + other.0[i]))
    }
}
impl Neg for Jet {
    type Output = Self;
    fn neg(self) -> Self {
        Self(self.0.map(|x| -x))
    }
}
impl Sub for Jet {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        self + -other
    }
}
impl Mul for Jet {
    type Output = Self;
    fn mul(self, other: Self) -> Self {
        Self(std::array::from_fn(|n| {
            (0..=n).fold(I::point(0.), |sum, i| sum + self.0[i] * other.0[n - i])
        }))
    }
}

impl Chamfer {
    fn trim_jets(
        &self,
        band: usize,
        active: bool,
        phase: I,
        start: u32,
    ) -> Result<[Jet; 5], Error> {
        let q = Jet::variable(phase);
        let constant = |x| Jet::constant(I::point(x));
        let breaks = self.tool.profile_breaks();
        let mut surface_u = q.scale(I::point(1.) / self.rate_interval()?.abs());
        let radius = if band == 0 || band == 4 {
            constant(self.tool.spec().major_radius_mm)
        } else if band == 2 {
            if let Some(root) = self.tool.root_circle() {
                let x = q - constant(root.phase);
                let square = constant(root.radius) * constant(root.radius) - x * x;
                let h = square.sqrt()?;
                let ratio = (phase - I::point(root.phase)) / I::point(root.radius);
                // The preceding strict positive circle radicand proves the
                // exact ratio lies inside (-1,1). Intersect only arithmetic
                // excess before evaluating asin, without extending its source.
                let ratio = I::new(ratio.lo.max(-1.), ratio.hi.min(1.));
                let mut angle = Jet::constant(I::new(
                    ratio.lo.asin().next_down().next_down(),
                    ratio.hi.asin().next_up().next_up(),
                ));
                let inverse_h = h.reciprocal()?;
                for i in 1..5 {
                    angle.0[i] = inverse_h.0[i - 1] / I::point(i as f64);
                }
                surface_u = angle.checked()?;
                constant(root.center_radius) - h
            } else {
                constant(self.tool.root_radius_mm())
            }
        } else {
            let at = if band == 1 { breaks[1] } else { breaks[4] };
            constant(self.tool.spec().major_radius_mm)
                + (q - constant(at)).scale(I::point(self.tool.slope_at_phase(at, band)))
        };
        let z = if active {
            (radius - constant(self.entry_radius_mm)).scale(I::point(1.) / self.cone_slope()?)
        } else {
            constant(0.)
        };
        let shifted =
            q + Jet::constant(I::point(f64::from(start)) * I::point(self.tool.spec().pitch_mm));
        let angle = (z - shifted).scale(I::point(1.) / self.rate_interval()?);
        let [sin, cos] = angle.sin_cos()?;
        Ok([
            (radius * cos).checked()?,
            (radius * sin).checked()?,
            z.checked()?,
            angle.checked()?,
            surface_u.checked()?,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bezier(poles: [Point; 4], t: f64) -> Point {
        let mut points = poles;
        for degree in (1..4).rev() {
            for i in 0..degree {
                points[i] = std::array::from_fn(|axis| {
                    (1. - t) * points[i][axis] + t * points[i + 1][axis]
                });
            }
        }
        points[0]
    }

    #[test]
    fn shared_smooth_trim_is_enclosed_against_every_actual_source_regime() {
        for left in [false, true] {
            for rounded in [false, true] {
                let source = super::super::tests::tool(left, 2, rounded);
                for fraction in [0.1, 1., 1.2] {
                    let body =
                        Chamfer::new(source, fraction * source.spec().depth_mm, None).unwrap();
                    let options = QueryOptions {
                        tolerance: 1e-6,
                        maximum_cells: 100_000,
                    };
                    let trims = prepare(&body, options, None).unwrap();
                    assert!(trims.iter().any(|trim| trim.active));
                    for trim in trims {
                        assert!(trim.band < 5);
                        assert!(trim.start < 2);
                        assert!(trim.source_error_mm <= options.tolerance);
                        assert!(trim.phase_forward);
                        assert!(
                            trim.height_poles
                                .into_iter()
                                .all(|z| z >= 0. && z <= body.entry_length_mm())
                        );
                        for i in 0..=16 {
                            let t = i as f64 / 16.;
                            let q = (1. - t) * trim.phase[0] + t * trim.phase[1];
                            let z = body.seam_station(q);
                            let angle = (z - q - f64::from(trim.start) * source.spec().pitch_mm)
                                / source.helical_rate();
                            let (sin, cos) = angle.sin_cos();
                            let r = source.radius_at_phase(q);
                            let expected = [r * cos, r * sin, z];
                            let point = bezier(trim.poles, t);
                            let error = expected
                                .into_iter()
                                .zip(point)
                                .map(|(a, b)| (a - b).abs())
                                .sum::<f64>();
                            assert!(
                                error <= trim.source_error_mm + 2e-13,
                                "{error} {}",
                                trim.source_error_mm
                            );
                            let height = bezier(trim.height_poles.map(|z| [0., 0., z]), t)[2];
                            assert!((height - z).abs() <= trim.height_error_mm + 2e-14);
                            let at_angle = bezier(trim.angle_poles.map(|x| [x, 0., 0.]), t)[0];
                            assert!((at_angle - angle).abs() <= trim.angle_error_rad + 2e-13);
                            let actual_u = if trim.band == 2 && source.root_circle().is_some() {
                                let root = source.root_circle().unwrap();
                                ((q - root.phase) / root.radius).asin()
                            } else {
                                q / source.helical_rate().abs()
                            };
                            let at_u = bezier(trim.surface_u_poles.map(|x| [x, 0., 0.]), t)[0];
                            assert!((at_u - actual_u).abs() <= trim.surface_u_error + 2e-13);
                            let h = source.helical_rate().abs();
                            let (uq, ur) = if trim.band == 2 && source.root_circle().is_some() {
                                let root = source.root_circle().unwrap();
                                (
                                    root.phase + root.radius * at_u.sin(),
                                    root.center_radius - root.radius * at_u.cos(),
                                )
                            } else {
                                let uq = at_u * h;
                                (uq, source.radius_at_phase(uq))
                            };
                            let surface_angle =
                                (height - uq - f64::from(trim.start) * source.spec().pitch_mm)
                                    / source.helical_rate();
                            let thread_point =
                                [ur * surface_angle.cos(), ur * surface_angle.sin(), height];
                            let distance = |target: Point| {
                                target
                                    .into_iter()
                                    .zip(point)
                                    .map(|(a, b)| (a - b).abs())
                                    .sum::<f64>()
                            };
                            assert!(distance(thread_point) <= options.tolerance + 2e-13);
                            if trim.active {
                                let radius = body.entry_radius_mm()
                                    + ((source.spec().major_radius_mm - body.entry_radius_mm())
                                        / body.entry_length_mm())
                                        * height;
                                assert!(
                                    distance([
                                        radius * at_angle.cos(),
                                        radius * at_angle.sin(),
                                        height
                                    ]) <= options.tolerance + 2e-13
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn bounded_trim_preparation_cancels_and_can_retry_without_mutating_source() {
        let source = super::super::tests::tool(false, 1, true);
        let body = Chamfer::new(source, 0.4, None).unwrap();
        assert!(matches!(
            prepare(
                &body,
                QueryOptions {
                    tolerance: 0.,
                    maximum_cells: 100
                },
                None
            ),
            Err(Error::Invalid)
        ));
        assert!(matches!(
            prepare(
                &body,
                QueryOptions {
                    tolerance: 1e-6,
                    maximum_cells: 0
                },
                None
            ),
            Err(Error::Limit)
        ));
        assert!(matches!(
            prepare(&body, QueryOptions::default(), Some(&|| true)),
            Err(Error::Cancelled)
        ));
        assert!(matches!(
            prepare(
                &body,
                QueryOptions {
                    tolerance: 1e-6,
                    maximum_cells: 1
                },
                None
            ),
            Err(Error::Limit)
        ));
        assert!(
            !prepare(&body, QueryOptions::default(), None)
                .unwrap()
                .is_empty()
        );
        assert_eq!(body.source_thread(), &source);
    }

    #[test]
    fn malformed_or_reversed_complete_curves_cannot_claim_source_chart_order() {
        let source = super::super::tests::tool(false, 1, true);
        let body = Chamfer::new(source, 0.4, None).unwrap();
        let trim = prepare(&body, QueryOptions::default(), None)
            .unwrap()
            .remove(0);
        let mut reverse = trim.poles;
        reverse.reverse();
        assert!(
            !advances_phase(
                reverse.map(|p| p.map(I::point)),
                body.rate_interval().unwrap()
            )
            .unwrap()
        );
        assert!(!advances_phase([[I::point(0.); 3]; 4], I::point(1.)).unwrap());
        let mut broken = trim.poles.map(|p| p.map(I::point));
        broken[1][0] = I::point(f64::NAN);
        assert_eq!(advances_phase(broken, I::point(1.)), Err(Error::Unresolved));
        assert!(!advances_pcurves([0.; 4], [0.; 4], [0., 1., 2., 3.], I::point(1.)).unwrap());
        assert!(
            !advances_pcurves([0.; 4], [0., -1., -2., -3.], [3., 2., 1., 0.], I::point(1.))
                .unwrap()
        );
        assert!(Jet::constant(I::point(-1.)).sqrt().is_err());
        assert!(Jet::constant(I::point(0.)).reciprocal().is_err());
        assert!(Jet::constant(I::point(f64::NAN)).sin_cos().is_err());
        let mut segment = body.phase_segments().unwrap()[0];
        segment.ends[1] = segment.ends[0];
        assert!(matches!(propose(&body, 0, segment), Err(Error::Unresolved)));
        assert!(
            body.trim_jets(
                2,
                true,
                I::point(source.root_circle().unwrap().phase + 2.),
                0
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn source_junctions_and_periodic_chart_controls_refuse_collapse_or_unproved_precision() {
        let source = super::super::tests::tool(false, 1, true);
        let body = Chamfer::new(source, 0.4, None).unwrap();
        let trim = prepare(&body, QueryOptions::default(), None)
            .unwrap()
            .remove(0);
        let domain = [trim.surface_u_poles[0], trim.surface_u_poles[3]];
        assert!(
            trim.clone()
                .surface_branch(&body, source.spec().starts + 1, domain)
                .is_err()
        );
        assert!(trim.clone().surface_branch(&body, 0, [0., 0.]).is_err());
        assert!(trim.clone().surface_branch(&body, 0, [0., 1e-100]).is_err());
        assert!(
            trim.clone()
                .with_endpoints(&body, [[f64::NAN, 0., 0.], trim.poles[3]])
                .is_err()
        );
        assert!(
            trim.clone()
                .with_endpoints(&body, [trim.poles[3], trim.poles[0]])
                .is_err()
        );
        let mut bad = trim.clone();
        bad.height_poles[0] = f64::NAN;
        assert!(bad.surface_branch(&body, 0, domain).is_err());
        let mut bad = trim.clone();
        bad.surface_u_poles = [0.; 4];
        assert!(
            bad.with_endpoints(&body, [trim.poles[0], trim.poles[3]])
                .is_err()
        );
        assert!(matches!(
            prepare(
                &body,
                QueryOptions {
                    tolerance: 1e-300,
                    maximum_cells: 1_000_000
                },
                None
            ),
            Err(Error::Unresolved)
        ));
        let mut bad = trim.clone();
        bad.angle_poles[0] = f64::NAN;
        assert!(
            bad.clone()
                .with_endpoints(&body, [trim.poles[0], trim.poles[3]])
                .is_err()
        );
        assert!(bad.ordered(&body).is_err());
        let mut bad = trim.clone();
        bad.poles[0][0] = f64::NAN;
        assert!(bad.ordered(&body).is_err());
        for tolerance in [1e-15, 3e-15, 1e-14, 3e-14, 1e-13] {
            let result = prepare(
                &body,
                QueryOptions {
                    tolerance,
                    maximum_cells: 1_000_000,
                },
                None,
            );
            assert!(result.is_ok() || matches!(result, Err(Error::Unresolved | Error::Limit)));
        }
        let mut inactive = body
            .phase_segments()
            .unwrap()
            .into_iter()
            .find(|s| !s.active)
            .unwrap();
        inactive.active = true;
        assert!(matches!(
            propose(&body, 0, inactive),
            Err(Error::Unresolved)
        ));
    }
}

//! Tensor-product STEP support for the inward helical cavity wall.
use super::{
    DEGREE, FACTORIAL_12, HALF_SPAN_POWER, Point, StepError, bezier_pole_count, circular_pole,
    helical, knot, multiplicities,
};
use crate::helical_tube_cut::{BoundaryEdge, Cut};
use helical::product;
use std::f64::consts::TAU;

// Reserve the other STEP entities for trims, stock faces, and shell incidence.
const MAX_POLES: usize = crate::step_read::MAX_STEP_ENTITIES / 3;
type Complex = [f64; 2];

/// A clamped, piecewise Bézier axis for a STEP spline surface.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceAxis {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub multiplicities: Vec<usize>,
}

/// A rectangular tensor product in the local stock frame. The first axis
/// is the profile angle, and the second axis is the lifted sweep angle.
/// Poles are stored profile-major, with sweep varying fastest.
#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub axes: [SurfaceAxis; 2],
    pub poles_mm: Vec<Point>,
}

#[derive(Clone, Copy, Debug)]
pub struct CavitySurface {
    radius: f64,
    tube: f64,
    tangent: f64,
    vertical: f64,
    rise: f64,
    phase: f64,
    start: f64,
    height: f64,
    domains: [[f64; 2]; 2],
    tolerance: f64,
    roundoff: f64,
}

impl CavitySurface {
    /// Construct only from the already certified helical cut. The support
    /// extends above the cap over part of its rectangular chart.
    pub fn new(cut: Cut, tolerance: f64) -> Result<Self, StepError> {
        let spec = cut.spec();
        let radius = spec.stock_radius_mm;
        let tube = spec.tube_radius_mm;
        if !(1e-10..=1e-3).contains(&tolerance) || tolerance > tube / 64. {
            return Err(StepError::InvalidGeometry);
        }
        let rise = spec.pitch_mm / TAU;
        let length = radius.hypot(rise);
        let tangent = rise / length;
        let vertical = radius / length;
        let phase = spec.start_angle_rad;
        let start = spec.start_height_mm;
        let height = spec.stock_height_mm;
        let profile = cut.edge_domain(BoundaryEdge::TopCavity);
        let end = (height - start + vertical * tube * profile[0].sin()) / rise;
        let roundoff =
            256. * f64::EPSILON * ((radius + tube) * (1. + phase.abs() + end) + height + 1.);
        if roundoff >= tolerance * 0.25 {
            return Err(StepError::Limit);
        }
        Ok(Self {
            radius,
            tube,
            tangent,
            vertical,
            rise,
            phase,
            start,
            height,
            domains: [profile, [0., end]],
            tolerance,
            roundoff,
        })
    }

    pub fn domains(self) -> [[f64; 2]; 2] {
        self.domains
    }

    /// Exact cap-trim graph on the rectangular support. The floor uses
    /// sweep angle zero, and the side rails are the profile endpoints.
    pub fn top_parameter(self, profile_angle: f64) -> Option<f64> {
        let [lo, hi] = self.domains[0];
        (lo..=hi).contains(&profile_angle).then(|| {
            (self.height - self.start + self.vertical * self.tube * profile_angle.sin()) / self.rise
        })
    }

    pub fn surface(self) -> Result<Surface, StepError> {
        let [[a, b], [c, d]] = self.domains;
        let circle = helical::Spline::new(a, b, [1.; 2], 1., self.tolerance / (64. * self.tube))?;
        let sweep = helical::Spline::new(
            c,
            d,
            [1.; 2],
            1.,
            self.tolerance / (64. * (self.radius + self.tube)),
        )?;
        let count = pole_budget([circle.poles.len(), sweep.poles.len()])?;
        let (sin, cos) = self.phase.sin_cos();
        let spin: Vec<_> = sweep
            .poles
            .into_iter()
            .zip(affine_parameters(&sweep.knots))
            .map(|(pole, t)| (product([cos, sin], pole), t))
            .collect();
        let mut poles_mm = Vec::with_capacity(count);
        for profile in circle.poles {
            poles_mm.extend(
                spin.iter()
                    .map(|&(rotation, t)| self.point(profile, rotation, t)),
            );
        }
        let axes = [circle.knots, sweep.knots].map(|knots| SurfaceAxis {
            degree: DEGREE,
            multiplicities: multiplicities(knots.len()),
            knots,
        });
        Ok(Surface { axes, poles_mm })
    }

    /// Certify independently decoded surface coefficients. The returned
    /// bound applies to the whole rectangular support, including its trim
    /// curves. A shell reader must still certify actual trim incidence.
    pub fn certify(self, surface: &Surface) -> Result<f64, StepError> {
        let [profile_axis, sweep_axis] = &surface.axes;
        let count_axis = |axis: &SurfaceAxis, domain| {
            bezier_pole_count(axis.degree, &axis.knots, &axis.multiplicities, domain)
        };
        let dimensions = [
            count_axis(profile_axis, self.domains[0])?,
            count_axis(sweep_axis, self.domains[1])?,
        ];
        let count = pole_budget(dimensions)?;
        if surface.poles_mm.len() != count
            || surface.poles_mm.iter().flatten().any(|x| !x.is_finite())
        {
            return Err(StepError::InvalidGeometry);
        }
        let [profile, sweep] = surface.axes.each_ref().map(|axis| axis.knots.as_slice());
        let spin = ideal_poles(sweep, self.phase)
            .zip(affine_parameters(sweep))
            .collect::<Vec<_>>();
        let mut difference: f64 = 0.;
        for (row, profile) in surface
            .poles_mm
            .chunks_exact(dimensions[1])
            .zip(ideal_poles(profile, 0.))
        {
            for (actual, &(rotation, t)) in row.iter().zip(&spin) {
                let expected = self.point(profile, rotation, t);
                let error = (actual[0] - expected[0])
                    .hypot(actual[1] - expected[1])
                    .hypot(actual[2] - expected[2]);
                if (actual[2] - expected[2]).abs() > self.roundoff {
                    return Err(StepError::InvalidGeometry);
                }
                difference = difference.max(error);
            }
        }
        let [e_profile, e_sweep] = [profile, sweep].map(circular_error);
        let radial = self.tube * e_profile;
        // Each normalized circular factor has unit norm. Their product
        // error is bounded by the two individual errors and the cross term.
        let xy = radial + (self.radius + self.tube + radial) * e_sweep;
        let z = self.vertical * radial;
        let bound = xy.hypot(z) + difference + self.roundoff;
        if bound > self.tolerance {
            return Err(StepError::InvalidGeometry);
        }
        Ok(bound)
    }

    fn point(self, [cos, sin]: Complex, rotation: Complex, t: f64) -> Point {
        let [x, y] = product(
            [
                self.tube.mul_add(cos, self.radius),
                self.tangent * self.tube * sin,
            ],
            rotation,
        );
        let z = self.rise.mul_add(t, self.start) - self.vertical * self.tube * sin;
        [x, y, z]
    }
}

fn pole_budget([a, b]: [usize; 2]) -> Result<usize, StepError> {
    if a > MAX_POLES / b {
        return Err(StepError::Limit);
    }
    Ok(a * b)
}

fn affine_parameters(knots: &[f64]) -> impl Iterator<Item = f64> + '_ {
    std::iter::once(knots[0]).chain(
        knots
            .windows(2)
            .flat_map(|span| (1..=DEGREE).map(move |i| knot(span[0], span[1], i))),
    )
}

fn ideal_poles(knots: &[f64], phase: f64) -> impl Iterator<Item = Complex> + '_ {
    std::iter::once(circular_pole(knots[0] + phase, 0., 0)).chain(knots.windows(2).flat_map(
        move |span| {
            let [a, b] = [span[0], span[1]];
            let h = b - a;
            (1..=DEGREE).map(move |i| {
                if i <= DEGREE / 2 {
                    circular_pole(a + phase, h, i)
                } else {
                    circular_pole(b + phase, -h, DEGREE - i)
                }
            })
        },
    ))
}

fn circular_error(knots: &[f64]) -> f64 {
    let max_span = knots
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .fold(0., f64::max);
    max_span.powi(12) / (FACTORIAL_12 * HALF_SPAN_POWER)
}

//! Exact spline editing in the authored parameter domain.
//! Segmentation uses homogeneous knot insertion. Continuation adds
//! a finite straight tangent span without fitting or extrapolating old spans.
use crate::elementary_curves::ElementaryCurve;
use crate::{BSpline2d, CurvedEdge2d, GeometryError, GeometryErrorKind, GeometryPolicy, ffi};

mod bernstein_projection;
mod linear_projection;

pub(crate) use bernstein_projection::{
    line_contacts, project_controlled as bernstein_project_controlled, regions,
};
pub(crate) type QueryCheckpoint<'a> = Option<&'a dyn Fn() -> Result<(), GeometryError>>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurveProjection2d {
    pub parameter: f64,
    pub point: [f64; 2],
    pub tangent: [f64; 2],
    pub distance_mm: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TangentContinuation2d {
    pub curve: BSpline2d,
    /// Native parameters of the unmodified original interval inside `curve`.
    pub original_range: [f64; 2],
}
fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "exact spline edit",
        kind,
        message: message.into(),
    }
}
impl CurvedEdge2d {
    /// All isolated nearest candidates within modeling tolerance, including
    /// finite endpoints. Distinct parameters at the same point remain distinct.
    pub fn project_parameters(
        &self,
        point: [f64; 2],
        policy: GeometryPolicy,
    ) -> Result<Vec<CurveProjection2d>, GeometryError> {
        self.project_parameters_with_provider(point, policy, crate::OCCT_LINKED)
    }

    pub(crate) fn rust_project_parameters(
        &self,
        point: [f64; 2],
        policy: GeometryPolicy,
    ) -> Result<Vec<CurveProjection2d>, GeometryError> {
        self.project_parameters_with_provider(point, policy, false)
    }

    fn project_parameters_with_provider(
        &self,
        point: [f64; 2],
        policy: GeometryPolicy,
        native_splines: bool,
    ) -> Result<Vec<CurveProjection2d>, GeometryError> {
        policy.point([point[0], point[1], 0.])?;
        if let Self::BSpline(spline) = self {
            if spline.degree == 1 {
                return linear_projection::project(spline, point, policy);
            }
            if !native_splines {
                if (2..=3).contains(&spline.degree) {
                    return bernstein_projection::project(spline, point, policy);
                }
                spline.validate(policy)?;
                return Err(failure(
                    GeometryErrorKind::Unsupported,
                    "Rust spline projection currently supports splines up to degree three",
                ));
            }
        }
        let data = self.data(policy, 0)?;
        if let Some(elementary) = ElementaryCurve::new(self, &data) {
            return elementary.project(point, policy.linear_tolerance_mm);
        }
        let result = ffi::curve_project(
            &data,
            &ffi::Point3 {
                x: point[0],
                y: point[1],
                z: 0.,
            },
            policy.linear_tolerance_mm,
        )
        .map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?;
        let domain = self.domain(policy)?;
        result
            .into_iter()
            .map(|v| {
                let value = CurveProjection2d {
                    parameter: v.parameter,
                    point: [v.point.x, v.point.y],
                    tangent: [v.tangent.x, v.tangent.y],
                    distance_mm: v.distance,
                };
                if !value.parameter.is_finite()
                    || value.parameter < domain[0]
                    || value.parameter > domain[1]
                    || !value.distance_mm.is_finite()
                    || value.distance_mm < 0.
                    || value
                        .point
                        .iter()
                        .chain(&value.tangent)
                        .any(|x| !x.is_finite())
                    || (value.tangent[0].hypot(value.tangent[1]) - 1.).abs() > 1e-9
                    || ((point[0] - value.point[0]).hypot(point[1] - value.point[1])
                        - value.distance_mm)
                        .abs()
                        > policy.linear_tolerance_mm * 4.
                {
                    return Err(failure(
                        GeometryErrorKind::InvalidResult,
                        "Native projection contains inconsistent point/parameter/tangent/distance",
                    ));
                }
                Ok(value)
            })
            .collect()
    }
}
impl BSpline2d {
    pub fn segment(
        &self,
        from: f64,
        to: f64,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        self.validate(policy)?;
        let (poles, basis) = self
            .basis()
            .segment(&self.poles, from, to)
            .map_err(|e| failure(GeometryErrorKind::Input, e.to_string()))?;
        let result = Self {
            degree: basis.degree,
            poles,
            knots: basis.knots,
            multiplicities: basis.multiplicities,
            weights: basis.weights,
        };
        result.validate(policy)?;
        self.verify_segment_endpoints(&result, from, to, policy)?;
        Ok(result)
    }

    pub(crate) fn segment_native(
        &self,
        from: f64,
        to: f64,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        self.validate(policy)?;
        let original = CurvedEdge2d::BSpline(self.clone());
        let native = ffi::spline_segment(&original.data(policy, 0)?, from, to)
            .map_err(|e| failure(GeometryErrorKind::Native, e.to_string()))?;
        let (poles, remainder) = native.poles.as_chunks::<2>();
        if !remainder.is_empty() {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Native spline segment returned an incomplete pole coordinate",
            ));
        }
        let result = Self {
            degree: native.degree,
            poles: poles.to_vec(),
            knots: native.knots,
            multiplicities: native.multiplicities,
            weights: (!native.weights.is_empty()).then_some(native.weights),
        };
        result.validate(policy)?;
        self.verify_segment_endpoints(&result, from, to, policy)?;
        Ok(result)
    }

    fn verify_segment_endpoints(
        &self,
        result: &Self,
        from: f64,
        to: f64,
        policy: GeometryPolicy,
    ) -> Result<(), GeometryError> {
        let domain = [result.knots[0], result.knots[result.knots.len() - 1]];
        for (before, after) in [(from, domain[0]), (to, domain[1])] {
            let a = self.evaluate(before, policy)?.point;
            let b = result.evaluate(after, policy)?.point;
            if (a[0] - b[0]).hypot(a[1] - b[1]) > policy.linear_tolerance_mm {
                return Err(failure(
                    GeometryErrorKind::InvalidResult,
                    "Spline segmentation changed a requested endpoint beyond modeling tolerance",
                ));
            }
        }
        Ok(())
    }
    /// Add one straight G1 tangent span. The original native parameter interval,
    /// poles and weights remain intact; the old clamped end becomes an internal
    /// degree-multiplicity knot. This makes no global parametric-C1 promise.
    pub fn tangent_continuation(
        &self,
        at_start: bool,
        max_length_mm: f64,
        policy: GeometryPolicy,
    ) -> Result<TangentContinuation2d, GeometryError> {
        self.validate(policy)?;
        policy.length(max_length_mm)?;
        let degree = self.degree as usize;
        if self.poles.len() + degree > crate::MAX_SPLINE_POLES {
            return Err(failure(
                GeometryErrorKind::Limit,
                "Tangent continuation exceeds the 256-pole basis budget",
            ));
        }
        let first = self.knots[0];
        let last = *self.knots.last().expect("validated knots");
        let endpoint = if at_start { first } else { last };
        let jet = self.evaluate(endpoint, policy)?;
        let speed = jet.derivative[0].hypot(jet.derivative[1]);
        if !speed.is_finite() || speed <= 0. {
            return Err(failure(
                GeometryErrorKind::Input,
                "Spline endpoint tangent is degenerate; a unique continuation is unavailable",
            ));
        }
        let span = max_length_mm / speed;
        let extended = if at_start { first - span } else { last + span };
        if !span.is_finite()
            || span < 1e-12
            || !extended.is_finite()
            || extended.abs() > 1e9
            || extended == endpoint
        {
            return Err(failure(
                GeometryErrorKind::Input,
                "Tangent continuation has an ill-conditioned or out-of-budget parameter span",
            ));
        }
        let direction = jet.derivative.map(|v| v / speed);
        let end: [f64; 2] = std::array::from_fn(|i| {
            jet.point[i] + direction[i] * max_length_mm * if at_start { -1. } else { 1. }
        });
        policy.point([end[0], end[1], 0.])?;
        let mut result = self.clone();
        let mut new_poles = vec![];
        if at_start {
            for j in 0..degree {
                new_poles.push(std::array::from_fn(|i| {
                    jet.point[i]
                        - direction[i] * max_length_mm * (degree - j) as f64 / degree as f64
                }));
            }
            new_poles.extend_from_slice(&self.poles);
            result.poles = new_poles;
            result.multiplicities[0] = self.degree;
            result.knots.insert(0, extended);
            result.multiplicities.insert(0, self.degree + 1);
            if let Some(weights) = &mut result.weights {
                let mut extra = vec![weights[0]; degree];
                extra.extend_from_slice(weights);
                *weights = extra;
            }
        } else {
            for j in 1..=degree {
                result.poles.push(std::array::from_fn(|i| {
                    jet.point[i] + direction[i] * max_length_mm * j as f64 / degree as f64
                }));
            }
            *result.multiplicities.last_mut().expect("validated mults") = self.degree;
            result.knots.push(extended);
            result.multiplicities.push(self.degree + 1);
            if let Some(weights) = &mut result.weights {
                weights.extend(std::iter::repeat_n(
                    *weights.last().expect("validated weights"),
                    degree,
                ));
            }
        }
        result.validate(policy)?;
        let actual = result.evaluate(extended, policy)?.point;
        if (actual[0] - end[0]).hypot(actual[1] - end[1]) > policy.linear_tolerance_mm {
            return Err(failure(
                GeometryErrorKind::InvalidResult,
                "Constructed tangent span does not reach its bounded endpoint",
            ));
        }
        let original_range = [first, last];
        Ok(TangentContinuation2d {
            curve: result,
            original_range,
        })
    }
}

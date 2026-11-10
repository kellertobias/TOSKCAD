//! STEP exchange of an analytic spur outline with certified cubic flanks.
use super::{normal_sweep::ProfileEdge, *};
use crate::spur_gear::{CubicSpline, Edge, Error, Involute, Profile};

/// Maximum geometric deviation of a STEP spur flank, in millimeters.
/// The writer also honors smaller caller tolerances.
pub const SPUR_STEP_MAX_ERROR_MM: f64 = 1e-7;

pub fn write_spur_gear(
    profile: &Profile,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_spur_gear(profile, origin, u, v, tolerance_mm))
}

pub(super) fn encode(
    file: &mut StepFile,
    profile: &Profile,
    placement: Placement,
    tolerance_mm: f64,
) -> Result<Entity, StepError> {
    if !tolerance_mm.is_finite() || tolerance_mm <= 0. {
        return Err(StepError::InvalidGeometry);
    }
    if profile
        .hub_extensions()
        .into_iter()
        .any(|extension| extension.is_some())
    {
        return Err(StepError::Unsupported);
    }
    let tolerance = tolerance_mm.min(SPUR_STEP_MAX_ERROR_MM);
    let outline = profile
        .outline()
        .iter()
        .map(|edge| match edge.edge {
            Edge::Planar(edge) => Ok(ProfileEdge::Analytic(edge)),
            Edge::Involute(flank) => FlankSpline::new(flank, tolerance).map(ProfileEdge::Cubic),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let bore = profile.bore().map(|bore| vec![ProfileEdge::Analytic(bore)]);
    normal_sweep::encode_rings(
        file,
        &mut std::iter::once(outline).chain(bore),
        profile.height_mm(),
        &[],
        placement,
        false,
    )
}

pub(super) struct FlankSpline {
    curve: CubicSpline,
    tolerance_mm: f64,
}

impl FlankSpline {
    fn new(flank: Involute, tolerance_mm: f64) -> Result<Self, StepError> {
        // Reserve half the budget for placing the poles in world coordinates
        // and snapping their endpoints to the shared STEP rim vertices.
        let curve = flank
            .cubic_spline(tolerance_mm / 2.)
            .map_err(|error| match error {
                Error::Limit => StepError::Limit,
                Error::InvalidDimensions | Error::InvalidProfile => StepError::InvalidGeometry,
            })?;
        Ok(Self {
            curve,
            tolerance_mm,
        })
    }

    pub(super) fn start(&self) -> [f64; 2] {
        self.curve.poles_mm[0]
    }

    pub(super) fn encode(
        &self,
        file: &mut StepFile,
        [from, to]: [Vertex; 2],
        placement: &Placement,
        z: f64,
    ) -> Result<(Entity, Entity), StepError> {
        if from.point == to.point {
            return Err(StepError::InvalidGeometry);
        }
        let points = &self.curve.poles_mm;
        let end = points.len() - 1;
        let discrepancy = |point: Point, pole: [f64; 2]| {
            let placed = placement.point_f64(pole, z);
            let [x, y, z] = std::array::from_fn(|i| point[i] - placed[i]);
            x.hypot(y).hypot(z)
        };
        let endpoint_error =
            discrepancy(from.point, points[0]).max(discrepancy(to.point, points[end]));
        let scale = points
            .iter()
            .map(|[x, y]| x.abs() + y.abs())
            .fold(0., f64::max)
            + placement
                .origin
                .into_iter()
                .map(f64::abs)
                .fold(0., f64::max)
            + z.abs();
        // Placement validates the near-orthonormal basis. The remaining
        // term bounds floating-point placement and decimal STEP rounding.
        let bound =
            self.curve.error_bound_mm * (1. + 1e-9) + endpoint_error + 16. * f64::EPSILON * scale;
        if !bound.is_finite() || bound > self.tolerance_mm {
            return Err(StepError::Limit);
        }
        let mut poles = Vec::with_capacity(points.len());
        poles.push(from.cartesian);
        poles.extend(
            points[1..end]
                .iter()
                .map(|&pole| file.point(placement.point_f64(pole, z))),
        );
        poles.push(to.cartesian);
        let multiplicities = self
            .curve
            .multiplicities
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let knots = self
            .curve
            .knots
            .iter()
            .map(|knot| format!("{knot:.16E}"))
            .collect::<Vec<_>>()
            .join(",");
        let curve = file.entity(format_args!(
            "B_SPLINE_CURVE_WITH_KNOTS('',3,{},.UNSPECIFIED.,.F.,.F.,({multiplicities}),({knots}),.UNSPECIFIED.)",
            Entities(&poles)
        ));
        let (start, end) = (from.vertex, to.vertex);
        let edge = file.entity(format_args!("EDGE_CURVE('',{start},{end},{curve},.T.)"));
        Ok((edge, curve))
    }
}

#[cfg(test)]
mod tests;

//! Exact nonperiodic clamped B-spline groundwork. Native queries use the basis
//! directly; display sampling never participates in profile construction.
use crate::elementary_curves::ElementaryCurve;
use crate::{GeometryError, GeometryErrorKind, GeometryPolicy, PlaneFrame, Solid, ffi};
use multicad_core::{
    document::EntityId,
    references::{Origin, ReferenceFrame},
};

pub const MAX_SPLINE_POLES: usize = 256;
pub const MAX_CURVED_PROFILE_EDGES: usize = 64;

// The native exact face builder checks pairs of curves. Keep a finite
// manufacturing budget independent of the much smaller interactive budget.
const MAX_DEVELOPED_SHEET_EDGES: usize = 2048;
const MAX_DEVELOPED_SHEET_HOLES: usize = multicad_core::manufacturing::MAX_PLANAR_LOOPS - 1;
#[derive(Clone, Copy)]
enum ProfileBudget {
    Interactive,
    CabinetPanel,
    DevelopedSheet,
}
impl ProfileBudget {
    fn native_manufacturing(self) -> bool {
        matches!(self, Self::CabinetPanel | Self::DevelopedSheet)
    }
}

fn error(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    GeometryError {
        operation: "curved profile",
        kind,
        message: message.into(),
    }
}
fn input(message: impl Into<String>) -> GeometryError {
    error(GeometryErrorKind::Input, message)
}
fn native(e: impl std::fmt::Display) -> GeometryError {
    error(GeometryErrorKind::Native, e.to_string())
}
fn p(p: [f64; 2]) -> ffi::Point3 {
    ffi::Point3 {
        x: p[0],
        y: p[1],
        z: 0.,
    }
}
fn empty_spline() -> ffi::SplineData {
    ffi::SplineData {
        degree: 0,
        poles: vec![],
        knots: vec![],
        multiplicities: vec![],
        weights: vec![],
    }
}
pub(crate) fn arc_sweep(center: [f64; 2], a: [f64; 2], b: [f64; 2], clockwise: bool) -> f64 {
    let first = (a[1] - center[1]).atan2(a[0] - center[0]);
    let last = (b[1] - center[1]).atan2(b[0] - center[0]);
    (if clockwise {
        first - last
    } else {
        last - first
    })
    .rem_euclid(std::f64::consts::TAU)
}
#[derive(Clone, Debug, PartialEq)]
pub struct BSpline2d {
    pub degree: u32,
    pub poles: Vec<[f64; 2]>,
    /// Distinct strictly increasing knot values, not an expanded knot vector.
    pub knots: Vec<f64>,
    pub multiplicities: Vec<u32>,
    pub weights: Option<Vec<f64>>,
}
impl BSpline2d {
    pub fn validate(&self, policy: GeometryPolicy) -> Result<(), GeometryError> {
        policy.validate()?;
        multicad_core::constraint_sketch::SplineBasis {
            degree: self.degree,
            knots: self.knots.clone(),
            multiplicities: self.multiplicities.clone(),
            weights: self.weights.clone(),
        }
        .validate(self.poles.len())
        .map_err(|e| input(e.to_string()))?;
        for point in &self.poles {
            policy.point([point[0], point[1], 0.])?;
        }
        if self.poles.iter().all(|p| {
            (p[0] - self.poles[0][0]).hypot(p[1] - self.poles[0][1]) < policy.minimum_feature_mm
        }) {
            return Err(input("Spline control polygon is degenerate"));
        }
        Ok(())
    }
    fn data(&self) -> ffi::SplineData {
        ffi::SplineData {
            degree: self.degree,
            poles: self.poles.iter().flatten().copied().collect(),
            knots: self.knots.clone(),
            multiplicities: self.multiplicities.clone(),
            weights: self.weights.clone().unwrap_or_default(),
        }
    }
    pub fn evaluate(
        &self,
        parameter: f64,
        policy: GeometryPolicy,
    ) -> Result<CurveSample2d, GeometryError> {
        CurvedEdge2d::BSpline(self.clone()).evaluate(parameter, policy)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum CurvedEdge2d {
    Line {
        a: [f64; 2],
        b: [f64; 2],
    },
    Arc {
        center: [f64; 2],
        a: [f64; 2],
        b: [f64; 2],
        clockwise: bool,
    },
    Circle {
        center: [f64; 2],
        radius_mm: f64,
    },
    BSpline(BSpline2d),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurveSample2d {
    pub point: [f64; 2],
    pub derivative: [f64; 2],
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurveIntersection2d {
    pub point: [f64; 2],
    pub first_parameter: f64,
    pub second_parameter: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct CurveIntersections2d {
    pub points: Vec<CurveIntersection2d>,
    pub overlap_segments: u32,
}
impl CurvedEdge2d {
    /// Native parameter domain: lines use arc length, splines retain their
    /// authored knot domain. No normalization or reparameterization is applied.
    pub fn domain(&self, policy: GeometryPolicy) -> Result<[f64; 2], GeometryError> {
        self.data(policy, 0)?;
        Ok(match self {
            Self::Line { a, b } => [0., (b[0] - a[0]).hypot(b[1] - a[1])],
            Self::Arc {
                center,
                a,
                b,
                clockwise,
            } => [0., arc_sweep(*center, *a, *b, *clockwise)],
            Self::Circle { .. } => [0., std::f64::consts::TAU],
            Self::BSpline(s) => [s.knots[0], *s.knots.last().expect("validated knots")],
        })
    }
    pub(crate) fn data(
        &self,
        policy: GeometryPolicy,
        source_index: u32,
    ) -> Result<ffi::CurvedEdgeData, GeometryError> {
        policy.validate()?;
        Ok(match self {
            Self::Line { a, b } => {
                for point in [a, b] {
                    policy.point([point[0], point[1], 0.])?;
                }
                policy.length((a[0] - b[0]).hypot(a[1] - b[1]))?;
                ffi::CurvedEdgeData {
                    kind: 0,
                    a: p(*a),
                    b: p(*b),
                    center: p([0.; 2]),
                    radius: 0.,
                    clockwise: false,
                    spline: empty_spline(),
                    source_index,
                }
            }
            Self::BSpline(s) => {
                s.validate(policy)?;
                ffi::CurvedEdgeData {
                    kind: 1,
                    a: p(s.poles[0]),
                    b: p(*s.poles.last().expect("validated poles")),
                    center: p([0.; 2]),
                    radius: 0.,
                    clockwise: false,
                    spline: s.data(),
                    source_index,
                }
            }
            Self::Arc {
                center,
                a,
                b,
                clockwise,
            } => {
                for point in [center, a, b] {
                    policy.point([point[0], point[1], 0.])?;
                }
                let radius = (a[0] - center[0]).hypot(a[1] - center[1]);
                policy.length(radius)?;
                if (radius - (b[0] - center[0]).hypot(b[1] - center[1])).abs()
                    > policy.linear_tolerance_mm
                    || (a[0] - b[0]).hypot(a[1] - b[1]) < policy.minimum_feature_mm
                {
                    return Err(input(
                        "Arc endpoints must be distinct and lie on the same authored circle",
                    ));
                }
                let sweep = arc_sweep(*center, *a, *b, *clockwise);
                if sweep <= 0. || sweep >= std::f64::consts::TAU {
                    return Err(input(
                        "Arc sweep must be positive and smaller than a full circle",
                    ));
                }
                ffi::CurvedEdgeData {
                    kind: 2,
                    a: p(*a),
                    b: p(*b),
                    center: p(*center),
                    radius,
                    clockwise: *clockwise,
                    spline: empty_spline(),
                    source_index,
                }
            }
            Self::Circle { center, radius_mm } => {
                policy.point([center[0], center[1], 0.])?;
                policy.length(*radius_mm)?;
                ffi::CurvedEdgeData {
                    kind: 3,
                    a: p([center[0] + radius_mm, center[1]]),
                    b: p([center[0] + radius_mm, center[1]]),
                    center: p(*center),
                    radius: *radius_mm,
                    clockwise: false,
                    spline: empty_spline(),
                    source_index,
                }
            }
        })
    }
    pub fn evaluate(
        &self,
        parameter: f64,
        policy: GeometryPolicy,
    ) -> Result<CurveSample2d, GeometryError> {
        let curve = self.data(policy, 0)?;
        if let Some(elementary) = ElementaryCurve::new(self, &curve) {
            return elementary.evaluate(parameter);
        }
        let out = ffi::curve_evaluate(&curve, parameter).map_err(native)?;
        let point = [out.point.x, out.point.y];
        let derivative = [out.derivative.x, out.derivative.y];
        if point.iter().chain(&derivative).any(|v| !v.is_finite()) {
            return Err(error(
                GeometryErrorKind::InvalidResult,
                "Native curve evaluation was nonfinite",
            ));
        }
        Ok(CurveSample2d { point, derivative })
    }
    pub fn intersections(
        &self,
        other: &Self,
        policy: GeometryPolicy,
    ) -> Result<CurveIntersections2d, GeometryError> {
        self.query(other, policy, false)
    }
    pub fn self_intersections(
        &self,
        policy: GeometryPolicy,
    ) -> Result<CurveIntersections2d, GeometryError> {
        self.query(self, policy, true)
    }
    fn query(
        &self,
        other: &Self,
        policy: GeometryPolicy,
        self_query: bool,
    ) -> Result<CurveIntersections2d, GeometryError> {
        let a = self.data(policy, 0)?;
        let b = other.data(policy, 0)?;
        let result = ffi::curve_intersections(&a, &b, policy.linear_tolerance_mm, self_query)
            .map_err(native)?;
        if result.points.iter().any(|i| {
            [i.point.x, i.point.y, i.first, i.second]
                .iter()
                .any(|v| !v.is_finite())
        }) {
            return Err(error(
                GeometryErrorKind::InvalidResult,
                "Native curve intersection was nonfinite",
            ));
        }
        let points = result
            .points
            .into_iter()
            .map(|i| CurveIntersection2d {
                point: [i.point.x, i.point.y],
                first_parameter: i.first,
                second_parameter: i.second,
            })
            .collect();
        Ok(CurveIntersections2d {
            points,
            overlap_segments: result.overlaps,
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct CurvedProfile {
    pub frame: PlaneFrame,
    pub outer: Vec<CurvedEdge2d>,
    pub holes: Vec<Vec<CurvedEdge2d>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurvePointLocation {
    Outside,
    Boundary,
    Inside,
}
pub(crate) struct Checked {
    pub(crate) frame: ffi::Frame3,
    pub(crate) edges: Vec<ffi::CurvedEdgeData>,
    pub(crate) offsets: Vec<u32>,
}
impl CurvedProfile {
    pub(crate) fn signed_area_mm2(&self, policy: GeometryPolicy) -> Result<f64, GeometryError> {
        let c = self.checked(policy)?;
        let area = ffi::curved_profile_signed_area(
            &c.frame,
            &c.edges,
            &c.offsets,
            policy.linear_tolerance_mm,
        )
        .map_err(native)?;
        if !area.is_finite() || area.abs() < policy.minimum_feature_mm.powi(2) {
            return Err(input("Curved profile has no finite bounded area"));
        }
        Ok(area)
    }
    pub(crate) fn checked(&self, policy: GeometryPolicy) -> Result<Checked, GeometryError> {
        self.checked_for(policy, ProfileBudget::Interactive)
    }

    fn checked_for(
        &self,
        policy: GeometryPolicy,
        budget: ProfileBudget,
    ) -> Result<Checked, GeometryError> {
        policy.validate()?;
        let frame = self.frame.checked(policy)?;
        let count = self.outer.len() + self.holes.iter().map(Vec::len).sum::<usize>();
        let (max_edges, max_holes, message) = match budget {
            ProfileBudget::Interactive => (
                MAX_CURVED_PROFILE_EDGES,
                16,
                "Curved profile exceeds the 64-edge/16-hole query budget",
            ),
            ProfileBudget::CabinetPanel => (
                128,
                32,
                "Panel profile exceeds the 128-edge/32-hole manufacturing budget",
            ),
            ProfileBudget::DevelopedSheet => (
                MAX_DEVELOPED_SHEET_EDGES,
                MAX_DEVELOPED_SHEET_HOLES,
                "Developed sheet exceeds the 2048-edge/255-hole manufacturing budget",
            ),
        };
        if count > max_edges || self.holes.len() > max_holes {
            return Err(error(GeometryErrorKind::Limit, message));
        }
        let mut edges = vec![];
        let mut offsets = vec![0];
        let mut poles = 0;
        for wire in std::iter::once(&self.outer).chain(&self.holes) {
            if wire.is_empty() {
                return Err(input("Curved profile wires cannot be empty"));
            }
            for curve in wire {
                let data = curve.data(policy, edges.len() as u32)?;
                poles += data.spline.poles.len() / 2;
                if poles > 2048 {
                    return Err(error(
                        GeometryErrorKind::Limit,
                        "Curved profile pole budget exceeded",
                    ));
                }
                match curve {
                    CurvedEdge2d::Line { a, b } => {
                        policy.point(self.frame.point(*a))?;
                        policy.point(self.frame.point(*b))?;
                    }
                    CurvedEdge2d::BSpline(s) => {
                        for pole in &s.poles {
                            policy.point(self.frame.point(*pole))?;
                        }
                    }
                    CurvedEdge2d::Arc { center, .. } | CurvedEdge2d::Circle { center, .. } => {
                        let radius = match curve {
                            CurvedEdge2d::Arc { a, .. } => {
                                (a[0] - center[0]).hypot(a[1] - center[1])
                            }
                            CurvedEdge2d::Circle { radius_mm, .. } => *radius_mm,
                            _ => unreachable!(),
                        };
                        for axis in 0..3 {
                            let extent = radius * self.frame.u[axis].hypot(self.frame.v[axis]);
                            let position = self.frame.point(*center)[axis];
                            if !position.is_finite()
                                || position.abs() + extent > policy.maximum_coordinate_mm
                            {
                                return Err(input(
                                    "Curved profile circle envelope exceeds model bounds",
                                ));
                            }
                        }
                    }
                }
                edges.push(data);
            }
            offsets.push(edges.len() as u32);
        }
        Ok(Checked {
            frame,
            edges,
            offsets,
        })
    }
    /// OCCT integrates the true bounded face, including holes; no display mesh.
    pub fn area_mm2(&self, policy: GeometryPolicy) -> Result<f64, GeometryError> {
        let c = self.checked(policy)?;
        let area =
            ffi::curved_profile_area(&c.frame, &c.edges, &c.offsets, policy.linear_tolerance_mm)
                .map_err(native)?;
        if !area.is_finite() || area < policy.minimum_feature_mm.powi(2) {
            return Err(error(
                GeometryErrorKind::InvalidResult,
                "Curved profile has no positive bounded area",
            ));
        }
        Ok(area)
    }
    pub fn classify(
        &self,
        point: [f64; 2],
        policy: GeometryPolicy,
    ) -> Result<CurvePointLocation, GeometryError> {
        policy.point([point[0], point[1], 0.])?;
        let c = self.checked(policy)?;
        match ffi::curved_profile_classify(
            &c.frame,
            &c.edges,
            &c.offsets,
            policy.linear_tolerance_mm,
            &p(point),
        )
        .map_err(native)?
        {
            0 => Ok(CurvePointLocation::Outside),
            1 => Ok(CurvePointLocation::Boundary),
            2 => Ok(CurvePointLocation::Inside),
            _ => Err(error(
                GeometryErrorKind::InvalidResult,
                "Unknown exact face classification",
            )),
        }
    }
}
impl Solid {
    pub fn extrude_curved_profile(
        profile: &CurvedProfile,
        height_mm: f64,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::extrude_curved_profile_vector(
            profile,
            profile.frame.normal().map(|n| n * height_mm),
            policy,
        )
    }
    /// Sweep an unchanged section by a world-space displacement. A nonzero
    /// component normal to the section is required for a volumetric result.
    pub fn extrude_curved_profile_vector(
        profile: &CurvedProfile,
        displacement_mm: [f64; 3],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::extrude_curved_profile_vector_for(
            profile,
            displacement_mm,
            policy,
            ProfileBudget::Interactive,
        )
    }

    /// Build the exact prism used to validate a developed sheet blank.
    /// This dedicated path has a larger bounded allowance for cut openings.
    pub fn extrude_developed_sheet_profile(
        profile: &CurvedProfile,
        displacement_mm: [f64; 3],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::extrude_curved_profile_vector_for(
            profile,
            displacement_mm,
            policy,
            ProfileBudget::DevelopedSheet,
        )
    }

    fn extrude_curved_profile_vector_for(
        profile: &CurvedProfile,
        displacement_mm: [f64; 3],
        policy: GeometryPolicy,
        budget: ProfileBudget,
    ) -> Result<Self, GeometryError> {
        let c = profile.checked_for(policy, budget)?;
        policy.point(displacement_mm)?;
        let length = displacement_mm
            .into_iter()
            .map(|v| v * v)
            .sum::<f64>()
            .sqrt();
        policy.length(length)?;
        let height = profile
            .frame
            .normal()
            .into_iter()
            .zip(displacement_mm)
            .map(|(n, v)| n * v)
            .sum::<f64>();
        if !height.is_finite() || height.abs() < policy.minimum_feature_mm {
            return Err(input(
                "Extrusion displacement is parallel to the profile or below the minimum normal feature height",
            ));
        }
        Self::adopt(
            ffi::curved_profile_extrude_vector(
                &c.frame,
                &c.edges,
                &c.offsets,
                policy.linear_tolerance_mm,
                &ffi::Point3 {
                    x: displacement_mm[0],
                    y: displacement_mm[1],
                    z: displacement_mm[2],
                },
                budget.native_manufacturing(),
            )
            .map_err(native)?,
            policy,
            "vector extrusion",
        )
    }
    pub fn extrude_curved_profile_named(
        profile: &CurvedProfile,
        height_mm: f64,
        feature: EntityId,
        origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::extrude_curved_profile_vector_named(
            profile,
            profile.frame.normal().map(|n| n * height_mm),
            feature,
            origins,
            policy,
        )
    }
    pub fn extrude_curved_profile_vector_named(
        profile: &CurvedProfile,
        displacement_mm: [f64; 3],
        feature: EntityId,
        origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::extrude_curved_profile_vector_named_for(
            profile,
            displacement_mm,
            feature,
            origins,
            policy,
            ProfileBudget::Interactive,
        )
    }

    /// Exact panel-stock profiles may include many finger joints and openings.
    /// Retain native curve provenance while keeping a separate bounded budget.
    pub fn extrude_panel_profile_vector_named(
        profile: &CurvedProfile,
        displacement_mm: [f64; 3],
        feature: EntityId,
        origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::extrude_curved_profile_vector_named_for(
            profile,
            displacement_mm,
            feature,
            origins,
            policy,
            ProfileBudget::CabinetPanel,
        )
    }

    fn extrude_curved_profile_vector_named_for(
        profile: &CurvedProfile,
        displacement_mm: [f64; 3],
        feature: EntityId,
        origins: &[Origin],
        policy: GeometryPolicy,
        budget: ProfileBudget,
    ) -> Result<Self, GeometryError> {
        let edges: Vec<_> = profile
            .outer
            .iter()
            .chain(profile.holes.iter().flatten())
            .collect();
        if origins.len() != edges.len() || feature.is_nil() {
            return Err(input(
                "Every curved profile edge needs an authored origin and feature identity",
            ));
        }
        let mut solid =
            Self::extrude_curved_profile_vector_for(profile, displacement_mm, policy, budget)?;
        let frames: Vec<_> = edges
            .into_iter()
            .map(|e| match e {
                CurvedEdge2d::Line { a, b } => {
                    let a = profile.frame.point(*a);
                    let b = profile.frame.point(*b);
                    let length = (0..3).map(|i| (b[i] - a[i]).powi(2)).sum::<f64>().sqrt();
                    let u = std::array::from_fn(|i| (b[i] - a[i]) / length);
                    let along = u
                        .into_iter()
                        .zip(displacement_mm)
                        .map(|(a, b)| a * b)
                        .sum::<f64>();
                    let perpendicular: [f64; 3] =
                        std::array::from_fn(|i| displacement_mm[i] - u[i] * along);
                    let length = perpendicular.into_iter().map(|v| v * v).sum::<f64>().sqrt();
                    Some(ReferenceFrame {
                        origin_mm: a,
                        u,
                        v: perpendicular.map(|v| v / length),
                    })
                }
                CurvedEdge2d::BSpline(_)
                | CurvedEdge2d::Arc { .. }
                | CurvedEdge2d::Circle { .. } => None,
            })
            .collect();
        solid.name_profile_vector_prism(
            profile.frame,
            displacement_mm,
            feature,
            origins,
            &frames,
        )?;
        Ok(solid)
    }
}

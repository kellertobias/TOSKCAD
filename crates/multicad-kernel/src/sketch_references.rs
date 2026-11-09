//! Exact finite B-rep queries. The caller supplies a B-rep already placed once
//! in the consumer coordinate system. Keys and native parameters are transient;
//! a model adapter must derive persistent selection from source provenance.
use crate::{
    BRepSelection, BSpline2d, CurvedEdge2d, GeometryError, GeometryErrorKind, GeometryPolicy,
    PlaneFrame, Solid, ffi,
};
use multicad_core::references::{TopologyKey, TopologyKind};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchReferenceMethod {
    Coplanar,
    Project,
    Intersect,
}
#[derive(Clone, Debug, PartialEq)]
pub enum SketchReferenceGeometry {
    Point([f64; 2]),
    Curve(CurvedEdge2d),
}
#[derive(Clone, Debug, PartialEq)]
pub struct SketchReferenceResult {
    pub geometry: SketchReferenceGeometry,
    pub support: Vec<TopologyKey>,
    /// Parameter on the original finite native edge, never a result ordinal.
    pub source_parameter: Option<f64>,
    /// Native source interval; rational conversion may reparameterize output.
    pub source_interval: Option<[f64; 2]>,
    /// Proven edge/plane point tangency, isolated section touch, or analytic
    /// cylinder/sphere section tangency. False does not certify arbitrary
    /// spline-surface transversality; that general classifier is unavailable.
    pub tangent: bool,
    pub crossing: Option<i8>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct SketchReferenceCatalog {
    pub results: Vec<SketchReferenceResult>,
    /// A source curve/face lies in the target plane. This is not a unique
    /// point/section; consumers must present the overlap explicitly.
    pub coincident: bool,
}
fn error(message: impl ToString) -> GeometryError {
    GeometryError {
        operation: "sketch reference",
        kind: GeometryErrorKind::InvalidResult,
        message: message.to_string(),
    }
}
impl Solid {
    pub fn sketch_reference_geometry(
        &self,
        selection: BRepSelection,
        frame: PlaneFrame,
        method: SketchReferenceMethod,
        policy: GeometryPolicy,
    ) -> Result<SketchReferenceCatalog, GeometryError> {
        policy.validate()?;
        let native_frame = frame.checked(policy)?;
        let (kind, index) = match selection {
            BRepSelection::Whole => (0, 0),
            BRepSelection::Face(i) => (1, i),
            BRepSelection::Edge(i) => (2, i),
            BRepSelection::Vertex(i) => (3, i),
        };
        let method = match method {
            SketchReferenceMethod::Coplanar => 0,
            SketchReferenceMethod::Project => 1,
            SketchReferenceMethod::Intersect => 2,
        };
        let out = ffi::sketch_reference_geometry(
            self.occt()?,
            kind,
            index,
            &native_frame,
            method,
            policy.linear_tolerance_mm,
        )
        .map_err(error)?;
        if out.results.len() > 256 {
            return Err(error("Native reference result budget exceeded"));
        }
        let point = |p: ffi::Point3| -> Result<[f64; 2], GeometryError> {
            policy.point([p.x, p.y, p.z])?;
            if p.z.abs() > policy.linear_tolerance_mm {
                return Err(error("Native reference is outside its sketch plane"));
            }
            Ok([p.x, p.y])
        };
        let mut results = Vec::new();
        for result in out.results {
            let geometry = if result.geometry == 0 {
                SketchReferenceGeometry::Point(point(result.point)?)
            } else if result.geometry == 1 {
                let c = result.curve;
                if c.kind == 1 && (c.spline.poles.len() % 2 != 0 || c.spline.poles.len() > 512) {
                    return Err(error("Invalid native exact spline pole array"));
                }
                let curve = match c.kind {
                    0 => CurvedEdge2d::Line {
                        a: point(c.a)?,
                        b: point(c.b)?,
                    },
                    1 => CurvedEdge2d::BSpline(BSpline2d {
                        degree: c.spline.degree,
                        poles: c.spline.poles.as_chunks::<2>().0.to_vec(),
                        knots: c.spline.knots,
                        multiplicities: c.spline.multiplicities,
                        weights: (!c.spline.weights.is_empty()).then_some(c.spline.weights),
                    }),
                    2 => CurvedEdge2d::Arc {
                        center: point(c.center)?,
                        a: point(c.a)?,
                        b: point(c.b)?,
                        clockwise: c.clockwise,
                    },
                    3 => CurvedEdge2d::Circle {
                        center: point(c.center)?,
                        radius_mm: c.radius,
                    },
                    _ => return Err(error("Unknown native sketch curve family")),
                };
                curve.data(policy, 0)?;
                SketchReferenceGeometry::Curve(curve)
            } else {
                return Err(error("Unknown native sketch result kind"));
            };
            if (result.has_parameter && !result.parameter.is_finite())
                || (result.has_interval
                    && (!result.first.is_finite()
                        || !result.last.is_finite()
                        || result.first >= result.last))
                || !(-1..=1).contains(&result.crossing)
            {
                return Err(error("Invalid native source parameter metadata"));
            }
            let mut support = Vec::new();
            for source in result.support {
                support.push(TopologyKey {
                    kind: match source.kind {
                        0 => TopologyKind::Face,
                        1 => TopologyKind::Edge,
                        2 => TopologyKind::Vertex,
                        _ => return Err(error("Unknown native source support kind")),
                    },
                    ordinal: source.index,
                });
            }
            support.sort();
            support.dedup();
            if support.is_empty() {
                return Err(error(
                    "Native reference has no actual source topology support",
                ));
            }
            results.push(SketchReferenceResult {
                geometry,
                support,
                source_parameter: result.has_parameter.then_some(result.parameter),
                source_interval: result.has_interval.then_some([result.first, result.last]),
                tangent: result.tangent,
                crossing: (result.crossing != 0).then_some(result.crossing),
            });
        }
        Ok(SketchReferenceCatalog {
            results,
            coincident: out.coincident,
        })
    }
}

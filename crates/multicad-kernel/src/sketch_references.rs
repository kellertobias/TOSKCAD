//! Exact finite B-rep queries. The caller supplies a B-rep already placed once
//! in the consumer coordinate system. Keys and native parameters are transient;
//! a model adapter must derive persistent selection from source provenance.
use crate::{
    BRepSelection, BSpline2d, CurvedEdge2d, GeometryError, GeometryErrorKind, GeometryPolicy,
    PlaneFrame, Solid, ffi,
};
use multicad_core::references::{AnalyticGeometry, ReferenceFrame, TopologyKey, TopologyKind};
pub use multicad_rust_kernel::sketch_reference::Method as SketchReferenceMethod;
use multicad_rust_kernel::sketch_reference::{self, Geometry, Outcome};
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
        if self.uses_rust_geometry() {
            return self.rust_sketch_reference(selection, frame, method, policy);
        }
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

    fn rust_sketch_reference(
        &self,
        selection: BRepSelection,
        frame: PlaneFrame,
        method: SketchReferenceMethod,
        policy: GeometryPolicy,
    ) -> Result<SketchReferenceCatalog, GeometryError> {
        let unsupported = || {
            GeometryError {
            operation: "sketch reference",
            kind: GeometryErrorKind::Unsupported,
            message: "Rust sketch references support finite vertices and straight edges, and can project involute flanks".into(),
        }
        };
        let (kind, ordinal) = match selection {
            BRepSelection::Vertex(index) => (TopologyKind::Vertex, index),
            BRepSelection::Edge(index) => (TopologyKind::Edge, index),
            _ => return Err(unsupported()),
        };
        let key = TopologyKey { kind, ordinal };
        let topology = self.rust_topology().ok_or_else(unsupported)??;
        let element = topology
            .elements
            .into_iter()
            .find(|element| element.key == key)
            .ok_or_else(|| error("Selected Rust reference topology is missing"))?;
        let axes = [frame.u, frame.v, frame.normal()];
        let local = |point: [f64; 3]| {
            let delta = std::array::from_fn::<_, 3, _>(|i| point[i] - frame.origin[i]);
            axes.map(|axis| delta.into_iter().zip(axis).map(|(a, b)| a * b).sum())
        };
        let tolerance = policy.linear_tolerance_mm;
        let outcome = match (kind, element.geometry) {
            (TopologyKind::Vertex, Some(AnalyticGeometry::Point { position_mm })) => {
                sketch_reference::vertex(local(position_mm), method, tolerance)
            }
            (TopologyKind::Edge, Some(AnalyticGeometry::Line { a_mm, b_mm })) => {
                sketch_reference::edge([a_mm, b_mm].map(local), method, tolerance)
            }
            (
                TopologyKind::Edge,
                Some(AnalyticGeometry::InvoluteArc {
                    frame: source,
                    base_radius_mm,
                    parameters,
                }),
            ) => {
                return involute_reference(
                    source,
                    base_radius_mm,
                    parameters,
                    frame,
                    method,
                    key,
                    policy,
                );
            }
            _ => return Err(unsupported()),
        }
        .map_err(|cause| match cause {
            sketch_reference::Error::Invalid => error("Invalid finite Rust sketch reference"),
            sketch_reference::Error::OffPlane => {
                error("Selected finite Rust reference is not coplanar with the sketch plane")
            }
        })?;
        let mut results = Vec::new();
        if let Outcome::Hit { geometry, contact } = outcome {
            let geometry = match geometry {
                Geometry::Point(point) => {
                    policy.point([point[0], point[1], 0.])?;
                    SketchReferenceGeometry::Point(point)
                }
                Geometry::Line([a, b]) => {
                    let curve = CurvedEdge2d::Line { a, b };
                    curve.data(policy, 0)?;
                    SketchReferenceGeometry::Curve(curve)
                }
            };
            results.push(SketchReferenceResult {
                geometry,
                support: vec![key],
                source_parameter: contact.map(|contact| contact.parameter),
                source_interval: (kind == TopologyKind::Edge).then_some([0., 1.]),
                tangent: contact.is_some_and(|contact| contact.crossing.is_none()),
                crossing: contact.and_then(|contact| contact.crossing),
            });
        }
        Ok(SketchReferenceCatalog {
            results,
            coincident: outcome == Outcome::Coincident,
        })
    }
}

fn involute_reference(
    source: ReferenceFrame,
    radius: f64,
    [a, b]: [f64; 2],
    target: PlaneFrame,
    method: SketchReferenceMethod,
    key: TopologyKey,
    policy: GeometryPolicy,
) -> Result<SketchReferenceCatalog, GeometryError> {
    use multicad_rust_kernel::spur_gear::{Edge, Involute};

    let curve = Involute::new(radius, [a.min(b), a.max(b)], 0.)
        .map_err(|_| error("Invalid Rust involute reference"))?;
    let curve = if a > b { curve.reversed() } else { curve };
    let dot = |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
    let n = target.normal();
    let axial = [dot(source.u, n), dot(source.v, n)];
    let delta = std::array::from_fn(|i| source.origin_mm[i] - target.origin[i]);
    let offset = dot(delta, n);
    let edge = Edge::Involute(curve);
    let lo = offset - edge.support(axial.map(|v| -v));
    let hi = offset + edge.support(axial);
    let tolerance = policy.linear_tolerance_mm;
    let coplanar = lo.abs().max(hi.abs()) <= tolerance;
    if method == SketchReferenceMethod::Intersect {
        if coplanar || lo > tolerance || hi < -tolerance {
            return Ok(SketchReferenceCatalog {
                results: vec![],
                coincident: coplanar,
            });
        }
        return Err(GeometryError {
            operation: "sketch reference",
            kind: GeometryErrorKind::Unsupported,
            message: "Rust sketch references cannot yet isolate an involute plane intersection"
                .into(),
        });
    }
    if method == SketchReferenceMethod::Coplanar && !coplanar {
        return Err(error(
            "Selected involute is not coplanar with the sketch plane",
        ));
    }
    if dot(source.normal(), n).abs() <= 1e-10 {
        return Err(error("The involute projection collapses into a line"));
    }
    let spline = curve.cubic_spline(tolerance / 2.).map_err(|cause| {
        error(format!(
            "Cannot approximate the involute within sketch precision: {cause:?}"
        ))
    })?;
    let axes = [target.u, target.v];
    let poles = spline
        .poles_mm
        .into_iter()
        .map(|[x, y]| {
            let point = std::array::from_fn(|i| delta[i] + x * source.u[i] + y * source.v[i]);
            axes.map(|axis| dot(point, axis))
        })
        .collect();
    let curve = CurvedEdge2d::BSpline(BSpline2d {
        degree: 3,
        poles,
        knots: spline.knots,
        multiplicities: spline.multiplicities.into_iter().map(u32::from).collect(),
        weights: None,
    });
    curve.data(policy, 0)?;
    Ok(SketchReferenceCatalog {
        results: vec![SketchReferenceResult {
            geometry: SketchReferenceGeometry::Curve(curve),
            support: vec![key],
            source_parameter: None,
            source_interval: Some([0., 1.]),
            tangent: false,
            crossing: None,
        }],
        coincident: false,
    })
}

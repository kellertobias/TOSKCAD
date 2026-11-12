//! Axial cylinder-wall Draft as an exact conical meridian.
use super::*;
use crate::TreatmentFace;
use multicad_core::references::TopologyKind;
use std::f64::consts::TAU;

fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    error("cylinder draft", kind, message)
}

impl Solid {
    pub(crate) fn draft_cylinder_wall(
        &self,
        faces: &[TreatmentFace],
        pull: Point,
        neutral: PlaneFrame,
        angle: f64,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Option<Result<Self, GeometryError>> {
        let (cylinder, frame) = self.axial_cylinder_frame()?;
        Some((|| {
            if self.policy != policy {
                return Err(failure(
                    GeometryErrorKind::Unsupported,
                    "Rust Draft requires the source geometry policy",
                ));
            }
            let [selection] = faces else {
                return Err(failure(
                    GeometryErrorKind::Unsupported,
                    "Cylinder Draft requires only the cylindrical wall",
                ));
            };
            if selection.key.ordinal != 2 {
                return Err(failure(
                    GeometryErrorKind::Unsupported,
                    "Cylinder Draft requires the cylindrical wall",
                ));
            }
            let radius = cylinder.circle().radius().as_f64();
            let height = cylinder.height().as_f64();
            let axes = [frame.u, frame.v, frame.normal()];
            let vector = |direction| axes.map(|axis| dot(axis, direction));
            let neutral_origin = local(frame, neutral.origin);
            let lever = 2. * radius
                + neutral_origin[0].abs()
                + neutral_origin[1].abs()
                + neutral_origin[2]
                    .abs()
                    .max((height - neutral_origin[2]).abs());
            let amplification = (1. + angle.tan().abs()).powi(2);
            let aligned = |direction: Point| {
                let direction = vector(direction);
                let residual = direction[0]
                    .abs()
                    .max(direction[1].abs())
                    .max((direction[2].abs() - 1.).abs());
                if residual > 64. * f64::EPSILON
                    || residual * lever * amplification > policy.linear_tolerance_mm * 0.25
                {
                    return Err(failure(
                        GeometryErrorKind::Unsupported,
                        "Cylinder Draft requires axial pull and a perpendicular neutral plane",
                    ));
                }
                Ok(direction[2].signum())
            };
            let sense = aligned(pull)?;
            aligned(neutral.normal())?;
            let radii =
                [0., height].map(|z| radius - sense * angle.tan() * (z - neutral_origin[2]));
            for r in radii {
                policy.length(r).map_err(|_| {
                    failure(
                        GeometryErrorKind::InvalidResult,
                        "Draft reaches the cylinder axis or exceeds model bounds; reduce the angle",
                    )
                })?;
            }
            let points = vec![[radii[0], 0.], [radii[1], height], [0., height], [0., 0.]];
            let geometry = Revolution::new(vec![points.clone()], TAU).map_err(|cause| {
                failure(
                    GeometryErrorKind::InvalidResult,
                    format!("Invalid cylinder Draft meridian: {cause:?}"),
                )
            })?;
            let snapshot = self.topology_snapshot(feature, 0)?;
            let source_face = |ordinal| {
                snapshot
                    .elements
                    .iter()
                    .find(|element| {
                        element.key.kind == TopologyKind::Face && element.key.ordinal == ordinal
                    })
                    .expect("validated cylinder face")
            };
            let segments = points
                .iter()
                .copied()
                .enumerate()
                .map(|(i, a)| {
                    let source = match i {
                        0 => Some(source_face(2)),
                        1 => Some(source_face(1)),
                        3 => Some(source_face(0)),
                        _ => None,
                    };
                    let Some(source) = source else {
                        return Ok(MeridianSegment {
                            origin: Origin {
                                source: feature,
                                role: "draft.axis".into(),
                            },
                            direct_identity: true,
                            inherited: vec![],
                            inherited_frames: vec![],
                            authored_start: a,
                        });
                    };
                    let origin = source
                        .origins
                        .iter()
                        .next()
                        .expect("authored cylinder face")
                        .clone();
                    let mut inherited: Vec<_> = source.origins.iter().cloned().collect();
                    if i == 0 {
                        inherited.push(selection.origin.clone());
                    }
                    Ok(MeridianSegment {
                        origin,
                        direct_identity: true,
                        inherited,
                        inherited_frames: source
                            .frames
                            .iter()
                            .map(|anchor| {
                                Ok(AnchoredFrame {
                                    origin: anchor.origin.clone(),
                                    frame: local_attachment(frame, anchor.frame, policy)?,
                                })
                            })
                            .collect::<Result<Vec<_>, GeometryError>>()?,
                        authored_start: a,
                    })
                })
                .collect::<Result<Vec<_>, GeometryError>>()?;
            let rings = topology::ring_edges(&geometry);
            let retained_rim_boundaries = [0, 1]
                .into_iter()
                .map(|end| {
                    let source = snapshot
                        .elements
                        .iter()
                        .find(|element| {
                            element.key.kind == TopologyKind::Edge
                                && element.key.ordinal == end as u32
                        })
                        .expect("validated cylinder rim");
                    // The meridian's first two points are its only nonzero
                    // radii, so full-revolution rim ordinals are exactly 0 and 1.
                    (end as u32, source.boundary.clone())
                })
                .collect();
            let body = RevolutionBody {
                geometry,
                feature,
                sketch: PlaneFrame {
                    origin: [0.; 3],
                    u: [1., 0., 0.],
                    v: [0., 0., 1.],
                },
                segments: vec![segments],
                rings,
                reverse_rims: self.cylinder_sketch().is_some_and(|sketch| sketch.reverse),
                retained_rim_boundaries,
            };
            let bounds = body.bounds(frame);
            policy.point(bounds.min)?;
            policy.point(bounds.max)?;
            let result = Self {
                body: SolidBody::Revolved(Rc::new(body), frame),
                policy,
                provenance: vec![],
                rust_origins: vec![],
                retained_placement: None,
            };
            result
                .topology_snapshot(feature, 0)?
                .validate()
                .map_err(|cause| failure(GeometryErrorKind::InvalidResult, cause))?;
            Ok(result)
        })())
    }
}

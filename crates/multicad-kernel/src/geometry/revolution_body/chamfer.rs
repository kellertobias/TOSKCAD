//! Exact conical end treatments of a circular cylinder.
use super::*;
use crate::TreatmentEdge;
use multicad_core::references::{TopologyKey, TopologySnapshot};
use std::f64::consts::TAU;

fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    error("cylinder chamfer", kind, message)
}

impl Solid {
    pub(crate) fn chamfer_cylinder_rims(
        &self,
        edges: &[TreatmentEdge],
        reference_faces: Option<&[TopologyKey]>,
        distances: [f64; 2],
        feature: EntityId,
        policy: GeometryPolicy,
        snapshot: &TopologySnapshot,
    ) -> Option<Result<Self, GeometryError>> {
        let (cylinder, frame) = self.axial_cylinder_frame()?;
        Some((|| {
            if self.policy != policy {
                return Err(failure(
                    GeometryErrorKind::Unsupported,
                    "Rust chamfer requires the source geometry policy",
                ));
            }
            let radius = cylinder.circle().radius().as_f64();
            let height = cylinder.height().as_f64();
            let mut selected = [None, None];
            for (index, edge) in edges.iter().enumerate() {
                let cap = edge.key.ordinal as usize;
                let Some(slot) = selected.get_mut(cap) else {
                    return Err(failure(
                        GeometryErrorKind::Unsupported,
                        "Cylinder chamfer requires circular cap rims",
                    ));
                };
                let mut cut = distances;
                if let Some(faces) = reference_faces {
                    let face = faces[index].ordinal as usize;
                    if face == 2 {
                        cut.swap(0, 1);
                    } else if face != cap {
                        return Err(failure(
                            GeometryErrorKind::Input,
                            "Chamfer reference face must touch the selected cylinder rim",
                        ));
                    }
                }
                for distance in cut {
                    policy.length(distance)?;
                }
                policy.length(radius - cut[0]).map_err(|_| {
                    failure(
                        GeometryErrorKind::InvalidResult,
                        "Chamfer reaches the cylinder axis; reduce the distances",
                    )
                })?;
                *slot = Some((edge, cut));
            }
            let [lower, upper] =
                selected.map(|selection| selection.map_or([0.; 2], |(_, cut)| cut));
            policy.length(height - lower[1] - upper[1]).map_err(|_| {
                failure(
                    GeometryErrorKind::InvalidResult,
                    "Chamfers overlap or remove the cylindrical wall; reduce the distances",
                )
            })?;
            let mut points = vec![[radius - lower[0], 0.]];
            if selected[0].is_some() {
                points.push([radius, lower[1]]);
            }
            points.push([radius, height - upper[1]]);
            if selected[1].is_some() {
                points.push([radius - upper[0], height]);
            }
            points.extend([[0., height], [0., 0.]]);
            let geometry = Revolution::new(vec![points.clone()], TAU).map_err(|cause| {
                failure(
                    GeometryErrorKind::InvalidResult,
                    format!("Invalid cylinder chamfer meridian: {cause:?}"),
                )
            })?;
            let face = |ordinal| {
                snapshot
                    .elements
                    .iter()
                    .find(|element| {
                        element.key.kind == multicad_core::references::TopologyKind::Face
                            && element.key.ordinal == ordinal
                    })
                    .expect("validated cylinder face")
            };
            let segments = points
                .iter()
                .copied()
                .zip(points.iter().copied().cycle().skip(1))
                .map(|(a, b)| {
                    let source = if a[1] == b[1] {
                        Some(face(u32::from(a[1] == height)))
                    } else if a[0] == b[0] && a[0] != 0. {
                        Some(face(2))
                    } else {
                        None
                    };
                    let (origin, inherited, inherited_frames) = if let Some(source) = source {
                        let primary = source
                            .origins
                            .iter()
                            .next()
                            .expect("authored cylinder face")
                            .clone();
                        let anchors = source
                            .frames
                            .iter()
                            .map(|anchor| AnchoredFrame {
                                origin: anchor.origin.clone(),
                                frame: local_frame(frame, anchor.frame),
                            })
                            .collect();
                        (primary, source.origins.iter().cloned().collect(), anchors)
                    } else if a[0] == 0. && b[0] == 0. {
                        (
                            Origin {
                                source: feature,
                                role: "chamfer.axis".into(),
                            },
                            vec![],
                            vec![],
                        )
                    } else {
                        let cap = usize::from(b[1] == height);
                        (
                            selected[cap]
                                .expect("selected chamfer segment")
                                .0
                                .origin
                                .clone(),
                            vec![],
                            vec![],
                        )
                    };
                    MeridianSegment {
                        origin,
                        direct_identity: true,
                        inherited,
                        inherited_frames,
                        authored_start: a,
                    }
                })
                .collect();
            let body = RevolutionBody {
                rings: topology::ring_edges(&geometry),
                geometry,
                feature,
                sketch: PlaneFrame {
                    origin: [0.; 3],
                    u: [1., 0., 0.],
                    v: [0., 0., 1.],
                },
                segments: vec![segments],
                reverse_rims: self.cylinder_sketch().is_some_and(|sketch| sketch.reverse),
            };
            let bounds = body.bounds(frame);
            policy.point(bounds.min)?;
            policy.point(bounds.max)?;
            let result = Self {
                body: SolidBody::Revolved(Rc::new(body), frame),
                policy,
                provenance: vec![],
                rust_origins: vec![],
            };
            result
                .topology_snapshot(feature, 0)?
                .validate()
                .map_err(|message| failure(GeometryErrorKind::InvalidResult, message))?;
            Ok(result)
        })())
    }
}

fn local_frame(frame: PlaneFrame, anchor: ReferenceFrame) -> ReferenceFrame {
    let axes = [frame.u, frame.v, frame.normal()];
    ReferenceFrame {
        origin_mm: local(frame, anchor.origin_mm),
        u: axes.map(|axis| dot(axis, anchor.u)),
        v: axes.map(|axis| dot(axis, anchor.v)),
    }
}

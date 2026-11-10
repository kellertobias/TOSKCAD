use super::*;
use crate::{
    model::{RigidPose, align_frames},
    references::{
        self, AnalyticGeometry, Diagnostic, ReferenceAddress, ReferenceReport, Resolution,
        TopologySnapshot,
    },
    sketch,
};
use std::collections::BTreeMap;
#[derive(Clone, Copy)]
struct Anchor {
    frame: Option<ReferenceFrame>,
    geometry: Option<AnalyticGeometry>,
}
impl Anchor {
    fn transformed(self, pose: RigidPose) -> Self {
        Self {
            frame: self.frame.map(|f| pose.frame(f)),
            geometry: self
                .geometry
                .map(|g| g.transformed(|p| pose.point(p), |v| pose.vector(v))),
        }
    }
    fn frame(self) -> Result<ReferenceFrame, DocumentError> {
        self.frame.ok_or_else(||invalid("Choose an explicit named planar attachment or datum to define face UV and roll; a descriptive plane basis is not an authored frame"))
    }
    fn position(self) -> Result<[f64; 3], DocumentError> {
        match self.geometry {
            Some(AnalyticGeometry::Point { position_mm }) => Ok(position_mm),
            _ => Ok(self.frame()?.origin_mm),
        }
    }
    fn axis_frame(self, up: [f64; 3]) -> Result<ReferenceFrame, DocumentError> {
        let (origin_mm, direction) = match self.geometry {
            Some(AnalyticGeometry::Line { a_mm, b_mm }) => {
                (a_mm, std::array::from_fn(|i| b_mm[i] - a_mm[i]))
            }
            Some(AnalyticGeometry::CircularArc { frame, .. })
            | Some(AnalyticGeometry::EllipticArc { frame, .. })
            | Some(AnalyticGeometry::Plane { frame }) => (frame.origin_mm, frame.normal()),
            Some(AnalyticGeometry::Point { .. }) => {
                return Err(invalid("A point does not define an alignment axis"));
            }
            Some(AnalyticGeometry::InvoluteArc { .. }) => {
                return Err(invalid(
                    "An involute flank does not define an alignment axis",
                ));
            }
            None => {
                let f = self.frame()?;
                (f.origin_mm, f.normal())
            }
        };
        let length = direction.iter().map(|v| v * v).sum::<f64>().sqrt();
        if !length.is_finite() || length < 1e-12 {
            return Err(invalid("Alignment source has a degenerate axis"));
        }
        let n = direction.map(|v| v / length);
        let dot = up.into_iter().zip(n).map(|(a, b)| a * b).sum::<f64>();
        let u = std::array::from_fn(|i| up[i] - n[i] * dot);
        let length = u.iter().map(|v| v * v).sum::<f64>().sqrt();
        if length < 1e-10 {
            return Err(invalid(
                "The explicit up vector is parallel to the alignment axis; choose another up direction",
            ));
        }
        let u = u.map(|v| v / length);
        let v = [
            n[1] * u[2] - n[2] * u[1],
            n[2] * u[0] - n[0] * u[2],
            n[0] * u[1] - n[1] * u[0],
        ];
        Ok(ReferenceFrame { origin_mm, u, v })
    }
}
fn datum(
    document: &Document,
    plane: EntityId,
    revision: u64,
    snapshots: &BTreeMap<EntityId, TopologySnapshot>,
) -> Result<ReferenceFrame, DocumentError> {
    let mut entries = vec![];
    let mut current = plane;
    let mut visited = BTreeSet::new();
    loop {
        if visited.len() >= 128 || !visited.insert(current) {
            return Err(invalid(
                "Datum alignment exceeds the plane chain budget or contains a cycle",
            ));
        }
        let Some(EntityData::Plane { anchor, .. }) = document.get(current).map(|e| &e.data) else {
            return Err(invalid("Alignment datum is missing or has the wrong kind"));
        };
        let Some(link) = anchor else {
            break;
        };
        if let Some(intent) = &link.geometry {
            let resolution = match snapshots.get(&link.target) {
                Some(snapshot) if snapshot.owner == link.target => {
                    references::resolve(intent, snapshot, revision)
                }
                _ => Resolution::MissingSource,
            };
            entries.push(Diagnostic {
                address: ReferenceAddress {
                    owner: current,
                    reference: intent.id,
                },
                source: link.target,
                dependency_hash: Some(references::dependency_hash(document, link.target)?),
                role: link.role.clone(),
                intent: intent.clone(),
                resolution,
                affected: vec![],
            });
            break;
        }
        if link.role == sketch::OFFSET_PLANE_ROLE {
            current = link.target;
        } else {
            break;
        }
    }
    let report = ReferenceReport {
        revision,
        entries,
        sketch_supports: Default::default(),
        external_results: Default::default(),
        external_errors: Default::default(),
    };
    let frame = sketch::inspect_plane(document, plane, revision, Some(&report))?;
    Ok(ReferenceFrame {
        origin_mm: frame.origin,
        u: frame.u,
        v: frame.v,
    })
}
fn anchor(
    document: &Document,
    revision: u64,
    anchor: &AlignmentAnchor,
    references: &[AlignmentInput],
    snapshots: &BTreeMap<EntityId, TopologySnapshot>,
) -> Result<Anchor, DocumentError> {
    match anchor {
        AlignmentAnchor::Frame { frame } => Ok(Anchor {
            frame: Some(*frame),
            geometry: None,
        }),
        AlignmentAnchor::Plane { plane } => Ok(Anchor {
            frame: Some(datum(document, *plane, revision, snapshots)?),
            geometry: None,
        }),
        AlignmentAnchor::Geometry { reference } => {
            let input = references
                .iter()
                .find(|r| r.intent.id == *reference)
                .ok_or_else(|| invalid("Alignment capture is missing"))?;
            let snapshot = snapshots
                .get(&input.source)
                .ok_or_else(|| invalid("Alignment source geometry is unavailable"))?;
            if snapshot.owner != input.source {
                return Err(invalid("Alignment topology belongs to a different source"));
            }
            match references::resolve(&input.intent, snapshot, revision) {
                Resolution::Resolved {
                    mut frame,
                    geometry,
                    ..
                } => {
                    if let (Some(semantic), Some(AnalyticGeometry::Plane { frame: plane })) =
                        (&mut frame, geometry)
                    {
                        let dot = semantic
                            .normal()
                            .into_iter()
                            .zip(plane.normal())
                            .map(|(a, b)| a * b)
                            .sum::<f64>();
                        if dot.abs() < 1. - 1e-8 {
                            return Err(invalid(
                                "Named alignment frame is not parallel to the current native face",
                            ));
                        }
                        if dot < 0. {
                            semantic.v = semantic.v.map(|v| -v);
                        }
                    }
                    Ok(Anchor { frame, geometry })
                }
                Resolution::Ambiguous { .. } => Err(invalid(
                    "Alignment reference is ambiguous; explicitly repair or replace its capture",
                )),
                Resolution::Stale { .. } => Err(invalid("Alignment source topology is stale")),
                _ => Err(invalid(
                    "Alignment reference is missing, invalid or unsupported; repair it before applying",
                )),
            }
        }
    }
}
/// Resolve exact current anchors. `base` transforms a moving instance's local
/// source anchor; targets are already world-space. Part features pass identity.
pub fn resolve_alignment(
    document: &Document,
    revision: u64,
    parameters: &EvaluatedParameters,
    control: &AlignmentParameters,
    references: &[AlignmentInput],
    snapshots: &BTreeMap<EntityId, TopologySnapshot>,
    base: RigidPose,
) -> Result<RigidPose, DocumentError> {
    links(control, references)?;
    base.validate()?;
    let (offset, roll) = control.numbers(parameters)?;
    let source =
        anchor(document, revision, &control.source, references, snapshots)?.transformed(base);
    let target = anchor(document, revision, &control.target, references, snapshots)?;
    match control.mode {
        AlignmentMode::Frame => align_frames(
            source.frame()?,
            target.frame()?,
            offset,
            roll,
            control.opposed,
        ),
        AlignmentMode::Axis => align_frames(
            source.axis_frame(control.up)?,
            target.axis_frame(control.up)?,
            offset,
            roll,
            control.opposed,
        ),
        AlignmentMode::Position => {
            if roll != 0. || control.opposed {
                return Err(invalid(
                    "Point snapping preserves rotation; use frame or axis mode for orientation",
                ));
            }
            let source = source.position()?;
            let target = target.position()?;
            let result = RigidPose {
                translation_mm: std::array::from_fn(|i| target[i] + offset[i] - source[i]),
                ..Default::default()
            };
            result.validate()?;
            Ok(result)
        }
    }
}

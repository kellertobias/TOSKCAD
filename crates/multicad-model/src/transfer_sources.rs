//! Exact source queries and parallel projection into one receiving local plane.
use crate::{ModelError, error};
use multicad_core::{
    document::EntityId,
    manufacturing::{PlanarEdge, PlanarLoop},
    model::{
        AlignmentInput, MAX_TRANSFER_SOURCES, ResolvedTransferTarget, RigidPose, TransferSelection,
        TransferSource, TransferTarget,
    },
    project_file::ContentHash,
    references::{
        self, AnalyticGeometry, Origin, Resolution, TopologyKey, TopologyKind, TopologySnapshot,
    },
};
use multicad_kernel::{CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, Solid};
use std::collections::BTreeSet;
pub(super) struct Inputs<'a> {
    pub feature: EntityId,
    pub threads: &'a super::threads::ThreadSources,
    pub source: &'a TransferSource,
    pub target: &'a TransferTarget,
    pub settings: ResolvedTransferTarget,
    pub captures: &'a [AlignmentInput],
    pub hardware: &'a Solid,
    pub topology: &'a TopologySnapshot,
    pub relative: RigidPose,
    pub plane: PlaneFrame,
    pub revision: u64,
    pub policy: GeometryPolicy,
}
pub(super) struct Cut {
    pub profile: CurvedProfile,
    pub origins: Vec<Origin>,
    pub label: String,
}
pub(super) fn capture(
    captures: &[AlignmentInput],
    id: EntityId,
    source: EntityId,
    feature: EntityId,
) -> Result<&AlignmentInput, ModelError> {
    let selected = captures
        .iter()
        .find(|c| c.intent.id == id && c.source == source)
        .ok_or_else(|| {
            error(
                Some(feature),
                format!("Transfer capture {id} is missing or belongs to another source"),
            )
        })?;
    if selected.intent.attachment.is_some() {
        return Err(error(
            Some(feature),
            "Transfer captures cannot carry datum offsets or rotations; repair with the actual face/edge capture",
        ));
    }
    Ok(selected)
}
pub(super) fn resolve(
    input: &AlignmentInput,
    snapshot: &TopologySnapshot,
    revision: u64,
    feature: EntityId,
) -> Result<(TopologyKey, Option<AnalyticGeometry>), ModelError> {
    match references::resolve(&input.intent, snapshot, revision) {
        Resolution::Resolved { key, geometry, .. } => Ok((key, geometry)),
        diagnostic => Err(error(
            Some(feature),
            format!(
                "Transfer capture {} needs explicit review/repair: {diagnostic:?}",
                input.intent.id
            ),
        )),
    }
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
impl Inputs<'_> {
    fn fail(&self, message: impl std::fmt::Display) -> ModelError {
        error(Some(self.feature), message)
    }
    fn uv(&self, point: [f64; 3]) -> [f64; 2] {
        let point = self.relative.point(point);
        let delta = std::array::from_fn(|i| point[i] - self.plane.origin[i]);
        [dot(delta, self.plane.u), dot(delta, self.plane.v)]
    }
    fn parity(&self, source_axis: [f64; 3]) -> Result<f64, ModelError> {
        let a = self.relative.vector(source_axis);
        let b = self.plane.normal();
        let cross = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let alignment = dot(a, b);
        if !alignment.is_finite()
            || cross.iter().map(|v| v * v).sum::<f64>().sqrt() > 1e-10
            || (alignment.abs() - 1.).abs() > 1e-9
        {
            return Err(self.fail("Transfer supports parallel or antiparallel source axes/planes only; tilted projection is unsupported"));
        }
        Ok(alignment.signum())
    }
    fn origin(&self, capture: EntityId, role: &str) -> Origin {
        Origin {
            source: self.feature,
            role: format!("transfer/{}/capture/{capture}/{role}", self.target.id),
        }
    }
    fn signature(&self, keys: &[TopologyKey]) -> Result<ContentHash, ModelError> {
        let mut origins = BTreeSet::new();
        for key in keys {
            let current = self
                .topology
                .elements
                .iter()
                .find(|e| e.key == *key)
                .ok_or_else(|| {
                    self.fail("Source contour edge is missing from the current exact topology")
                })?;
            if key.kind != TopologyKind::Edge || current.origins.is_empty() {
                return Err(self.fail("Source contour has no stable keyed edge provenance; explicitly repair its source"));
            }
            origins.extend(current.origins.iter().cloned());
            if origins.len() > 1024 {
                return Err(self.fail("Source contour exceeds its stable-origin naming budget"));
            }
        }
        if origins.is_empty() {
            return Err(self.fail("Source contour has no stable native provenance"));
        }
        let bytes = serde_json::to_vec(&origins).map_err(|e| self.fail(e))?;
        Ok(ContentHash::of(&bytes))
    }
    fn hole(
        &self,
        capture: EntityId,
        identity: &str,
        center: [f64; 2],
        radius: f64,
    ) -> Result<Cut, ModelError> {
        let radius = self.settings.diameter_mm.map_or(radius, |d| d * 0.5);
        let profile = CurvedProfile {
            frame: self.plane,
            outer: vec![CurvedEdge2d::Circle {
                center,
                radius_mm: radius,
            }],
            holes: vec![],
        }
        .offset_closed_boundary(self.settings.clearance_mm, self.policy)
        .map_err(|e| self.fail(e))?;
        Ok(Cut {
            profile,
            origins: vec![self.origin(capture, identity)],
            label: format!("{capture}/{identity}"),
        })
    }
    fn face(
        &self,
        id: EntityId,
    ) -> Result<(PlaneFrame, multicad_kernel::SourcedPlanarContours), ModelError> {
        let selected = capture(self.captures, id, self.source.component, self.feature)?;
        let (key, geometry) = resolve(selected, self.topology, self.revision, self.feature)?;
        let Some(AnalyticGeometry::Plane { frame }) =
            geometry.filter(|_| key.kind == TopologyKind::Face)
        else {
            return Err(
                self.fail("Face-driven transfer needs a current captured planar source face")
            );
        };
        frame.validate().map_err(|e| self.fail(e))?;
        let frame = PlaneFrame {
            origin: frame.origin_mm,
            u: frame.u,
            v: frame.v,
        };
        self.parity(frame.normal())?;
        let contours = self
            .hardware
            .planar_face_contours_with_sources(key.ordinal, frame)
            .map_err(|e| self.fail(e))?;
        Ok((frame, contours))
    }
    fn circle_loop(&self, wire: &PlanarLoop) -> Result<([f64; 2], f64), ModelError> {
        let Some(PlanarEdge::Arc { a, center, .. }) = wire.edges.first() else {
            return Err(self.fail("All source inner loops must be full circular holes; noncircular inner loops require a separate explicit cutout transfer"));
        };
        let radius = distance(*a, *center);
        let mut sweep = 0.;
        for edge in &wire.edges {
            let PlanarEdge::Arc {
                a,
                b,
                center: other,
                sweep_rad,
            } = edge
            else {
                return Err(self.fail("All source inner loops must be circular; no noncircular hole is silently skipped"));
            };
            if distance(*center, *other) > self.policy.linear_tolerance_mm
                || (distance(*a, *other) - radius).abs() > self.policy.linear_tolerance_mm
                || (distance(*b, *other) - radius).abs() > self.policy.linear_tolerance_mm
                || *sweep_rad >= 0.
            {
                return Err(self.fail("Source inner loop is not one exact consistently wound circle; repair the boundary"));
            }
            sweep += sweep_rad;
        }
        if (sweep.abs() - std::f64::consts::TAU).abs() > 1e-9 {
            return Err(self.fail("Source inner loop is not a complete circular boundary"));
        }
        Ok((*center, radius))
    }
    fn edge(&self, edge: &PlanarEdge, frame: PlaneFrame, parity: f64) -> CurvedEdge2d {
        match *edge {
            PlanarEdge::Line { a, b } => CurvedEdge2d::Line {
                a: self.uv(frame.point(a)),
                b: self.uv(frame.point(b)),
            },
            PlanarEdge::Arc {
                a,
                b,
                center,
                sweep_rad,
            } => {
                if (sweep_rad.abs() - std::f64::consts::TAU).abs() <= 1e-9 {
                    CurvedEdge2d::Circle {
                        center: self.uv(frame.point(center)),
                        radius_mm: distance(a, center),
                    }
                } else {
                    CurvedEdge2d::Arc {
                        center: self.uv(frame.point(center)),
                        a: self.uv(frame.point(a)),
                        b: self.uv(frame.point(b)),
                        clockwise: sweep_rad * parity < 0.,
                    }
                }
            }
        }
    }
}
pub(super) fn profiles(input: Inputs<'_>) -> Result<Vec<Cut>, ModelError> {
    let mut cuts = vec![];
    match &input.source.selection {
        TransferSelection::Circles { references } => {
            for id in references {
                let capture = capture(input.captures, *id, input.source.component, input.feature)?;
                let (key, _) = resolve(capture, input.topology, input.revision, input.feature)?;
                let axis = input.hardware.transfer_hole_axis(key).map_err(|e| {
                    input.fail(if input.threads.has_threads() {
                        format!("{e}. {}", input.threads.diagnostic())
                    } else {
                        e.to_string()
                    })
                })?;
                input
                    .threads
                    .check_axis(axis.origin_mm, axis.axis, input.policy.linear_tolerance_mm)
                    .map_err(|e| input.fail(e))?;
                input
                    .threads
                    .check_provenance(input.topology, &[key])
                    .map_err(|e| input.fail(e))?;
                input.parity(axis.axis)?;
                cuts.push(input.hole(*id, "circle", input.uv(axis.origin_mm), axis.radius_mm)?);
            }
        }
        TransferSelection::FaceHoles { face } => {
            let (frame, current) = input.face(*face)?;
            let mut seen = BTreeSet::new();
            for (wire, keys) in current
                .contours
                .loops
                .iter()
                .zip(&current.edge_keys)
                .filter(|(w, _)| !w.outer)
            {
                if cuts.len() >= MAX_TRANSFER_SOURCES {
                    return Err(input.fail("Source face exceeds the 256-hole transfer budget"));
                }
                let (center, radius) = input.circle_loop(wire).map_err(|e| {
                    if input.threads.has_threads() {
                        input.fail(format!("{e}. {}", input.threads.diagnostic()))
                    } else {
                        e
                    }
                })?;
                input
                    .threads
                    .check_axis(
                        frame.point(center),
                        frame.normal(),
                        input.policy.linear_tolerance_mm,
                    )
                    .map_err(|e| input.fail(e))?;
                input
                    .threads
                    .check_provenance(input.topology, keys)
                    .map_err(|e| input.fail(e))?;
                let signature = input.signature(keys)?;
                if !seen.insert(signature) {
                    return Err(input.fail("Current source holes have ambiguous duplicate provenance; explicit repair is required"));
                }
                cuts.push(input.hole(
                    *face,
                    &format!("hole/{signature}"),
                    input.uv(frame.point(center)),
                    radius,
                )?);
            }
        }
        TransferSelection::FaceBoundary { face } => {
            let (frame, current) = input.face(*face)?;
            let parity = input.parity(frame.normal())?;
            let outer: Vec<_> = current
                .contours
                .loops
                .iter()
                .zip(&current.edge_keys)
                .filter(|(w, _)| w.outer)
                .collect();
            let [(wire, keys)] = outer.as_slice() else {
                return Err(input.fail("Source cutout face must have one explicit outer wire"));
            };
            if wire.edges.len() > multicad_kernel::MAX_CURVED_PROFILE_EDGES {
                return Err(input.fail("Source cutout exceeds the 64-exact-edge machining budget"));
            }
            let mut origins = vec![];
            let mut seen = BTreeSet::new();
            for key in *keys {
                let signature = input.signature(std::slice::from_ref(key))?;
                if !seen.insert(signature) {
                    return Err(input.fail("Source cutout edges have ambiguous provenance; explicit repair is required"));
                }
                origins.push(input.origin(*face, &format!("edge/{signature}")));
            }
            let profile = CurvedProfile {
                frame: input.plane,
                outer: wire
                    .edges
                    .iter()
                    .map(|edge| input.edge(edge, frame, parity))
                    .collect(),
                holes: vec![],
            }
            .offset_closed_boundary(input.settings.clearance_mm, input.policy)
            .map_err(|e| input.fail(e))?;
            cuts.push(Cut {
                profile,
                origins,
                label: format!("{face}/outer-boundary (inner loops intentionally excluded)"),
            });
        }
    }
    Ok(cuts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        model::{BodyInput, OccurrencePath, TransferTargetParameters},
        references::GeometryIntent,
    };
    fn settings() -> ResolvedTransferTarget {
        ResolvedTransferTarget {
            diameter_mm: None,
            clearance_mm: 0.,
            blind_depth_mm: None,
            reverse: true,
        }
    }
    fn half_disk() -> CurvedProfile {
        CurvedProfile {
            frame: Default::default(),
            outer: vec![
                CurvedEdge2d::Line {
                    a: [-5., 0.],
                    b: [5., 0.],
                },
                CurvedEdge2d::Arc {
                    center: [0., 0.],
                    a: [5., 0.],
                    b: [-5., 0.],
                    clockwise: false,
                },
            ],
            holes: vec![],
        }
    }
    #[test]
    fn antiparallel_cutout_projection_preserves_arc_region_and_rejects_tilt() {
        let producer = EntityId::new();
        let names: Vec<_> = (0..2)
            .map(|_| Origin {
                source: EntityId::new(),
                role: "curve".into(),
            })
            .collect();
        let hardware = Solid::extrude_curved_profile_named(
            &half_disk(),
            3.,
            producer,
            &names,
            Default::default(),
        )
        .unwrap();
        let component = EntityId::new();
        let topology = hardware.topology_snapshot(component, 0).unwrap();
        let top = hardware
            .faces()
            .unwrap()
            .iter()
            .find(|f| {
                f.plane
                    .is_some_and(|(p, n)| (p[2] - 3.).abs() < 1e-6 && n[2] > 0.99)
            })
            .unwrap()
            .index;
        let intent = GeometryIntent::capture(
            &topology,
            TopologyKey {
                kind: TopologyKind::Face,
                ordinal: top,
            },
        )
        .unwrap();
        let source = TransferSource {
            component,
            path: OccurrencePath {
                root: component,
                members: vec![],
            },
            selection: TransferSelection::FaceBoundary { face: intent.id },
        };
        let captures = vec![AlignmentInput {
            source: component,
            intent,
        }];
        let target = TransferTarget::new(
            BodyInput {
                body: EntityId::new(),
                producer: EntityId::new(),
            },
            OccurrencePath {
                root: EntityId::new(),
                members: vec![],
            },
            EntityId::new(),
            TransferTargetParameters::default(),
        );
        let plane = PlaneFrame {
            origin: [0.; 3],
            u: [1., 0., 0.],
            v: [0., -1., 0.],
        };
        let threads = super::super::threads::ThreadSources::default();
        let input = |relative| Inputs {
            threads: &threads,
            feature: target.feature,
            source: &source,
            target: &target,
            settings: settings(),
            captures: &captures,
            hardware: &hardware,
            topology: &topology,
            relative,
            plane,
            revision: 0,
            policy: Default::default(),
        };
        let cuts = profiles(input(Default::default())).unwrap();
        assert_eq!(cuts.len(), 1);
        let area = cuts[0].profile.area_mm2(Default::default()).unwrap();
        assert!((area - std::f64::consts::PI * 12.5).abs() < 1e-6);
        assert_eq!(
            cuts[0]
                .profile
                .classify([0., -2.], Default::default())
                .unwrap(),
            multicad_kernel::CurvePointLocation::Inside
        );
        assert_eq!(
            cuts[0]
                .profile
                .classify([0., 2.], Default::default())
                .unwrap(),
            multicad_kernel::CurvePointLocation::Outside
        );
        let angle = std::f64::consts::FRAC_PI_4;
        let (s, c) = angle.sin_cos();
        let relative = RigidPose {
            rotation: [[1., 0., 0.], [0., c, -s], [0., s, c]],
            translation_mm: [0.; 3],
        };
        assert!(
            profiles(input(relative))
                .err()
                .unwrap()
                .message
                .contains("tilted")
        );
    }
}

//! Semantic prism sources and exact native ancestry, separate from display data.
use crate::{
    AffineTransform, GeometryError, GeometryErrorKind, GeometryPolicy, PlaneFrame, Profile,
    ProfileEdge, RigidTransform, Solid, ffi,
};
use multicad_core::{document::EntityId, references::*};
use std::collections::{BTreeMap, BTreeSet};

fn failure(message: impl std::fmt::Display) -> GeometryError {
    GeometryError {
        operation: "provenance",
        kind: GeometryErrorKind::InvalidResult,
        message: message.to_string(),
    }
}
fn key(kind: u8, ordinal: u32) -> Result<TopologyKey, GeometryError> {
    Ok(TopologyKey {
        kind: match kind {
            0 => TopologyKind::Face,
            1 => TopologyKind::Edge,
            2 => TopologyKind::Vertex,
            _ => return Err(failure("Unknown native topology kind")),
        },
        ordinal,
    })
}
fn empty(key: TopologyKey) -> TopologyElement {
    TopologyElement {
        key,
        origins: BTreeSet::new(),
        boundary: BTreeSet::new(),
        frames: vec![],
        geometry: None,
    }
}
fn reference_frame(frame: PlaneFrame) -> ReferenceFrame {
    ReferenceFrame {
        origin_mm: frame.origin,
        u: frame.u,
        v: frame.v,
    }
}
fn transformed(frame: ReferenceFrame, transform: RigidTransform) -> ReferenceFrame {
    let rotate = |v: [f64; 3]| {
        std::array::from_fn(|i| {
            transform.rotation[i]
                .iter()
                .zip(v)
                .map(|(a, b)| a * b)
                .sum()
        })
    };
    let origin: [f64; 3] = rotate(frame.origin_mm);
    ReferenceFrame {
        origin_mm: std::array::from_fn(|i| origin[i] + transform.translation_mm[i]),
        u: rotate(frame.u),
        v: rotate(frame.v),
    }
}
fn side_frame(profile: &Profile, edge: &ProfileEdge, height: f64) -> Option<ReferenceFrame> {
    let ProfileEdge::Line { a, b } = edge else {
        return None;
    };
    let a = profile.frame.point(*a);
    let b = profile.frame.point(*b);
    let length = (0..3).map(|i| (b[i] - a[i]).powi(2)).sum::<f64>().sqrt();
    Some(ReferenceFrame {
        origin_mm: a,
        u: std::array::from_fn(|i| (b[i] - a[i]) / length),
        v: profile.frame.normal().map(|v| v * height.signum()),
    })
}

impl Solid {
    pub(crate) fn name_gear(
        &mut self,
        feature: EntityId,
        teeth: u32,
        sweep: crate::GearSweep,
    ) -> Result<(), GeometryError> {
        let mut elements: BTreeMap<_, _> = self
            .topology_snapshot(feature, 0)?
            .elements
            .into_iter()
            .map(|element| (element.key, element))
            .collect();
        let chevron = matches!(
            sweep,
            crate::GearSweep::Chevron | crate::GearSweep::GappedChevron { .. }
        );
        let mut caps = [false; 2];
        for seed in ffi::seeds(self.occt()?).map_err(failure)? {
            let role = match seed.role {
                0 | 1 => {
                    caps[seed.role as usize] = true;
                    if seed.role == 0 {
                        "gear/end.start"
                    } else {
                        "gear/end.end"
                    }
                    .into()
                }
                2 => "gear/bore".into(),
                3 => "gear/root.core".into(),
                9 => "gear/hub.side".into(),
                10 => "gear/hub.end.start".into(),
                11 => "gear/hub.end.end".into(),
                8 if matches!(sweep, crate::GearSweep::InternalSpur { .. }) => {
                    "gear/rim.outer".into()
                }
                4 | 5 if matches!(sweep, crate::GearSweep::GappedChevron { .. }) => {
                    if seed.role == 4 {
                        "gear/gap.lower"
                    } else {
                        "gear/gap.upper"
                    }
                    .into()
                }
                6 | 7 if seed.source_index / 8 < teeth && (seed.role == 6 || chevron) => {
                    let band = if !chevron {
                        "whole"
                    } else if seed.role == 6 {
                        "lower"
                    } else {
                        "upper"
                    };
                    let side = [
                        "fillet.left",
                        "continuation.left",
                        "involute.left",
                        "tip",
                        "involute.right",
                        "continuation.right",
                        "fillet.right",
                        "root",
                    ][(seed.source_index % 8) as usize];
                    format!("gear/{band}/tooth/{}/{side}", seed.source_index / 8)
                }
                _ => return Err(failure("Unknown native gear face source")),
            };
            let element = elements
                .get_mut(&key(0, seed.face)?)
                .ok_or_else(|| failure("Gear ancestry names an absent face"))?;
            let origin = Origin {
                source: feature,
                role,
            };
            element.origins.insert(origin.clone());
            if let Some(AnalyticGeometry::Plane { frame }) = element.geometry
                && !element.frames.iter().any(|known| known.origin == origin)
            {
                element.frames.push(AnchoredFrame { origin, frame });
            }
        }
        if !caps.into_iter().all(|cap| cap)
            || elements
                .values()
                .any(|element| element.key.kind == TopologyKind::Face && element.origins.is_empty())
        {
            return Err(failure(
                "Native gear needs complete face ancestry and both end caps",
            ));
        }
        let nodes = ffi::topology(self.occt()?).map_err(failure)?;
        self.finish_provenance(elements, &nodes)
    }
    pub(crate) fn name_thread(
        &mut self,
        feature: EntityId,
        starts: u32,
    ) -> Result<(), GeometryError> {
        let mut elements: BTreeMap<_, _> = self
            .topology_snapshot(feature, 0)?
            .elements
            .into_iter()
            .map(|element| (element.key, element))
            .collect();
        let mut caps = [false; 2];
        for seed in ffi::seeds(self.occt()?).map_err(failure)? {
            let role = match seed.role {
                0 | 1 => {
                    caps[seed.role as usize] = true;
                    if seed.role == 0 {
                        "cap.start".into()
                    } else {
                        "cap.end".into()
                    }
                }
                2 if seed.source_index / 4 < starts && seed.source_index % 4 < 3 => {
                    let flank = ["lower", "crest", "upper"][(seed.source_index % 4) as usize];
                    format!("thread/start/{}/{flank}", seed.source_index / 4)
                }
                3 if seed.source_index == 0 => "thread.root".into(),
                4 if seed.source_index < 2 && starts == 1 => {
                    let side = if seed.source_index == 0 {
                        "lower"
                    } else {
                        "upper"
                    };
                    format!("thread/start/0/root/{side}")
                }
                _ => return Err(failure("Unknown native thread face source")),
            };
            let element = elements
                .get_mut(&key(0, seed.face)?)
                .ok_or_else(|| failure("Thread ancestry names an absent face"))?;
            let origin = Origin {
                source: feature,
                role,
            };
            element.origins.insert(origin.clone());
            if seed.role == 4 {
                element.origins.insert(Origin {
                    source: feature,
                    role: "thread.root".into(),
                });
            }
            if let Some(AnalyticGeometry::Plane { frame }) = element.geometry
                && !element.frames.iter().any(|known| known.origin == origin)
            {
                element.frames.push(AnchoredFrame { origin, frame });
            }
        }
        if !caps.into_iter().all(|cap| cap)
            || elements
                .values()
                .any(|element| element.key.kind == TopologyKind::Face && element.origins.is_empty())
        {
            return Err(failure(
                "Native thread needs complete face ancestry and both end caps",
            ));
        }
        let nodes = ffi::topology(self.occt()?).map_err(failure)?;
        self.finish_provenance(elements, &nodes)
    }
    pub(crate) fn name_advanced_profile(
        &mut self,
        feature: EntityId,
        origins: &[Origin],
    ) -> Result<(), GeometryError> {
        let mut elements: BTreeMap<_, _> = self
            .topology_snapshot(feature, 0)?
            .elements
            .into_iter()
            .map(|element| (element.key, element))
            .collect();
        let mut caps = [false; 2];
        for seed in ffi::seeds(self.occt()?).map_err(failure)? {
            let element = elements
                .get_mut(&key(0, seed.face)?)
                .ok_or_else(|| failure("Profile history names an absent face"))?;
            let names = match seed.role {
                0 | 1 => {
                    caps[seed.role as usize] = true;
                    if !matches!(element.geometry, Some(AnalyticGeometry::Plane { .. })) {
                        return Err(failure(
                            "Native profile end cap has no supported exact plane",
                        ));
                    }
                    vec![Origin {
                        source: feature,
                        role: if seed.role == 0 {
                            "cap.start"
                        } else {
                            "cap.end"
                        }
                        .into(),
                    }]
                }
                2 => {
                    let origin = origins
                        .get(seed.source_index as usize)
                        .ok_or_else(|| failure("Profile history names an absent authored curve"))?;
                    vec![
                        origin.clone(),
                        Origin {
                            source: feature,
                            role: format!("side/{}/{}", origin.source, origin.role),
                        },
                    ]
                }
                _ => return Err(failure("Unknown advanced profile history role")),
            };
            for origin in names {
                element.origins.insert(origin.clone());
                if let Some(AnalyticGeometry::Plane { frame }) = element.geometry
                    && !element.frames.iter().any(|f| f.origin == origin)
                {
                    element.frames.push(AnchoredFrame { origin, frame });
                }
            }
        }
        if !caps.into_iter().all(|cap| cap) {
            return Err(failure("Both native profile end caps need exact history"));
        }
        let nodes = ffi::topology(self.occt()?).map_err(failure)?;
        self.finish_provenance(elements, &nodes)
    }
    pub fn with_member_origin(self, feature: EntityId, index: u32) -> Result<Self, GeometryError> {
        if feature.is_nil() || index >= 128 {
            return Err(failure(
                "Pattern member identity exceeds the feature/member budget",
            ));
        }
        let origin = Origin {
            source: feature,
            role: format!("member/{index}"),
        };
        self.with_identity_origin(origin)
    }
    pub fn with_occurrence_origin(
        self,
        owner: EntityId,
        member: EntityId,
    ) -> Result<Self, GeometryError> {
        if owner.is_nil() || member.is_nil() {
            return Err(failure(
                "Assembly member provenance needs non-nil owner/member identities",
            ));
        }
        self.with_identity_origin(Origin {
            source: owner,
            role: format!("assembly/member/{member}"),
        })
    }
    /// Stable source-owned identity for a composite's exact material/component
    /// member. It remains distinct from evaluation-local solid ordinals.
    pub fn with_component_origin(
        self,
        owner: EntityId,
        member: EntityId,
    ) -> Result<Self, GeometryError> {
        if owner.is_nil() || member.is_nil() {
            return Err(failure(
                "Component provenance needs non-nil owner/member identities",
            ));
        }
        if self
            .topology_snapshot(owner, 0)?
            .elements
            .iter()
            .any(|element| {
                element
                    .origins
                    .iter()
                    .any(|origin| origin.source == owner && origin.role.starts_with("component/"))
            })
        {
            return Err(failure(
                "This component already has an identity for the same owner",
            ));
        }
        self.with_identity_origin(Origin {
            source: owner,
            role: format!("component/{member}"),
        })
    }
    fn with_identity_origin(mut self, origin: Origin) -> Result<Self, GeometryError> {
        let snapshot = self.topology_snapshot(origin.source, 0)?;
        if snapshot
            .elements
            .iter()
            .any(|e| e.origins.contains(&origin))
        {
            return Err(failure(
                "Member origin collides with existing source provenance",
            ));
        }
        if self.uses_rust_geometry() {
            self.rust_origins.push(origin);
            return Ok(self);
        }
        let elements: BTreeMap<_, _> = snapshot
            .elements
            .into_iter()
            .map(|mut element| {
                element.origins.insert(origin.clone());
                if let Some(AnalyticGeometry::Plane { frame }) = element.geometry {
                    element.frames.push(AnchoredFrame {
                        origin: origin.clone(),
                        frame,
                    });
                }
                element.boundary.clear();
                (element.key, element)
            })
            .collect();
        let nodes = ffi::topology(self.occt()?).map_err(failure)?;
        self.finish_provenance(elements, &nodes)?;
        Ok(self)
    }
    pub(crate) fn inherit_advanced_provenance(
        &mut self,
        source: &Self,
        selected: &[Origin],
        owner: EntityId,
    ) -> Result<(), GeometryError> {
        self.inherit_current_provenance(source, owner)?;
        let source_snapshot = source.topology_snapshot(owner, 0)?;
        let inputs: BTreeMap<_, _> = source_snapshot
            .elements
            .iter()
            .map(|e| (e.key, e))
            .collect();
        let mut elements: BTreeMap<_, _> = self
            .topology_snapshot(owner, 0)?
            .elements
            .into_iter()
            .map(|mut e| {
                e.boundary.clear();
                (e.key, e)
            })
            .collect();
        for seed in ffi::seeds(self.occt()?).map_err(failure)? {
            let origins: Vec<_> = match seed.role {
                3 => vec![
                    selected
                        .get(seed.source_index as usize)
                        .ok_or_else(|| failure("Advanced history names an absent selected face"))?
                        .clone(),
                ],
                5 => inputs
                    .get(&key(0, seed.source_index)?)
                    .ok_or_else(|| failure("Offset history names an absent source face"))?
                    .origins
                    .iter()
                    .map(|origin| Origin {
                        source: owner,
                        role: format!("offset/{}/{}", origin.source, origin.role),
                    })
                    .collect(),
                _ => return Err(failure("Unknown advanced history role")),
            };
            let element = elements
                .get_mut(&key(0, seed.face)?)
                .ok_or_else(|| failure("Advanced history names an absent generated face"))?;
            for origin in origins {
                GeometryIntent {
                    id: owner,
                    kind: TopologyKind::Face,
                    origins: BTreeSet::from([origin.clone()]),
                    boundary: None,
                    attachment: None,
                }
                .validate()
                .map_err(failure)?;
                element.origins.insert(origin.clone());
                if let Some(AnalyticGeometry::Plane { frame }) = element.geometry
                    && !element.frames.iter().any(|f| f.origin == origin)
                {
                    element.frames.push(AnchoredFrame { origin, frame });
                }
            }
        }
        let nodes = ffi::topology(self.occt()?).map_err(failure)?;
        self.finish_provenance(elements, &nodes)
    }
    /// Deformation/reflection frames must describe current native planes rather
    /// than preserve the pre-operation normals or support-plane positions.
    pub(crate) fn inherit_current_provenance(
        &mut self,
        source: &Self,
        owner: EntityId,
    ) -> Result<(), GeometryError> {
        self.inherit_provenance(&[source], None)?;
        let snapshot = self.topology_snapshot(owner, 0)?;
        self.provenance = snapshot
            .elements
            .into_iter()
            .map(|mut element| {
                let anchors: Vec<_> = element
                    .frames
                    .iter()
                    .map(|frame| frame.origin.clone())
                    .collect();
                element.frames.clear();
                if let Some(AnalyticGeometry::Plane { frame }) = element.geometry {
                    for origin in anchors {
                        element.frames.push(AnchoredFrame { origin, frame });
                    }
                }
                element
            })
            .collect();
        self.topology_snapshot(owner, 0)?;
        Ok(())
    }
    pub(crate) fn inherit_treatment_provenance(
        &mut self,
        source: &Self,
        edges: &[crate::TreatmentEdge],
        feature: EntityId,
    ) -> Result<(), GeometryError> {
        self.inherit_provenance(&[source], None)?;
        // Fresh descriptors come from the actual result. A generated chamfer
        // plane can anchor a sketch; a curved blend never inherits a fake plane.
        let snapshot = self.topology_snapshot(feature, 0)?;
        let mut elements: BTreeMap<_, _> = snapshot
            .elements
            .into_iter()
            .map(|mut element| {
                element.boundary.clear();
                (element.key, element)
            })
            .collect();
        let mut generated = 0;
        for seed in ffi::seeds(self.occt()?).map_err(failure)? {
            if seed.role != 3 {
                return Err(failure("Unexpected native edge-treatment seed role"));
            }
            let origin = &edges
                .get(seed.source_index as usize)
                .ok_or_else(|| failure("Treatment history refers to an absent selected edge"))?
                .origin;
            let element = elements
                .get_mut(&key(0, seed.face)?)
                .ok_or_else(|| failure("Treatment history refers to an absent generated face"))?;
            element.origins.insert(origin.clone());
            if let Some(AnalyticGeometry::Plane { frame }) = element.geometry {
                element.frames.push(AnchoredFrame {
                    origin: origin.clone(),
                    frame,
                });
            }
            generated += 1;
        }
        if generated == 0 {
            return Err(failure(
                "Native edge treatment produced no attributable generated faces",
            ));
        }
        let nodes = ffi::topology(self.occt()?).map_err(failure)?;
        self.finish_provenance(elements, &nodes)
    }
    /// Curve origins are supplied in authored outer-wire then hole-wire order.
    /// Rewinding a wire for OCCT preserves each curve's supplied identity.
    pub fn extrude_profile_named(
        profile: &Profile,
        height_mm: f64,
        feature: EntityId,
        curves: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if feature.is_nil() {
            return Err(failure("Feature identity is nil"));
        }
        let edges: Vec<_> = profile
            .outer
            .iter()
            .chain(profile.holes.iter().flatten())
            .collect();
        if curves.len() != edges.len()
            || curves.iter().collect::<BTreeSet<_>>().len() != curves.len()
        {
            return Err(failure(
                "Every profile curve needs distinct source provenance",
            ));
        }
        // Validate before invoking native code, including the qualified side role.
        for curve in curves {
            let probe = GeometryIntent {
                id: EntityId::new(),
                kind: TopologyKind::Face,
                origins: BTreeSet::from([
                    curve.clone(),
                    Origin {
                        source: feature,
                        role: format!("side/{}/{}", curve.source, curve.role),
                    },
                ]),
                boundary: None,
                attachment: None,
            };
            probe.validate().map_err(failure)?;
        }
        let mut result = Self::extrude_profile(profile, height_mm, policy)?;
        let side_frames: Vec<_> = edges
            .iter()
            .map(|edge| side_frame(profile, edge, height_mm))
            .collect();
        result.name_profile_prism(profile.frame, height_mm, feature, curves, &side_frames)?;
        Ok(result)
    }
    pub(crate) fn name_profile_prism(
        &mut self,
        frame: PlaneFrame,
        height_mm: f64,
        feature: EntityId,
        curves: &[Origin],
        side_frames: &[Option<ReferenceFrame>],
    ) -> Result<(), GeometryError> {
        self.name_profile_vector_prism(
            frame,
            frame.normal().map(|n| n * height_mm),
            feature,
            curves,
            side_frames,
        )
    }
    pub(crate) fn name_profile_vector_prism(
        &mut self,
        frame: PlaneFrame,
        displacement_mm: [f64; 3],
        feature: EntityId,
        curves: &[Origin],
        side_frames: &[Option<ReferenceFrame>],
    ) -> Result<(), GeometryError> {
        let start = reference_frame(frame);
        let mut end = start;
        end.origin_mm = std::array::from_fn(|i| start.origin_mm[i] + displacement_mm[i]);
        self.name_profile_sweep([Some(start), Some(end)], feature, curves, side_frames)
    }
    pub(crate) fn name_profile_sweep(
        &mut self,
        cap_frames: [Option<ReferenceFrame>; 2],
        feature: EntityId,
        curves: &[Origin],
        side_frames: &[Option<ReferenceFrame>],
    ) -> Result<(), GeometryError> {
        if feature.is_nil()
            || curves.len() != side_frames.len()
            || curves.iter().collect::<BTreeSet<_>>().len() != curves.len()
        {
            return Err(failure(
                "Every exact profile edge needs a distinct authored origin",
            ));
        }
        for curve in curves {
            GeometryIntent {
                id: EntityId::new(),
                kind: TopologyKind::Face,
                origins: BTreeSet::from([
                    curve.clone(),
                    Origin {
                        source: feature,
                        role: format!("side/{}/{}", curve.source, curve.role),
                    },
                ]),
                boundary: None,
                attachment: None,
            }
            .validate()
            .map_err(failure)?;
        }
        let result = self;
        let nodes = ffi::topology(result.occt()?).map_err(failure)?;
        let mut elements: BTreeMap<_, _> = nodes
            .iter()
            .map(|n| key(n.kind, n.index).map(|k| (k, empty(k))))
            .collect::<Result<_, _>>()?;
        for seed in ffi::seeds(result.occt()?).map_err(failure)? {
            let key = key(0, seed.face)?;
            let element = elements
                .get_mut(&key)
                .ok_or_else(|| failure("Native seed refers to absent face"))?;
            let (origin, frame, curve) = match seed.role {
                0 | 1 => {
                    let cap_frame = cap_frames[seed.role as usize]
                        .ok_or_else(|| failure("Native sweep supplied an unexpected cap face"))?;
                    (
                        Origin {
                            source: feature,
                            role: if seed.role == 0 {
                                "cap.start"
                            } else {
                                "cap.end"
                            }
                            .into(),
                        },
                        Some(cap_frame),
                        None,
                    )
                }
                2 => {
                    let index = seed.source_index as usize;
                    let curve = curves
                        .get(index)
                        .ok_or_else(|| failure("Native seed refers to absent profile source"))?;
                    (
                        Origin {
                            source: feature,
                            role: format!("side/{}/{}", curve.source, curve.role),
                        },
                        *side_frames
                            .get(index)
                            .ok_or_else(|| failure("Native profile source index is absent"))?,
                        Some(curve.clone()),
                    )
                }
                _ => return Err(failure("Unknown native prism seed")),
            };
            if !element.origins.is_empty() {
                return Err(failure("Conflicting native face seeds"));
            }
            element.origins.insert(origin.clone());
            if let Some(curve) = curve {
                element.origins.insert(curve);
            }
            if let Some(frame) = frame {
                element.frames.push(AnchoredFrame { origin, frame });
            }
        }
        if elements
            .values()
            .any(|e| e.key.kind == TopologyKind::Face && e.origins.is_empty())
        {
            return Err(failure(
                "Native prism did not provide complete face ancestry",
            ));
        }
        result.finish_provenance(elements, &nodes)?;
        Ok(())
    }
    /// Share completed immutable geometry with a new exclusive source owner.
    /// Only source identities change; roles, face ordinals, boundaries and
    /// numeric attachment frames stay intact. Foreign or unnamed ancestry is
    /// refused rather than silently relabeled.
    pub fn rebind_exclusive_owner(
        &self,
        previous: EntityId,
        next: EntityId,
    ) -> Result<Self, GeometryError> {
        if previous.is_nil()
            || next.is_nil()
            || self.provenance.is_empty()
            || !ffi::valid(self.occt()?).map_err(failure)?
            || self.solid_count()? != 1
            || self.volume_mm3()? <= 0.
        {
            return Err(failure(
                "Owner rebinding requires a completed positive owned solid",
            ));
        }
        let snapshot = self.topology_snapshot(previous, 0)?;
        if snapshot.elements.iter().any(|element| {
            (element.key.kind == TopologyKind::Face && element.origins.is_empty())
                || element
                    .origins
                    .iter()
                    .chain(&element.boundary)
                    .chain(element.frames.iter().map(|frame| &frame.origin))
                    .any(|origin| origin.source != previous)
        }) {
            return Err(failure(
                "Owner rebinding refuses unnamed or foreign source ancestry",
            ));
        }
        let mut result = self.share()?;
        for element in &mut result.provenance {
            let rebind = |origin: &Origin| Origin {
                source: next,
                role: origin.role.clone(),
            };
            element.origins = element.origins.iter().map(rebind).collect();
            element.boundary = element.boundary.iter().map(rebind).collect();
            for frame in &mut element.frames {
                frame.origin.source = next;
            }
        }
        result
            .topology_snapshot(next, 0)?
            .validate()
            .map_err(failure)?;
        Ok(result)
    }

    /// Simultaneously relabel this feature's default start/end caps. Used for
    /// the backward half of a two-sided sweep before exact union. Geometry and
    /// attachment frames stay unchanged; foreign feature provenance is retained.
    pub fn with_feature_cap_roles(
        mut self,
        feature: EntityId,
        start: &str,
        end: &str,
    ) -> Result<Self, GeometryError> {
        let invalid = |message: &str| GeometryError {
            operation: "cap roles",
            kind: GeometryErrorKind::Input,
            message: message.into(),
        };
        if feature.is_nil()
            || start.trim().is_empty()
            || end.trim().is_empty()
            || start.len() > 256
            || end.len() > 256
            || start == end
        {
            return Err(invalid(
                "Cap roles need a non-nil feature and distinct nonblank names of at most 256 bytes",
            ));
        }
        let old = [
            Origin {
                source: feature,
                role: "cap.start".into(),
            },
            Origin {
                source: feature,
                role: "cap.end".into(),
            },
        ];
        let mut found = [false; 2];
        let faces = self.faces()?;
        for element in &self.provenance {
            for origin in element
                .origins
                .iter()
                .chain(&element.boundary)
                .chain(element.frames.iter().map(|f| &f.origin))
            {
                if origin.source == feature
                    && !old.contains(origin)
                    && (origin.role == start || origin.role == end)
                {
                    return Err(invalid(
                        "Cap role collides with other provenance owned by this feature",
                    ));
                }
            }
            if element.key.kind != TopologyKind::Face {
                continue;
            }
            for (i, cap) in old.iter().enumerate() {
                if element.origins.contains(cap) {
                    if !element.frames.iter().any(|frame| frame.origin == *cap)
                        || !faces
                            .iter()
                            .any(|face| face.index == element.key.ordinal && face.plane.is_some())
                    {
                        return Err(invalid(
                            "Cap origin also identifies non-cap or unanchored geometry; explicit repair is required",
                        ));
                    }
                    found[i] = true;
                }
            }
        }
        if !found.into_iter().all(|present| present) {
            return Err(invalid(
                "Both owned start/end planar caps are required for a cap-role override",
            ));
        }
        let remap = |origin: &Origin| {
            if origin == &old[0] {
                Origin {
                    source: feature,
                    role: start.into(),
                }
            } else if origin == &old[1] {
                Origin {
                    source: feature,
                    role: end.into(),
                }
            } else {
                origin.clone()
            }
        };
        for element in &mut self.provenance {
            element.origins = element.origins.iter().map(&remap).collect();
            element.boundary = element.boundary.iter().map(&remap).collect();
            for frame in &mut element.frames {
                frame.origin = remap(&frame.origin);
            }
        }
        self.topology_snapshot(feature, 0)?;
        Ok(self)
    }
    pub fn box_named(
        size_mm: [f64; 3],
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        if size_mm.iter().any(|v| {
            !v.is_finite() || *v < policy.minimum_feature_mm || *v > policy.maximum_coordinate_mm
        }) {
            return Err(GeometryError {
                operation: "box",
                kind: GeometryErrorKind::Input,
                message: "Box dimensions must be positive and within the numerical policy".into(),
            });
        }
        let curves: Vec<_> = ["y.min", "x.max", "y.max", "x.min"]
            .into_iter()
            .map(|role| Origin {
                source: feature,
                role: role.into(),
            })
            .collect();
        Self::extrude_profile_named(
            &Profile::rectangle(size_mm[0], size_mm[1]),
            size_mm[2],
            feature,
            &curves,
            policy,
        )
    }
    /// Evaluation data: source Links persist intent, never these local ordinals.
    pub fn topology_snapshot(
        &self,
        owner: EntityId,
        revision: u64,
    ) -> Result<TopologySnapshot, GeometryError> {
        if self.uses_rust_geometry() {
            let mut snapshot = if let Some(placed) = self.polygon_prism() {
                placed.snapshot(owner, revision).map_err(failure)?
            } else if let Some((cylinder, frame)) = self.axial_cylinder_frame() {
                crate::rust_prism_topology::cylinder_snapshot(
                    cylinder,
                    frame,
                    self.cylinder_sketch(),
                    owner,
                    revision,
                )
                .map_err(failure)?
            } else if let Some((prism, frame)) = self.rust_prism_frame() {
                crate::rust_prism_topology::snapshot(prism, frame, owner, revision)
                    .map_err(failure)?
            } else if let Some((body, frame)) = self.orthogonal_body() {
                body.topology(frame, owner, revision)
            } else {
                self.rust_compound()
                    .expect("checked Rust body")
                    .topology(owner, revision)?
            };
            if !self.rust_origins.is_empty() {
                for element in &mut snapshot.elements {
                    for origin in &self.rust_origins {
                        element.origins.insert(origin.clone());
                        if !element.boundary.is_empty() {
                            element.boundary.insert(origin.clone());
                        }
                        if let Some(AnalyticGeometry::Plane { frame }) = element.geometry {
                            element.frames.push(AnchoredFrame {
                                origin: origin.clone(),
                                frame,
                            });
                        }
                    }
                }
                snapshot.validate().map_err(failure)?;
            }
            return Ok(snapshot);
        }
        let mut elements: Vec<TopologyElement> = if self.provenance.is_empty() {
            ffi::topology(self.occt()?)
                .map_err(failure)?
                .iter()
                .map(|n| key(n.kind, n.index).map(empty))
                .collect::<Result<_, _>>()?
        } else {
            self.provenance.clone()
        };
        let xyz = |p: ffi::Point3| [p.x, p.y, p.z];
        let mut geometry = BTreeMap::new();
        for edge in ffi::analytic_edges(self.occt()?).map_err(failure)? {
            let exact = match edge.kind {
                0 => AnalyticGeometry::Line {
                    a_mm: xyz(edge.a),
                    b_mm: xyz(edge.b),
                },
                1 => AnalyticGeometry::CircularArc {
                    frame: ReferenceFrame {
                        origin_mm: xyz(edge.frame.origin),
                        u: xyz(edge.frame.u),
                        v: xyz(edge.frame.v),
                    },
                    radius_mm: edge.radius,
                    start_rad: edge.start,
                    sweep_rad: edge.sweep,
                },
                _ => return Err(failure("Unknown native analytic edge kind")),
            };
            geometry.insert(key(1, edge.index)?, exact);
        }
        for vertex in ffi::vertices(self.occt()?).map_err(failure)? {
            geometry.insert(
                key(2, vertex.index)?,
                AnalyticGeometry::Point {
                    position_mm: xyz(vertex.point),
                },
            );
        }
        for face in self.faces()? {
            if let Some((origin_mm, normal)) = face.plane {
                let seed = if normal[0].abs() < 0.9 {
                    [1., 0., 0.]
                } else {
                    [0., 1., 0.]
                };
                let dot = seed.iter().zip(normal).map(|(a, b)| a * b).sum::<f64>();
                let mut u: [f64; 3] = std::array::from_fn(|i| seed[i] - normal[i] * dot);
                let length = u.iter().map(|v| v * v).sum::<f64>().sqrt();
                u = u.map(|v| v / length);
                let v = [
                    normal[1] * u[2] - normal[2] * u[1],
                    normal[2] * u[0] - normal[0] * u[2],
                    normal[0] * u[1] - normal[1] * u[0],
                ];
                geometry.insert(
                    key(0, face.index)?,
                    AnalyticGeometry::Plane {
                        frame: ReferenceFrame { origin_mm, u, v },
                    },
                );
            }
        }
        for element in &mut elements {
            // Native supported geometry takes precedence. Proven affine
            // Plane/Line/Point descriptors remain exact when OCCT stores their
            // output as NURBS; these are never inferred from sampled meshes.
            if let Some(current) = geometry.remove(&element.key) {
                element.geometry = Some(current);
            }
        }
        let snapshot = TopologySnapshot {
            owner,
            revision,
            elements,
        };
        snapshot.validate().map_err(failure)?;
        Ok(snapshot)
    }
    pub(crate) fn inherit_provenance(
        &mut self,
        operands: &[&Self],
        transform: Option<RigidTransform>,
    ) -> Result<(), GeometryError> {
        if operands.iter().all(|s| s.provenance.is_empty()) {
            return Ok(());
        }
        let nodes = ffi::topology(self.occt()?).map_err(failure)?;
        let mut elements: BTreeMap<_, _> = nodes
            .iter()
            .map(|n| key(n.kind, n.index).map(|k| (k, empty(k))))
            .collect::<Result<_, _>>()?;
        let inputs: Vec<BTreeMap<_, _>> = operands
            .iter()
            .map(|s| s.provenance.iter().map(|e| (e.key, e)).collect())
            .collect();
        let mut frames: BTreeMap<TopologyKey, BTreeMap<Origin, Option<ReferenceFrame>>> =
            BTreeMap::new();
        for relation in ffi::ancestry(self.occt()?).map_err(failure)? {
            let out = key(relation.kind, relation.output)?;
            let source = key(relation.kind, relation.input)?;
            let operand = inputs
                .get(relation.operand as usize)
                .ok_or_else(|| failure("Invalid native history operand"))?;
            let Some(input) = operand.get(&source) else {
                continue;
            };
            let element = elements
                .get_mut(&out)
                .ok_or_else(|| failure("Invalid native history result"))?;
            element.origins.extend(input.origins.iter().cloned());
            if let Some(transform) = transform
                && let Some(geometry) = input.geometry
            {
                element.geometry = AffineTransform {
                    matrix: transform.rotation,
                    translation_mm: transform.translation_mm,
                }
                .analytic(geometry)?;
            }
            for anchored in &input.frames {
                let frame = transform.map_or(anchored.frame, |t| transformed(anchored.frame, t));
                frames
                    .entry(out)
                    .or_default()
                    .entry(anchored.origin.clone())
                    .and_modify(|old| {
                        if *old != Some(frame) {
                            *old = None;
                        }
                    })
                    .or_insert(Some(frame));
            }
        }
        for (key, origins) in frames {
            let element = elements.get_mut(&key).expect("mapped native element");
            element.frames = origins
                .into_iter()
                .filter_map(|(origin, frame)| frame.map(|frame| AnchoredFrame { origin, frame }))
                .collect();
        }
        self.finish_provenance(elements, &nodes)
    }
    pub(crate) fn inherit_affine_provenance(
        &mut self,
        source: &Self,
        transform: AffineTransform,
        owner: EntityId,
    ) -> Result<(), GeometryError> {
        let snapshot = source.topology_snapshot(owner, 0)?;
        let inputs: BTreeMap<_, _> = snapshot.elements.iter().map(|e| (e.key, e)).collect();
        let nodes = ffi::topology(self.occt()?).map_err(failure)?;
        let mut elements: BTreeMap<_, _> = nodes
            .iter()
            .map(|n| key(n.kind, n.index).map(|k| (k, empty(k))))
            .collect::<Result<_, _>>()?;
        let mut mapped = BTreeSet::new();
        for relation in ffi::ancestry(self.occt()?).map_err(failure)? {
            let out = key(relation.kind, relation.output)?;
            if relation.operand != 0 || !mapped.insert(out) {
                return Err(failure("Affine history is not a unique source mapping"));
            }
            let input = inputs
                .get(&key(relation.kind, relation.input)?)
                .ok_or_else(|| failure("Affine source topology is absent"))?;
            let element = elements
                .get_mut(&out)
                .ok_or_else(|| failure("Affine output topology is absent"))?;
            element.origins = input.origins.clone();
            element.frames = input
                .frames
                .iter()
                .map(|a| {
                    Ok(AnchoredFrame {
                        origin: a.origin.clone(),
                        frame: transform.frame(a.frame)?,
                    })
                })
                .collect::<Result<_, GeometryError>>()?;
            element.geometry = input
                .geometry
                .map(|g| transform.analytic(g))
                .transpose()?
                .flatten();
        }
        if mapped.len() != elements.len() {
            return Err(failure("Affine output has incomplete source topology"));
        }
        self.finish_provenance(elements, &nodes)
    }
    /// Only the dedicated plane-clip adapter emits this one boundary seed family.
    /// The cap identity follows its authored side, never a native face ordinal.
    pub(crate) fn seed_split_plane(
        &mut self,
        feature: EntityId,
        role: &str,
        frame: PlaneFrame,
    ) -> Result<(), GeometryError> {
        let mut elements: BTreeMap<_, _> = self
            .topology_snapshot(feature, 0)?
            .elements
            .into_iter()
            .map(|element| (element.key, element))
            .collect();
        let origin = Origin {
            source: feature,
            role: role.into(),
        };
        let seeds = ffi::seeds(self.occt()?).map_err(failure)?;
        if seeds.is_empty() {
            return Err(failure("Plane Split lacks exact boundary-face ancestry"));
        }
        for seed in seeds {
            if seed.role != 0 || seed.source_index != 0 {
                return Err(failure("Unexpected plane Split boundary seed"));
            }
            let element = elements
                .get_mut(&TopologyKey {
                    kind: TopologyKind::Face,
                    ordinal: seed.face,
                })
                .ok_or_else(|| failure("Plane Split boundary names an absent face"))?;
            element.origins.insert(origin.clone());
            element.frames.push(AnchoredFrame {
                origin: origin.clone(),
                frame: reference_frame(frame),
            });
        }
        let nodes = ffi::topology(self.occt()?).map_err(failure)?;
        self.finish_provenance(elements, &nodes)
    }
    pub(crate) fn finish_provenance(
        &mut self,
        mut elements: BTreeMap<TopologyKey, TopologyElement>,
        nodes: &[ffi::TopologyNode],
    ) -> Result<(), GeometryError> {
        let faces: BTreeMap<_, _> = elements
            .values()
            .filter(|e| e.key.kind == TopologyKind::Face)
            .map(|e| (e.key.ordinal, e.origins.clone()))
            .collect();
        for node in nodes {
            let element = elements
                .get_mut(&key(node.kind, node.index)?)
                .expect("enumerated element");
            for index in &node.incident_faces {
                let origins = faces
                    .get(index)
                    .ok_or_else(|| failure("Invalid native adjacency face"))?;
                element.boundary.extend(origins.iter().cloned());
            }
            if element.key.kind != TopologyKind::Face {
                element.origins.extend(element.boundary.iter().cloned());
            }
        }
        self.provenance = elements.into_values().collect();
        // Validate native-derived data before it can enter the shared API.
        self.topology_snapshot(EntityId::new(), 0)?;
        Ok(())
    }
}

#[cfg(test)]
mod exclusive_owner_tests {
    use super::*;
    #[test]
    fn exclusive_owner_rebind_preserves_geometry_and_all_source_roles_without_mutation() {
        let old = EntityId::new();
        let next = EntityId::new();
        let policy = GeometryPolicy::default();
        let source = Solid::box_named([4., 5., 6.], old, policy).unwrap();
        let before = source.topology_snapshot(old, 0).unwrap();
        let brep = source.to_brep().unwrap();
        let rebound = source.rebind_exclusive_owner(old, next).unwrap();
        let after = rebound.topology_snapshot(next, 0).unwrap();
        assert_eq!(before.elements.len(), after.elements.len());
        for (a, b) in before.elements.iter().zip(&after.elements) {
            assert_eq!(a.key, b.key);
            assert_eq!(a.geometry, b.geometry);
            assert_eq!(
                a.origins
                    .iter()
                    .map(|o| o.role.clone())
                    .collect::<BTreeSet<_>>(),
                b.origins.iter().map(|o| o.role.clone()).collect()
            );
            assert_eq!(
                a.boundary
                    .iter()
                    .map(|o| o.role.clone())
                    .collect::<BTreeSet<_>>(),
                b.boundary.iter().map(|o| o.role.clone()).collect()
            );
            assert_eq!(a.frames.len(), b.frames.len());
            for (x, y) in a.frames.iter().zip(&b.frames) {
                assert_eq!(x.frame, y.frame);
                assert_eq!(x.origin.role, y.origin.role);
            }
            assert!(
                b.origins
                    .iter()
                    .chain(&b.boundary)
                    .chain(b.frames.iter().map(|f| &f.origin))
                    .all(|o| o.source == next)
            );
        }
        assert_eq!(rebound.to_brep().unwrap(), brep);
        assert_eq!(source.topology_snapshot(old, 0).unwrap(), before);
        assert_eq!(source.to_brep().unwrap(), brep);
        assert!(
            source
                .rebind_exclusive_owner(EntityId::nil(), next)
                .is_err()
        );
        assert!(source.rebind_exclusive_owner(old, EntityId::nil()).is_err());
        assert!(source.rebind_exclusive_owner(next, old).is_err());
        for field in 0..3 {
            let mut foreign = source.share().unwrap();
            let origin = Origin {
                source: EntityId::new(),
                role: "foreign".into(),
            };
            match field {
                0 => {
                    foreign.provenance[0].origins.insert(origin);
                }
                1 => {
                    foreign.provenance[0].boundary.insert(origin);
                }
                _ => {
                    let frame = foreign
                        .provenance
                        .iter_mut()
                        .flat_map(|e| &mut e.frames)
                        .next()
                        .unwrap();
                    frame.origin = origin;
                }
            }
            assert!(foreign.rebind_exclusive_owner(old, next).is_err());
        }
        let unnamed = Solid::box_solid([4., 5., 6.], policy).unwrap();
        assert!(unnamed.rebind_exclusive_owner(old, next).is_err());
    }
}

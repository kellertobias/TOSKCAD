//! Persistent source intent is separate from evaluation-local topology indices.
//! Resolution uses provenance and explicit boundary intent, never nearest-face
//! guessing. Missing or ambiguous results remain inspectable and repairable.
use crate::{
    commands::{Actor, CommandError, ErrorCode, Prepared, Request, Session},
    document::{Document, EntityData, EntityId, Link, Mutation},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
mod analytic;
mod dependency;
pub use analytic::AnalyticGeometry;
pub use dependency::dependency_hash;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TopologyKind {
    Face,
    Edge,
    Vertex,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    pub source: EntityId,
    pub role: String,
}
impl Origin {
    fn validate(&self) -> Result<(), String> {
        if self.source.is_nil() || self.role.trim().is_empty() || self.role.len() > 256 {
            Err("Invalid geometry provenance".into())
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    pub anchor: Origin,
    pub offset_mm: [f64; 3],
    pub angle_rad: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryIntent {
    pub id: EntityId,
    pub kind: TopologyKind,
    pub origins: BTreeSet<Origin>,
    /// Explicit repair can distinguish fragments by their adjacent provenance.
    pub boundary: Option<BTreeSet<Origin>>,
    pub attachment: Option<Attachment>,
}
fn origins_valid(origins: &BTreeSet<Origin>) -> Result<(), String> {
    if origins.len() > 1024 {
        return Err("Geometry provenance budget exceeded".into());
    }
    for origin in origins {
        origin.validate()?;
    }
    Ok(())
}
impl GeometryIntent {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_nil() || self.origins.is_empty() {
            return Err("Geometry reference needs an identity and source provenance".into());
        }
        origins_valid(&self.origins)?;
        if let Some(boundary) = &self.boundary {
            origins_valid(boundary)?;
        }
        if let Some(attachment) = &self.attachment
            && (self.kind != TopologyKind::Face
                || !self.origins.contains(&attachment.anchor)
                || attachment
                    .offset_mm
                    .iter()
                    .any(|v| !v.is_finite() || v.abs() > 1e9)
                || !attachment.angle_rad.is_finite()
                || attachment.angle_rad.abs() > 1e6)
        {
            return Err("Invalid planar attachment intent".into());
        }
        Ok(())
    }
    /// Capture the selected element. A fragment discriminator is included only
    /// when its source provenance alone names multiple current elements.
    pub fn capture(snapshot: &TopologySnapshot, key: TopologyKey) -> Result<Self, String> {
        snapshot.validate()?;
        let candidate = snapshot
            .elements
            .iter()
            .find(|e| e.key == key)
            .ok_or("Selected topology is missing")?;
        if candidate.origins.is_empty() {
            return Err("Selected topology has no stable source provenance".into());
        }
        let mut intent = Self {
            id: EntityId::new(),
            kind: key.kind,
            origins: candidate.origins.clone(),
            boundary: None,
            attachment: None,
        };
        if matching(&intent, snapshot).len() != 1 {
            intent.boundary = Some(candidate.boundary.clone());
        }
        if matching(&intent, snapshot).len() != 1 {
            return Err("Selected fragment cannot be named uniquely from its provenance".into());
        }
        intent.validate()?;
        Ok(intent)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceFrame {
    pub origin_mm: [f64; 3],
    pub u: [f64; 3],
    pub v: [f64; 3],
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
impl ReferenceFrame {
    pub fn normal(self) -> [f64; 3] {
        [
            self.u[1] * self.v[2] - self.u[2] * self.v[1],
            self.u[2] * self.v[0] - self.u[0] * self.v[2],
            self.u[0] * self.v[1] - self.u[1] * self.v[0],
        ]
    }
    pub fn validate(self) -> Result<(), String> {
        if self
            .origin_mm
            .iter()
            .chain(&self.u)
            .chain(&self.v)
            .any(|v| !v.is_finite())
            || self.origin_mm.iter().any(|v| v.abs() > 1e9)
            || (dot(self.u, self.u) - 1.).abs() > 1e-9
            || (dot(self.v, self.v) - 1.).abs() > 1e-9
            || dot(self.u, self.v).abs() > 1e-9
        {
            Err("Invalid reference frame".into())
        } else {
            Ok(())
        }
    }
    fn attached(self, attachment: &Attachment) -> Self {
        let n = self.normal();
        let origin_mm = std::array::from_fn(|i| {
            self.origin_mm[i]
                + self.u[i] * attachment.offset_mm[0]
                + self.v[i] * attachment.offset_mm[1]
                + n[i] * attachment.offset_mm[2]
        });
        let (s, c) = attachment.angle_rad.sin_cos();
        Self {
            origin_mm,
            u: std::array::from_fn(|i| self.u[i] * c + self.v[i] * s),
            v: std::array::from_fn(|i| self.v[i] * c - self.u[i] * s),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TopologyKey {
    pub kind: TopologyKind,
    pub ordinal: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchoredFrame {
    pub origin: Origin,
    pub frame: ReferenceFrame,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyElement {
    pub key: TopologyKey,
    pub origins: BTreeSet<Origin>,
    pub boundary: BTreeSet<Origin>,
    /// Semantic source frames, not arbitrary current surface UV origins.
    pub frames: Vec<AnchoredFrame>,
    /// Current B-rep geometry, never sampled display edges or authored intent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<AnalyticGeometry>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopologySnapshot {
    pub owner: EntityId,
    pub revision: u64,
    pub elements: Vec<TopologyElement>,
}
impl TopologySnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.owner.is_nil() || self.elements.len() > 100_000 {
            return Err("Invalid topology snapshot identity/budget".into());
        }
        let mut keys = BTreeSet::new();
        for element in &self.elements {
            if !keys.insert(element.key) {
                return Err("Duplicate evaluation-local topology key".into());
            }
            origins_valid(&element.origins)?;
            origins_valid(&element.boundary)?;
            if let Some(geometry) = element.geometry {
                geometry.validate()?;
                if geometry.topology_kind() != element.key.kind {
                    return Err("Analytic geometry kind differs from its topology element".into());
                }
            }
            if element.frames.len() > element.origins.len() {
                return Err("Frame provenance budget exceeded".into());
            }
            let mut frame_origins = BTreeSet::new();
            for anchored in &element.frames {
                let origin = &anchored.origin;
                let frame = &anchored.frame;
                if !frame_origins.insert(origin) {
                    return Err("Duplicate planar source frame".into());
                }
                if element.key.kind != TopologyKind::Face || !element.origins.contains(origin) {
                    return Err("Frame is not attached to a face source".into());
                }
                frame.validate()?;
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Resolution {
    Resolved {
        key: TopologyKey,
        frame: Option<ReferenceFrame>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        geometry: Option<AnalyticGeometry>,
    },
    Missing,
    MissingSource,
    Ambiguous {
        candidates: Vec<TopologyKey>,
    },
    Stale {
        snapshot_revision: u64,
    },
    UnavailableFrame,
    Invalid {
        message: String,
    },
}
fn matching<'a>(
    intent: &GeometryIntent,
    snapshot: &'a TopologySnapshot,
) -> Vec<&'a TopologyElement> {
    snapshot
        .elements
        .iter()
        .filter(|e| {
            e.key.kind == intent.kind
                && intent.origins.is_subset(&e.origins)
                && intent.boundary.as_ref().is_none_or(|b| *b == e.boundary)
        })
        .collect()
}
pub fn resolve(intent: &GeometryIntent, snapshot: &TopologySnapshot, revision: u64) -> Resolution {
    if let Err(message) = intent.validate().and_then(|()| snapshot.validate()) {
        return Resolution::Invalid { message };
    }
    if snapshot.revision != revision {
        return Resolution::Stale {
            snapshot_revision: snapshot.revision,
        };
    }
    let matches = matching(intent, snapshot);
    if matches.is_empty() {
        return Resolution::Missing;
    }
    if matches.len() > 1 {
        return Resolution::Ambiguous {
            candidates: matches.iter().map(|e| e.key).collect(),
        };
    }
    let element = matches[0];
    let frame = if let Some(attachment) = &intent.attachment {
        let Some(frame) = element
            .frames
            .iter()
            .find(|f| f.origin == attachment.anchor)
            .map(|f| &f.frame)
        else {
            return Resolution::UnavailableFrame;
        };
        let frame = frame.attached(attachment);
        if let Err(message) = frame.validate() {
            return Resolution::Invalid { message };
        }
        Some(frame)
    } else {
        None
    };
    Resolution::Resolved {
        key: element.key,
        frame,
        geometry: element.geometry,
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ReferenceAddress {
    pub owner: EntityId,
    pub reference: EntityId,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub address: ReferenceAddress,
    pub source: EntityId,
    pub role: String,
    pub intent: GeometryIntent,
    pub resolution: Resolution,
    pub affected: Vec<EntityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_hash: Option<crate::project_file::ContentHash>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReferenceReport {
    pub revision: u64,
    pub entries: Vec<Diagnostic>,
}
impl ReferenceReport {
    pub fn all_resolved(&self) -> bool {
        self.entries
            .iter()
            .all(|e| matches!(e.resolution, Resolution::Resolved { .. }))
    }
    pub fn get(&self, address: ReferenceAddress) -> Option<&Diagnostic> {
        self.entries.iter().find(|e| e.address == address)
    }
}
pub fn inspect(
    document: &Document,
    revision: u64,
    snapshots: &BTreeMap<EntityId, TopologySnapshot>,
) -> ReferenceReport {
    let mut entries = vec![];
    let mut hashes = BTreeMap::new();
    for entity in document.entities() {
        for link in entity.references() {
            if let Some(intent) = &link.geometry {
                let resolution = if document.get(link.target).is_none() {
                    Resolution::MissingSource
                } else {
                    match snapshots.get(&link.target) {
                        None => Resolution::MissingSource,
                        Some(snapshot) if snapshot.owner != link.target => Resolution::Invalid {
                            message: "Topology snapshot belongs to another source".into(),
                        },
                        Some(snapshot) => resolve(intent, snapshot, revision),
                    }
                };
                entries.push(Diagnostic {
                    address: ReferenceAddress {
                        owner: entity.id,
                        reference: intent.id,
                    },
                    source: link.target,
                    role: link.role.clone(),
                    intent: intent.clone(),
                    resolution,
                    affected: document.affected([entity.id]).unwrap_or_default(),
                    dependency_hash: *hashes
                        .entry(link.target)
                        .or_insert_with(|| dependency_hash(document, link.target).ok()),
                });
            }
        }
    }
    ReferenceReport { revision, entries }
}
/// Authored datum coordinates are never a fallback for unresolved attachments.
pub(crate) fn plane_frame(
    document: &Document,
    plane: EntityId,
    u: [f64; 3],
    evaluated: Option<(u64, &ReferenceReport)>,
) -> Result<crate::sketch::Frame, crate::document::DocumentError> {
    use crate::document::DocumentError;
    let invalid = |message: &str| DocumentError::Invalid(message.into());
    let Some(EntityData::Plane {
        origin_mm,
        normal,
        anchor,
    }) = document.get(plane).map(|e| &e.data)
    else {
        return Err(invalid("Sketch plane is missing"));
    };
    if anchor
        .as_ref()
        .is_some_and(|link| link.role == crate::sketch::OFFSET_PLANE_ROLE)
    {
        return crate::sketch::offset_plane_frame(document, plane, u, evaluated);
    }
    if let Some(link) = anchor
        && let Some(intent) = &link.geometry
    {
        let Some((revision, report)) = evaluated else {
            return Err(invalid(
                "Attached sketch requires current reference evaluation",
            ));
        };
        let diagnostic = report
            .get(ReferenceAddress {
                owner: plane,
                reference: intent.id,
            })
            .ok_or_else(|| invalid("Sketch attachment has no reference diagnostic"))?;
        if report.revision != revision
            || diagnostic.intent != *intent
            || diagnostic.source != link.target
        {
            return Err(invalid(
                "Sketch attachment evaluation is stale or belongs to another source",
            ));
        }
        let Resolution::Resolved {
            frame: Some(frame), ..
        } = diagnostic.resolution
        else {
            return Err(invalid(
                "Sketch attachment is unresolved or has no planar frame",
            ));
        };
        frame.validate().map_err(DocumentError::Invalid)?;
        return Ok(crate::sketch::Frame {
            origin: frame.origin_mm,
            u: frame.u,
            v: frame.v,
            n: frame.normal(),
        });
    }
    let n = *normal;
    Ok(crate::sketch::Frame {
        origin: *origin_mm,
        u,
        n,
        v: [
            n[1] * u[2] - n[2] * u[1],
            n[2] * u[0] - n[0] * u[2],
            n[0] * u[1] - n[1] * u[0],
        ],
    })
}
/// A failed reference attempt retains the last valid presentation. The host
/// supplies its actual document revision; stale reports cannot publish a result.
pub struct ReferenceGate<T> {
    visible: Option<T>,
    attempt: Option<ReferenceReport>,
}
impl<T> Default for ReferenceGate<T> {
    fn default() -> Self {
        Self {
            visible: None,
            attempt: None,
        }
    }
}
impl<T> ReferenceGate<T> {
    pub fn visible(&self) -> Option<&T> {
        self.visible.as_ref()
    }
    pub fn report(&self) -> Option<&ReferenceReport> {
        self.attempt.as_ref()
    }
    pub fn consider(
        &mut self,
        revision: u64,
        candidate: T,
        report: ReferenceReport,
    ) -> Result<bool, String> {
        if report.revision != revision
            || self.attempt.as_ref().is_some_and(|r| r.revision > revision)
        {
            return Err("Stale reference report cannot publish".into());
        }
        let valid = report.all_resolved();
        self.attempt = Some(report);
        if valid {
            self.visible = Some(candidate);
        }
        Ok(valid)
    }
}
fn command_error(code: ErrorCode, message: impl Into<String>) -> CommandError {
    CommandError {
        code,
        message: message.into(),
        involved: vec![],
        parameter: None,
    }
}
fn links_mut(data: &mut EntityData) -> Vec<&mut Link> {
    match data {
        EntityData::Plane { anchor, .. } => anchor.iter_mut().collect(),
        EntityData::Parameter { inputs, .. }
        | EntityData::Sketch { inputs, .. }
        | EntityData::Feature { inputs, .. }
        | EntityData::Drawing { inputs, .. }
        | EntityData::ExportSet { inputs, .. }
        | EntityData::Extension { inputs, .. } => inputs.iter_mut().collect(),
        _ => vec![],
    }
}
pub struct RepairPreview {
    pub prepared: Prepared,
    pub affected: Vec<EntityId>,
    pub selected: TopologyKey,
}
/// Repair binds an explicitly chosen current candidate. No evaluation-local
/// ordinal is stored; provenance and a boundary discriminator replace intent.
pub fn prepare_repair(
    session: &Session,
    revision: u64,
    actor: Actor,
    address: ReferenceAddress,
    snapshot: &TopologySnapshot,
    selected: TopologyKey,
    anchor: Option<Origin>,
) -> Result<RepairPreview, CommandError> {
    if session.revision() != revision || snapshot.revision != revision {
        return Err(command_error(
            ErrorCode::StaleRevision,
            "Reference repair requires the current source revision",
        ));
    }
    if session.document().get(snapshot.owner).is_none() {
        return Err(command_error(
            ErrorCode::InvalidRequest,
            "Replacement source is missing",
        ));
    }
    snapshot
        .validate()
        .map_err(|e| command_error(ErrorCode::InvalidRequest, e))?;
    let candidate = snapshot
        .elements
        .iter()
        .find(|e| e.key == selected)
        .ok_or_else(|| {
            command_error(ErrorCode::InvalidRequest, "Replacement topology is missing")
        })?;
    let mut entity = session
        .document()
        .get(address.owner)
        .ok_or_else(|| command_error(ErrorCode::InvalidRequest, "Reference owner is missing"))?
        .clone();
    let old = links_mut(&mut entity.data)
        .into_iter()
        .find(|l| {
            l.geometry
                .as_ref()
                .is_some_and(|i| i.id == address.reference)
        })
        .ok_or_else(|| command_error(ErrorCode::InvalidRequest, "Geometry reference is missing"))?;
    let intent = old.geometry.as_ref().expect("geometry intent");
    if intent.kind != selected.kind {
        return Err(command_error(
            ErrorCode::InvalidRequest,
            "Replacement topology has the wrong kind",
        ));
    }
    let mut replacement = GeometryIntent::capture(snapshot, selected)
        .map_err(|e| command_error(ErrorCode::InvalidRequest, e))?;
    replacement.id = intent.id;
    if let Some(attachment) = &intent.attachment {
        let origin = anchor
            .or_else(|| {
                candidate
                    .frames
                    .iter()
                    .any(|f| f.origin == attachment.anchor)
                    .then(|| attachment.anchor.clone())
            })
            .or_else(|| (candidate.frames.len() == 1).then(|| candidate.frames[0].origin.clone()))
            .ok_or_else(|| {
                command_error(
                    ErrorCode::InvalidRequest,
                    "Choose the replacement planar source frame",
                )
            })?;
        if !candidate.frames.iter().any(|f| f.origin == origin) {
            return Err(command_error(
                ErrorCode::InvalidRequest,
                "Replacement has no requested planar source frame",
            ));
        }
        replacement.attachment = Some(Attachment {
            anchor: origin,
            ..attachment.clone()
        });
    }
    if !matches!(resolve(&replacement,snapshot,revision),Resolution::Resolved{key,..} if key==selected)
    {
        return Err(command_error(
            ErrorCode::InvalidRequest,
            "Replacement reference remains ambiguous or has no frame",
        ));
    }
    old.target = snapshot.owner;
    old.geometry = Some(replacement);
    let affected = session.document().affected([address.owner])?;
    let prepared = session.prepare(Request {
        expected_revision: revision,
        actor,
        label: "Repair geometry reference".into(),
        edits: vec![Mutation::Replace(entity)],
    })?;
    Ok(RepairPreview {
        prepared,
        affected,
        selected,
    })
}

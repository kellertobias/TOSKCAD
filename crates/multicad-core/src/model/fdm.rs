//! FDM design aids: breakaway tabs, thin sacrificial bridges and removable
//! support blocks. Each aid is an `fdm.aid` feature producing its own Part,
//! so the finished Part it serves is never changed and an export recipe can
//! include or exclude the aid by its identity. An optional `fdm.process`
//! extension records the explicit process allowances the native inspection
//! checks the aids against.
//!
//! Aids are placed in world millimeters with +Z as the build direction. A
//! support's height is not authored: the native evaluator derives it from
//! the target Part's current solid over the support footprint.
use super::{
    common::{error, number, parameter_links, revision},
    source::{BodyOutput, check_dimension, invalid},
};
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    model::{CreatedBody, ModelPrepared},
    parameters::{EvaluatedParameters, Unit},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const AID_TYPE: &str = "fdm.aid";
pub const AID_SCHEMA: u32 = 1;
pub const AID_CAPABILITY: &str = "fdm.aid.1";
pub const PROCESS_KIND: &str = "fdm.process";
pub const PROCESS_CAPABILITY: &str = "fdm.process.1";

/// The geometry of one aid, in world millimeters with +Z up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AidShape {
    /// A flat pad on the build plane. `origin` is the contact point on the
    /// part outline; the pad runs `length` outward along `angle` (from +X
    /// about +Z) and `overlap` back under the part, `width` across and
    /// `thickness` up from `origin`.
    Tab {
        origin: [Dimension; 3],
        angle: Dimension,
        length: Dimension,
        width: Dimension,
        thickness: Dimension,
        overlap: Dimension,
    },
    /// A thin bar whose cross-section centre runs from `start` to `end`.
    /// The width is horizontal; the thickness is perpendicular to the bar
    /// and the width. `other` is an optional second Part the bar ties to.
    Bridge {
        start: [Dimension; 3],
        end: [Dimension; 3],
        width: Dimension,
        thickness: Dimension,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        other: Option<EntityId>,
    },
    /// A flat-topped block rising from height `base` over a rectangular
    /// footprint (`origin` corner, `length` along `angle`, `width` across).
    /// Its top stops `gap` below the lowest target surface over the footprint.
    Support {
        origin: [Dimension; 2],
        angle: Dimension,
        length: Dimension,
        width: Dimension,
        base: Dimension,
        gap: Dimension,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AidKind {
    Tab,
    Bridge,
    Support,
}
impl AidKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tab => "breakaway tab",
            Self::Bridge => "bridge",
            Self::Support => "support",
        }
    }
}
impl AidShape {
    pub fn kind(&self) -> AidKind {
        match self {
            Self::Tab { .. } => AidKind::Tab,
            Self::Bridge { .. } => AidKind::Bridge,
            Self::Support { .. } => AidKind::Support,
        }
    }
    fn lengths(&self) -> Vec<&Dimension> {
        match self {
            Self::Tab {
                origin,
                length,
                width,
                thickness,
                overlap,
                ..
            } => {
                let mut out: Vec<_> = origin.iter().collect();
                out.extend([length, width, thickness, overlap]);
                out
            }
            Self::Bridge {
                start,
                end,
                width,
                thickness,
                ..
            } => {
                let mut out: Vec<_> = start.iter().chain(end).collect();
                out.extend([width, thickness]);
                out
            }
            Self::Support {
                origin,
                length,
                width,
                base,
                gap,
                ..
            } => {
                let mut out: Vec<_> = origin.iter().collect();
                out.extend([length, width, base, gap]);
                out
            }
        }
    }
    fn angle(&self) -> Option<&Dimension> {
        match self {
            Self::Tab { angle, .. } | Self::Support { angle, .. } => Some(angle),
            Self::Bridge { .. } => None,
        }
    }
    pub fn other(&self) -> Option<EntityId> {
        match self {
            Self::Bridge { other, .. } => *other,
            _ => None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AidDefinition {
    pub output: BodyOutput,
    /// The finished Part this aid serves. Its solid is never changed.
    pub target: EntityId,
    pub shape: AidShape,
    /// Process allowances the aid is inspected against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<EntityId>,
}
/// Explicit FDM process allowances. Nothing here is a slicer setting; the
/// values only drive the inspection of aids against their source geometry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessDefinition {
    pub layer_height: Dimension,
    pub line_width: Dimension,
    /// Minimum tab and bridge thickness, in layers.
    pub min_layers: u32,
    /// Minimum tab, bridge and support width, in extrusion lines.
    pub min_lines: u32,
    /// Smallest Z gap between a support top and the part above it.
    pub support_gap: Dimension,
    /// Smallest clearance between an aid and any Part it does not serve.
    pub fit_clearance: Dimension,
}
impl Default for ProcessDefinition {
    fn default() -> Self {
        use super::length;
        Self {
            layer_height: length(0.2),
            line_width: length(0.4),
            min_layers: 2,
            min_lines: 2,
            support_gap: length(0.2),
            fit_clearance: length(0.3),
        }
    }
}

/// An aid shape resolved to millimeters and radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResolvedAid {
    Tab {
        origin: [f64; 3],
        angle_rad: f64,
        length_mm: f64,
        width_mm: f64,
        thickness_mm: f64,
        overlap_mm: f64,
    },
    Bridge {
        start: [f64; 3],
        end: [f64; 3],
        width_mm: f64,
        thickness_mm: f64,
    },
    Support {
        origin: [f64; 2],
        angle_rad: f64,
        length_mm: f64,
        width_mm: f64,
        base_mm: f64,
        gap_mm: f64,
    },
}
impl ResolvedAid {
    /// The aid's thin dimension checked against the minimum layers.
    pub fn thickness_mm(&self) -> Option<f64> {
        match self {
            Self::Tab { thickness_mm, .. } | Self::Bridge { thickness_mm, .. } => {
                Some(*thickness_mm)
            }
            Self::Support { .. } => None,
        }
    }
    /// The narrowest authored width, checked against the minimum lines.
    pub fn width_mm(&self) -> f64 {
        match self {
            Self::Tab { width_mm, .. } | Self::Bridge { width_mm, .. } => *width_mm,
            Self::Support {
                length_mm,
                width_mm,
                ..
            } => length_mm.min(*width_mm),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedProcess {
    pub layer_height_mm: f64,
    pub line_width_mm: f64,
    pub min_layers: u32,
    pub min_lines: u32,
    pub support_gap_mm: f64,
    pub fit_clearance_mm: f64,
}
impl ResolvedProcess {
    pub fn min_thickness_mm(&self) -> f64 {
        self.layer_height_mm * f64::from(self.min_layers)
    }
    pub fn min_width_mm(&self) -> f64 {
        self.line_width_mm * f64::from(self.min_lines)
    }
}

fn definition<T: Serialize>(type_id: &str, data: &T) -> Result<Definition, DocumentError> {
    Ok(Definition {
        type_id: type_id.into(),
        schema_version: 1,
        data: serde_json::to_value(data).map_err(|e| invalid(e.to_string()))?,
    })
}
fn parse<T: for<'a> Deserialize<'a>>(d: &Definition, type_id: &str) -> Result<T, DocumentError> {
    if d.type_id != type_id || d.schema_version != 1 {
        return Err(invalid(format!("Expected {type_id} schema 1")));
    }
    serde_json::from_value(d.data.clone()).map_err(|e| invalid(e.to_string()))
}
fn mm(d: &Dimension, p: &EvaluatedParameters) -> Result<f64, DocumentError> {
    number(d, Unit::Millimeter, p)
}
fn xyz(d: &[Dimension; 3], p: &EvaluatedParameters) -> Result<[f64; 3], DocumentError> {
    Ok([mm(&d[0], p)?, mm(&d[1], p)?, mm(&d[2], p)?])
}
fn positive(value: f64, what: &str) -> Result<f64, DocumentError> {
    if value > 0. {
        Ok(value)
    } else {
        Err(invalid(format!("The {what} must be positive")))
    }
}

impl AidDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.output.body.is_nil() || self.target.is_nil() {
            return Err(invalid("An FDM aid needs an output Part and a target Part"));
        }
        if self.output.body == self.target {
            return Err(invalid("An FDM aid cannot be its own target"));
        }
        if let Some(other) = self.shape.other()
            && (other.is_nil() || other == self.target || other == self.output.body)
        {
            return Err(invalid(
                "A bridge's second Part must differ from the aid and its target",
            ));
        }
        if self.process.is_some_and(|p| p.is_nil()) {
            return Err(invalid("FDM process identity is nil"));
        }
        for d in self.shape.lengths() {
            check_dimension(d, Unit::Millimeter)?;
        }
        if let Some(a) = self.shape.angle() {
            check_dimension(a, Unit::Degree)?;
        }
        Ok(())
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        definition(AID_TYPE, self)
    }
    pub fn from_definition(d: &Definition) -> Result<Self, DocumentError> {
        let value: Self = parse(d, AID_TYPE)?;
        value.validate()?;
        Ok(value)
    }
    pub fn inputs(&self) -> Vec<Link> {
        let mut dims = self.shape.lengths();
        dims.extend(self.shape.angle());
        let mut links = parameter_links(dims);
        links.push(Link::new(self.target, "target"));
        if let Some(other) = self.shape.other() {
            links.push(Link::new(other, "other"));
        }
        if let Some(process) = self.process {
            links.push(Link::new(process, "process"));
        }
        links
    }
    /// Resolve dimensions; refuses empty or degenerate aids before any
    /// native geometry is built.
    pub fn resolve(&self, p: &EvaluatedParameters) -> Result<ResolvedAid, DocumentError> {
        let angle = |d: &Dimension| number(d, Unit::Radian, p);
        Ok(match &self.shape {
            AidShape::Tab {
                origin,
                angle: a,
                length,
                width,
                thickness,
                overlap,
            } => {
                let overlap_mm = mm(overlap, p)?;
                if overlap_mm < 0. {
                    return Err(invalid("The tab overlap must not be negative"));
                }
                ResolvedAid::Tab {
                    origin: xyz(origin, p)?,
                    angle_rad: angle(a)?,
                    length_mm: positive(mm(length, p)?, "tab length")?,
                    width_mm: positive(mm(width, p)?, "tab width")?,
                    thickness_mm: positive(mm(thickness, p)?, "tab thickness")?,
                    overlap_mm,
                }
            }
            AidShape::Bridge {
                start,
                end,
                width,
                thickness,
                ..
            } => {
                let (start, end) = (xyz(start, p)?, xyz(end, p)?);
                let d: Vec<f64> = (0..3).map(|i| end[i] - start[i]).collect();
                let span = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if span <= 1e-6 {
                    return Err(invalid("A bridge needs distinct start and end points"));
                }
                if (d[0] * d[0] + d[1] * d[1]).sqrt() <= 1e-6 * span {
                    return Err(invalid(
                        "A bridge cannot run vertically; it must span horizontally",
                    ));
                }
                ResolvedAid::Bridge {
                    start,
                    end,
                    width_mm: positive(mm(width, p)?, "bridge width")?,
                    thickness_mm: positive(mm(thickness, p)?, "bridge thickness")?,
                }
            }
            AidShape::Support {
                origin,
                angle: a,
                length,
                width,
                base,
                gap,
            } => {
                let gap_mm = mm(gap, p)?;
                if gap_mm < 0. {
                    return Err(invalid("The support gap must not be negative"));
                }
                ResolvedAid::Support {
                    origin: [mm(&origin[0], p)?, mm(&origin[1], p)?],
                    angle_rad: angle(a)?,
                    length_mm: positive(mm(length, p)?, "support length")?,
                    width_mm: positive(mm(width, p)?, "support width")?,
                    base_mm: mm(base, p)?,
                    gap_mm,
                }
            }
        })
    }
}
impl ProcessDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        for d in self.dimensions() {
            check_dimension(d, Unit::Millimeter)?;
        }
        if !(1..=1000).contains(&self.min_layers) || !(1..=1000).contains(&self.min_lines) {
            return Err(invalid(
                "Minimum layers and lines must each be between 1 and 1000",
            ));
        }
        Ok(())
    }
    fn dimensions(&self) -> Vec<&Dimension> {
        vec![
            &self.layer_height,
            &self.line_width,
            &self.support_gap,
            &self.fit_clearance,
        ]
    }
    pub fn resolve(&self, p: &EvaluatedParameters) -> Result<ResolvedProcess, DocumentError> {
        let support_gap_mm = mm(&self.support_gap, p)?;
        let fit_clearance_mm = mm(&self.fit_clearance, p)?;
        if support_gap_mm < 0. || fit_clearance_mm < 0. {
            return Err(invalid(
                "Process gap and clearance allowances must not be negative",
            ));
        }
        Ok(ResolvedProcess {
            layer_height_mm: positive(mm(&self.layer_height, p)?, "layer height")?,
            line_width_mm: positive(mm(&self.line_width, p)?, "line width")?,
            min_layers: self.min_layers,
            min_lines: self.min_lines,
            support_gap_mm,
            fit_clearance_mm,
        })
    }
    pub(crate) fn entity_data(&self) -> Result<EntityData, DocumentError> {
        self.validate()?;
        Ok(EntityData::Extension {
            definition: definition(PROCESS_KIND, self)?,
            inputs: parameter_links(self.dimensions()),
        })
    }
}

fn is_aid_feature(entity: Option<&Entity>) -> bool {
    matches!(entity.map(|e| &e.data), Some(EntityData::Feature { definition, .. }) if definition.type_id == AID_TYPE)
}
pub fn aids(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(|e| is_aid_feature(Some(e)))
}
pub fn processes(document: &Document) -> impl Iterator<Item = &Entity> {
    document.entities().filter(
        |e| matches!(&e.data, EntityData::Extension { definition, .. } if definition.type_id == PROCESS_KIND),
    )
}
/// The aid feature producing a Part, if the Part is a removable FDM aid.
pub fn aid_feature(document: &Document, part: EntityId) -> Option<EntityId> {
    let Some(EntityData::Part { features, .. }) = document.get(part).map(|e| &e.data) else {
        return None;
    };
    features
        .iter()
        .copied()
        .find(|f| is_aid_feature(document.get(*f)))
}
/// Every Part classified as a removable FDM aid by its producing feature.
/// Names and tags never classify a Part.
pub fn aid_parts(document: &Document) -> BTreeSet<EntityId> {
    document
        .entities()
        .filter(|e| aid_feature(document, e.id).is_some())
        .map(|e| e.id)
        .collect()
}
pub fn read_aid(document: &Document, feature: EntityId) -> Result<AidDefinition, DocumentError> {
    let Some(EntityData::Feature {
        definition, inputs, ..
    }) = document.get(feature).map(|e| &e.data)
    else {
        return Err(invalid("Choose an FDM aid feature"));
    };
    let aid = AidDefinition::from_definition(definition)?;
    let expected = aid.inputs();
    if inputs.len() != expected.len() || expected.iter().any(|l| !inputs.contains(l)) {
        return Err(invalid("FDM aid source and canonical input links disagree"));
    }
    if let Some(entity) = document.get(aid.output.body) {
        let EntityData::Part { features, .. } = &entity.data else {
            return Err(invalid("FDM aid output must identify a Part"));
        };
        if !features.contains(&feature) {
            return Err(invalid("FDM aid output Part must contain its aid feature"));
        }
    }
    Ok(aid)
}
pub fn read_process(document: &Document, id: EntityId) -> Result<ProcessDefinition, DocumentError> {
    match document.get(id).map(|e| &e.data) {
        Some(EntityData::Extension { definition, .. }) if definition.type_id == PROCESS_KIND => {
            let value: ProcessDefinition = parse(definition, PROCESS_KIND)?;
            value.validate()?;
            Ok(value)
        }
        Some(_) => Err(invalid(format!("{id} is not an FDM process"))),
        None => Err(DocumentError::Missing(id)),
    }
}
fn require_part(document: &Document, id: EntityId, what: &str) -> Result<(), DocumentError> {
    match document.get(id).map(|e| &e.data) {
        Some(EntityData::Part { .. }) => Ok(()),
        Some(_) => Err(invalid(format!("The {what} must be a Part"))),
        None => Err(DocumentError::Missing(id)),
    }
}
pub(super) fn validate_document(document: &Document) -> Result<(), DocumentError> {
    for e in processes(document) {
        read_process(document, e.id)?;
    }
    for e in aids(document) {
        let aid = read_aid(document, e.id)?;
        // A removed target leaves a dangling link, reported by the document.
        if document.get(aid.target).is_some() {
            require_part(document, aid.target, "aid target")?;
            if aid_feature(document, aid.target).is_some() {
                return Err(invalid("An FDM aid cannot serve another aid"));
            }
        }
    }
    Ok(())
}

fn check_name(name: &str, what: &str) -> Result<(), CommandError> {
    if name.trim().is_empty() || name.len() > 1000 {
        return Err(error(format!("{what} name must contain 1–1000 bytes")));
    }
    Ok(())
}
fn check_references(document: &Document, aid: &AidDefinition) -> Result<(), CommandError> {
    require_part(document, aid.target, "aid target")?;
    if aid_feature(document, aid.target).is_some() {
        return Err(error(
            "Choose a finished Part as the target, not another aid",
        ));
    }
    if let Some(other) = aid.shape.other() {
        require_part(document, other, "bridge's second Part")?;
    }
    if let Some(process) = aid.process {
        read_process(document, process)?;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct AidRequest {
    pub feature: EntityId,
    pub body: EntityId,
    pub name: String,
    pub aid: AidDefinition,
    /// The aid Part's material; by default the target Part's material.
    pub material: Option<EntityId>,
}
impl AidRequest {
    pub fn new(name: impl Into<String>, mut aid: AidDefinition) -> Self {
        let body = EntityId::new();
        aid.output.body = body;
        Self {
            feature: EntityId::new(),
            body,
            name: name.into(),
            aid,
            material: None,
        }
    }
}
/// Create one aid feature and its own Part. The target Part is untouched.
pub fn prepare_aid(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &AidRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    check_name(&request.name, "Aid")?;
    let document = session.document();
    if request.aid.output.body != request.body {
        return Err(error("Aid output must name the requested Part"));
    }
    for id in [request.feature, request.body] {
        if id.is_nil() || document.get(id).is_some() || request.feature == request.body {
            return Err(error("Aid identities must be new, distinct and non-nil"));
        }
    }
    check_references(document, &request.aid)?;
    request.aid.resolve(session.parameters())?;
    let material = match request.material {
        Some(m) => match document.get(m).map(|e| &e.data) {
            Some(EntityData::Material { .. }) => Some(m),
            _ => return Err(error("The aid material must be a Material")),
        },
        None => match document.get(request.aid.target).map(|e| &e.data) {
            Some(EntityData::Part { material, .. }) => *material,
            _ => None,
        },
    };
    let mut feature = Entity::new(
        format!("{} feature", request.name),
        EntityData::Feature {
            definition: request.aid.definition()?,
            inputs: request.aid.inputs(),
            suppressed: false,
        },
    );
    feature.id = request.feature;
    let mut body = Entity::new(
        request.name.clone(),
        EntityData::Part {
            features: vec![request.feature],
            material,
            tags: BTreeSet::new(),
        },
    );
    body.id = request.body;
    let prepared = session.prepare_scoped(Request {
        expected_revision,
        actor,
        label: format!("Create {}", request.aid.shape.kind().label()),
        edits: vec![Mutation::Insert(feature), Mutation::Insert(body)],
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![CreatedBody {
            feature: request.feature,
            body: request.body,
        }],
    })
}
/// Edit an aid's shape, target or process; the aid keeps its feature and
/// Part identities.
pub fn prepare_aid_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    aid: &AidDefinition,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    let previous = read_aid(document, feature)?;
    if previous.output != aid.output {
        return Err(error("An aid edit keeps its output Part"));
    }
    check_references(document, aid)?;
    aid.resolve(session.parameters())?;
    let mut entity = document
        .get(feature)
        .cloned()
        .ok_or_else(|| error("Aid is missing"))?;
    entity.data = EntityData::Feature {
        definition: aid.definition()?,
        inputs: aid.inputs(),
        suppressed: false,
    };
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: format!("Edit {}", aid.shape.kind().label()),
        edits: vec![Mutation::Replace(entity)],
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![CreatedBody {
            feature,
            body: aid.output.body,
        }],
    })
}
/// Remove an aid feature and its Part, detaching it from any group. The
/// target Part is untouched.
pub fn prepare_aid_remove(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
) -> Result<Prepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    let aid = read_aid(document, feature)?;
    let body = aid.output.body;
    if let Some(EntityData::Part { features, .. }) = document.get(body).map(|e| &e.data)
        && features.len() > 1
    {
        return Err(error(
            "The aid Part carries later features; remove those first",
        ));
    }
    let mut edits = vec![Mutation::Remove(body), Mutation::Remove(feature)];
    let removed = [body, feature];
    let mut state = document.hierarchy().clone();
    for members in state.members.values_mut() {
        members.retain(|m| !removed.contains(m));
    }
    if &state != document.hierarchy() {
        edits.insert(0, Mutation::SetHierarchy(state));
    }
    session.prepare(Request {
        expected_revision,
        actor,
        label: format!("Remove {}", aid.shape.kind().label()),
        edits,
    })
}
/// Create or replace an FDM process allowance record.
pub fn prepare_process(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: Option<EntityId>,
    name: &str,
    process: &ProcessDefinition,
) -> Result<(EntityId, Prepared), CommandError> {
    revision(session, expected_revision)?;
    check_name(name, "Process")?;
    process.resolve(session.parameters())?;
    let document = session.document();
    let (id, mutation) = match id {
        Some(id) => {
            read_process(document, id)?;
            let mut entity = document.get(id).cloned().ok_or_else(|| error("Missing"))?;
            entity.name = name.into();
            entity.data = process.entity_data()?;
            (id, Mutation::Replace(entity))
        }
        None => {
            let entity = Entity::new(name, process.entity_data()?);
            (entity.id, Mutation::Insert(entity))
        }
    };
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Set FDM process allowances".into(),
        edits: vec![mutation],
    })?;
    Ok((id, prepared))
}

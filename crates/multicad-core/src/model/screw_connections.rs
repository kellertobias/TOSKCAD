//! One canonical connection owns references and role identities. Hardware and
//! receiving geometry remain their ordinary typed providers, not copied recipes.
use super::{
    common::*,
    source::{check_dimension, invalid},
    *,
};
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    constraint_sketch::Dimension,
    document::{
        Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation,
        Placement,
    },
    parameters::{EvaluatedParameters, Unit},
    symbols::{self, SymbolDefinition, SymbolPlaceRequest},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub const SCREW_CONNECTION_TYPE: &str = "model.screw_connection";
pub const SCREW_CONNECTION_CAPABILITY: &str = "model.screw_connection.1";
pub const SCREW_CONNECTION_GRIP_CAPABILITY: &str = "model.screw_connection.face_grip.1";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrewConnectionCheck {
    pub connection: EntityId,
    pub mode: ScrewConnectionMode,
    pub hardware_instances: Vec<EntityId>,
    pub receiver_features: Vec<EntityId>,
    pub grip_mm: f64,
    pub minimum_engagement_mm: f64,
    pub blind_tip_clearance_mm: f64,
    pub actual_engagement_mm: Option<f64>,
    #[serde(default)]
    pub actual_wood_engagement_mm: Option<f64>,
    pub native_component_collision_checked: bool,
    pub scope: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrewConnectionMode {
    ThroughNut,
    TappedReceiver,
    DirectWood,
    WoodInsert,
    HangerBolt,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrewConnectionHardwareRole {
    Screw,
    HangerBolt,
    HeadWasher,
    NutWasher,
    Nut,
    Insert,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrewConnectionAxisRequest {
    pub id: EntityId,
    pub source: ScrewConnectionAxisDefinition,
    pub face: FaceInput,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScrewConnectionHardwareSource {
    Generated {
        part: Box<MechanicalPartRequest>,
    },
    Library {
        definition: Box<SymbolDefinition>,
        placement: Box<SymbolPlaceRequest>,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrewConnectionHardwareRequest {
    pub role_id: EntityId,
    pub role: ScrewConnectionHardwareRole,
    pub name: String,
    pub source: ScrewConnectionHardwareSource,
    pub instance: EntityId,
    pub driver: EntityId,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrewConnectionEndReference {
    pub receiver: BodyInput,
    pub occurrence: OccurrencePath,
    pub face: FaceInput,
}
impl ScrewConnectionEndReference {
    pub fn axis(&self) -> ScrewConnectionAxisDefinition {
        ScrewConnectionAxisDefinition::new(
            self.receiver.clone(),
            self.occurrence.clone(),
            self.face.intent.id,
        )
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.axis().inputs(&self.face).map(|links| {
            links
                .into_iter()
                .map(|mut link| {
                    link.role = format!("connection/end/{}", link.role);
                    link
                })
                .collect()
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrewConnectionParameters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_reference: Option<ScrewConnectionEndReference>,
    pub grip: Dimension,
    pub minimum_engagement: Dimension,
    pub blind_tip_clearance: Dimension,
}
impl Default for ScrewConnectionParameters {
    fn default() -> Self {
        Self {
            end_reference: None,
            grip: length(10.),
            minimum_engagement: length(2.),
            blind_tip_clearance: length(1.),
        }
    }
}
impl ScrewConnectionParameters {
    fn dimensions(&self) -> Vec<&Dimension> {
        vec![
            &self.grip,
            &self.minimum_engagement,
            &self.blind_tip_clearance,
        ]
    }
    fn validate(&self) -> Result<(), DocumentError> {
        if let Some(end) = &self.end_reference {
            end.inputs()?;
            if self.grip != length(0.) {
                return Err(invalid(
                    "A captured grip end face requires zero manual grip; the native face is authoritative",
                ));
            }
        }
        for d in self.dimensions() {
            check_dimension(d, Unit::Millimeter)?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrewConnectionRequest {
    pub connection: EntityId,
    pub name: String,
    pub mode: ScrewConnectionMode,
    pub axis: ScrewConnectionAxisRequest,
    pub hardware: Vec<ScrewConnectionHardwareRequest>,
    pub receivers: Vec<ConnectionReceiverRequest>,
    pub parameters: ScrewConnectionParameters,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScrewConnectionHardwareBinding {
    Generated { feature: EntityId, body: EntityId },
    Library { record: EntityId },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrewConnectionRoleBinding {
    pub role_id: EntityId,
    pub role: ScrewConnectionHardwareRole,
    pub name: String,
    pub source: ScrewConnectionHardwareBinding,
    pub instance: EntityId,
    pub driver: EntityId,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrewConnectionDefinition {
    pub name: String,
    pub mode: ScrewConnectionMode,
    pub axis: EntityId,
    pub hardware: Vec<ScrewConnectionRoleBinding>,
    pub receivers: Vec<EntityId>,
    pub parameters: ScrewConnectionParameters,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrewConnectionRemoveRequest {
    pub connection: EntityId,
}
impl ScrewConnectionDefinition {
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.parameters.validate()?;
        if self.name.trim().is_empty()
            || self.name.len() > 1000
            || self.axis.is_nil()
            || self.hardware.is_empty()
            || self.hardware.len() > 64
            || self.receivers.is_empty()
            || self.receivers.len() > 16
        {
            return Err(invalid(
                "A connection needs a name, axis, 1–64 hardware roles and 1–16 receivers",
            ));
        }
        let mut ids = BTreeSet::from([self.axis]);
        for row in &self.hardware {
            for id in [row.role_id, row.instance, row.driver] {
                if id.is_nil() || !ids.insert(id) {
                    return Err(invalid(
                        "Connection role identities must be distinct and non-nil",
                    ));
                }
            }
            let source_ids = match row.source {
                ScrewConnectionHardwareBinding::Generated { feature, body } => vec![feature, body],
                ScrewConnectionHardwareBinding::Library { record } => vec![record],
            };
            for id in source_ids {
                if id.is_nil() || !ids.insert(id) {
                    return Err(invalid(
                        "Each hardware role requires its own canonical source identities",
                    ));
                }
            }
        }
        for id in &self.receivers {
            if id.is_nil() || !ids.insert(*id) {
                return Err(invalid(
                    "Connection receiver identities must be distinct and non-nil",
                ));
            }
        }
        Ok(Definition {
            type_id: SCREW_CONNECTION_TYPE.into(),
            schema_version: 1,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(d: &Definition) -> Result<Self, DocumentError> {
        if d.type_id != SCREW_CONNECTION_TYPE || d.schema_version != 1 {
            return Err(invalid("Unsupported screw connection provider/version"));
        }
        let s: Self = serde_json::from_value(d.data.clone()).map_err(|e| invalid(e.to_string()))?;
        s.definition()?;
        Ok(s)
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.definition()?;
        let mut links = parameter_links(self.parameters.dimensions());
        if let Some(end) = &self.parameters.end_reference {
            links.extend(end.inputs()?);
        }
        links.push(Link::new(self.axis, "connection/axis"));
        for row in &self.hardware {
            links.push(Link::new(
                row.instance,
                format!("connection/role/{}/instance", row.role_id),
            ));
            links.push(Link::new(
                row.driver,
                format!("connection/role/{}/driver", row.role_id),
            ));
            match row.source {
                ScrewConnectionHardwareBinding::Generated { feature, body } => {
                    links.push(Link::new(
                        feature,
                        format!("connection/role/{}/feature", row.role_id),
                    ));
                    links.push(Link::new(
                        body,
                        format!("connection/role/{}/body", row.role_id),
                    ));
                }
                ScrewConnectionHardwareBinding::Library { record } => links.push(Link::new(
                    record,
                    format!("connection/role/{}/symbol", row.role_id),
                )),
            }
        }
        for receiver in &self.receivers {
            links.push(Link::new(
                *receiver,
                format!("connection/receiver/{receiver}"),
            ));
        }
        Ok(links)
    }
}
pub fn read_screw_connection(
    document: &Document,
    id: EntityId,
) -> Result<ScrewConnectionDefinition, DocumentError> {
    let Some(EntityData::Extension { definition, inputs }) = document.get(id).map(|e| &e.data)
    else {
        return Err(invalid("Select a saved screw connection"));
    };
    let source = ScrewConnectionDefinition::from_definition(definition)?;
    let expected = source.inputs()?;
    if inputs.len() != expected.len() || expected.iter().any(|l| !inputs.contains(l)) {
        return Err(invalid("Connection source and dependency links disagree"));
    }
    Ok(source)
}
pub fn read_screw_connection_request(
    document: &Document,
    id: EntityId,
) -> Result<ScrewConnectionRequest, DocumentError> {
    let source = read_screw_connection(document, id)?;
    let (axis, face) = read_screw_connection_axis(document, source.axis)?;
    let hardware = source
        .hardware
        .iter()
        .map(|row| {
            let part_source = match row.source {
                ScrewConnectionHardwareBinding::Generated { feature, body } => {
                    let p = read_mechanical_part(document, feature)?;
                    if p.output.body != body {
                        return Err(invalid("Owned connection hardware changed its output body"));
                    }
                    ScrewConnectionHardwareSource::Generated {
                        part: Box::new(MechanicalPartRequest {
                            feature,
                            body,
                            name: document
                                .get(body)
                                .ok_or_else(|| invalid("Hardware feature missing"))?
                                .name
                                .clone(),
                            parameters: p.parameters,
                            provenance: p.provenance,
                        }),
                    }
                }
                ScrewConnectionHardwareBinding::Library { record } => {
                    let p = symbols::read_instance(document, record)?;
                    if p.instance != row.instance {
                        return Err(invalid(
                            "Connection Library binding belongs to another occurrence",
                        ));
                    }
                    let definition = symbols::read_definition(document, p.definition)?;
                    if definition.pin()? != p.pin {
                        return Err(invalid(
                            "Connection Library pin changed; repair its captured roles",
                        ));
                    }
                    let Some(EntityData::Instance { placement, .. }) =
                        document.get(row.instance).map(|e| &e.data)
                    else {
                        return Err(invalid("Connection hardware occurrence missing"));
                    };
                    ScrewConnectionHardwareSource::Library {
                        definition: Box::new(definition),
                        placement: Box::new(SymbolPlaceRequest {
                            record,
                            definition: p.definition,
                            instance: p.instance,
                            wrapper: p.wrapper,
                            wrapper_metadata: p.wrapper_metadata,
                            member: p.member,
                            ids: p.ids,
                            name: document
                                .get(row.instance)
                                .expect("checked occurrence")
                                .name
                                .clone(),
                            overrides: p.overrides,
                            placement: *placement,
                        }),
                    }
                }
            };
            Ok(ScrewConnectionHardwareRequest {
                role_id: row.role_id,
                role: row.role,
                name: row.name.clone(),
                source: part_source,
                instance: row.instance,
                driver: row.driver,
            })
        })
        .collect::<Result<Vec<_>, DocumentError>>()?;
    let receivers = source
        .receivers
        .iter()
        .map(|feature| {
            Ok(ConnectionReceiverRequest {
                feature: *feature,
                name: document
                    .get(*feature)
                    .ok_or_else(|| invalid("Connection receiver missing"))?
                    .name
                    .clone(),
                source: read_connection_receiver(document, *feature)?,
            })
        })
        .collect::<Result<Vec<_>, DocumentError>>()?;
    Ok(ScrewConnectionRequest {
        connection: id,
        name: source.name,
        mode: source.mode,
        axis: ScrewConnectionAxisRequest {
            id: source.axis,
            source: axis,
            face,
        },
        hardware,
        receivers,
        parameters: source.parameters,
    })
}
/// Exact native planar face intersection; stock continuity is proved separately
/// by the full native connection validator, never inferred from this distance.
pub fn resolve_screw_connection_grip(
    document: &Document,
    parameters: &EvaluatedParameters,
    request: &ScrewConnectionRequest,
    revision: u64,
    snapshots: &BTreeMap<EntityId, crate::references::TopologySnapshot>,
) -> Result<f64, DocumentError> {
    request.parameters.validate()?;
    let Some(end) = &request.parameters.end_reference else {
        return number(&request.parameters.grip, Unit::Millimeter, parameters);
    };
    validate_end_reference(request)?;
    let axis = request.axis.source.resolve(
        document,
        parameters,
        &request.axis.face,
        revision,
        snapshots,
    )?;
    let plane = end
        .axis()
        .resolve(document, parameters, &end.face, revision, snapshots)?;
    let normal = plane.vector([0., 0., 1.]);
    let direction = axis.vector([0., 0., 1.]);
    let denominator = normal
        .iter()
        .zip(direction)
        .map(|(a, b)| a * b)
        .sum::<f64>();
    if denominator.abs() < 1. - 1e-9 {
        return Err(invalid(
            "The captured grip end face must be parallel to the entry face",
        ));
    }
    let grip = (0..3)
        .map(|i| (plane.translation_mm[i] - axis.translation_mm[i]) * normal[i])
        .sum::<f64>()
        / denominator;
    if !grip.is_finite() || grip < -1e-7 {
        return Err(invalid(
            "The captured grip end face is behind the fastening entry",
        ));
    }
    Ok(grip.max(0.))
}
fn validate_end_reference(request: &ScrewConnectionRequest) -> Result<(), DocumentError> {
    if let Some(end) = &request.parameters.end_reference {
        end.inputs()?;
        if !request
            .receivers
            .iter()
            .any(|row| row.source.input == end.receiver && row.source.occurrence == end.occurrence)
        {
            return Err(invalid(
                "Capture the grip end on an exact selected receiving predecessor and occurrence",
            ));
        }
    }
    Ok(())
}
pub fn resolve_screw_connection_hardware(
    document: &Document,
    parameters: &EvaluatedParameters,
    row: &ScrewConnectionHardwareRequest,
) -> Result<ResolvedMechanicalPart, DocumentError> {
    match &row.source {
        ScrewConnectionHardwareSource::Generated { part } => {
            read_mechanical_part(document, part.feature)?
                .parameters
                .resolve(parameters)
        }
        ScrewConnectionHardwareSource::Library {
            definition,
            placement,
        } => {
            let part = placement
                .ids
                .get(&definition.root)
                .and_then(|id| document.get(*id))
                .ok_or_else(|| invalid("Library connection role lost its mapped root"))?;
            let EntityData::Part { features, .. } = &part.data else {
                return Err(invalid(
                    "A connection hardware Library role must be a single mechanical Part",
                ));
            };
            let feature = features
                .iter()
                .rev()
                .find(|id| {
                    matches!(
                        document.get(**id).map(|e| &e.data),
                        Some(EntityData::Feature {
                            suppressed: false,
                            ..
                        })
                    )
                })
                .ok_or_else(|| invalid("Hardware Library part has no active producer"))?;
            read_mechanical_part(document, *feature)?
                .parameters
                .resolve(parameters)
        }
    }
}

fn candidate_session(document: &Document) -> Result<Session, CommandError> {
    Session::new(document.clone())
}
fn stage(
    document: &mut Document,
    actor: &Actor,
    label: &str,
    edits: Vec<Mutation>,
) -> Result<(), CommandError> {
    let session = candidate_session(document)?;
    *document = session
        .prepare(Request {
            expected_revision: 0,
            actor: actor.clone(),
            label: label.into(),
            edits,
        })?
        .document()
        .clone();
    Ok(())
}
fn binding(row: &ScrewConnectionHardwareRequest) -> ScrewConnectionRoleBinding {
    ScrewConnectionRoleBinding {
        role_id: row.role_id,
        role: row.role,
        name: row.name.clone(),
        instance: row.instance,
        driver: row.driver,
        source: match &row.source {
            ScrewConnectionHardwareSource::Generated { part } => {
                ScrewConnectionHardwareBinding::Generated {
                    feature: part.feature,
                    body: part.body,
                }
            }
            ScrewConnectionHardwareSource::Library { placement, .. } => {
                ScrewConnectionHardwareBinding::Library {
                    record: placement.record,
                }
            }
        },
    }
}
fn owned(
    document: &Document,
    id: EntityId,
    source: &ScrewConnectionDefinition,
) -> Result<BTreeSet<EntityId>, DocumentError> {
    let mut ids = BTreeSet::from([id, source.axis]);
    ids.extend(source.receivers.iter().copied());
    for row in &source.hardware {
        ids.extend([row.instance, row.driver]);
        match row.source {
            ScrewConnectionHardwareBinding::Generated { feature, body } => {
                ids.extend([feature, body]);
            }
            ScrewConnectionHardwareBinding::Library { record } => {
                let s = symbols::read_instance(document, record)?;
                ids.extend([record, s.instance, s.wrapper, s.wrapper_metadata, s.member]);
                ids.extend(
                    s.ids
                        .values()
                        .copied()
                        .filter(|id| document.get(*id).is_some()),
                );
            }
        }
    }
    Ok(ids)
}
fn remove_owned(
    document: &mut Document,
    actor: &Actor,
    ids: &BTreeSet<EntityId>,
) -> Result<(), CommandError> {
    let mut edits = vec![];
    for entity in document.entities() {
        if ids.contains(&entity.id) {
            continue;
        }
        let mut replacement = entity.clone();
        if let EntityData::Part { features, .. } = &mut replacement.data {
            features.retain(|id| !ids.contains(id));
        }
        if replacement
            .references()
            .iter()
            .any(|link| ids.contains(&link.target))
        {
            return Err(error(
                "Another feature or occurrence uses connection-owned geometry; explicitly repair or remove that dependency first",
            ));
        }
        if replacement != *entity {
            edits.push(Mutation::Replace(replacement));
        }
    }
    edits.extend(
        ids.iter()
            .filter(|id| document.get(**id).is_some())
            .copied()
            .map(Mutation::Remove),
    );
    let mut hierarchy = document.hierarchy().clone();
    hierarchy.members.retain(|id, _| !ids.contains(id));
    for members in hierarchy.members.values_mut() {
        members.retain(|id| !ids.contains(id));
    }
    if hierarchy != *document.hierarchy() {
        edits.push(Mutation::SetHierarchy(hierarchy));
    }
    stage(document, actor, "Remove owned connection source", edits)
}
pub fn prepare_remove_screw_connection(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &ScrewConnectionRemoveRequest,
) -> Result<Prepared, CommandError> {
    revision(session, expected_revision)?;
    let source = read_screw_connection(session.document(), request.connection)?;
    let ids = owned(session.document(), request.connection, &source)?;
    let mut candidate = session.document().clone();
    remove_owned(&mut candidate, &actor, &ids)?;
    session.prepare(Request {
        expected_revision,
        actor,
        label: "Remove screw connection and owned hardware/holes".into(),
        edits: super::extrude_boolean::delta(session.document(), &candidate)?,
    })
}
pub(super) fn mount(part: &ResolvedMechanicalPart) -> RigidPose {
    if let ResolvedMechanicalPart::CustomConnection { parameters } = part {
        let frame = parameters.mount_frame();
        let normal = frame.normal();
        return RigidPose {
            rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], normal[i]]),
            translation_mm: frame.origin_mm,
        };
    }
    let frame = match part {
        ResolvedMechanicalPart::CustomConnection { .. } => unreachable!("handled custom mount"),
        ResolvedMechanicalPart::WoodScrew { parameters } => parameters.frame,
        ResolvedMechanicalPart::MachineScrew { thread, .. }
        | ResolvedMechanicalPart::HexNut { thread, .. } => thread.frame,
        ResolvedMechanicalPart::Washer { frame, .. }
        | ResolvedMechanicalPart::BearingEnvelope { frame, .. } => *frame,
    };
    RigidPose {
        rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], frame.n[i]]),
        translation_mm: frame.origin,
    }
}
fn washer_thickness(part: &ResolvedMechanicalPart) -> Result<f64, CommandError> {
    match part {
        ResolvedMechanicalPart::Washer { thickness_mm, .. } => Ok(*thickness_mm),
        _ => Err(error(
            "A washer role must select an actual parametric washer",
        )),
    }
}

pub fn prepare_screw_connection(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &ScrewConnectionRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    if request.connection.is_nil() || request.connection == request.axis.id {
        return Err(error(
            "Connection and axis identities must be distinct and non-nil",
        ));
    }
    request.parameters.validate()?;
    validate_end_reference(request)?;
    let definition = ScrewConnectionDefinition {
        name: request.name.clone(),
        mode: request.mode,
        axis: request.axis.id,
        hardware: request.hardware.iter().map(binding).collect(),
        receivers: request.receivers.iter().map(|r| r.feature).collect(),
        parameters: request.parameters.clone(),
    };
    definition.definition()?;
    let old = if session.document().get(request.connection).is_some() {
        Some(read_screw_connection(
            session.document(),
            request.connection,
        )?)
    } else {
        None
    };
    if old.as_ref().is_some_and(|s| s.axis != request.axis.id) {
        return Err(error("Connection edits retain the original axis identity"));
    }
    let old_owned = old
        .as_ref()
        .map(|s| owned(session.document(), request.connection, s))
        .transpose()?
        .unwrap_or_default();
    let mut candidate = session.document().clone();
    if old.is_some() {
        stage(
            &mut candidate,
            &actor,
            "Detach connection ownership during composition",
            vec![Mutation::Remove(request.connection)],
        )?;
    }
    let axis = Entity {
        description: String::new(),
        id: request.axis.id,
        name: format!("{} axis", request.name),
        data: EntityData::Extension {
            definition: request.axis.source.definition()?,
            inputs: request.axis.source.inputs(&request.axis.face)?,
        },
    };
    if candidate.get(axis.id).is_some() && !old_owned.contains(&axis.id) {
        return Err(error(
            "Axis belongs to another source; choose a new connection identity",
        ));
    }
    let edit = if candidate.get(axis.id).is_some() {
        Mutation::Replace(axis)
    } else {
        Mutation::Insert(axis)
    };
    stage(
        &mut candidate,
        &actor,
        "Compose connection axis",
        vec![edit],
    )?;
    let mut outputs = vec![];
    for row in &request.hardware {
        let scratch = candidate_session(&candidate)?;
        match &row.source {
            ScrewConnectionHardwareSource::Generated { part } => {
                let result = if candidate.get(part.feature).is_some() {
                    if !old_owned.contains(&part.feature) || !old_owned.contains(&part.body) {
                        return Err(error(
                            "Generated hardware identity belongs to unrelated geometry",
                        ));
                    }
                    let old = read_mechanical_part(&candidate, part.feature)?;
                    if old.output.body != part.body || old.provenance != part.provenance {
                        return Err(error(
                            "Hardware edits retain their original body and immutable reference provenance",
                        ));
                    }
                    prepare_mechanical_part_edit(
                        &scratch,
                        0,
                        actor.clone(),
                        part.feature,
                        &part.parameters,
                    )?
                } else {
                    prepare_mechanical_part(&scratch, 0, actor.clone(), part)?
                };
                outputs.extend(result.outputs);
                candidate = result.prepared.document().clone();
                let mut renames = vec![];
                for (id, name) in [
                    (part.body, part.name.clone()),
                    (part.feature, format!("{} feature", part.name)),
                ] {
                    let entity = candidate
                        .get(id)
                        .ok_or_else(|| error("Generated hardware source is missing"))?;
                    if entity.name != name {
                        let mut entity = entity.clone();
                        entity.name = name;
                        renames.push(Mutation::Replace(entity));
                    }
                }
                if !renames.is_empty() {
                    stage(
                        &mut candidate,
                        &actor,
                        "Name generated connection hardware",
                        renames,
                    )?;
                }
            }
            ScrewConnectionHardwareSource::Library {
                definition,
                placement,
            } => {
                if placement.instance != row.instance {
                    return Err(error(
                        "Library hardware role and placement occurrence disagree",
                    ));
                }
                let result = if candidate.get(placement.record).is_some() {
                    if !old_owned.contains(&placement.record) {
                        return Err(error("Library occurrence belongs to another source"));
                    }
                    let existing = symbols::read_instance(&candidate, placement.record)?;
                    if existing.definition != placement.definition
                        || existing.pin != definition.pin()?
                        || existing.ids != placement.ids
                    {
                        return Err(error(
                            "Connection Library pin or private identities changed; explicitly repair the role",
                        ));
                    }
                    symbols::prepare_edit(
                        &scratch,
                        0,
                        actor.clone(),
                        &symbols::SymbolEditRequest {
                            record: placement.record,
                            overrides: placement.overrides.clone(),
                        },
                    )?
                } else {
                    symbols::prepare_library_place(
                        &scratch,
                        0,
                        actor.clone(),
                        definition,
                        placement,
                        None,
                    )?
                };
                candidate = result.prepared.document().clone();
            }
        }
    }
    let scratch = candidate_session(&candidate)?;
    let grip = number(
        &request.parameters.grip,
        Unit::Millimeter,
        scratch.parameters(),
    )?;
    let minimum = number(
        &request.parameters.minimum_engagement,
        Unit::Millimeter,
        scratch.parameters(),
    )?;
    let clearance = number(
        &request.parameters.blind_tip_clearance,
        Unit::Millimeter,
        scratch.parameters(),
    )?;
    if grip < 0. || minimum <= 0. || clearance < 0. {
        return Err(error(
            "Grip and blind clearance must be nonnegative; required engagement must be positive",
        ));
    }
    let resolved = request
        .hardware
        .iter()
        .map(|row| resolve_screw_connection_hardware(&candidate, scratch.parameters(), row))
        .collect::<Result<Vec<_>, _>>()?;
    let head_sum = request
        .hardware
        .iter()
        .zip(&resolved)
        .filter(|(r, _)| r.role == ScrewConnectionHardwareRole::HeadWasher)
        .map(|(_, p)| washer_thickness(p))
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .sum::<f64>();
    let nut_sum = request
        .hardware
        .iter()
        .zip(&resolved)
        .filter(|(r, _)| r.role == ScrewConnectionHardwareRole::NutWasher)
        .map(|(_, p)| washer_thickness(p))
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .sum::<f64>();
    let screw = request
        .hardware
        .iter()
        .zip(&resolved)
        .find(|(r, _)| {
            matches!(
                r.role,
                ScrewConnectionHardwareRole::Screw | ScrewConnectionHardwareRole::HangerBolt
            )
        })
        .ok_or_else(|| error("Select exactly one screw or hanger bolt"))?;
    let bearing_station = if request.mode == ScrewConnectionMode::HangerBolt {
        if request
            .receivers
            .iter()
            .any(|row| row.source.seat.is_some())
        {
            return Err(error(
                "Headless hanger connections do not support a screw-head seat",
            ));
        }
        -head_sum
    } else {
        resolve_screw_connection_head_station(request, scratch.parameters(), screw.1, head_sum)?
    };
    let screw_data = match screw.1 {
        ResolvedMechanicalPart::MachineScrew {
            underhead_length_mm,
            thread,
            ..
        } => Some((
            *underhead_length_mm - thread.length_mm + bearing_station,
            thread.pitch_mm,
            thread.hand,
        )),
        ResolvedMechanicalPart::CustomConnection { parameters } => match parameters.as_ref() {
            ResolvedCustomConnectionHardware::HangerBolt {
                metric_thread,
                plain_length_mm,
                ..
            } => Some((
                grip - plain_length_mm - metric_thread.length_mm,
                metric_thread.pitch_mm,
                metric_thread.hand,
            )),
            _ => None,
        },
        _ => None,
    };
    let mut screw_roll = 0.;
    if request.mode == ScrewConnectionMode::TappedReceiver {
        let receiver = request
            .receivers
            .iter()
            .find(|r| matches!(r.source.cut, ConnectionReceiverCut::Tapped { .. }))
            .ok_or_else(|| error("Tapped mode requires an actual threaded receiver"))?;
        if let Some((start, pitch, hand)) = screw_data {
            let sign = if hand == ThreadHand::Right { 1. } else { -1. };
            screw_roll = sign
                * std::f64::consts::TAU
                * (start
                    - number(
                        &receiver.source.station,
                        Unit::Millimeter,
                        scratch.parameters(),
                    )?)
                / pitch;
        }
    }
    let mut head_station = bearing_station;
    let mut nut_station = if request.mode == ScrewConnectionMode::HangerBolt {
        -nut_sum
    } else {
        grip
    };
    for (row, part) in request.hardware.iter().zip(&resolved) {
        let (station, roll) = match row.role {
            ScrewConnectionHardwareRole::HeadWasher => {
                let s = head_station;
                head_station += washer_thickness(part)?;
                (s, 0.)
            }
            ScrewConnectionHardwareRole::NutWasher => {
                let s = nut_station;
                nut_station += washer_thickness(part)?;
                (s, 0.)
            }
            ScrewConnectionHardwareRole::Nut => {
                let station = if request.mode == ScrewConnectionMode::HangerBolt {
                    let ResolvedMechanicalPart::HexNut { thread, .. } = part else {
                        return Err(error("A hanger bolt needs an actual hex nut"));
                    };
                    -nut_sum - thread.length_mm
                } else {
                    grip + nut_sum
                };
                let roll = screw_data.map_or(0., |(start, pitch, hand)| {
                    screw_roll
                        + if hand == ThreadHand::Right { 1. } else { -1. }
                            * std::f64::consts::TAU
                            * (station - start)
                            / pitch
                });
                (station, roll)
            }
            ScrewConnectionHardwareRole::Screw => (bearing_station, screw_roll),
            ScrewConnectionHardwareRole::HangerBolt => (grip, 0.),
            ScrewConnectionHardwareRole::Insert => (
                grip,
                screw_data.map_or(0., |(start, pitch, hand)| {
                    let sign = if hand == ThreadHand::Right { 1. } else { -1. };
                    screw_roll + sign * std::f64::consts::TAU * (grip - start) / pitch
                }),
            ),
        };
        let source = ScrewHardwarePlacementDefinition {
            owner: row.instance,
            axis: request.axis.id,
            connection: None,
            station: length(station),
            roll: angle(roll.to_degrees()),
            local_mount: mount(part),
        };
        let driver = Entity {
            description: String::new(),
            id: row.driver,
            name: format!("{} connection placement", row.name),
            data: EntityData::Extension {
                definition: source.definition()?,
                inputs: source.inputs()?,
            },
        };
        let mut edits = vec![if candidate.get(row.driver).is_some() {
            if !old_owned.contains(&row.driver) {
                return Err(error("Hardware driver belongs to unrelated source"));
            }
            Mutation::Replace(driver)
        } else {
            Mutation::Insert(driver)
        }];
        let mut instance = if let Some(instance) = candidate.get(row.instance) {
            if !old_owned.contains(&row.instance)
                && !matches!(row.source, ScrewConnectionHardwareSource::Library { .. })
            {
                return Err(error("Hardware occurrence belongs to unrelated source"));
            }
            instance.clone()
        } else {
            let ScrewConnectionHardwareSource::Generated { part } = &row.source else {
                return Err(error(
                    "Library placement did not produce its hardware occurrence",
                ));
            };
            Entity {
                description: String::new(),
                id: row.instance,
                name: row.name.clone(),
                data: EntityData::Instance {
                    component: part.body,
                    placement: Placement::default(),
                    placement_source: None,
                },
            }
        };
        let EntityData::Instance {
            component,
            placement_source,
            ..
        } = &mut instance.data
        else {
            return Err(error("Hardware role needs an Instance"));
        };
        if let ScrewConnectionHardwareSource::Generated { part } = &row.source {
            *component = part.body;
        }
        *placement_source = Some(row.driver);
        instance.name = match &row.source {
            ScrewConnectionHardwareSource::Generated { .. } => row.name.clone(),
            ScrewConnectionHardwareSource::Library { placement, .. } => placement.name.clone(),
        };
        edits.push(if candidate.get(instance.id).is_some() {
            Mutation::Replace(instance)
        } else {
            Mutation::Insert(instance)
        });
        stage(&mut candidate, &actor, "Compose connection hardware", edits)?;
    }
    for receiver in &request.receivers {
        if receiver.source.axis != request.axis.id {
            return Err(error(
                "Every receiving cut must use this connection's shared axis",
            ));
        }
        let scratch = candidate_session(&candidate)?;
        let result = if candidate.get(receiver.feature).is_some() {
            if !old_owned.contains(&receiver.feature) {
                return Err(error("Receiver feature belongs to unrelated source"));
            }
            prepare_connection_receiver_edit(
                &scratch,
                0,
                actor.clone(),
                receiver.feature,
                &receiver.source,
            )?
        } else {
            prepare_connection_receiver(&scratch, 0, actor.clone(), receiver)?
        };
        outputs.extend(result.outputs);
        candidate = result.prepared.document().clone();
        let entity = candidate
            .get(receiver.feature)
            .ok_or_else(|| error("Receiving feature is missing"))?;
        if entity.name != receiver.name {
            let mut entity = entity.clone();
            entity.name = receiver.name.clone();
            stage(
                &mut candidate,
                &actor,
                "Name connection receiving feature",
                vec![Mutation::Replace(entity)],
            )?;
        }
    }
    let record = Entity {
        description: String::new(),
        id: request.connection,
        name: request.name.clone(),
        data: EntityData::Extension {
            definition: definition.definition()?,
            inputs: definition.inputs()?,
        },
    };
    // Remove obsolete roles while the previous ownership record is detached.
    // Publishing the new record before this cleanup would make old associative
    // drivers refer to roles which intentionally no longer exist.
    let keep = owned(&candidate, request.connection, &definition)?;
    let obsolete = old_owned.difference(&keep).copied().collect();
    remove_owned(&mut candidate, &actor, &obsolete)?;
    let mut ownership = vec![Mutation::Insert(record)];
    for row in &request.hardware {
        let mut driver = candidate
            .get(row.driver)
            .ok_or_else(|| error("The composed hardware driver is missing"))?
            .clone();
        let source = ScrewHardwarePlacementDefinition {
            owner: row.instance,
            axis: request.axis.id,
            connection: Some(ScrewHardwareConnectionBinding {
                connection: request.connection,
                role: row.role_id,
            }),
            // The canonical providers are the sole pose authority. There is
            // no stale copied station, helical phase or component mount.
            station: length(0.),
            roll: angle(0.),
            local_mount: RigidPose::default(),
        };
        driver.data = EntityData::Extension {
            definition: source.definition()?,
            inputs: source.inputs()?,
        };
        ownership.push(Mutation::Replace(driver));
    }
    stage(
        &mut candidate,
        &actor,
        "Retain associative connection ownership",
        ownership,
    )?;
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: if old.is_some() {
            "Edit screw connection"
        } else {
            "Create screw connection"
        }
        .into(),
        edits: super::extrude_boolean::delta(session.document(), &candidate)?,
    })?;
    if prepared.document() != &candidate {
        return Err(error(
            "Connection composition changed during final validation",
        ));
    }
    Ok(ModelPrepared { prepared, outputs })
}

/// Generated connection parts are reusable component definitions. Their actual
/// outer Instances are the physical occurrences, as with Assembly definitions.
pub fn screw_connection_definition_parts(
    document: &Document,
) -> Result<BTreeSet<EntityId>, DocumentError> {
    let mut parts = BTreeSet::new();
    for entity in document.entities() {
        if !matches!(&entity.data, EntityData::Extension { definition, .. } if definition.type_id == SCREW_CONNECTION_TYPE)
        {
            continue;
        }
        let source = read_screw_connection(document, entity.id)?;
        for row in source.hardware {
            if let ScrewConnectionHardwareBinding::Generated { feature, body } = row.source {
                let actual = read_mechanical_part(document, feature)?;
                if actual.output.body != body
                    || !matches!(document.get(body).map(|e| &e.data), Some(EntityData::Part { features, .. }) if features == &[feature])
                    || !matches!(document.get(row.instance).map(|e| &e.data), Some(EntityData::Instance { component, placement_source: Some(driver), .. }) if *component == body && *driver == row.driver)
                {
                    return Err(invalid(
                        "Generated connection hardware must retain its owned mechanical source and placed occurrence",
                    ));
                }
                parts.insert(body);
            }
        }
    }
    Ok(parts)
}

pub(super) fn validate_document(document: &Document) -> Result<(), DocumentError> {
    let mut ownership = BTreeMap::new();
    for entity in document.entities() {
        if !matches!(&entity.data,EntityData::Extension{definition,..}if definition.type_id==SCREW_CONNECTION_TYPE)
        {
            continue;
        }
        let source = read_screw_connection(document, entity.id)?;
        for row in &source.hardware {
            if document.get(row.driver).is_some() {
                let driver = read_screw_hardware_placement(document, row.driver)?;
                if driver.owner != row.instance || driver.axis != source.axis {
                    return Err(invalid(
                        "Connection hardware driver no longer matches its owning role and axis",
                    ));
                }
            }
            for id in [row.instance, row.driver] {
                if ownership.insert(id, entity.id).is_some() {
                    return Err(invalid("Hardware cannot be owned by two connections"));
                }
            }
        }
        if ownership.insert(source.axis, entity.id).is_some() {
            return Err(invalid("Each connection retains its own axis"));
        }
        for id in &source.receivers {
            if document.get(*id).is_some()
                && read_connection_receiver(document, *id)?.axis != source.axis
            {
                return Err(invalid(
                    "Owned receiving feature must use the connection axis",
                ));
            }
            if ownership.insert(*id, entity.id).is_some() {
                return Err(invalid(
                    "A receiving feature cannot be owned by two connections",
                ));
            }
        }
    }
    Ok(())
}

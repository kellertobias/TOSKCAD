//! Retained connector recipes and stable mounting sites on sourced profile members.
mod frames;
use super::{
    BodyInput, BodyOutput, CreatedBody, MemberEnd, ModelPrepared, OccurrencePath,
    ProfileGrooveSide, ProfileSection,
    common::{error, number, parameter_links, revision},
    source::{check_dimension, invalid},
};
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    parameters::{EvaluatedParameters, Unit},
};
pub use frames::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const PROFILE_CONNECTION_TYPE: &str = "model.profile_connection";
pub const PROFILE_CONNECTION_CAPABILITY: &str = "model.profile_connection.1";
pub const PROFILE_CONNECTION_OUTPUT_TYPE: &str = "model.profile_connection_output";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileConnectionKind {
    AngleBracket,
    JoiningPlate,
    UniversalEnd,
}
impl ProfileConnectionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::AngleBracket => "Line 8 angle bracket",
            Self::JoiningPlate => "Line 8 flat joining plate",
            Self::UniversalEnd => "Line 8 universal end fasteners",
        }
    }

    /// Supplier recipe identity retained by schema 1.
    pub fn article(self) -> &'static str {
        match self {
            Self::AngleBracket => "0.0.692.14",
            Self::JoiningPlate => "0.0.692.16",
            Self::UniversalEnd => "0.0.026.92",
        }
    }

    pub fn roles(self) -> &'static [ProfilePartRole] {
        use ProfilePartRole::*;
        match self {
            Self::AngleBracket | Self::JoiningPlate => {
                &[Connector(0), Screw(0), Nut(0), Screw(1), Nut(1)]
            }
            Self::UniversalEnd => &[
                Connector(0),
                Screw(0),
                Nut(0),
                Connector(1),
                Screw(1),
                Nut(1),
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfilePartRole {
    Connector(usize),
    Screw(usize),
    Nut(usize),
}
impl ProfilePartRole {
    fn name(self, kind: ProfileConnectionKind) -> String {
        match self {
            Self::Connector(index) if kind == ProfileConnectionKind::UniversalEnd => {
                format!("Line 8 universal fastener {}", index + 1)
            }
            Self::Connector(_) => kind.label().into(),
            Self::Screw(index) => format!("Line 8 M8 screw {}", index + 1),
            Self::Nut(index) => format!("Line 8 M8 T-slot nut {}", index + 1),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProfileConnectorAnchor {
    Groove {
        side: ProfileGrooveSide,
        station: Dimension,
    },
    End {
        end: MemberEnd,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileAttachment {
    pub member: BodyInput,
    pub occurrence: OccurrencePath,
    pub anchor: ProfileConnectorAnchor,
}
impl ProfileAttachment {
    fn validate(&self) -> Result<(), DocumentError> {
        if self.member.body.is_nil()
            || self.member.producer.is_nil()
            || self.occurrence.root.is_nil()
            || self.occurrence.members.len() > 32
            || self.occurrence.members.iter().any(|id| id.is_nil())
            || std::iter::once(&self.occurrence.root)
                .chain(&self.occurrence.members)
                .collect::<BTreeSet<_>>()
                .len()
                != self.occurrence.members.len() + 1
        {
            return Err(invalid("Choose an exact profile member occurrence"));
        }
        if let ProfileConnectorAnchor::Groove { station, .. } = &self.anchor {
            check_dimension(station, Unit::Millimeter)?;
        }
        Ok(())
    }

    pub fn groove_station(
        &self,
        parameters: &EvaluatedParameters,
    ) -> Result<Option<f64>, DocumentError> {
        match &self.anchor {
            ProfileConnectorAnchor::Groove { station, .. } => {
                Ok(Some(number(station, Unit::Millimeter, parameters)?))
            }
            ProfileConnectorAnchor::End { .. } => Ok(None),
        }
    }

    fn validate_member(&self, document: &Document) -> Result<(), DocumentError> {
        self.member.resolve_producer(document)?;
        if !matches!(
            document
                .get(self.member.producer)
                .map(|entity| &entity.data),
            Some(EntityData::Feature {
                suppressed: false,
                ..
            })
        ) {
            return Err(invalid("The selected profile member is suppressed"));
        }
        let member = super::read_member(document, self.member.producer)?;
        let stock = super::read_profile_stock(document, member.stock)?;
        if !ProfileSection::from_stock(&stock)?.is_item_line_8_40x40() {
            return Err(invalid(
                "Choose item Line 8 profile 40 × 40 stock. Outside size alone does not establish groove compatibility.",
            ));
        }
        super::validate_transfer_path(document, &self.occurrence, self.member.body)?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConnectionRequest {
    pub connection: EntityId,
    pub name: String,
    pub kind: ProfileConnectionKind,
    pub first: ProfileAttachment,
    pub second: ProfileAttachment,
}
impl ProfileConnectionRequest {
    pub fn new(
        name: impl Into<String>,
        kind: ProfileConnectionKind,
        first: ProfileAttachment,
        second: ProfileAttachment,
    ) -> Self {
        Self {
            connection: EntityId::new(),
            name: name.into(),
            kind,
            first,
            second,
        }
    }

    fn validate(&self) -> Result<(), DocumentError> {
        if self.connection.is_nil()
            || self.name.trim().is_empty()
            || self.name.len() > 1000
            || self.first.occurrence == self.second.occurrence
        {
            return Err(invalid(
                "A named profile connection needs two different member occurrences",
            ));
        }
        self.first.validate()?;
        self.second.validate()?;
        let compatible = match self.kind {
            ProfileConnectionKind::AngleBracket | ProfileConnectionKind::JoiningPlate => {
                matches!(self.first.anchor, ProfileConnectorAnchor::Groove { .. })
                    && matches!(self.second.anchor, ProfileConnectorAnchor::Groove { .. })
            }
            ProfileConnectionKind::UniversalEnd => {
                matches!(self.first.anchor, ProfileConnectorAnchor::End { .. })
                    && matches!(self.second.anchor, ProfileConnectorAnchor::Groove { .. })
            }
        };
        if !compatible {
            return Err(invalid(match self.kind {
                ProfileConnectionKind::AngleBracket | ProfileConnectionKind::JoiningPlate => {
                    "Choose a groove on each profile member"
                }
                ProfileConnectionKind::UniversalEnd => {
                    "Choose the end of the first member and a groove on the second"
                }
            }));
        }
        Ok(())
    }

    fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        let sites = [&self.first, &self.second];
        let mut links = parameter_links(
            sites
                .iter()
                .filter_map(|site| match &site.anchor {
                    ProfileConnectorAnchor::Groove { station, .. } => Some(station),
                    _ => None,
                })
                .collect(),
        );
        for (index, site) in sites.into_iter().enumerate() {
            links.push(Link::new(
                site.member.producer,
                format!("profile_connection/member/{index}"),
            ));
            if site.occurrence.root != site.member.body || !site.occurrence.members.is_empty() {
                for (step, id) in std::iter::once(site.occurrence.root)
                    .chain(site.occurrence.members.iter().copied())
                    .enumerate()
                {
                    links.push(Link::new(
                        id,
                        format!("profile_connection/occurrence/{index}/{step}"),
                    ));
                }
            }
        }
        Ok(links)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartIds {
    feature: EntityId,
    body: EntityId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileConnectionSource {
    request: ProfileConnectionRequest,
    parts: Vec<PartIds>,
    machining: Option<EntityId>,
}
impl ProfileConnectionSource {
    fn validate(&self) -> Result<(), DocumentError> {
        self.request.validate()?;
        if self.parts.len() != self.request.kind.roles().len()
            || self.machining.is_some()
                != (self.request.kind == ProfileConnectionKind::UniversalEnd)
        {
            return Err(invalid(
                "Profile connection outputs disagree with its recipe",
            ));
        }
        let mut used = BTreeSet::from([self.request.connection]);
        for id in self
            .parts
            .iter()
            .flat_map(|part| [part.feature, part.body])
            .chain(self.machining)
        {
            if id.is_nil() || !used.insert(id) {
                return Err(invalid(
                    "Profile connection output identities must be unique",
                ));
            }
        }
        Ok(())
    }

    fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: PROFILE_CONNECTION_TYPE.into(),
            schema_version: 1,
            data: serde_json::to_value(self).map_err(|error| invalid(error.to_string()))?,
        })
    }
}

fn read_source(
    document: &Document,
    id: EntityId,
) -> Result<ProfileConnectionSource, DocumentError> {
    let Some(EntityData::Extension { definition, inputs }) =
        document.get(id).map(|entity| &entity.data)
    else {
        return Err(invalid("Choose a profile connection"));
    };
    if definition.type_id != PROFILE_CONNECTION_TYPE || definition.schema_version != 1 {
        return Err(invalid(
            "Unsupported profile connection provider or version",
        ));
    }
    let source: ProfileConnectionSource = serde_json::from_value(definition.data.clone())
        .map_err(|error| invalid(error.to_string()))?;
    source.validate()?;
    if source.request.connection != id || source.request.inputs()? != *inputs {
        return Err(invalid(
            "Profile connection source and canonical dependencies disagree",
        ));
    }
    Ok(source)
}

pub fn read_profile_connection_request(
    document: &Document,
    id: EntityId,
) -> Result<ProfileConnectionRequest, DocumentError> {
    Ok(read_source(document, id)?.request)
}

pub(super) fn validate_document(document: &Document) -> Result<(), DocumentError> {
    for entity in document.entities() {
        match &entity.data {
            EntityData::Extension { definition, .. }
                if definition.type_id == PROFILE_CONNECTION_TYPE =>
            {
                let source = read_source(document, entity.id)?;
                for part in source.parts {
                    read_profile_connection_output(document, part.feature)?;
                    if !matches!(
                        document.get(part.body).map(|entity| &entity.data),
                        Some(EntityData::Part { features, .. }) if features.contains(&part.feature)
                    ) {
                        return Err(invalid("A profile connection lost an owned Part"));
                    }
                }
                if let Some(cut) = source.machining {
                    let output = read_profile_connection_output(document, cut)?;
                    if !matches!(
                        document.get(output.output.body).map(|entity| &entity.data),
                        Some(EntityData::Part { features, .. }) if features.contains(&cut)
                    ) {
                        return Err(invalid("A profile connection lost its owned access bores"));
                    }
                }
            }
            EntityData::Feature { definition, .. }
                if definition.type_id == PROFILE_CONNECTION_OUTPUT_TYPE =>
            {
                read_profile_connection_output(document, entity.id)?;
            }
            _ => {}
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProfileOutputKind {
    Part { index: usize },
    AccessBores { target: BodyInput },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConnectionOutput {
    pub connection: EntityId,
    pub output: BodyOutput,
    pub geometry: ProfileOutputKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_bores: Option<EntityId>,
}
impl ProfileConnectionOutput {
    fn definition(&self) -> Result<Definition, DocumentError> {
        Ok(Definition {
            type_id: PROFILE_CONNECTION_OUTPUT_TYPE.into(),
            schema_version: 1,
            data: serde_json::to_value(self).map_err(|error| invalid(error.to_string()))?,
        })
    }

    fn inputs(&self) -> Vec<Link> {
        let mut links = vec![Link::new(self.connection, "profile_connection")];
        if let ProfileOutputKind::AccessBores { target } = &self.geometry {
            links.push(Link::new(target.producer, "receiving_profile"));
        }
        if let Some(cut) = self.access_bores {
            links.push(Link::new(cut, "profile_access_bores"));
        }
        links
    }

    fn entity(&self, id: EntityId, name: impl Into<String>) -> Result<Entity, DocumentError> {
        Ok(Entity {
            id,
            name: name.into(),
            description: String::new(),
            data: EntityData::Feature {
                definition: self.definition()?,
                inputs: self.inputs(),
                suppressed: false,
            },
        })
    }
}

pub fn read_profile_connection_output(
    document: &Document,
    id: EntityId,
) -> Result<ProfileConnectionOutput, DocumentError> {
    let Some(EntityData::Feature {
        definition,
        inputs,
        suppressed: false,
    }) = document.get(id).map(|entity| &entity.data)
    else {
        return Err(invalid("Profile connection output feature is missing"));
    };
    if definition.type_id != PROFILE_CONNECTION_OUTPUT_TYPE || definition.schema_version != 1 {
        return Err(invalid(
            "Unsupported profile connection output provider or version",
        ));
    }
    let output: ProfileConnectionOutput = serde_json::from_value(definition.data.clone())
        .map_err(|error| invalid(error.to_string()))?;
    let source = read_source(document, output.connection)?;
    let valid = match &output.geometry {
        ProfileOutputKind::Part { index } => {
            output.access_bores == source.machining
                && source
                    .parts
                    .get(*index)
                    .is_some_and(|part| part.feature == id && part.body == output.output.body)
        }
        ProfileOutputKind::AccessBores { target } => {
            source.machining == Some(id)
                && target == &source.request.first.member
                && target.body == output.output.body
                && output.access_bores.is_none()
        }
    };
    if !valid || output.inputs() != *inputs {
        return Err(invalid(
            "Profile connection output ownership disagrees with its source",
        ));
    }
    Ok(output)
}

pub fn profile_connection_for(document: &Document, selected: EntityId) -> Option<EntityId> {
    let entity = document.get(selected)?;
    match &entity.data {
        EntityData::Extension { definition, .. }
            if definition.type_id == PROFILE_CONNECTION_TYPE =>
        {
            Some(selected)
        }
        EntityData::Feature { definition, .. }
            if definition.type_id == PROFILE_CONNECTION_OUTPUT_TYPE =>
        {
            read_profile_connection_output(document, selected)
                .ok()
                .map(|output| output.connection)
        }
        EntityData::Part { features, .. } => features.iter().rev().find_map(|id| {
            read_profile_connection_output(document, *id)
                .ok()
                .map(|output| output.connection)
        }),
        _ => None,
    }
}

fn machining_body(
    document: &Document,
    request: &ProfileConnectionRequest,
    existing: Option<EntityId>,
) -> Result<Entity, DocumentError> {
    let site = &request.first;
    if site.occurrence.root != site.member.body
        || !site.occurrence.members.is_empty()
        || document.entities().any(|entity| {
            matches!(&entity.data, EntityData::Instance { component, .. } if *component == site.member.body)
        })
    {
        return Err(invalid(
            "The access bores need a unique receiving profile Part. Use a root member with its own source before machining.",
        ));
    }
    let body = document
        .get(site.member.body)
        .ok_or(DocumentError::Missing(site.member.body))?;
    let EntityData::Part { features, .. } = &body.data else {
        return Err(invalid("The machined profile member is missing"));
    };
    if !features
        .iter()
        .copied()
        .eq(std::iter::once(site.member.producer).chain(existing))
    {
        return Err(invalid(
            "The receiving member has other body operations. Repair or remove those operations before cutting connector access bores.",
        ));
    }
    Ok(body.clone())
}

pub fn prepare_profile_connection(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &ProfileConnectionRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    request.validate()?;
    let document = session.document();
    for site in [&request.first, &request.second] {
        site.validate_member(document)?;
        site.groove_station(session.parameters())?;
    }
    let prior = document
        .get(request.connection)
        .map(|_| read_source(document, request.connection))
        .transpose()?;
    if let Some(prior) = &prior
        && (prior.request.kind != request.kind
            || (prior.machining.is_some() && prior.request.first.member != request.first.member))
    {
        return Err(error(
            "Remove the old connection before changing its recipe or machined Part",
        ));
    }
    let mut edits = vec![];
    let source = if let Some(mut prior) = prior {
        if prior.machining.is_some() {
            machining_body(document, request, prior.machining)?;
        }
        prior.request = request.clone();
        prior
    } else {
        let parts = request
            .kind
            .roles()
            .iter()
            .map(|_| PartIds {
                feature: EntityId::new(),
                body: EntityId::new(),
            })
            .collect();
        let machining = if request.kind == ProfileConnectionKind::UniversalEnd {
            let mut body = machining_body(document, request, None)?;
            let feature = EntityId::new();
            let EntityData::Part { features, .. } = &mut body.data else {
                unreachable!()
            };
            features.push(feature);
            let output = ProfileConnectionOutput {
                connection: request.connection,
                output: BodyOutput { body: body.id },
                geometry: ProfileOutputKind::AccessBores {
                    target: request.first.member.clone(),
                },
                access_bores: None,
            };
            edits.push(Mutation::Insert(
                output.entity(feature, "Line 8 universal access bores")?,
            ));
            edits.push(Mutation::Replace(body));
            Some(feature)
        } else {
            None
        };
        ProfileConnectionSource {
            request: request.clone(),
            parts,
            machining,
        }
    };
    let record = Entity {
        id: request.connection,
        name: request.name.clone(),
        description: format!("item {} · Line 8", request.kind.article()),
        data: EntityData::Extension {
            definition: source.definition()?,
            inputs: request.inputs()?,
        },
    };
    if document.get(request.connection).is_some() {
        edits.push(Mutation::Replace(record));
    } else {
        edits.push(Mutation::Insert(record));
        for (index, (part, role)) in source.parts.iter().zip(request.kind.roles()).enumerate() {
            let name = role.name(request.kind);
            let output = ProfileConnectionOutput {
                connection: request.connection,
                output: BodyOutput { body: part.body },
                geometry: ProfileOutputKind::Part { index },
                access_bores: source.machining,
            };
            edits.push(Mutation::Insert(
                output.entity(part.feature, format!("{name} feature"))?,
            ));
            edits.push(Mutation::Insert(Entity {
                id: part.body,
                name,
                description: format!(
                    "item {} · retained connection recipe",
                    request.kind.article()
                ),
                data: EntityData::Part {
                    features: vec![part.feature],
                    material: None,
                    tags: BTreeSet::new(),
                },
            }));
        }
    }
    let outputs = source
        .parts
        .iter()
        .map(|part| CreatedBody {
            feature: part.feature,
            body: part.body,
        })
        .collect();
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Set profile connection".into(),
        edits,
    })?;
    Ok(ModelPrepared { prepared, outputs })
}

pub fn prepare_remove_profile_connection(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    connection: EntityId,
) -> Result<Prepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    let source = read_source(document, connection)?;
    let mut ids = BTreeSet::from([connection]);
    let mut edits = vec![];
    for part in &source.parts {
        ids.extend([part.feature, part.body]);
    }
    if let Some(cut) = source.machining {
        let mut body = machining_body(document, &source.request, Some(cut))?;
        let EntityData::Part { features, .. } = &mut body.data else {
            unreachable!()
        };
        features.retain(|id| *id != cut);
        edits.push(Mutation::Replace(body));
        ids.insert(cut);
    }
    let mut hierarchy = document.hierarchy().clone();
    hierarchy.members.retain(|owner, _| !ids.contains(owner));
    for members in hierarchy.members.values_mut() {
        members.retain(|id| !ids.contains(id));
    }
    if &hierarchy != document.hierarchy() {
        edits.push(Mutation::SetHierarchy(hierarchy));
    }
    edits.extend(ids.into_iter().map(Mutation::Remove));
    session.prepare(Request {
        expected_revision,
        actor,
        label: "Remove profile connection".into(),
        edits,
    })
}

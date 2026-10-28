use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    references::GeometryIntent,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const TYPE_ID: &str = "inspection.measurement";
pub const CAPABILITY: &str = "inspection.measurement.1";
pub const MAX_PINS: usize = 128;
fn invalid(message: impl ToString) -> DocumentError {
    DocumentError::Invalid(message.to_string())
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryReference {
    pub source: EntityId,
    pub intent: GeometryIntent,
}
impl GeometryReference {
    pub fn validate(&self) -> Result<(), String> {
        if self.source.is_nil() {
            return Err("Measurement source identity is required".into());
        }
        self.intent.validate()
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementQuery {
    pub a: GeometryReference,
    pub b: GeometryReference,
}
impl MeasurementQuery {
    pub fn validate(&self) -> Result<(), String> {
        self.a.validate()?;
        self.b.validate()?;
        if self.a.intent.id == self.b.intent.id {
            return Err("A and B need independent capture identities, even when selecting the same geometry".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementDefinition {
    pub a: EntityId,
    pub b: EntityId,
    /// Visual label displacement only. The measured endpoints are regenerated.
    pub label_offset_mm: [f64; 3],
    pub visible: bool,
}
impl MeasurementDefinition {
    fn validate(&self) -> Result<(), DocumentError> {
        if self.a.is_nil()
            || self.b.is_nil()
            || self
                .label_offset_mm
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 1e7)
        {
            return Err(invalid(
                "Pinned measurement needs valid reference identities and a bounded label offset",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinRequest {
    pub id: EntityId,
    pub name: String,
    pub query: MeasurementQuery,
    pub label_offset_mm: [f64; 3],
    pub visible: bool,
}
impl PinRequest {
    pub fn new(name: impl Into<String>, query: MeasurementQuery) -> Self {
        Self {
            id: EntityId::new(),
            name: name.into(),
            query,
            label_offset_mm: [0.; 3],
            visible: true,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct PinnedMeasurement {
    pub id: EntityId,
    pub name: String,
    pub query: MeasurementQuery,
    pub label_offset_mm: [f64; 3],
    pub visible: bool,
}
pub fn pins(document: &Document) -> impl Iterator<Item = EntityId> + '_ {
    document.entities().filter(|e|matches!(&e.data,EntityData::Extension{definition,..}if definition.type_id==TYPE_ID)).map(|e|e.id)
}
pub fn read(document: &Document, id: EntityId) -> Result<PinnedMeasurement, DocumentError> {
    let entity = document.get(id).ok_or(DocumentError::Missing(id))?;
    let EntityData::Extension { definition, inputs } = &entity.data else {
        return Err(invalid("Choose a pinned measurement"));
    };
    if definition.type_id != TYPE_ID || definition.schema_version != 1 {
        return Err(invalid("Unsupported pinned measurement schema"));
    }
    let source: MeasurementDefinition =
        serde_json::from_value(definition.data.clone()).map_err(invalid)?;
    source.validate()?;
    if inputs.len() != 2 {
        return Err(invalid(
            "A pinned measurement needs exactly two link-owned geometry references",
        ));
    }
    let reference = |role: &str, intent: EntityId| -> Result<GeometryReference, DocumentError> {
        let mut links = inputs.iter().filter(|l| l.role == role);
        let link = links
            .next()
            .ok_or_else(|| invalid("Pinned measurement reference link is missing"))?;
        if links.next().is_some() {
            return Err(invalid("Pinned measurement reference link is duplicated"));
        }
        let geometry = link
            .geometry
            .as_ref()
            .filter(|g| g.id == intent)
            .ok_or_else(|| invalid("Pinned measurement link and reference identity disagree"))?;
        let result = GeometryReference {
            source: link.target,
            intent: geometry.clone(),
        };
        result.validate().map_err(invalid)?;
        if let Some(entity) = document.get(link.target)
            && !matches!(
                entity.data,
                EntityData::Feature { .. } | EntityData::Part { .. } | EntityData::Instance { .. }
            )
        {
            return Err(invalid(
                "Measurement reference source has an unsupported entity kind",
            ));
        }
        Ok(result)
    };
    let query = MeasurementQuery {
        a: reference("measurement.a", source.a)?,
        b: reference("measurement.b", source.b)?,
    };
    query.validate().map_err(invalid)?;
    Ok(PinnedMeasurement {
        id,
        name: entity.name.clone(),
        query,
        label_offset_mm: source.label_offset_mm,
        visible: source.visible,
    })
}
pub fn validate(document: &Document) -> Result<(), DocumentError> {
    let ids: Vec<_> = pins(document).collect();
    if ids.len() > MAX_PINS {
        return Err(invalid("Pinned measurement budget exceeded"));
    }
    for id in ids {
        read(document, id)?;
    }
    Ok(())
}
pub fn required_capability(document: &Document) -> bool {
    pins(document).next().is_some()
}
pub fn prepare_pin(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &PinRequest,
) -> Result<Prepared, CommandError> {
    request.query.validate().map_err(invalid)?;
    if request.id.is_nil() || request.id == session.document().id() {
        return Err(invalid(
            "Pinned measurement identity must be non-nil and distinct from the document",
        )
        .into());
    }
    let source = MeasurementDefinition {
        a: request.query.a.intent.id,
        b: request.query.b.intent.id,
        label_offset_mm: request.label_offset_mm,
        visible: request.visible,
    };
    source.validate()?;
    let link = |role: &str, r: &GeometryReference| Link {
        target: r.source,
        role: role.into(),
        geometry: Some(r.intent.clone()),
    };
    let mut entity = Entity::new(
        &request.name,
        EntityData::Extension {
            definition: Definition {
                type_id: TYPE_ID.into(),
                schema_version: 1,
                data: serde_json::to_value(source).map_err(invalid)?,
            },
            inputs: vec![
                link("measurement.a", &request.query.a),
                link("measurement.b", &request.query.b),
            ],
        },
    );
    entity.id = request.id;
    let edit = if session.document().get(request.id).is_some() {
        read(session.document(), request.id)?;
        Mutation::Replace(entity)
    } else {
        Mutation::Insert(entity)
    };
    session.prepare(Request {
        expected_revision: revision,
        actor,
        label: "Pin or edit measurement".into(),
        edits: vec![edit],
    })
}
pub fn prepare_remove(
    session: &Session,
    revision: u64,
    actor: Actor,
    id: EntityId,
) -> Result<Prepared, CommandError> {
    read(session.document(), id)?;
    session.prepare(Request {
        expected_revision: revision,
        actor,
        label: "Remove pinned measurement".into(),
        edits: vec![Mutation::Remove(id)],
    })
}
/// Source IDs for metadata invalidation; no geometry or feature dependency is
/// introduced by a measurement result.
pub fn sources(query: &MeasurementQuery) -> BTreeSet<EntityId> {
    [query.a.source, query.b.source].into()
}

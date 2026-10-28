use super::*;
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    document::{Document, DocumentError, Entity, EntityData, EntityId, Mutation},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Atomic upsert of authored library entities, retaining caller-allocated IDs.
/// Deletion intentionally leaves assignments repairable and diagnosed.
pub fn prepare_library(
    session: &Session,
    revision: u64,
    actor: Actor,
    entities: &[Entity],
) -> Result<Prepared, CommandError> {
    let mut edits = Vec::new();
    let mut ids = BTreeSet::new();
    for e in entities {
        if !ids.insert(e.id) {
            return Err(DocumentError::Duplicate(e.id).into());
        }
        if !matches!(e.data, EntityData::Material { .. }) && read(e)?.is_none() {
            return Err(DocumentError::Invalid(
                "Expected a manufacturing library definition".into(),
            )
            .into());
        }
        if let Some(old) = session.document().get(e.id) {
            let same = match (&old.data, &e.data) {
                (EntityData::Material { .. }, EntityData::Material { .. }) => true,
                (
                    EntityData::Extension { definition: a, .. },
                    EntityData::Extension { definition: b, .. },
                ) => a.type_id == b.type_id,
                _ => false,
            };
            if !same {
                return Err(DocumentError::Invalid(format!(
                    "Cannot change library kind for {}",
                    e.id
                ))
                .into());
            }
            edits.push(Mutation::Replace(e.clone()));
        } else {
            edits.push(Mutation::Insert(e.clone()));
        }
    }
    let preview = session.prepare(Request {
        expected_revision: revision,
        actor,
        label: "Edit material library".into(),
        edits,
    })?;
    if let Some(d) = diagnostics(preview.document())?.into_iter().find(|d| {
        ids.contains(&d.owner)
            && matches!(
                d.kind,
                DiagnosticKind::Missing
                    | DiagnosticKind::WrongKind
                    | DiagnosticKind::MaterialMismatch
            )
    }) {
        return Err(DocumentError::Invalid(d.message).into());
    }
    Ok(preview)
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
    pub material: Option<EntityId>,
    pub stock: Option<EntityId>,
    pub appearance: Option<EntityId>,
    pub hatch: Option<EntityId>,
}
impl Assignment {
    pub fn from_part(document: &Document, part: EntityId) -> Result<Self, DocumentError> {
        let Some(EntityData::Part { material, .. }) = document.get(part).map(|e| &e.data) else {
            return Err(DocumentError::Invalid("Expected a Part".into()));
        };
        let props = part_properties(document, part)?.map(|(_, p)| p);
        Ok(Self {
            material: *material,
            stock: props.as_ref().and_then(|p| p.stock.as_ref().map(|r| r.id)),
            appearance: props
                .as_ref()
                .and_then(|p| p.appearance.as_ref().map(|r| r.id)),
            hatch: props.as_ref().and_then(|p| p.hatch.as_ref().map(|r| r.id)),
        })
    }
}
/// Reassigning explicitly acknowledges current library content. Other part data,
/// feature history, tags, placement and containment remain unchanged.
pub fn prepare_assignment(
    session: &Session,
    revision: u64,
    actor: Actor,
    parts: &[EntityId],
    assignment: &Assignment,
) -> Result<Prepared, CommandError> {
    if parts.is_empty()
        || parts.len() > 1024
        || parts.iter().collect::<BTreeSet<_>>().len() != parts.len()
    {
        return Err(DocumentError::Invalid("Select 1–1024 distinct Parts".into()).into());
    }
    let doc = session.document();
    let bind = |id: Option<EntityId>| id.map(|id| LibraryRef::capture(doc, id)).transpose();
    let mut edits = Vec::new();
    for part in parts {
        let mut e = doc.get(*part).ok_or(DocumentError::Missing(*part))?.clone();
        let EntityData::Part { material, .. } = &mut e.data else {
            return Err(DocumentError::Invalid(format!("{part} is not a Part")).into());
        };
        *material = assignment.material;
        edits.push(Mutation::Replace(e));
        let previous = part_properties(doc, *part)?;
        let properties = LibraryDefinition::PartProperties(PartProperties {
            part: *part,
            material: bind(assignment.material)?,
            stock: bind(assignment.stock)?,
            appearance: bind(assignment.appearance)?,
            hatch: bind(assignment.hatch)?,
        });
        let id = previous.as_ref().map_or_else(EntityId::new, |s| s.0);
        let e = properties.entity(id, "Part manufacturing properties")?;
        edits.push(if previous.is_some() {
            Mutation::Replace(e)
        } else {
            Mutation::Insert(e)
        });
    }
    let preview = session.prepare(Request {
        expected_revision: revision,
        actor,
        label: "Assign part properties".into(),
        edits,
    })?;
    let resolver = Resolver::new(preview.document())?;
    for part in parts {
        resolver.resolve(*part)?;
    }
    Ok(preview)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    content = "tags",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TagEdit {
    Add(BTreeSet<String>),
    Remove(BTreeSet<String>),
    Replace(BTreeSet<String>),
}
/// Tags must be trimmed, case-sensitive UTF-8 tokens (1–128 bytes). Query matching is
/// exact; no substring, case folding, names, visibility or implicit hierarchy.
pub fn checked_tags(tags: &BTreeSet<String>) -> Result<(), DocumentError> {
    if tags.len() > 256
        || tags.iter().any(|s| {
            s.trim() != s || s.is_empty() || s.len() > 128 || s.chars().any(char::is_control)
        })
    {
        return Err(DocumentError::Invalid(
            "Tags need 1–128 bytes without outer whitespace or control characters; maximum 256"
                .into(),
        ));
    }
    Ok(())
}
pub fn prepare_tags(
    session: &Session,
    revision: u64,
    actor: Actor,
    parts: &[EntityId],
    edit: &TagEdit,
) -> Result<Prepared, CommandError> {
    if parts.is_empty()
        || parts.len() > 1024
        || parts.iter().collect::<BTreeSet<_>>().len() != parts.len()
    {
        return Err(DocumentError::Invalid("Select distinct Parts for tags".into()).into());
    }
    let (TagEdit::Add(values) | TagEdit::Remove(values) | TagEdit::Replace(values)) = edit;
    checked_tags(values)?;
    let mut edits = Vec::new();
    for id in parts {
        let mut entity = session
            .document()
            .get(*id)
            .ok_or(DocumentError::Missing(*id))?
            .clone();
        let EntityData::Part { tags, .. } = &mut entity.data else {
            return Err(DocumentError::Invalid(format!("{id} is not a Part")).into());
        };
        match edit {
            TagEdit::Add(v) => tags.extend(v.iter().cloned()),
            TagEdit::Remove(v) => tags.retain(|s| !v.contains(s)),
            TagEdit::Replace(v) => *tags = v.clone(),
        };
        checked_tags(tags)?;
        edits.push(Mutation::Replace(entity));
    }
    session.prepare(Request {
        expected_revision: revision,
        actor,
        label: "Edit part tags".into(),
        edits,
    })
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagQuery {
    pub all: BTreeSet<String>,
    pub any: BTreeSet<String>,
    pub none: BTreeSet<String>,
}
impl TagQuery {
    pub fn select(&self, document: &Document) -> Result<Vec<EntityId>, DocumentError> {
        checked_tags(&self.all)?;
        checked_tags(&self.any)?;
        checked_tags(&self.none)?;
        Ok(document
            .entities()
            .filter_map(|e| match &e.data {
                EntityData::Part { tags, .. }
                    if self.all.is_subset(tags)
                        && (self.any.is_empty() || !self.any.is_disjoint(tags))
                        && self.none.is_disjoint(tags) =>
                {
                    Some(e.id)
                }
                _ => None,
            })
            .collect())
    }
}

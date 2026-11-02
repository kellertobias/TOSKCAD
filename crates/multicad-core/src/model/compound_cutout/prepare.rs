use super::super::{
    self as model, CreatedBody, ModelPrepared,
    common::{error, revision},
};
use super::*;
use crate::{
    commands::{Actor, CommandError, Request, Session},
    document::{Entity, EntityData, EntityId, Mutation},
    hierarchy,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub struct CompoundCutoutRequest {
    pub feature: EntityId,
    pub name: String,
    pub definition: CompoundCutoutDefinition,
}
impl CompoundCutoutRequest {
    pub fn new(name: impl Into<String>, definition: CompoundCutoutDefinition) -> Self {
        Self {
            feature: EntityId::new(),
            name: name.into(),
            definition,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum CompoundCutoutChange {
    Set(Box<CompoundCutoutRequest>),
    Remove(EntityId),
}
impl CompoundCutoutChange {
    pub fn target(&self) -> EntityId {
        match self {
            Self::Set(request) => request.feature,
            Self::Remove(feature) => *feature,
        }
    }
}
fn entity(
    id: EntityId,
    name: impl Into<String>,
    definition: crate::document::Definition,
    inputs: Vec<crate::document::Link>,
) -> Entity {
    let mut entity = Entity::new(
        name,
        EntityData::Feature {
            definition,
            inputs,
            suppressed: false,
        },
    );
    entity.id = id;
    entity
}
fn part(session: &Session, id: EntityId) -> Result<Entity, CommandError> {
    let entity = session
        .document()
        .get(id)
        .ok_or_else(|| error("Cutout receiving Part is missing; repair its target"))?;
    if !matches!(entity.data, EntityData::Part { .. }) {
        return Err(error("Cutout receiver must identify an existing Part"));
    }
    Ok(entity.clone())
}
fn remove_last(
    session: &Session,
    receiver: &CompoundCutoutReceiver,
) -> Result<(Entity, CreatedBody), CommandError> {
    let mut entity = part(session, receiver.target.body)?;
    let EntityData::Part { features, .. } = &mut entity.data else {
        unreachable!()
    };
    if features.last() != Some(&receiver.feature) {
        return Err(error(
            "Cannot remove a compound receiver with downstream history; repair or remove its dependents first",
        ));
    }
    features.pop();
    let producer = features
        .iter()
        .rev()
        .find(|id| {
            matches!(
                session.document().get(**id).map(|entity| &entity.data),
                Some(EntityData::Feature {
                    suppressed: false,
                    ..
                })
            )
        })
        .copied()
        .ok_or_else(|| {
            error("Removing the cutout would leave its Part without a valid producer")
        })?;
    Ok((
        entity,
        CreatedBody {
            feature: producer,
            body: receiver.target.body,
        },
    ))
}
/// Prepare the complete source/history change. A caller must still obtain a
/// successful native receipt for every returned output before committing.
pub fn prepare_compound_cutout(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    change: &CompoundCutoutChange,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let id = change.target();
    if id.is_nil() || id == session.document().id() {
        return Err(error(
            "Compound cutout needs a distinct non-nil feature identity",
        ));
    }
    let previous = if session.document().get(id).is_some() {
        Some(read_compound_cutout(session.document(), id)?)
    } else {
        None
    };
    let mut edits = vec![];
    let mut outputs = vec![];
    let mut parts = BTreeMap::new();
    let mut removed = BTreeSet::new();
    let mut hierarchy = session.document().hierarchy().clone();
    let index = hierarchy::Index::new(session.document()).map_err(|e| error(e.to_string()))?;
    let label = match change {
        CompoundCutoutChange::Set(request) => {
            if request.name.trim().is_empty() || request.name.len() > 1000 {
                return Err(error("Cutout name must contain 1–1000 bytes"));
            }
            request.definition.validate()?;
            resolve_compound_cutout(
                session.document(),
                session.parameters(),
                &request.definition,
            )?;
            let retained: BTreeSet<_> = request
                .definition
                .receivers
                .iter()
                .map(|receiver| receiver.target.body)
                .collect();
            if let Some(old) = &previous {
                for receiver in &old.receivers {
                    if !retained.contains(&receiver.target.body) {
                        let (part, output) = remove_last(session, receiver)?;
                        parts.insert(part.id, part);
                        outputs.push(output);
                        removed.insert(receiver.feature);
                    }
                }
            }
            for receiver in &request.definition.receivers {
                if receiver.feature == id
                    || receiver.target.body == id
                    || receiver.target.producer == id
                {
                    return Err(error(
                        "Compound source and receiver identities must be distinct",
                    ));
                }
                let old = previous.as_ref().and_then(|source| {
                    source
                        .receivers
                        .iter()
                        .find(|old| old.target.body == receiver.target.body)
                });
                if let Some(old) = old {
                    if old.feature != receiver.feature || old.target != receiver.target {
                        return Err(error(
                            "Retained receivers must preserve their generated identity and frozen input; repair the receiver history explicitly",
                        ));
                    }
                } else {
                    if session.document().get(receiver.feature).is_some() {
                        return Err(error(
                            "New generated cut identities must not overwrite existing entities",
                        ));
                    }
                    model::boolean::available(session.document(), &receiver.target, true)?;
                    let mut target = part(session, receiver.target.body)?;
                    let EntityData::Part { features, .. } = &mut target.data else {
                        unreachable!()
                    };
                    features.push(receiver.feature);
                    parts.insert(target.id, target);
                    if let Some(parent) = index
                        .parent(receiver.target.body)
                        .map_err(|e| error(e.to_string()))?
                    {
                        hierarchy
                            .members
                            .entry(parent)
                            .or_default()
                            .push(receiver.feature);
                    }
                }
                let source = CompoundCutoutReceiverDefinition {
                    source: id,
                    target: receiver.target.clone(),
                    output: model::BodyOutput {
                        body: receiver.target.body,
                    },
                };
                let feature = entity(
                    receiver.feature,
                    format!("{} / receiving cut", request.name),
                    source.definition()?,
                    source.inputs(),
                );
                edits.push(if old.is_some() {
                    Mutation::Replace(feature)
                } else {
                    Mutation::Insert(feature)
                });
                outputs.push(CreatedBody {
                    feature: receiver.feature,
                    body: receiver.target.body,
                });
            }
            let feature = entity(
                id,
                &request.name,
                request.definition.definition()?,
                request.definition.inputs()?,
            );
            edits.push(if previous.is_some() {
                Mutation::Replace(feature)
            } else {
                if let Some(parent) = index
                    .parent(request.definition.receivers[0].target.body)
                    .map_err(|e| error(e.to_string()))?
                {
                    hierarchy.members.entry(parent).or_default().push(id);
                }
                Mutation::Insert(feature)
            });
            if previous.is_some() {
                "Edit compound cutout"
            } else {
                "Create compound cutout"
            }
        }
        CompoundCutoutChange::Remove(_) => {
            let previous = previous.as_ref().ok_or_else(|| {
                error("Compound cutout is missing; choose an existing source to remove")
            })?;
            for receiver in &previous.receivers {
                let (part, output) = remove_last(session, receiver)?;
                parts.insert(part.id, part);
                outputs.push(output);
                removed.insert(receiver.feature);
            }
            removed.insert(id);
            "Remove compound cutout"
        }
    };
    for entity in session.document().entities() {
        if removed.contains(&entity.id) || parts.contains_key(&entity.id) {
            continue;
        }
        if entity
            .references()
            .iter()
            .any(|link| removed.contains(&link.target))
        {
            return Err(error(format!(
                "Cannot remove cutout history referenced by {}; repair its dependencies first",
                entity.name
            )));
        }
    }
    edits.extend(parts.into_values().map(Mutation::Replace));
    edits.extend(removed.iter().copied().map(Mutation::Remove));
    for members in hierarchy.members.values_mut() {
        members.retain(|id| !removed.contains(id));
    }
    if hierarchy != *session.document().hierarchy() {
        edits.push(Mutation::SetHierarchy(hierarchy));
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: label.into(),
        edits,
    })?;
    Ok(ModelPrepared { prepared, outputs })
}

use super::super::{
    GrainAxis, PanelDefinition, PanelEdges, PanelSource,
    prepare::{panel_entities, remove_panel, restock},
};
use super::*;
use crate::{
    commands::{Actor, CommandError, Request, Session},
    document::{Entity, EntityData, EntityId, Mutation},
    model::{
        BodyOutput, CreatedBody, ModelPrepared,
        common::{error, revision},
    },
};

#[derive(Clone, Debug, PartialEq)]
pub struct WoodBoxRequest {
    pub id: EntityId,
    pub name: String,
    pub definition: WoodBoxDefinition,
}
impl WoodBoxRequest {
    pub fn new(name: impl Into<String>, definition: WoodBoxDefinition) -> Self {
        Self {
            id: EntityId::new(),
            name: name.into(),
            definition,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum WoodBoxChange {
    Set(Box<WoodBoxRequest>),
    Remove(EntityId),
}
impl WoodBoxChange {
    pub fn target(&self) -> EntityId {
        match self {
            Self::Set(request) => request.id,
            Self::Remove(id) => *id,
        }
    }
}
#[derive(Debug)]
pub struct WoodBoxPrepared {
    pub model: ModelPrepared,
    pub recipe: EntityId,
    pub group: EntityId,
    pub panels: Vec<(WoodBoxFace, CreatedBody)>,
    pub removed_parts: Vec<EntityId>,
}
fn panel_source(recipe: EntityId, face: WoodBoxFace, ids: &WoodBoxPanelIds) -> PanelDefinition {
    PanelDefinition {
        output: BodyOutput { body: ids.body },
        source: PanelSource::WoodBox { recipe, face },
        grain: if matches!(face, WoodBoxFace::Left | WoodBoxFace::Right) {
            GrainAxis::V
        } else {
            GrainAxis::U
        },
        edges: PanelEdges::default(),
        oversize: None,
    }
}
fn removal(
    document: &crate::document::Document,
    ids: &WoodBoxPanelIds,
) -> Result<Vec<Mutation>, CommandError> {
    let Some(EntityData::Part { features, .. }) = document.get(ids.body).map(|entity| &entity.data)
    else {
        return Err(error("Wood Box panel is missing; repair its source"));
    };
    if features != &[ids.feature] {
        return Err(error(
            "Cannot omit a Wood Box face with downstream history; repair or remove its operations first",
        ));
    }
    remove_panel(document, ids.feature, ids.body)
}
pub fn prepare_wood_box(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    change: &WoodBoxChange,
) -> Result<WoodBoxPrepared, CommandError> {
    revision(session, expected_revision)?;
    let id = change.target();
    let document = session.document();
    if id.is_nil() || id == document.id() {
        return Err(error("Wood Box recipe needs a distinct non-nil identity"));
    }
    let previous = if document.get(id).is_some() {
        Some(read_wood_box(document, id)?)
    } else {
        None
    };
    let mut edits = vec![];
    let mut panels = vec![];
    let mut removed_parts = vec![];
    let mut hierarchy = document.hierarchy().clone();
    let (group, label) = match change {
        WoodBoxChange::Set(request) => {
            if request.name.trim().is_empty() || request.name.len() > 1000 {
                return Err(error("Wood Box name must contain 1–1000 bytes"));
            }
            let source = &request.definition;
            source.validate()?;
            if source.group == id
                || source.panels.values().any(|panel| {
                    [id, document.id()].contains(&panel.body)
                        || [id, document.id()].contains(&panel.feature)
                })
                || source.group == document.id()
            {
                return Err(error(
                    "Wood Box face/group identities collide with the recipe or document",
                ));
            }
            if let Some(old) = &previous {
                if source.group != old.group || source.panels != old.panels {
                    return Err(error(
                        "A Wood Box edit must retain its group and named face identities",
                    ));
                }
            } else if document.get(source.group).is_some()
                || source.panels.values().any(|panel| {
                    document.get(panel.body).is_some() || document.get(panel.feature).is_some()
                })
            {
                return Err(error(
                    "New Wood Box identities must not overwrite existing entities",
                ));
            }
            resolve_wood_box(document, session.parameters(), source)?;
            let mut recipe = Entity::new(&request.name, source.entity_data()?);
            recipe.id = id;
            edits.push(if previous.is_some() {
                Mutation::Replace(recipe)
            } else {
                Mutation::Insert(recipe)
            });
            if let Some(old) = &previous {
                for face in old.faces.difference(&source.faces) {
                    edits.extend(removal(document, &old.panels[face])?);
                    removed_parts.push(old.panels[face].body);
                }
                let mut group = document
                    .get(source.group)
                    .cloned()
                    .ok_or_else(|| error("Wood Box group is missing"))?;
                group.name = request.name.clone();
                edits.push(Mutation::Replace(group));
            } else {
                let mut group = Entity::new(&request.name, EntityData::Group {});
                group.id = source.group;
                edits.push(Mutation::Insert(group));
                let parent = document
                    .hierarchy()
                    .active_component
                    .unwrap_or(document.id());
                hierarchy
                    .members
                    .entry(parent)
                    .or_default()
                    .push(source.group);
                hierarchy.members.insert(source.group, vec![]);
            }
            for face in WoodBoxFace::ALL
                .into_iter()
                .filter(|face| source.faces.contains(face))
            {
                let ids = &source.panels[&face];
                if previous
                    .as_ref()
                    .is_some_and(|old| old.faces.contains(&face))
                {
                    if previous
                        .as_ref()
                        .is_some_and(|old| old.stock != source.stock)
                    {
                        edits.extend(restock(document, ids.body, source.stock)?);
                    }
                } else {
                    edits.extend(panel_entities(
                        document,
                        &CreatedBody {
                            feature: ids.feature,
                            body: ids.body,
                        },
                        &format!("{} {}", request.name, face.label()),
                        &panel_source(id, face, ids),
                        source.stock,
                    )?);
                    hierarchy
                        .members
                        .entry(source.group)
                        .or_default()
                        .extend([ids.body, ids.feature]);
                }
                panels.push((
                    face,
                    CreatedBody {
                        feature: ids.feature,
                        body: ids.body,
                    },
                ));
            }
            (
                source.group,
                if previous.is_some() {
                    "Edit Wood Box"
                } else {
                    "Create Wood Box"
                },
            )
        }
        WoodBoxChange::Remove(_) => {
            let source = previous
                .as_ref()
                .ok_or_else(|| error("Choose an existing Wood Box recipe to remove"))?;
            let owned: std::collections::BTreeSet<_> = source
                .panels
                .values()
                .flat_map(|ids| [ids.feature, ids.body])
                .collect();
            if hierarchy
                .members
                .get(&source.group)
                .is_some_and(|members| members.iter().any(|member| !owned.contains(member)))
            {
                return Err(error(
                    "Move unrelated members out of the Wood Box group before removing the recipe",
                ));
            }
            for face in &source.faces {
                edits.extend(removal(document, &source.panels[face])?);
                removed_parts.push(source.panels[face].body);
            }
            edits.extend([Mutation::Remove(id), Mutation::Remove(source.group)]);
            (source.group, "Remove Wood Box")
        }
    };
    let removed = super::records::removed_ids(&edits);
    for entity in document.entities() {
        if removed.contains(&entity.id) {
            continue;
        }
        if entity
            .references()
            .iter()
            .any(|link| removed.contains(&link.target))
        {
            return Err(error(format!(
                "Cannot remove a Wood Box face referenced by {}; repair its dependencies first",
                entity.name
            )));
        }
    }
    for members in hierarchy.members.values_mut() {
        members.retain(|id| !removed.contains(id));
    }
    for id in &removed {
        hierarchy.members.remove(id);
    }
    hierarchy.hidden.retain(|id| !removed.contains(id));
    if hierarchy != *document.hierarchy() {
        edits.push(Mutation::SetHierarchy(hierarchy));
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: label.into(),
        edits,
    })?;
    Ok(WoodBoxPrepared {
        model: ModelPrepared {
            prepared,
            outputs: panels.iter().map(|(_, output)| output.clone()).collect(),
        },
        recipe: id,
        group,
        panels,
        removed_parts,
    })
}

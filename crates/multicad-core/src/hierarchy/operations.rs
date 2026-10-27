use super::*;
use crate::{
    commands::{Actor, CommandError, ErrorCode, Prepared, Request, Session},
    document::{Entity, Mutation},
};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Create {
        kind: ContainerKind,
        name: String,
        parent: Option<EntityId>,
    },
    Rename {
        id: EntityId,
        name: String,
    },
    Move {
        entries: Vec<EntityId>,
        parent: Option<EntityId>,
        before: Option<EntityId>,
    },
    SetVisible {
        id: EntityId,
        visible: bool,
    },
    SetActive {
        component: Option<EntityId>,
    },
    /// Explicitly migrate an unshared legacy instance list to ordered membership.
    NormalizeLegacy {
        component: EntityId,
    },
}
#[derive(Debug)]
pub struct HierarchyPrepared {
    pub prepared: Prepared,
    pub created: Option<EntityId>,
}
fn command_error(message: impl Into<String>) -> CommandError {
    CommandError {
        code: ErrorCode::InvalidRequest,
        message: message.into(),
        involved: vec![],
        parameter: None,
    }
}
fn destination(document: &Document, parent: Option<EntityId>) -> Result<EntityId, HierarchyError> {
    let parent = parent.unwrap_or(document.id());
    if parent != document.id()
        && !document
            .get(parent)
            .is_some_and(|entity| container_kind(&entity.data).is_some())
    {
        return Err(invalid("Destination must be a component, group or root"));
    }
    Ok(parent)
}
fn detach_legacy(document: &Document, ids: &BTreeSet<EntityId>) -> Vec<Mutation> {
    document
        .entities()
        .filter_map(|entity| {
            if let EntityData::Assembly { instances } = &entity.data {
                let retained: Vec<_> = instances
                    .iter()
                    .copied()
                    .filter(|id| !ids.contains(id))
                    .collect();
                if retained.len() != instances.len() {
                    let mut replacement = entity.clone();
                    replacement.data = EntityData::Assembly {
                        instances: retained,
                    };
                    return Some(Mutation::Replace(replacement));
                }
            }
            None
        })
        .collect()
}
fn move_members(
    document: &Document,
    state: &mut Hierarchy,
    entries: &[EntityId],
    parent: Option<EntityId>,
    before: Option<EntityId>,
) -> Result<Vec<Mutation>, HierarchyError> {
    let parent = destination(document, parent)?;
    let ids: BTreeSet<_> = entries.iter().copied().collect();
    if ids.is_empty() || ids.len() != entries.len() {
        return Err(invalid("Select one or more distinct model entries to move"));
    }
    for id in entries {
        if !document
            .get(*id)
            .is_some_and(|entity| is_model_entry(&entity.data))
        {
            return Err(invalid(format!("Unsupported model entry {id}")));
        }
    }
    if before.is_some_and(|id| ids.contains(&id)) {
        return Err(invalid(
            "The ordering anchor cannot be one of the moved entries",
        ));
    }
    // Preserve a selected subtree, rather than silently flattening descendants.
    let children = combined_children(document);
    for id in entries {
        let mut pending = children.get(id).cloned().unwrap_or_default();
        let mut seen = BTreeSet::new();
        while let Some(child) = pending.pop() {
            if !seen.insert(child) {
                continue;
            }
            if ids.contains(&child) {
                return Err(invalid("Move a container or its descendants, not both"));
            }
            pending.extend(children.get(&child).into_iter().flatten().copied());
        }
    }
    for members in state.members.values_mut() {
        members.retain(|id| !ids.contains(id));
    }
    let target = state.members.entry(parent).or_default();
    let offset=match before {Some(anchor)=>target.iter().position(|id|*id==anchor).ok_or_else(||invalid("The ordering anchor must be an explicit member of the destination; normalize a legacy container first"))?,None=>target.len()};
    target.splice(offset..offset, entries.iter().copied());
    Ok(detach_legacy(document, &ids))
}
pub fn prepare(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    action: Action,
) -> Result<HierarchyPrepared, CommandError> {
    if expected_revision != session.revision() {
        return Err(CommandError {
            code: ErrorCode::StaleRevision,
            message: format!(
                "Stale revision {expected_revision}; current revision is {}",
                session.revision()
            ),
            involved: vec![],
            parameter: None,
        });
    }
    let document = session.document();
    let mut state = document.hierarchy().clone();
    let mut edits = vec![];
    let mut created = None;
    let label = match action {
        Action::Create { kind, name, parent } => {
            let parent =
                destination(document, parent).map_err(|error| command_error(error.to_string()))?;
            let entity = Entity::new(
                name,
                match kind {
                    ContainerKind::Component => EntityData::Assembly { instances: vec![] },
                    ContainerKind::Group => EntityData::Group {},
                },
            );
            let id = entity.id;
            created = Some(id);
            state.members.entry(id).or_default();
            state.members.entry(parent).or_default().push(id);
            edits.push(Mutation::Insert(entity));
            "Create model container"
        }
        Action::Rename { id, name } => {
            let mut entity = document
                .get(id)
                .ok_or_else(|| command_error("Unknown model container"))?
                .clone();
            if container_kind(&entity.data).is_none() {
                return Err(command_error("Rename a component or group"));
            }
            entity.name = name;
            edits.push(Mutation::Replace(entity));
            "Rename model container"
        }
        Action::Move {
            entries,
            parent,
            before,
        } => {
            edits.extend(
                move_members(document, &mut state, &entries, parent, before)
                    .map_err(|error| command_error(error.to_string()))?,
            );
            "Move model entries"
        }
        Action::SetVisible { id, visible } => {
            if id != document.id()
                && !document
                    .get(id)
                    .is_some_and(|entity| is_model_entry(&entity.data))
            {
                return Err(command_error("Unknown or unsupported model entry"));
            }
            if visible {
                state.hidden.remove(&id);
            } else {
                state.hidden.insert(id);
            }
            "Change model visibility"
        }
        Action::SetActive { component } => {
            if let Some(id) = component
                && !document
                    .get(id)
                    .is_some_and(|entity| matches!(entity.data, EntityData::Assembly { .. }))
            {
                return Err(command_error("Activate a component or the document root"));
            }
            state.active_component = component;
            "Activate modeling component"
        }
        Action::NormalizeLegacy { component } => {
            let entity = document
                .get(component)
                .ok_or_else(|| command_error("Unknown legacy component"))?;
            let EntityData::Assembly { instances } = &entity.data else {
                return Err(command_error("Normalize an Assembly component"));
            };
            let index = Index::new(document).map_err(|error| command_error(error.to_string()))?;
            for id in instances {
                if !document
                    .get(*id)
                    .is_some_and(|target| matches!(target.data, EntityData::Instance { .. }))
                {
                    return Err(command_error(
                        "Repair legacy instance references before normalization",
                    ));
                }
                index
                    .parent(*id)
                    .map_err(|error| command_error(error.to_string()))?;
            }
            let members = state.members.entry(component).or_default();
            // Existing legacy order is retained before already explicit members.
            let mut normalized = instances.clone();
            normalized.extend(members.iter().filter(|id| !instances.contains(id)).copied());
            *members = normalized;
            let mut replacement = entity.clone();
            replacement.data = EntityData::Assembly { instances: vec![] };
            edits.push(Mutation::Replace(replacement));
            "Normalize legacy component membership"
        }
    };
    if &state != document.hierarchy() {
        edits.push(Mutation::SetHierarchy(state));
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: label.into(),
        edits,
    })?;
    Ok(HierarchyPrepared { prepared, created })
}
/// Attach every newly inserted model entry to the active component in the same
/// transaction. Global parameters/materials and existing linked sources stay put.
pub(crate) fn scope_insertions(
    document: &Document,
    edits: &mut Vec<Mutation>,
) -> Result<(), HierarchyError> {
    if edits
        .iter()
        .any(|edit| matches!(edit, Mutation::SetHierarchy(_)))
    {
        return Err(invalid(
            "Scoped insertion cannot also replace hierarchy state explicitly",
        ));
    }
    let Some(active) = document.hierarchy().active_component else {
        return Ok(());
    };
    let ids: Vec<_> = edits
        .iter()
        .filter_map(|edit| {
            if let Mutation::Insert(entity) = edit {
                is_model_entry(&entity.data).then_some(entity.id)
            } else {
                None
            }
        })
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let mut state = document.hierarchy().clone();
    state.members.entry(active).or_default().extend(ids);
    edits.push(Mutation::SetHierarchy(state));
    Ok(())
}
/// Include affected containment subtrees in shared change notifications without
/// introducing organizational edges into the feature dependency DAG.
pub(crate) fn changed_entries(before: &Document, after: &Document) -> BTreeSet<EntityId> {
    if before.hierarchy() == after.hierarchy() {
        return BTreeSet::new();
    }
    let old = before.hierarchy();
    let new = after.hierarchy();
    let mut seeds = BTreeSet::new();
    for id in old.members.keys().chain(new.members.keys()) {
        if old.members.get(id) != new.members.get(id) {
            seeds.insert(*id);
            seeds.extend(old.members.get(id).into_iter().flatten());
            seeds.extend(new.members.get(id).into_iter().flatten());
        }
    }
    seeds.extend(old.hidden.symmetric_difference(&new.hidden));
    if old.active_component != new.active_component {
        seeds.extend(old.active_component);
        seeds.extend(new.active_component);
    }
    let old_children = combined_children(before);
    let new_children = combined_children(after);
    let mut changed = seeds.clone();
    let mut pending: Vec<_> = seeds.into_iter().collect();
    while let Some(id) = pending.pop() {
        for child in old_children
            .get(&id)
            .into_iter()
            .flatten()
            .chain(new_children.get(&id).into_iter().flatten())
        {
            if changed.insert(*child) {
                pending.push(*child);
            }
        }
    }
    changed.retain(|id| before.get(*id).is_some() || after.get(*id).is_some());
    changed
}

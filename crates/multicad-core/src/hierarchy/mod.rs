//! Persistent source ownership and organizational containment, separate from
//! feature dependencies and reusable instance placement.
mod index;
mod operations;
use crate::document::{Document, EntityData, EntityId};
pub use index::*;
pub use operations::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const CAPABILITY: &str = "core.hierarchy.1";
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hierarchy {
    /// The document UUID denotes the root. Other keys are Assembly or Group IDs.
    #[serde(
        default,
        deserialize_with = "unique_members",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub members: BTreeMap<EntityId, Vec<EntityId>>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub hidden: BTreeSet<EntityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_component: Option<EntityId>,
}
fn unique_members<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<EntityId, Vec<EntityId>>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = BTreeMap<EntityId, Vec<EntityId>>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("unique hierarchy container identities")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut members = BTreeMap::new();
            while let Some((id, entries)) = map.next_entry::<EntityId, Vec<EntityId>>()? {
                if members.insert(id, entries).is_some() {
                    return Err(serde::de::Error::custom(format!(
                        "Duplicate hierarchy container {id}"
                    )));
                }
            }
            Ok(members)
        }
    }
    deserializer.deserialize_map(Visitor)
}
impl Hierarchy {
    pub fn is_empty(&self) -> bool {
        self.members.is_empty() && self.hidden.is_empty() && self.active_component.is_none()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerKind {
    Component,
    Group,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HierarchyError {
    Invalid(String),
    ContainmentCycle(Vec<EntityId>),
    ExpansionCycle(Vec<EntityId>),
    AmbiguousOwner {
        entry: EntityId,
        parents: Vec<EntityId>,
    },
}
impl std::fmt::Display for HierarchyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
            Self::ContainmentCycle(path) => write!(
                f,
                "Containment cycle: {}",
                path.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ),
            Self::ExpansionCycle(path) => write!(
                f,
                "Component/instance expansion cycle: {}",
                path.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ),
            Self::AmbiguousOwner { entry, parents } => write!(
                f,
                "Entry {entry} has multiple legacy parents: {}. Move it explicitly to establish ownership.",
                parents
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}
impl std::error::Error for HierarchyError {}
fn invalid(message: impl Into<String>) -> HierarchyError {
    HierarchyError::Invalid(message.into())
}
pub fn container_kind(data: &EntityData) -> Option<ContainerKind> {
    match data {
        EntityData::Assembly { .. } => Some(ContainerKind::Component),
        EntityData::Group {} => Some(ContainerKind::Group),
        _ => None,
    }
}
pub fn is_model_entry(data: &EntityData) -> bool {
    matches!(
        data,
        EntityData::Plane { .. }
            | EntityData::Sketch { .. }
            | EntityData::Feature { .. }
            | EntityData::Part { .. }
            | EntityData::Assembly { .. }
            | EntityData::Instance { .. }
            | EntityData::Group {}
    )
}
pub fn required_capability(document: &Document) -> bool {
    !document.hierarchy().is_empty()
        || document
            .entities()
            .any(|entity| matches!(entity.data, EntityData::Group {}))
}
fn legacy_children(document: &Document) -> BTreeMap<EntityId, Vec<EntityId>> {
    document
        .entities()
        .filter_map(|entity| {
            if let EntityData::Assembly { instances } = &entity.data {
                Some((
                    entity.id,
                    instances
                        .iter()
                        .copied()
                        .filter(|id| {
                            document.get(*id).is_some_and(|target| {
                                matches!(target.data, EntityData::Instance { .. })
                            })
                        })
                        .collect(),
                ))
            } else {
                None
            }
        })
        .collect()
}
fn combined_children(document: &Document) -> BTreeMap<EntityId, Vec<EntityId>> {
    let mut children = legacy_children(document);
    for (container, members) in &document.hierarchy().members {
        let list = children.entry(*container).or_default();
        for id in members {
            if !list.contains(id) {
                list.push(*id);
            }
        }
    }
    children
}
/// An iterative graph walk returns an actual cycle without recursive stack growth.
fn cycle(edges: &BTreeMap<EntityId, BTreeSet<EntityId>>) -> Option<Vec<EntityId>> {
    let mut colors: BTreeMap<EntityId, u8> = BTreeMap::new();
    for start in edges.keys() {
        if colors.get(start) == Some(&2) {
            continue;
        }
        let mut stack = vec![(
            *start,
            edges.get(start).cloned().unwrap_or_default().into_iter(),
        )];
        let mut path = vec![*start];
        colors.insert(*start, 1);
        while let Some((_, pending)) = stack.last_mut() {
            if let Some(next) = pending.next() {
                match colors.get(&next) {
                    Some(1) => {
                        let index = path.iter().position(|id| *id == next).unwrap();
                        let mut found = path[index..].to_vec();
                        found.push(next);
                        return Some(found);
                    }
                    Some(2) => {}
                    _ => {
                        colors.insert(next, 1);
                        path.push(next);
                        stack.push((
                            next,
                            edges.get(&next).cloned().unwrap_or_default().into_iter(),
                        ));
                    }
                }
            } else {
                let (id, _) = stack.pop().unwrap();
                colors.insert(id, 2);
                path.pop();
            }
        }
    }
    None
}
pub fn validate(document: &Document) -> Result<(), HierarchyError> {
    let root = document.id();
    let state = document.hierarchy();
    let mut explicit_parents = BTreeMap::new();
    for (container, members) in &state.members {
        if *container != root
            && !document
                .get(*container)
                .is_some_and(|entity| container_kind(&entity.data).is_some())
        {
            return Err(invalid(format!(
                "Hierarchy container {container} is missing or not a component/group"
            )));
        }
        let mut seen = BTreeSet::new();
        for id in members {
            let entity = document.get(*id).ok_or_else(|| {
                invalid(format!(
                    "Hierarchy member {id} is missing; detach it in the same transaction"
                ))
            })?;
            if !is_model_entry(&entity.data) {
                return Err(invalid(format!(
                    "Entity {id} cannot be a model-tree member"
                )));
            }
            if !seen.insert(*id) {
                return Err(invalid(format!(
                    "Duplicate member {id} in container {container}"
                )));
            }
            if explicit_parents.insert(*id, *container).is_some() {
                return Err(invalid(format!(
                    "Member {id} has multiple explicit containers"
                )));
            }
        }
    }
    let children = combined_children(document);
    for (parent, legacy) in legacy_children(document) {
        for child in legacy {
            if let Some(explicit) = explicit_parents.get(&child)
                && *explicit != parent
            {
                return Err(invalid(format!(
                    "Member {child} still belongs to legacy assembly {parent}; move it atomically"
                )));
            }
        }
    }
    let edges: BTreeMap<_, _> = children
        .iter()
        .map(|(id, members)| (*id, members.iter().copied().collect()))
        .collect();
    if let Some(path) = cycle(&edges) {
        return Err(HierarchyError::ContainmentCycle(path));
    }
    let mut expansion = edges;
    for entity in document.entities() {
        if let EntityData::Instance { component, .. } = entity.data {
            expansion.entry(entity.id).or_default().insert(component);
        }
    }
    if let Some(path) = cycle(&expansion) {
        return Err(HierarchyError::ExpansionCycle(path));
    }
    if let Some(active) = state.active_component
        && !document
            .get(active)
            .is_some_and(|entity| matches!(entity.data, EntityData::Assembly { .. }))
    {
        return Err(invalid(
            "The active component must be an existing Assembly, or root",
        ));
    }
    for hidden in &state.hidden {
        if *hidden != root
            && !document
                .get(*hidden)
                .is_some_and(|entity| is_model_entry(&entity.data))
        {
            return Err(invalid(format!(
                "Hidden model entry {hidden} is missing or unsupported"
            )));
        }
    }
    Ok(())
}

/// Organizational ancestors are invalidation recipients, without becoming
/// evaluation prerequisites or creating false feature dependency cycles.
pub(crate) fn invalidation_edges(document: &Document) -> BTreeMap<EntityId, Vec<EntityId>> {
    combined_children(document)
        .into_iter()
        .filter(|(parent, _)| *parent != document.id())
        .collect()
}

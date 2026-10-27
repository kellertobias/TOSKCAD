//! CAD-51 model boundary. Serde is a data interchange boundary, not the native
//! project container (migrations/recovery/kernel payloads belong to CAD-56).
mod graph;
mod model;
use crate::hierarchy::Hierarchy;
pub use graph::*;
pub use model::*;
use serde::{Deserialize, Serialize, de};
use std::collections::BTreeMap;

pub const DOCUMENT_SCHEMA: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Document {
    schema_version: u32,
    id: EntityId,
    name: String,
    entities: BTreeMap<EntityId, Entity>,
    #[serde(default, skip_serializing_if = "Hierarchy::is_empty")]
    hierarchy: Hierarchy,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireDocument {
    schema_version: u32,
    id: EntityId,
    name: String,
    #[serde(deserialize_with = "unique_entities")]
    entities: BTreeMap<EntityId, Entity>,
    #[serde(default)]
    hierarchy: Hierarchy,
}
fn unique_entities<'de, D: de::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<EntityId, Entity>, D::Error> {
    struct Visitor;
    impl<'de> de::Visitor<'de> for Visitor {
        type Value = BTreeMap<EntityId, Entity>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("unique entity identities")
        }
        fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut values = BTreeMap::new();
            while let Some((id, entity)) = map.next_entry::<EntityId, Entity>()? {
                if values.insert(id, entity).is_some() {
                    return Err(de::Error::custom(format!("Duplicate entity {id}")));
                }
            }
            Ok(values)
        }
    }
    deserializer.deserialize_map(Visitor)
}
impl<'de> Deserialize<'de> for Document {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = WireDocument::deserialize(deserializer)?;
        let value = Self {
            schema_version: wire.schema_version,
            id: wire.id,
            name: wire.name,
            entities: wire.entities,
            hierarchy: wire.hierarchy,
        };
        value.validate().map_err(de::Error::custom)?;
        Ok(value)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentError {
    Invalid(String),
    Duplicate(EntityId),
    Missing(EntityId),
    Cycle(Vec<EntityId>),
}
impl std::fmt::Display for DocumentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
            Self::Duplicate(id) => write!(f, "Duplicate entity {id}"),
            Self::Missing(id) => write!(f, "Entity {id} does not exist"),
            Self::Cycle(path) => write!(
                f,
                "Dependency cycle: {}",
                path.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ),
        }
    }
}
impl std::error::Error for DocumentError {}
impl Document {
    pub fn new(name: impl Into<String>) -> Result<Self, DocumentError> {
        let doc = Self {
            schema_version: DOCUMENT_SCHEMA,
            id: EntityId::new(),
            name: name.into(),
            entities: BTreeMap::new(),
            hierarchy: Hierarchy::default(),
        };
        doc.validate()?;
        Ok(doc)
    }
    pub fn id(&self) -> EntityId {
        self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }
    pub fn hierarchy(&self) -> &Hierarchy {
        &self.hierarchy
    }
    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }
    pub fn rename(&mut self, name: impl Into<String>) -> Result<(), DocumentError> {
        let name = name.into();
        valid_name(&name)?;
        self.name = name;
        Ok(())
    }
    pub fn insert(&mut self, entity: Entity) -> Result<(), DocumentError> {
        let id = entity.id;
        if self.entities.contains_key(&id) {
            return Err(DocumentError::Duplicate(id));
        }
        self.entities.insert(id, entity);
        if let Err(error) = self.validate() {
            self.entities.remove(&id);
            return Err(error);
        }
        Ok(())
    }
    pub fn replace(&mut self, entity: Entity) -> Result<(), DocumentError> {
        let id = entity.id;
        let previous = self
            .entities
            .get(&id)
            .ok_or(DocumentError::Missing(id))?
            .clone();
        self.entities.insert(id, entity);
        if let Err(error) = self.validate() {
            self.entities.insert(id, previous);
            return Err(error);
        }
        Ok(())
    }
    /// Dependents remain intact and produce actionable missing-reference
    /// diagnostics. Call affected() before/after removal to schedule invalidation.
    pub fn remove(&mut self, id: EntityId) -> Result<Entity, DocumentError> {
        self.entities.remove(&id).ok_or(DocumentError::Missing(id))
    }
    /// Validate structure and acyclicity. Missing/wrong-kind links intentionally
    /// remain repairable; evaluators must also check reference_diagnostics().
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.id.is_nil() {
            return Err(DocumentError::Invalid(
                "Document identity cannot be nil".into(),
            ));
        }
        if self.schema_version != DOCUMENT_SCHEMA {
            return Err(DocumentError::Invalid(format!(
                "Unsupported document schema {}",
                self.schema_version
            )));
        }
        valid_name(&self.name)?;
        for (id, entity) in &self.entities {
            if id.is_nil() || *id != entity.id || *id == self.id {
                return Err(DocumentError::Invalid(format!(
                    "Invalid identity mapping for {id}"
                )));
            }
            entity
                .validate()
                .map_err(|message| DocumentError::Invalid(format!("Entity {id}: {message}")))?;
        }
        self.topological_order()?;
        crate::hierarchy::validate(self)
            .map_err(|error| DocumentError::Invalid(error.to_string()))?;
        crate::sketch::validate(self)?;
        crate::constraint_sketch::source::validate(self)?;
        Ok(())
    }
}
fn valid_name(name: &str) -> Result<(), DocumentError> {
    if name.trim().is_empty() || name.len() > 1024 {
        Err(DocumentError::Invalid(
            "Name must contain 1–1024 bytes".into(),
        ))
    } else {
        Ok(())
    }
}

/// Ordered source edits committed only after the complete candidate validates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "operation",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Mutation {
    Insert(Entity),
    Replace(Entity),
    Remove(EntityId),
    RenameDocument(String),
    SetHierarchy(Hierarchy),
}
impl Document {
    pub fn edited(&self, edits: &[Mutation]) -> Result<Self, DocumentError> {
        let mut candidate = self.clone();
        for edit in edits {
            match edit {
                Mutation::Insert(entity) => {
                    if candidate.entities.contains_key(&entity.id) {
                        return Err(DocumentError::Duplicate(entity.id));
                    }
                    candidate.entities.insert(entity.id, entity.clone());
                }
                Mutation::Replace(entity) => {
                    if !candidate.entities.contains_key(&entity.id) {
                        return Err(DocumentError::Missing(entity.id));
                    }
                    candidate.entities.insert(entity.id, entity.clone());
                }
                Mutation::Remove(id) => {
                    if candidate.entities.remove(id).is_none() {
                        return Err(DocumentError::Missing(*id));
                    }
                }
                Mutation::RenameDocument(name) => candidate.name = name.clone(),
                Mutation::SetHierarchy(hierarchy) => candidate.hierarchy = hierarchy.clone(),
            }
        }
        candidate.validate()?;
        Ok(candidate)
    }
}

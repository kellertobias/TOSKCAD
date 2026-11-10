//! CAD-51 model boundary. Serde is a data interchange boundary, not the native
//! project container (migrations/recovery/kernel payloads belong to CAD-56).
mod graph;
mod model;
use crate::{hierarchy::Hierarchy, project_file::ContentHash};
pub use graph::*;
pub use model::*;
use serde::{Deserialize, Serialize, de};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

pub const DOCUMENT_SCHEMA: u32 = 1;

#[derive(Clone, Debug)]
pub struct Document(Arc<DocumentRevision>);

#[derive(Clone, Debug)]
struct DocumentRevision {
    source: DocumentSource,
    hash: OnceLock<ContentHash>,
    validated_order: OnceLock<Arc<[EntityId]>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename = "Document")]
struct DocumentSource {
    schema_version: u32,
    id: EntityId,
    name: String,
    entities: BTreeMap<EntityId, Entity>,
    #[serde(default, skip_serializing_if = "Hierarchy::is_empty")]
    hierarchy: Hierarchy,
}

impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.source == other.0.source
    }
}

impl Serialize for Document {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.source.serialize(serializer)
    }
}

/// A checked source borrow and its canonical dependency order. Holding the
/// borrow keeps both valid for the duration of an evaluation.
pub struct ValidatedDocument<'a> {
    document: &'a Document,
    order: &'a [EntityId],
}

impl<'a> ValidatedDocument<'a> {
    pub fn document(&self) -> &'a Document {
        self.document
    }

    pub fn topological_order(&self) -> &[EntityId] {
        self.order
    }
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
        let value = Self::from_source(DocumentSource {
            schema_version: wire.schema_version,
            id: wire.id,
            name: wire.name,
            entities: wire.entities,
            hierarchy: wire.hierarchy,
        });
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
    fn from_source(source: DocumentSource) -> Self {
        Self(Arc::new(DocumentRevision {
            source,
            hash: OnceLock::new(),
            validated_order: OnceLock::new(),
        }))
    }

    fn source_mut(&mut self) -> &mut DocumentSource {
        let revision = Arc::make_mut(&mut self.0);
        revision.hash.take();
        revision.validated_order.take();
        &mut revision.source
    }

    pub(crate) fn canonical_hash(&self) -> Result<ContentHash, String> {
        if let Some(hash) = self.0.hash.get() {
            return Ok(*hash);
        }
        let bytes = serde_json::to_vec(&self.0.source).map_err(|e| e.to_string())?;
        Ok(*self.0.hash.get_or_init(|| ContentHash::of(&bytes)))
    }

    pub fn new(name: impl Into<String>) -> Result<Self, DocumentError> {
        let doc = Self::from_source(DocumentSource {
            schema_version: DOCUMENT_SCHEMA,
            id: EntityId::new(),
            name: name.into(),
            entities: BTreeMap::new(),
            hierarchy: Hierarchy::default(),
        });
        doc.validate()?;
        Ok(doc)
    }
    pub fn id(&self) -> EntityId {
        self.0.source.id
    }
    pub fn name(&self) -> &str {
        &self.0.source.name
    }
    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        self.0.source.entities.get(&id)
    }
    pub fn hierarchy(&self) -> &Hierarchy {
        &self.0.source.hierarchy
    }
    /// Visibility and file information are authored presentation state.
    /// Reuse exact evaluation only if every model input still matches.
    pub fn differs_only_in_presentation(&self, other: &Self) -> bool {
        let source = &self.0.source;
        let other_source = &other.0.source;
        if source.schema_version != other_source.schema_version
            || source.id != other_source.id
            || source.hierarchy.members != other_source.hierarchy.members
            || source.hierarchy.active_component != other_source.hierarchy.active_component
        {
            return false;
        }
        if source.entities == other_source.entities {
            return source.name != other_source.name
                || source.hierarchy.hidden != other_source.hierarchy.hidden;
        }
        let is_file_info = |entity: &&Entity| {
            matches!(&entity.data,
                EntityData::Extension { definition, inputs }
                    if definition.type_id == crate::project_file::info::FILE_INFO_TYPE && inputs.is_empty()
            )
        };
        let metadata: std::collections::BTreeSet<_> = self
            .entities()
            .chain(other.entities())
            .filter(is_file_info)
            .map(|entity| entity.id)
            .collect();
        if metadata.is_empty() {
            return false;
        }
        let before = source
            .entities
            .values()
            .filter(|entity| !is_file_info(entity));
        let after = other_source
            .entities
            .values()
            .filter(|entity| !is_file_info(entity));
        before.clone().eq(after)
            && !before.clone().any(|entity| {
                entity
                    .references()
                    .iter()
                    .any(|link| metadata.contains(&link.target))
            })
    }
    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.0.source.entities.values()
    }
    pub fn rename(&mut self, name: impl Into<String>) -> Result<(), DocumentError> {
        let name = name.into();
        valid_name(&name)?;
        self.source_mut().name = name;
        Ok(())
    }
    pub fn insert(&mut self, entity: Entity) -> Result<(), DocumentError> {
        let id = entity.id;
        if self.get(id).is_some() {
            return Err(DocumentError::Duplicate(id));
        }
        self.source_mut().entities.insert(id, entity);
        if let Err(error) = self.validate() {
            self.source_mut().entities.remove(&id);
            return Err(error);
        }
        Ok(())
    }
    pub fn replace(&mut self, entity: Entity) -> Result<(), DocumentError> {
        let id = entity.id;
        let previous = self.get(id).ok_or(DocumentError::Missing(id))?.clone();
        self.source_mut().entities.insert(id, entity);
        if let Err(error) = self.validate() {
            self.source_mut().entities.insert(id, previous);
            return Err(error);
        }
        Ok(())
    }
    /// Dependents remain intact and produce actionable missing-reference
    /// diagnostics. Call affected() before/after removal to schedule invalidation.
    pub fn remove(&mut self, id: EntityId) -> Result<Entity, DocumentError> {
        self.source_mut()
            .entities
            .remove(&id)
            .ok_or(DocumentError::Missing(id))
    }
    /// Validate structure and acyclicity. Missing/wrong-kind links intentionally
    /// remain repairable; evaluators must also check reference_diagnostics().
    pub fn validate(&self) -> Result<(), DocumentError> {
        self.validated().map(drop)
    }

    pub fn validated(&self) -> Result<ValidatedDocument<'_>, DocumentError> {
        if let Some(order) = self.0.validated_order.get() {
            return Ok(ValidatedDocument {
                document: self,
                order,
            });
        }
        let source = &self.0.source;
        if source.id.is_nil() {
            return Err(DocumentError::Invalid(
                "Document identity cannot be nil".into(),
            ));
        }
        if source.schema_version != DOCUMENT_SCHEMA {
            return Err(DocumentError::Invalid(format!(
                "Unsupported document schema {}",
                source.schema_version
            )));
        }
        valid_name(&source.name)?;
        for (id, entity) in &source.entities {
            if id.is_nil() || *id != entity.id || *id == source.id {
                return Err(DocumentError::Invalid(format!(
                    "Invalid identity mapping for {id}"
                )));
            }
            entity
                .validate()
                .map_err(|message| DocumentError::Invalid(format!("Entity {id}: {message}")))?;
        }
        let order = self.topological_order()?;
        crate::hierarchy::validate(self)
            .map_err(|error| DocumentError::Invalid(error.to_string()))?;
        crate::sketch::validate(self)?;
        crate::constraint_sketch::source::validate(self)?;
        crate::model::validate(self)?;
        crate::drawings::validate(self)?;
        crate::materials::validate(self)?;
        crate::sketch_external::validate(self)?;
        crate::exchange::validate(self)?;
        crate::inspection::validate(self)?;
        crate::symbols::validate(self)?;
        crate::plants::validate(self)?;
        crate::equipment::validate(self)?;
        crate::sketch_import::provenance::validate(self)?;
        crate::project_file::info::validate(self)?;
        Ok(ValidatedDocument {
            document: self,
            order: self.0.validated_order.get_or_init(|| order.into()),
        })
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
            let source = candidate.source_mut();
            match edit {
                Mutation::Insert(entity) => {
                    if source.entities.contains_key(&entity.id) {
                        return Err(DocumentError::Duplicate(entity.id));
                    }
                    source.entities.insert(entity.id, entity.clone());
                }
                Mutation::Replace(entity) => {
                    if !source.entities.contains_key(&entity.id) {
                        return Err(DocumentError::Missing(entity.id));
                    }
                    source.entities.insert(entity.id, entity.clone());
                }
                Mutation::Remove(id) => {
                    if source.entities.remove(id).is_none() {
                        return Err(DocumentError::Missing(*id));
                    }
                }
                Mutation::RenameDocument(name) => source.name = name.clone(),
                Mutation::SetHierarchy(hierarchy) => source.hierarchy = hierarchy.clone(),
            }
        }
        candidate.validate()?;
        Ok(candidate)
    }
}

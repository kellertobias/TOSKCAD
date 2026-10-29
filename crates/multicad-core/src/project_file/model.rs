use crate::{
    document::{Document, EntityData, EntityId},
    parameters,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ContentHash(pub [u8; 32]);
impl ContentHash {
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
    pub fn document(document: &Document) -> Result<Self, String> {
        serde_json::to_vec(document)
            .map(|bytes| Self::of(&bytes))
            .map_err(|e| e.to_string())
    }
}
impl std::fmt::Display for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolPin {
    pub symbol: EntityId,
    pub revision: String,
    pub content: ContentHash,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PayloadDescriptor {
    pub id: EntityId,
    pub kind: String,
    pub schema_version: u32,
    pub media_type: String,
    pub owner: Option<EntityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_pin: Option<SymbolPin>,
    #[serde(default, flatten)]
    pub metadata: BTreeMap<String, Value>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Payload {
    pub descriptor: PayloadDescriptor,
    pub bytes: Arc<[u8]>,
}
impl Payload {
    pub fn new(
        kind: impl Into<String>,
        media_type: impl Into<String>,
        owner: Option<EntityId>,
        bytes: impl Into<Arc<[u8]>>,
    ) -> Self {
        Self {
            descriptor: PayloadDescriptor {
                id: EntityId::new(),
                kind: kind.into(),
                schema_version: 1,
                media_type: media_type.into(),
                owner,
                symbol_pin: None,
                metadata: BTreeMap::new(),
            },
            bytes: bytes.into(),
        }
    }
    pub fn pinned_symbol(
        symbol: EntityId,
        revision: impl Into<String>,
        project: &ProjectFile,
    ) -> Result<Self, String> {
        let mut bytes = Vec::new();
        super::write(project, &mut bytes, super::FileLimits::default())?;
        let mut payload = Self::new(
            "symbol.definition",
            "application/vnd.multicad.project",
            None,
            bytes,
        );
        payload.descriptor.symbol_pin = Some(SymbolPin {
            symbol,
            revision: revision.into(),
            content: ContentHash::of(&payload.bytes),
        });
        payload.validate()?;
        Ok(payload)
    }
    pub fn embedded_symbol(&self) -> Result<super::OpenedProject, String> {
        self.validate()?;
        if self.descriptor.kind != "symbol.definition" || self.descriptor.symbol_pin.is_none() {
            return Err("Payload is not a pinned symbol definition".into());
        }
        super::read(
            &mut self.bytes.as_ref(),
            super::FileLimits::default(),
            &supported_capabilities(),
        )
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        let d = &self.descriptor;
        if d.metadata.keys().any(|key| {
            [
                "id",
                "kind",
                "schema_version",
                "media_type",
                "owner",
                "symbol_pin",
            ]
            .contains(&key.as_str())
        }) {
            return Err("Payload metadata shadows a required descriptor field".into());
        }
        if d.id.is_nil()
            || d.schema_version == 0
            || d.kind.trim().is_empty()
            || d.kind.len() > 256
            || d.media_type.trim().is_empty()
            || d.media_type.len() > 256
            || d.owner.is_some_and(|id| id.is_nil())
        {
            return Err("Invalid project payload identity or descriptor".into());
        }
        if let Some(pin) = &d.symbol_pin
            && (pin.symbol.is_nil()
                || pin.revision.trim().is_empty()
                || pin.revision.len() > 1024
                || pin.content != ContentHash::of(&self.bytes)
                || d.kind != "symbol.definition")
        {
            return Err("Pinned symbol identity or content hash is invalid".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheKey {
    pub source: ContentHash,
    pub producer: String,
    pub producer_version: String,
    pub schema_version: u32,
    pub dependencies: BTreeMap<String, ContentHash>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}
impl CacheKey {
    pub fn new(
        document: &Document,
        producer: impl Into<String>,
        producer_version: impl Into<String>,
        schema_version: u32,
        mut dependencies: BTreeMap<String, ContentHash>,
    ) -> Result<Self, String> {
        dependencies.insert(
            "multicad.core.semantics".into(),
            ContentHash::of(
                b"document.1;plane.offset.1;hierarchy.1;constrained_sketch.2;constrained_sketch.3;drawings.book.1;constrained_sketch.4;sketch.external.1;sketch.external.2;inspection.measurement.1;symbols.1",
            ),
        );
        Ok(Self {
            source: ContentHash::document(document)?,
            producer: producer.into(),
            producer_version: producer_version.into(),
            schema_version,
            dependencies,
            extensions: BTreeMap::new(),
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct DerivedCache {
    pub descriptor: PayloadDescriptor,
    pub key: CacheKey,
    pub(crate) bytes: Arc<[u8]>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebuildReason {
    SourceChanged,
    ProducerChanged,
    SchemaChanged,
    DependenciesChanged,
}
impl DerivedCache {
    pub fn new(payload: Payload, key: CacheKey) -> Self {
        Self {
            descriptor: payload.descriptor,
            key,
            bytes: payload.bytes,
        }
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        Payload {
            descriptor: self.descriptor.clone(),
            bytes: self.bytes.clone(),
        }
        .validate()?;
        if self.key.schema_version == 0
            || self.key.producer.trim().is_empty()
            || self.key.producer.len() > 256
            || self.key.producer_version.trim().is_empty()
            || self.key.producer_version.len() > 1024
            || self.key.dependencies.len() > 1024
            || self.key.dependencies.keys().any(|k| k.len() > 256)
            || self.key.extensions.keys().any(|key| {
                [
                    "source",
                    "producer",
                    "producer_version",
                    "schema_version",
                    "dependencies",
                ]
                .contains(&key.as_str())
            })
        {
            return Err("Invalid derived cache producer or key".into());
        }
        Ok(())
    }
    /// No unvalidated cache accessor is provided. Providers compute the current
    /// expected key from source, their own version and all external dependencies.
    pub fn validated(&self, expected: &CacheKey) -> Result<&[u8], RebuildReason> {
        if self.key.source != expected.source {
            return Err(RebuildReason::SourceChanged);
        }
        if self.key.producer != expected.producer
            || self.key.producer_version != expected.producer_version
        {
            return Err(RebuildReason::ProducerChanged);
        }
        if self.key.schema_version != expected.schema_version {
            return Err(RebuildReason::SchemaChanged);
        }
        if self.key.dependencies != expected.dependencies
            || self.key.extensions != expected.extensions
        {
            return Err(RebuildReason::DependenciesChanged);
        }
        Ok(&self.bytes)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryInfo {
    pub id: EntityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<EntityId>,
    pub created_unix_ms: u64,
    pub source_revision: u64,
    pub original_path: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectResources {
    pub payloads: BTreeMap<EntityId, Payload>,
    pub caches: BTreeMap<EntityId, DerivedCache>,
    pub metadata: BTreeMap<String, Value>,
    pub required_capabilities: BTreeSet<String>,
}
impl ProjectResources {
    /// A digest of authored resources: payload descriptors and bytes, project
    /// metadata and required capabilities. Derived caches are excluded.
    pub fn fingerprint(&self) -> String {
        let mut hash = Sha256::new();
        for (id, payload) in &self.payloads {
            hash.update(id.to_string());
            hash.update(serde_json::to_vec(&payload.descriptor).unwrap_or_default());
            hash.update(ContentHash::of(&payload.bytes).0);
        }
        hash.update(serde_json::to_vec(&self.metadata).unwrap_or_default());
        hash.update(serde_json::to_vec(&self.required_capabilities).unwrap_or_default());
        ContentHash(hash.finalize().into()).to_string()
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectFile {
    pub document: Document,
    pub resources: ProjectResources,
    pub recovery: Option<RecoveryInfo>,
}
impl ProjectFile {
    /// Conservative default: every critical payload and core capability is a
    /// dependency, so pinned symbols and embedded kernel edits invalidate caches.
    pub fn cache_key(
        &self,
        producer: impl Into<String>,
        version: impl Into<String>,
        schema: u32,
        mut dependencies: BTreeMap<String, ContentHash>,
    ) -> Result<CacheKey, String> {
        for (id, payload) in &self.resources.payloads {
            dependencies.insert(format!("payload:{id}"), ContentHash::of(&payload.bytes));
        }
        dependencies.insert(
            "multicad.core.capabilities".into(),
            ContentHash::of(&serde_json::to_vec(&self.capabilities()).map_err(|e| e.to_string())?),
        );
        CacheKey::new(&self.document, producer, version, schema, dependencies)
    }
    pub fn new(document: Document) -> Result<Self, String> {
        document.validate().map_err(|e| e.to_string())?;
        let document = parameters::evaluate(&document)
            .map_err(|e| e.to_string())?
            .document;
        Ok(Self {
            document,
            resources: ProjectResources::default(),
            recovery: None,
        })
    }
    pub fn with_resources(document: Document, resources: ProjectResources) -> Result<Self, String> {
        let mut project = Self::new(document)?;
        project.resources = resources;
        project.validate()?;
        Ok(project)
    }
    pub fn capabilities(&self) -> BTreeSet<String> {
        let mut required = self.resources.required_capabilities.clone();
        required.insert("core.document.1".into());
        if self.document.entities().any(|e|matches!(&e.data,EntityData::Plane{anchor:Some(link),..} if link.role=="offset_plane")) {required.insert("core.plane.offset.1".into());}
        if crate::hierarchy::required_capability(&self.document) {
            required.insert(crate::hierarchy::CAPABILITY.into());
        }
        if crate::constraint_sketch::source::requires_v2(&self.document) {
            required.insert(crate::constraint_sketch::source::V2_CAPABILITY.into());
        }
        if crate::constraint_sketch::source::requires_v3(&self.document) {
            required.insert(crate::constraint_sketch::source::V3_CAPABILITY.into());
        }
        if crate::constraint_sketch::source::requires_v4(&self.document) {
            required.insert(crate::constraint_sketch::source::V4_CAPABILITY.into());
        }
        if crate::sketch_external::required_capability(&self.document) {
            required.insert(crate::sketch_external::CAPABILITY.into());
        }
        required.extend(crate::model::required_capabilities(&self.document));
        if self
            .document
            .entities()
            .any(|e| crate::sketch_external::is_occurrence_reference(&self.document, e.id))
        {
            required.insert(crate::sketch_external::OCCURRENCE_CAPABILITY.into());
        }
        if crate::exchange::required_capability(&self.document) {
            required.insert(crate::exchange::CAPABILITY.into());
        }
        if crate::exchange::sets(&self.document).any(|id| {
            crate::exchange::read(&self.document, id).is_ok_and(|s| !s.planar_faces.is_empty())
        }) {
            required.insert(crate::exchange::PLANAR_CAPABILITY.into());
        }
        if crate::sketch_import::provenance::required_capability(&self.document) {
            required.insert(crate::sketch_import::CAPABILITY.into());
        }
        if crate::drawings::required_capability(&self.document) {
            required.insert(crate::drawings::CAPABILITY.into());
        }
        if crate::drawings::books(&self.document).any(|id| {
            self.document.get(id).is_some_and(|e| matches!(&e.data, crate::document::EntityData::Drawing{definition,..} if definition.schema_version >= 2))
        }) {
            required.insert(crate::drawings::VIEW_CAPABILITY.into());
        }
        if crate::drawings::books(&self.document).any(|id| {
            crate::drawings::read(&self.document, id)
                .is_ok_and(|book| crate::drawings::has_section_limits(&book))
        }) {
            required.insert(crate::drawings::SECTION_CAPABILITY.into());
        }
        if crate::drawings::books(&self.document).any(|id| {
            crate::drawings::read(&self.document, id)
                .is_ok_and(|book| crate::drawings::has_annotations(&book))
        }) {
            required.insert(crate::drawings::ANNOTATION_CAPABILITY.into());
        }
        if crate::drawings::guides(&self.document).next().is_some() {
            required.insert(crate::drawings::GUIDE_CAPABILITY.into());
        }
        if crate::drawings::books(&self.document).any(|id| {
            crate::drawings::read(&self.document, id)
                .is_ok_and(|book| crate::drawings::has_guided_views(&book))
        }) {
            required.insert(crate::drawings::GUIDED_VIEW_CAPABILITY.into());
        }
        if crate::inspection::required_capability(&self.document) {
            required.insert(crate::inspection::CAPABILITY.into());
        }
        if crate::materials::required_capability(&self.document) {
            required.insert(crate::materials::CAPABILITY.into());
        }
        if self.document.entities().any(|entity| matches!(&entity.data,EntityData::Extension{definition,..} if matches!(definition.type_id.as_str(),crate::symbols::DEFINITION_TYPE|crate::symbols::INSTANCE_TYPE))) {
            required.insert(crate::symbols::CAPABILITY.into());
        }
        required
    }
    pub fn validate(&self) -> Result<(), String> {
        self.document.validate().map_err(|e| e.to_string())?;
        parameters::evaluate(&self.document).map_err(|e| e.to_string())?;
        let mut ids = BTreeSet::new();
        for (id, payload) in &self.resources.payloads {
            payload.validate()?;
            if *id != payload.descriptor.id || !ids.insert(*id) {
                return Err("Duplicate or mismatched project payload identity".into());
            }
        }
        for (id, cache) in &self.resources.caches {
            cache.validate()?;
            if *id != cache.descriptor.id || !ids.insert(*id) {
                return Err("Invalid derived cache identity or producer".into());
            }
        }
        if self.resources.metadata.keys().any(|key| {
            [
                "identity",
                "source",
                "required_capabilities",
                "payloads",
                "caches",
                "recovery",
            ]
            .contains(&key.as_str())
        }) {
            return Err("Project metadata shadows a required manifest field".into());
        }
        let capabilities = self.capabilities();
        if capabilities.len() > 128
            || capabilities
                .iter()
                .any(|c| c.trim().is_empty() || c.len() > 256)
        {
            return Err("Invalid required core capability list".into());
        }
        if self
            .recovery
            .as_ref()
            .is_some_and(|r| r.id.is_nil() || r.slot.is_some_and(|slot| slot != r.id))
        {
            return Err("Invalid recovery identity".into());
        }
        Ok(())
    }
}
pub fn supported_capabilities() -> BTreeSet<String> {
    BTreeSet::from([
        crate::symbols::CAPABILITY.into(),
        "core.document.1".into(),
        crate::drawings::CAPABILITY.into(),
        crate::drawings::VIEW_CAPABILITY.into(),
        crate::drawings::SECTION_CAPABILITY.into(),
        crate::drawings::ANNOTATION_CAPABILITY.into(),
        crate::drawings::GUIDE_CAPABILITY.into(),
        crate::drawings::GUIDED_VIEW_CAPABILITY.into(),
        "core.plane.offset.1".into(),
        crate::hierarchy::CAPABILITY.into(),
        crate::constraint_sketch::source::V2_CAPABILITY.into(),
        crate::constraint_sketch::source::V3_CAPABILITY.into(),
        crate::materials::CAPABILITY.into(),
        crate::inspection::CAPABILITY.into(),
        crate::constraint_sketch::source::V4_CAPABILITY.into(),
        crate::sketch_external::CAPABILITY.into(),
        crate::sketch_external::OCCURRENCE_CAPABILITY.into(),
        crate::exchange::CAPABILITY.into(),
        crate::exchange::PLANAR_CAPABILITY.into(),
        crate::sketch_import::CAPABILITY.into(),
        crate::model::CAPABILITY.into(),
        crate::model::PRIMITIVE_CAPABILITY.into(),
        crate::model::REVOLVE_CAPABILITY.into(),
        crate::model::BOOLEAN_CAPABILITY.into(),
        crate::model::SPLIT_CAPABILITY.into(),
        crate::model::TRANSFORM_CAPABILITY.into(),
        crate::model::SCALE_CAPABILITY.into(),
        crate::model::EDGE_TREATMENT_CAPABILITY.into(),
        crate::model::EDGE_TREATMENT_V2_CAPABILITY.into(),
        crate::model::ADVANCED_CAPABILITY.into(),
        crate::model::TRANSFER_CAPABILITY.into(),
        crate::model::TRANSFER_GROUP_CAPABILITY.into(),
        crate::model::JOINT_CAPABILITY.into(),
        crate::model::ASSEMBLY_CAPABILITY.into(),
        crate::model::NAMED_POSE_CAPABILITY.into(),
        crate::model::ALIGN_CAPABILITY.into(),
        crate::model::INSTANCE_PLACEMENT_CAPABILITY.into(),
        crate::model::INSTANCE_ALIGNMENT_CAPABILITY.into(),
        crate::model::PART_REFERENCE_CAPABILITY.into(),
        crate::model::EXTRUDE_GROUP_CAPABILITY.into(),
        crate::model::EXTRUDE_GROUP_V2_CAPABILITY.into(),
        crate::model::WALL_CAPABILITY.into(),
        crate::model::WALL_TYPE_CAPABILITY.into(),
        crate::model::OPENING_TYPE_CAPABILITY.into(),
        crate::model::LEVEL_CAPABILITY.into(),
        crate::model::SLAB_CAPABILITY.into(),
        crate::model::STAIR_CAPABILITY.into(),
        crate::model::ENVELOPE_CAPABILITY.into(),
        crate::model::PANEL_CAPABILITY.into(),
        crate::model::CARCASS_CAPABILITY.into(),
        crate::model::STACK_CAPABILITY.into(),
    ])
}

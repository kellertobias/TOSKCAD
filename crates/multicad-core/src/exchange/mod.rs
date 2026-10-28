//! Saved output intent and snapshot queries. Native providers own manufacturing
//! geometry and filesystem publication; display state is never an export input.
mod evaluation;
mod names;
mod resolve;
mod source;
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    document::{Document, DocumentError, EntityData, EntityId, Mutation},
};
pub use evaluation::*;
pub use names::*;
pub use resolve::*;
pub use source::*;

pub const TYPE_ID: &str = "exchange.export_set";
pub const CAPABILITY: &str = "exchange.export_set.1";
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Invalid,
    Missing,
    Unsupported,
    Stale,
    Budget,
    Empty,
    Io,
    Cancelled,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportError {
    pub kind: ErrorKind,
    pub entity: Option<EntityId>,
    pub message: String,
}
impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ExportError {}
impl ExportError {
    pub fn new(kind: ErrorKind, entity: Option<EntityId>, message: impl Into<String>) -> Self {
        Self {
            kind,
            entity,
            message: message.into(),
        }
    }
}
impl From<DocumentError> for ExportError {
    fn from(e: DocumentError) -> Self {
        Self::new(ErrorKind::Invalid, None, e.to_string())
    }
}
pub fn sets(document: &Document) -> impl Iterator<Item = EntityId> + '_ {
    document.entities().filter_map(|e| {
        matches!(&e.data,EntityData::ExportSet{definition,..} if definition.type_id==TYPE_ID)
            .then_some(e.id)
    })
}
pub fn prepare_set(
    session: &Session,
    revision: u64,
    actor: Actor,
    id: EntityId,
    name: &str,
    definition: &SetDefinition,
) -> Result<Prepared, CommandError> {
    let entity = definition.entity(id, name)?;
    if let Some(existing) = session.document().get(id)
        && !matches!(&existing.data,EntityData::ExportSet{definition,..} if definition.type_id==TYPE_ID)
    {
        return Err(DocumentError::Invalid(format!("{id} is not an export set")).into());
    }
    let mutation = if session.document().get(id).is_some() {
        Mutation::Replace(entity)
    } else {
        Mutation::Insert(entity)
    };
    let preview = session.prepare(Request {
        expected_revision: revision,
        actor,
        label: "Save export set".into(),
        edits: vec![mutation],
    })?;
    resolve(preview.document(), revision, id).map_err(|e| DocumentError::Invalid(e.to_string()))?;
    Ok(preview)
}
pub fn required_capability(document: &Document) -> bool {
    sets(document).next().is_some()
}
pub fn validate(document: &Document) -> Result<(), DocumentError> {
    for id in sets(document) {
        read(document, id).map_err(|e| DocumentError::Invalid(e.to_string()))?;
    }
    Ok(())
}

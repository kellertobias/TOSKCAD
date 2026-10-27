//! Source transactions shared by UI, command-line and external control adapters.
use crate::{
    document::{Document, DocumentError, EntityId, Mutation},
    parameters::{self, EvaluatedParameters, ParameterError},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Ui,
    CommandLine,
    Mcp,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub id: String,
    pub channel: Channel,
}
impl Actor {
    pub fn local(channel: Channel) -> Self {
        Self {
            id: "local-user".into(),
            channel,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub expected_revision: u64,
    pub actor: Actor,
    pub label: String,
    pub edits: Vec<Mutation>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum ErrorCode {
    StaleRevision,
    ForeignPreview,
    InvalidRequest,
    InvalidDocument,
    InvalidParameter,
    NoHistory,
    RevisionLimit,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CommandError {
    pub code: ErrorCode,
    pub message: String,
    pub involved: Vec<EntityId>,
    pub parameter: Option<ParameterError>,
}
impl CommandError {
    fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            involved: vec![],
            parameter: None,
        }
    }
}
impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for CommandError {}
impl From<DocumentError> for CommandError {
    fn from(error: DocumentError) -> Self {
        Self::new(ErrorCode::InvalidDocument, error.to_string())
    }
}
impl From<ParameterError> for CommandError {
    fn from(error: ParameterError) -> Self {
        Self {
            code: ErrorCode::InvalidParameter,
            message: error.to_string(),
            involved: error.entity.into_iter().collect(),
            parameter: Some(error),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Change {
    pub revision: u64,
    pub changed: Vec<EntityId>,
    pub invalidated: Vec<EntityId>,
    pub rebuild_order: Vec<EntityId>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ActivityKind {
    Apply,
    Undo,
    Redo,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Activity {
    pub actor: Actor,
    pub label: String,
    pub kind: ActivityKind,
    pub change: Change,
}
#[derive(Clone, Debug)]
struct History {
    before: Document,
    after: Document,
    label: String,
}
/// An owned, attributable candidate. Dropping it cancels without side effects.
#[derive(Clone, Debug)]
pub struct Prepared {
    session: EntityId,
    base_revision: u64,
    actor: Actor,
    label: String,
    evaluated: EvaluatedParameters,
}
impl Prepared {
    pub fn document(&self) -> &Document {
        &self.evaluated.document
    }
    pub fn parameters(&self) -> &EvaluatedParameters {
        &self.evaluated
    }
    pub fn actor(&self) -> &Actor {
        &self.actor
    }
    pub fn label(&self) -> &str {
        &self.label
    }
    pub fn base_revision(&self) -> u64 {
        self.base_revision
    }
}
pub struct Session {
    id: EntityId,
    current: EvaluatedParameters,
    revision: u64,
    saved: Option<Document>,
    undo: VecDeque<History>,
    redo: VecDeque<History>,
    activity: VecDeque<Activity>,
}
fn difference(before: &Document, after: &Document, revision: u64) -> Result<Change, CommandError> {
    let ids: BTreeSet<_> = before
        .entities()
        .chain(after.entities())
        .map(|e| e.id)
        .collect();
    let changed: Vec<_> = ids
        .into_iter()
        .filter(|id| before.get(*id) != after.get(*id))
        .collect();
    let mut invalidated: BTreeSet<_> = changed.iter().copied().collect();
    invalidated.extend(before.affected(changed.iter().copied())?);
    let rebuild_order = after.affected(changed.iter().copied())?;
    invalidated.extend(rebuild_order.iter().copied());
    Ok(Change {
        revision,
        changed,
        invalidated: invalidated.into_iter().collect(),
        rebuild_order,
    })
}
impl Session {
    pub fn new(document: Document) -> Result<Self, CommandError> {
        Ok(Self {
            id: EntityId::new(),
            current: parameters::evaluate(&document)?,
            revision: 0,
            saved: None,
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            activity: VecDeque::new(),
        })
    }
    pub fn document(&self) -> &Document {
        &self.current.document
    }
    pub fn parameters(&self) -> &EvaluatedParameters {
        &self.current
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn dirty(&self) -> bool {
        self.saved.as_ref() != Some(self.document())
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn activity(&self) -> impl Iterator<Item = &Activity> {
        self.activity.iter()
    }
    fn check_revision(&self, expected: u64) -> Result<(), CommandError> {
        if expected != self.revision {
            Err(CommandError::new(
                ErrorCode::StaleRevision,
                format!(
                    "Stale revision {expected}; current revision is {}",
                    self.revision
                ),
            ))
        } else {
            Ok(())
        }
    }
    fn next_revision(&self) -> Result<u64, CommandError> {
        self.revision.checked_add(1).ok_or_else(|| {
            CommandError::new(ErrorCode::RevisionLimit, "Document revision limit reached")
        })
    }
    pub fn mark_saved(&mut self, expected: u64) -> Result<(), CommandError> {
        self.check_revision(expected)?;
        self.saved = Some(self.document().clone());
        Ok(())
    }
    /// Acknowledge the exact immutable snapshot written by a completed file job.
    /// Later source edits and history remain untouched; dirty compares against disk.
    pub fn acknowledge_saved(&mut self, snapshot: &Document) -> Result<(), CommandError> {
        if snapshot.id() != self.document().id() {
            return Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "Saved snapshot belongs to another document",
            ));
        }
        let evaluated = parameters::evaluate(snapshot)?;
        self.saved = Some(evaluated.document);
        Ok(())
    }
    fn validate_actor(actor: &Actor) -> Result<(), CommandError> {
        if actor.id.trim().is_empty() || actor.id.len() > 256 {
            Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "Invalid actor identity",
            ))
        } else {
            Ok(())
        }
    }
    pub fn prepare(&self, request: Request) -> Result<Prepared, CommandError> {
        self.check_revision(request.expected_revision)?;
        if request.actor.id.trim().is_empty()
            || request.actor.id.len() > 256
            || request.label.trim().is_empty()
            || request.label.len() > 1024
            || request.edits.len() > 4096
        {
            return Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "Invalid actor, label or transaction size",
            ));
        }
        let involved: Vec<_> = request
            .edits
            .iter()
            .filter_map(|m| match m {
                Mutation::Insert(e) | Mutation::Replace(e) => Some(e.id),
                Mutation::Remove(id) => Some(*id),
                Mutation::RenameDocument(_) => None,
            })
            .collect();
        let candidate = self.document().edited(&request.edits).map_err(|error| {
            let mut error = CommandError::from(error);
            error.involved = involved.clone();
            error
        })?;
        let evaluated = parameters::evaluate(&candidate).map_err(|error| {
            let mut error = CommandError::from(error);
            error.involved.extend(involved);
            error.involved.sort();
            error.involved.dedup();
            error
        })?;
        Ok(Prepared {
            session: self.id,
            base_revision: self.revision,
            actor: request.actor,
            label: request.label,
            evaluated,
        })
    }
    pub fn commit(&mut self, preview: Prepared) -> Result<Change, CommandError> {
        if preview.session != self.id {
            return Err(CommandError::new(
                ErrorCode::ForeignPreview,
                "Preview belongs to a different session",
            ));
        }
        self.check_revision(preview.base_revision)?;
        if preview.document() == self.document() {
            return difference(self.document(), self.document(), self.revision);
        }
        let revision = self.next_revision()?;
        let change = difference(self.document(), preview.document(), revision)?;
        self.undo.push_back(History {
            before: self.document().clone(),
            after: preview.document().clone(),
            label: preview.label.clone(),
        });
        if self.undo.len() > 128 {
            self.undo.pop_front();
        }
        self.redo.clear();
        self.current = preview.evaluated;
        self.revision = revision;
        self.record(Activity {
            actor: preview.actor,
            label: preview.label,
            kind: ActivityKind::Apply,
            change: change.clone(),
        });
        Ok(change)
    }
    pub fn apply(&mut self, request: Request) -> Result<Change, CommandError> {
        let prepared = self.prepare(request)?;
        self.commit(prepared)
    }
    pub fn undo(&mut self, expected: u64, actor: Actor) -> Result<Change, CommandError> {
        self.check_revision(expected)?;
        Self::validate_actor(&actor)?;
        let revision = self.next_revision()?;
        let history = self
            .undo
            .back()
            .ok_or_else(|| CommandError::new(ErrorCode::NoHistory, "Nothing to undo"))?;
        let evaluated = parameters::evaluate(&history.before)?;
        let change = difference(self.document(), &evaluated.document, revision)?;
        let history = self.undo.pop_back().unwrap();
        let label = format!("Undo: {}", history.label);
        self.redo.push_back(history);
        self.current = evaluated;
        self.revision = revision;
        self.record(Activity {
            actor,
            label,
            kind: ActivityKind::Undo,
            change: change.clone(),
        });
        Ok(change)
    }
    pub fn redo(&mut self, expected: u64, actor: Actor) -> Result<Change, CommandError> {
        self.check_revision(expected)?;
        Self::validate_actor(&actor)?;
        let revision = self.next_revision()?;
        let history = self
            .redo
            .back()
            .ok_or_else(|| CommandError::new(ErrorCode::NoHistory, "Nothing to redo"))?;
        let evaluated = parameters::evaluate(&history.after)?;
        let change = difference(self.document(), &evaluated.document, revision)?;
        let history = self.redo.pop_back().unwrap();
        let label = format!("Redo: {}", history.label);
        self.undo.push_back(history);
        self.current = evaluated;
        self.revision = revision;
        self.record(Activity {
            actor,
            label,
            kind: ActivityKind::Redo,
            change: change.clone(),
        });
        Ok(change)
    }
    fn record(&mut self, event: Activity) {
        self.activity.push_back(event);
        if self.activity.len() > 1024 {
            self.activity.pop_front();
        }
    }
}

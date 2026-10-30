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

/// Immutable command preparation input for a bounded background worker.
/// Prepared results retain the originating Session identity and base revision;
/// only that live Session can commit them after normal freshness checks.
pub struct PreparationSnapshot {
    session: Session,
}
impl PreparationSnapshot {
    /// Existing preparation/query adapters can borrow this source, but cannot
    /// mutate it or obtain an independent document/history owner.
    ///
    /// ```compile_fail
    /// use multicad_core::{commands::Session, document::Document};
    /// let session = Session::new(Document::new("Example").unwrap()).unwrap();
    /// let snapshot = session.preparation_snapshot();
    /// snapshot.session().mark_saved(0).unwrap();
    /// ```
    pub fn session(&self) -> &Session {
        &self.session
    }
}
fn difference(before: &Document, after: &Document, revision: u64) -> Result<Change, CommandError> {
    let ids: BTreeSet<_> = before
        .entities()
        .chain(after.entities())
        .map(|e| e.id)
        .collect();
    let hierarchy_changed = crate::hierarchy::changed_entries(before, after);
    let changed: Vec<_> = ids
        .into_iter()
        .filter(|id| before.get(*id) != after.get(*id) || hierarchy_changed.contains(id))
        .collect();
    let mut invalidated: BTreeSet<_> = changed.iter().copied().collect();
    invalidated.extend(before.invalidated(changed.iter().copied())?);
    let rebuild_order = after.invalidated(changed.iter().copied())?;
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
    pub fn preparation_snapshot(&self) -> PreparationSnapshot {
        PreparationSnapshot {
            session: Self {
                id: self.id,
                current: self.current.clone(),
                revision: self.revision,
                saved: None,
                undo: VecDeque::new(),
                redo: VecDeque::new(),
                activity: VecDeque::new(),
            },
        }
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
    pub fn prepare_scoped(&self, mut request: Request) -> Result<Prepared, CommandError> {
        self.check_revision(request.expected_revision)?;
        crate::hierarchy::scope_insertions(self.document(), &mut request.edits)
            .map_err(|error| CommandError::new(ErrorCode::InvalidRequest, error.to_string()))?;
        self.prepare(request)
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
                Mutation::RenameDocument(_) | Mutation::SetHierarchy(_) => None,
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
    /// Check preview scope and freshness without committing or altering history.
    /// A consumer can inspect/adopt the same owned preview before final Apply.
    pub fn validate_preview(&self, preview: &Prepared) -> Result<(), CommandError> {
        if preview.session != self.id {
            return Err(CommandError::new(
                ErrorCode::ForeignPreview,
                "Preview belongs to a different session",
            ));
        }
        self.check_revision(preview.base_revision)?;
        Ok(())
    }
    pub fn commit(&mut self, preview: Prepared) -> Result<Change, CommandError> {
        self.validate_preview(&preview)?;
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
    /// Replace the file preview image (CAD-141 save-time capture) without an
    /// Undo entry. The preview is presentation state: it is written into the
    /// current source and into every Undo/Redo snapshot, so Undo and Redo never
    /// bring back an older picture and never make the file dirty by themselves.
    /// The revision advances so outstanding previews and editors revalidate.
    pub fn replace_file_preview(
        &mut self,
        preview: Option<crate::project_file::info::PreviewImage>,
        actor: Actor,
    ) -> Result<Change, CommandError> {
        Self::validate_actor(&actor)?;
        let invalid = |message: String| CommandError::new(ErrorCode::InvalidRequest, message);
        let document = crate::project_file::info::with_preview(self.document(), preview.clone())
            .map_err(invalid)?;
        if &document == self.document() {
            return difference(self.document(), self.document(), self.revision);
        }
        let evaluated = parameters::evaluate(&document)?;
        let revision = self.next_revision()?;
        let change = difference(self.document(), &evaluated.document, revision)?;
        for history in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            for snapshot in [&mut history.before, &mut history.after] {
                // A snapshot whose information cannot be edited (newer schema)
                // keeps its own preview.
                if let Ok(updated) =
                    crate::project_file::info::with_preview(snapshot, preview.clone())
                {
                    *snapshot = updated;
                }
            }
        }
        self.current = evaluated;
        self.revision = revision;
        self.record(Activity {
            actor,
            label: "Capture file preview".into(),
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

//! Owner-thread jobs for external control adapters. An adapter never holds
//! a Session itself: it submits a job, and whoever owns the canonical
//! document (a headless server or the desktop window) runs it in place.
//!
//! The owner also keeps a [`ControlLedger`]: an append-only audit of every
//! external request and its outcome, and the queue of changes awaiting review.
use crate::{
    commands::{ActivityKind, Actor, Prepared, Session},
    document::EntityId,
    project_file::ProjectResources,
};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
};

/// Whether an actor's changes commit directly or wait for the owner's review.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewPolicy {
    Trusted,
    Reviewed,
}

/// The canonical document owner a control job runs against.
pub trait SessionOwner {
    fn session(&self) -> &Session;
    fn session_mut(&mut self) -> &mut Session;
    fn resources(&self) -> &ProjectResources;
    fn project_path(&self) -> Option<&Path>;
    /// Why external edits must wait, such as an unapplied draft in the
    /// window. Inspection stays available; commits, Undo and Redo do not.
    fn edit_blocked(&self) -> Option<String> {
        None
    }
    /// Persist the current canonical document to the owner's project path.
    fn save(&mut self) -> Result<PathBuf, String>;
    fn review_policy(&self, _actor: &Actor) -> ReviewPolicy {
        ReviewPolicy::Trusted
    }
    /// The Session and its ledger together, for queueing and accepting.
    fn control(&mut self) -> (&mut Session, &mut ControlLedger);
    fn ledger(&self) -> &ControlLedger;
}

/// The actor of an MCP client, from the name it gave at initialization.
pub fn mcp_actor(client: &str) -> Actor {
    let name = client.trim();
    let name = if name.is_empty() { "unnamed" } else { name };
    Actor {
        id: format!("mcp:{}", name.chars().take(64).collect::<String>()),
        channel: crate::commands::Channel::Mcp,
    }
}

/// Runs exactly once on the owner thread, with the owner or the reason it
/// is unavailable (for example, the controlled project was closed).
pub type ControlJob = Box<dyn FnOnce(Result<&mut dyn SessionOwner, String>) + Send>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    /// A read-only request.
    Inspected,
    /// A checked candidate queued for review; nothing changed yet.
    Previewed,
    Applied,
    Rejected,
    Cancelled,
    Failed,
    Undone,
    Redone,
    /// A queued change whose base revision is no longer current. It cannot
    /// apply; the client must prepare it again.
    Stale,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AuditEntry {
    pub sequence: u64,
    pub actor: Actor,
    /// The tool or command; empty for window actions imported from history.
    pub tool: String,
    pub arguments: Value,
    pub label: String,
    pub base_revision: u64,
    /// The revision after the action, when it changed the document.
    pub revision: Option<u64>,
    pub outcome: AuditOutcome,
    pub review: Option<u64>,
    pub affected: Vec<EntityId>,
    pub message: String,
}

#[derive(Debug)]
pub enum PendingAction {
    Commit(Box<Prepared>),
    Undo,
    Redo,
}
/// A change an external client asked for, held until the owner accepts or
/// rejects it. The candidate was fully prepared against `base_revision`.
#[derive(Debug)]
pub struct PendingChange {
    pub id: u64,
    pub actor: Actor,
    pub tool: String,
    pub arguments: Value,
    pub label: String,
    pub base_revision: u64,
    pub action: PendingAction,
    /// Entities the candidate changes, from a diff against the base.
    pub affected: Vec<EntityId>,
}
impl PendingChange {
    pub fn candidate(&self) -> Option<&Prepared> {
        match &self.action {
            PendingAction::Commit(prepared) => Some(prepared),
            _ => None,
        }
    }
    pub fn is_stale(&self, session: &Session) -> bool {
        self.base_revision != session.revision()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ReviewState {
    Pending,
    Applied { revision: u64 },
    Rejected,
    Stale,
    Failed,
}

/// The fields of an audit entry before the ledger numbers it.
#[derive(Clone, Debug)]
pub struct AuditRecord {
    pub actor: Actor,
    pub tool: String,
    pub arguments: Value,
    pub label: String,
    pub base_revision: u64,
    pub revision: Option<u64>,
    pub outcome: AuditOutcome,
    pub review: Option<u64>,
    pub affected: Vec<EntityId>,
    pub message: String,
}

/// Maximum retained audit entries; older entries are counted, not kept.
pub const AUDIT_LIMIT: usize = 10_000;
/// Maximum changes awaiting review per document.
pub const PENDING_LIMIT: usize = 64;

/// Append-only control history and the review queue of one document. Undo
/// and Redo add entries; they never remove earlier ones.
#[derive(Debug, Default)]
pub struct ControlLedger {
    entries: VecDeque<AuditEntry>,
    dropped: u64,
    next_sequence: u64,
    next_review: u64,
    pending: Vec<PendingChange>,
    states: BTreeMap<u64, ReviewState>,
    /// Session activity up to this revision is already in the audit.
    synced_revision: u64,
}
impl ControlLedger {
    pub fn entries(&self) -> impl DoubleEndedIterator<Item = &AuditEntry> {
        self.entries.iter()
    }
    /// Entries discarded beyond [`AUDIT_LIMIT`].
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
    pub fn pending(&self) -> &[PendingChange] {
        &self.pending
    }
    pub fn state(&self, review: u64) -> Option<ReviewState> {
        self.states.get(&review).copied()
    }
    pub fn record(&mut self, record: AuditRecord) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        let synced = record.revision;
        self.entries.push_back(AuditEntry {
            sequence,
            actor: record.actor,
            tool: record.tool,
            arguments: record.arguments,
            label: record.label,
            base_revision: record.base_revision,
            revision: record.revision,
            outcome: record.outcome,
            review: record.review,
            affected: record.affected,
            message: record.message,
        });
        if let Some(revision) = synced {
            self.synced_revision = self.synced_revision.max(revision);
        }
        if self.entries.len() > AUDIT_LIMIT {
            self.entries.pop_front();
            self.dropped += 1;
        }
        sequence
    }
    /// Queue a checked change and record its preview. Returns the review ID.
    /// Earlier Session activity is imported first, so the audit stays in order.
    pub fn queue(&mut self, mut change: PendingChange, session: &Session) -> Result<u64, String> {
        self.sync(session);
        if self.pending.len() >= PENDING_LIMIT {
            return Err(format!(
                "{PENDING_LIMIT} changes already await review; accept or reject some first"
            ));
        }
        self.next_review += 1;
        change.id = self.next_review;
        self.record(AuditRecord {
            actor: change.actor.clone(),
            tool: change.tool.clone(),
            arguments: change.arguments.clone(),
            label: change.label.clone(),
            base_revision: change.base_revision,
            revision: None,
            outcome: AuditOutcome::Previewed,
            review: Some(change.id),
            affected: change.affected.clone(),
            message: "Awaiting review".into(),
        });
        self.states.insert(change.id, ReviewState::Pending);
        let id = change.id;
        self.pending.push(change);
        Ok(id)
    }
    fn close(
        &mut self,
        change: &PendingChange,
        outcome: AuditOutcome,
        revision: Option<u64>,
        message: String,
    ) {
        self.states.insert(
            change.id,
            match outcome {
                AuditOutcome::Applied | AuditOutcome::Undone | AuditOutcome::Redone => {
                    ReviewState::Applied {
                        revision: revision.unwrap_or(change.base_revision),
                    }
                }
                AuditOutcome::Rejected | AuditOutcome::Cancelled => ReviewState::Rejected,
                AuditOutcome::Stale => ReviewState::Stale,
                _ => ReviewState::Failed,
            },
        );
        self.record(AuditRecord {
            actor: change.actor.clone(),
            tool: change.tool.clone(),
            arguments: change.arguments.clone(),
            label: change.label.clone(),
            base_revision: change.base_revision,
            revision,
            outcome,
            review: Some(change.id),
            affected: change.affected.clone(),
            message,
        });
    }
    fn take(&mut self, review: u64) -> Result<PendingChange, String> {
        let index = self
            .pending
            .iter()
            .position(|c| c.id == review)
            .ok_or("That change is no longer awaiting review")?;
        Ok(self.pending.remove(index))
    }
    /// Apply a queued change as one transaction attributed to its client.
    /// A change whose base revision moved is marked stale, never rebased.
    pub fn accept(&mut self, review: u64, session: &mut Session) -> Result<u64, String> {
        self.sync(session);
        let change = self.take(review)?;
        if change.is_stale(session) {
            let message = format!(
                "Prepared at revision {}, but the document is at {}; the client must prepare it again",
                change.base_revision,
                session.revision()
            );
            self.close(&change, AuditOutcome::Stale, None, message.clone());
            return Err(message);
        }
        let (result, outcome) = match &change.action {
            PendingAction::Commit(prepared) => (
                session.commit(prepared.as_ref().clone()),
                AuditOutcome::Applied,
            ),
            PendingAction::Undo => (
                session.undo(change.base_revision, change.actor.clone()),
                AuditOutcome::Undone,
            ),
            PendingAction::Redo => (
                session.redo(change.base_revision, change.actor.clone()),
                AuditOutcome::Redone,
            ),
        };
        match result {
            Ok(applied) => {
                self.close(&change, outcome, Some(applied.revision), "Accepted".into());
                Ok(applied.revision)
            }
            Err(error) => {
                self.close(&change, AuditOutcome::Failed, None, error.to_string());
                Err(error.to_string())
            }
        }
    }
    pub fn reject(&mut self, review: u64, reason: &str) -> Result<(), String> {
        let change = self.take(review)?;
        self.close(&change, AuditOutcome::Rejected, None, reason.into());
        Ok(())
    }
    /// Close every queued change whose base revision is no longer current.
    pub fn expire_stale(&mut self, session: &Session) -> usize {
        self.sync(session);
        let (stale, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|c| c.is_stale(session));
        self.pending = keep;
        let count = stale.len();
        for change in stale {
            let message = format!(
                "The document moved from revision {} to {}; prepare it again",
                change.base_revision,
                session.revision()
            );
            self.close(&change, AuditOutcome::Stale, None, message);
        }
        count
    }
    /// Import Session activity not yet in the audit, such as window edits
    /// and Undo/Redo, so the log covers every channel.
    pub fn sync(&mut self, session: &Session) {
        self.sync_below(session, None);
    }
    /// Import activity only below `revision`, before recording that revision
    /// with its request details.
    pub fn sync_below(&mut self, session: &Session, revision: Option<u64>) {
        let new: Vec<_> = session
            .activity()
            .filter(|a| {
                a.change.revision > self.synced_revision
                    && revision.is_none_or(|r| a.change.revision < r)
            })
            .cloned()
            .collect();
        for activity in new {
            self.record(AuditRecord {
                actor: activity.actor,
                tool: String::new(),
                arguments: Value::Null,
                label: activity.label,
                base_revision: activity.change.revision - 1,
                revision: Some(activity.change.revision),
                outcome: match activity.kind {
                    ActivityKind::Apply => AuditOutcome::Applied,
                    ActivityKind::Undo => AuditOutcome::Undone,
                    ActivityKind::Redo => AuditOutcome::Redone,
                },
                review: None,
                affected: activity.change.changed,
                message: String::new(),
            });
        }
    }
}

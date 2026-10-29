//! The control ledger queues reviewed changes, applies each as one attributed
//! transaction, refuses rebasing, and keeps an append-only audit.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    control::{
        AuditOutcome, AuditRecord, ControlLedger, PendingAction, PendingChange, ReviewState,
    },
    document::{Document, Mutation},
};
use serde_json::json;

fn client() -> Actor {
    Actor {
        id: "mcp:reviewer-test".into(),
        channel: Channel::Mcp,
    }
}
fn rename(session: &Session, name: &str) -> PendingChange {
    let prepared = session
        .prepare(Request {
            expected_revision: session.revision(),
            actor: client(),
            label: format!("Rename to {name}"),
            edits: vec![Mutation::RenameDocument(name.into())],
        })
        .unwrap();
    PendingChange {
        id: 0,
        actor: client(),
        tool: "rename".into(),
        arguments: json!({ "name": name }),
        label: format!("Rename to {name}"),
        base_revision: session.revision(),
        action: PendingAction::Commit(Box::new(prepared)),
        affected: vec![],
    }
}
fn outcomes(ledger: &ControlLedger) -> Vec<AuditOutcome> {
    ledger.entries().map(|e| e.outcome).collect()
}

#[test]
fn accepted_changes_apply_once_and_rejected_or_stale_ones_never_do() {
    let mut session = Session::new(Document::new("Ledger").unwrap()).unwrap();
    let mut ledger = ControlLedger::default();
    let first = ledger.queue(rename(&session, "First"), &session).unwrap();
    let second = ledger.queue(rename(&session, "Second"), &session).unwrap();
    assert_eq!(ledger.pending().len(), 2);
    assert_eq!(
        session.revision(),
        0,
        "queued changes do not touch the document"
    );
    assert_eq!(ledger.state(first), Some(ReviewState::Pending));

    assert_eq!(ledger.accept(first, &mut session).unwrap(), 1);
    assert_eq!(session.document().name(), "First");
    let last = session.activity().last().unwrap();
    assert_eq!(
        (last.actor.clone(), last.label.as_str()),
        (client(), "Rename to First")
    );
    assert_eq!(
        ledger.state(first),
        Some(ReviewState::Applied { revision: 1 })
    );

    // The second was prepared at revision 0: it is stale, not rebased.
    let error = ledger.accept(second, &mut session).unwrap_err();
    assert!(error.contains("prepare it again"), "{error}");
    assert_eq!(session.document().name(), "First");
    assert_eq!(ledger.state(second), Some(ReviewState::Stale));
    assert!(
        ledger.accept(second, &mut session).is_err(),
        "closed reviews stay closed"
    );

    let third = ledger.queue(rename(&session, "Third"), &session).unwrap();
    ledger.reject(third, "Not wanted").unwrap();
    assert_eq!(ledger.state(third), Some(ReviewState::Rejected));
    assert_eq!(session.revision(), 1);

    // Window Undo is imported once; a queued Undo applies as a reviewed change.
    session.undo(1, Actor::local(Channel::Ui)).unwrap();
    let undo = ledger
        .queue(
            PendingChange {
                id: 0,
                actor: client(),
                tool: "redo".into(),
                arguments: json!({}),
                label: "Redo".into(),
                base_revision: session.revision(),
                action: PendingAction::Redo,
                affected: vec![],
            },
            &session,
        )
        .unwrap();
    ledger.accept(undo, &mut session).unwrap();
    assert_eq!(session.document().name(), "First");
    ledger.sync(&session);
    ledger.sync(&session);
    assert_eq!(
        outcomes(&ledger),
        [
            AuditOutcome::Previewed,
            AuditOutcome::Previewed,
            AuditOutcome::Applied,
            AuditOutcome::Stale,
            AuditOutcome::Previewed,
            AuditOutcome::Rejected,
            AuditOutcome::Undone,
            AuditOutcome::Previewed,
            AuditOutcome::Redone,
        ]
    );
    let undone = ledger
        .entries()
        .find(|e| e.outcome == AuditOutcome::Undone)
        .unwrap();
    assert_eq!(undone.actor.channel, Channel::Ui);
    // History moving backwards removed nothing: every entry is still there.
    assert_eq!(ledger.entries().count(), 9);
    let sequences: Vec<_> = ledger.entries().map(|e| e.sequence).collect();
    assert!(sequences.windows(2).all(|w| w[1] == w[0] + 1));
}

#[test]
fn stale_changes_expire_and_the_queue_and_audit_are_bounded() {
    let mut session = Session::new(Document::new("Ledger").unwrap()).unwrap();
    let mut ledger = ControlLedger::default();
    let old = ledger.queue(rename(&session, "Old"), &session).unwrap();
    session
        .apply(Request {
            expected_revision: 0,
            actor: Actor::local(Channel::Ui),
            label: "Window edit".into(),
            edits: vec![Mutation::RenameDocument("Window".into())],
        })
        .unwrap();
    assert_eq!(ledger.expire_stale(&session), 1);
    assert_eq!(ledger.state(old), Some(ReviewState::Stale));
    assert!(ledger.pending().is_empty());
    for _ in 0..multicad_core::control::PENDING_LIMIT {
        ledger.queue(rename(&session, "Many"), &session).unwrap();
    }
    assert!(
        ledger
            .queue(rename(&session, "One more"), &session)
            .is_err()
    );
    let mut bounded = ControlLedger::default();
    for _ in 0..multicad_core::control::AUDIT_LIMIT + 5 {
        bounded.record(AuditRecord {
            actor: client(),
            tool: "document_info".into(),
            arguments: json!({}),
            label: String::new(),
            base_revision: 0,
            revision: None,
            outcome: AuditOutcome::Inspected,
            review: None,
            affected: vec![],
            message: String::new(),
        });
    }
    assert_eq!(
        bounded.entries().count(),
        multicad_core::control::AUDIT_LIMIT
    );
    assert_eq!(bounded.dropped(), 5);
    assert_eq!(bounded.entries().next().unwrap().sequence, 5);
}

use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Request, Session},
    document::{Document, Mutation},
};

fn rename(session: &Session, name: &str) -> Request {
    Request {
        expected_revision: session.revision(),
        actor: Actor::local(Channel::Ui),
        label: "Rename source".into(),
        edits: vec![Mutation::RenameDocument(name.into())],
    }
}

#[test]
fn background_preparation_retains_owner_and_revision_without_mutating_live_source() {
    let mut live = Session::new(Document::new("Original").unwrap()).unwrap();
    live.apply(rename(&live, "Revision one")).unwrap();
    let original = live.document().clone();
    let snapshot = live.preparation_snapshot();
    let prepared = std::thread::spawn(move || {
        assert_eq!(snapshot.session().revision(), 1);
        assert_eq!(snapshot.session().document().name(), "Revision one");
        assert!(!snapshot.session().can_undo());
        assert!(!snapshot.session().can_redo());
        snapshot
            .session()
            .prepare(rename(snapshot.session(), "Checked on worker"))
            .unwrap()
    })
    .join()
    .unwrap();
    assert_eq!(live.document(), &original);
    assert_eq!(live.revision(), 1);
    live.validate_preview(&prepared).unwrap();
    live.commit(prepared).unwrap();
    assert_eq!(live.document().name(), "Checked on worker");
    assert_eq!(live.revision(), 2);
    live.undo(2, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(live.document(), &original);
}

#[test]
fn a_late_worker_result_cannot_overwrite_a_newer_source_revision() {
    let mut live = Session::new(Document::new("Original").unwrap()).unwrap();
    let snapshot = live.preparation_snapshot();
    live.apply(rename(&live, "Newer source")).unwrap();
    let prepared = snapshot
        .session()
        .prepare(rename(snapshot.session(), "Outdated worker"))
        .unwrap();
    let before = live.document().clone();
    assert_eq!(
        live.commit(prepared).unwrap_err().code,
        ErrorCode::StaleRevision
    );
    assert_eq!(live.document(), &before);
    assert_eq!(live.revision(), 1);
    live.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(live.document().name(), "Original");
}

#[test]
fn identical_source_in_another_session_does_not_authorize_the_worker_result() {
    let live = Session::new(Document::new("Original").unwrap()).unwrap();
    let mut foreign = Session::new(live.document().clone()).unwrap();
    let snapshot = live.preparation_snapshot();
    let prepared = snapshot
        .session()
        .prepare(rename(snapshot.session(), "Worker edit"))
        .unwrap();
    assert_eq!(
        foreign.commit(prepared).unwrap_err().code,
        ErrorCode::ForeignPreview
    );
    assert_eq!(foreign.document(), live.document());
    assert_eq!(foreign.revision(), 0);
    assert!(!foreign.can_undo());
}

use multicad_core::{
    commands::*,
    constraint_sketch::{ConstraintKind, Dimension, source::Source},
    document::*,
    sketch::{self, Datum},
};
use multicad_solver::{SolveState, prepare_edit};
fn setup() -> (Session, EntityId) {
    let mut session = Session::new(Document::new("Constrained").unwrap()).unwrap();
    let (id, edits) = Source::create(Datum::Xz, "Outline").unwrap();
    session
        .apply(Request {
            expected_revision: 0,
            actor: Actor::local(Channel::Ui),
            label: "New sketch".into(),
            edits,
        })
        .unwrap();
    (session, id)
}
#[test]
fn solve_preview_commit_undo_reopen_and_attribution_share_one_source() {
    let (mut session, id) = setup();
    let (mut source, _, plane) = Source::read(session.document(), id).unwrap();
    let edge = source.add_rectangle([0., 0.], [39., 60.]).unwrap();
    source.add_constraint(ConstraintKind::Length {
        line: edge,
        dimension: Dimension::Literal {
            value: 40.,
            unit: "mm".into(),
        },
    });
    let before = session.document().clone();
    let revision = session.revision();
    let edit = prepare_edit(
        &session,
        revision,
        Actor {
            id: "external-test".into(),
            channel: Channel::Mcp,
        },
        id,
        source,
    )
    .unwrap();
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), revision);
    let preview = edit.preview.unwrap();
    assert_eq!(preview.actor().channel, Channel::Mcp);
    session.commit(preview).unwrap();
    assert_eq!(session.activity().last().unwrap().actor.id, "external-test");
    let after = session.document().clone();
    let (source, frame, restored_plane) = Source::read(&after, id).unwrap();
    assert_eq!(restored_plane, plane);
    assert_eq!(frame.n, [0., -1., 0.]);
    assert!(source.sketch.curves.iter().any(|c| c.id == edge));
    let wire = serde_json::to_vec(&after).unwrap();
    let reopened: Document = serde_json::from_slice(&wire).unwrap();
    assert_eq!(after, reopened);
    assert_eq!(
        sketch::Sketch::read(&reopened, id)
            .unwrap()
            .0
            .geometry
            .len(),
        4
    );
    session
        .undo(session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    assert_eq!(session.document(), &before);
    session
        .redo(session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    assert_eq!(session.document(), &after);
    // Convenience legacy mutations cannot silently erase constraint source.
    assert!(sketch::remove_geometry(session.document(), id, edge).is_err());
}
#[test]
fn conflict_returns_no_preview_and_stale_candidates_cannot_commit() {
    let (mut session, id) = setup();
    let (mut source, _, _) = Source::read(session.document(), id).unwrap();
    let edge = source.add_rectangle([0., 0.], [40., 60.]).unwrap();
    for value in [40., 41.] {
        source.add_constraint(ConstraintKind::Length {
            line: edge,
            dimension: Dimension::Literal {
                value,
                unit: "mm".into(),
            },
        });
    }
    let before = session.document().clone();
    let edit = prepare_edit(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        id,
        source.clone(),
    )
    .unwrap();
    assert_eq!(edit.evaluation.diagnostics.state, SolveState::Conflict);
    assert!(edit.preview.is_none());
    assert_eq!(session.document(), &before);
    source.sketch.constraints.last_mut().unwrap().reference = true;
    let first = prepare_edit(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        id,
        source.clone(),
    )
    .unwrap()
    .preview
    .unwrap();
    let stale = first.clone();
    session.commit(first).unwrap();
    let after = session.document().clone();
    assert!(session.commit(stale).is_err());
    assert!(prepare_edit(&session, 0, Actor::local(Channel::Ui), id, source).is_err());
    assert_eq!(session.document(), &after);
}
#[test]
fn canonical_boundary_rejects_malformed_source_and_unsupported_versions() {
    let (mut session, id) = setup();
    let (mut source, _, _) = Source::read(session.document(), id).unwrap();
    source.add_rectangle([0., 0.], [40., 60.]).unwrap();
    let mut edits = source.replace(session.document(), id).unwrap();
    let Mutation::Replace(entity) = &mut edits[0] else {
        unreachable!()
    };
    if let EntityData::Sketch { definition, .. } = &mut entity.data {
        definition.schema_version = 99;
    }
    let before = session.document().clone();
    assert!(
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: Actor::local(Channel::Mcp),
                label: "bad version".into(),
                edits
            })
            .is_err()
    );
    assert_eq!(session.document(), &before);
    source.sketch.points[0].id = source.sketch.points[1].id;
    assert!(source.definition().is_err());
}

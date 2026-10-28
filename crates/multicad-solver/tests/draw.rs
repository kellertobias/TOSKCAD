use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Request, Session},
    constraint_sketch::{ConstraintKind, CurveKind, Dimension, source::Source},
    document::*,
    references::*,
    sketch::{self, CreationPlane, Datum, PlaneOptions},
    sketch_draw::{DrawInput, DrawRequest, PointInput},
};
use multicad_solver::{EditError, SolveState, draw::prepare_draw};
use std::collections::{BTreeMap, BTreeSet};
fn actor() -> Actor {
    Actor {
        id: "drawing-client".into(),
        channel: Channel::Mcp,
    }
}
fn p(x: f64, y: f64) -> PointInput {
    PointInput::Position([x, y])
}
fn request(input: DrawInput) -> DrawRequest {
    DrawRequest {
        input,
        construction: false,
    }
}
fn setup() -> (Session, EntityId) {
    let (id, edits) = Source::create(Datum::Yz, "Outline").unwrap();
    let document = Document::new("Typed drawing")
        .unwrap()
        .edited(&edits)
        .unwrap();
    (Session::new(document).unwrap(), id)
}
#[test]
fn every_typed_drawing_kind_solves_previews_and_commits_in_one_step() {
    let inputs = [
        DrawInput::Line {
            start: p(0., 0.),
            end: p(20., 10.),
        },
        DrawInput::Rectangle {
            a: p(0., 0.),
            b: p(40., 20.),
        },
        DrawInput::Circle {
            center: p(0., 0.),
            radius_mm: 5.,
        },
        DrawInput::Arc {
            center: p(0., 0.),
            start: p(10., 0.),
            end: p(0., 10.),
            clockwise: true,
        },
        DrawInput::Polyline {
            vertices: vec![p(0., 0.), p(20., 0.), p(20., 10.)],
            closed: true,
        },
    ];
    for input in inputs {
        let (mut session, id) = setup();
        let original = session.document().clone();
        let (before, _, plane) = Source::read(&original, id).unwrap();
        let mut input = request(input);
        input.construction = true;
        let result = prepare_draw(&session, 0, actor(), id, None, &input).unwrap();
        assert_eq!(session.document(), &original);
        assert!(result.edit.evaluation.candidate.is_some());
        assert!(!result.created_curves.is_empty());
        let ids = result.created_curves;
        let prepared = result.edit.preview.unwrap();
        assert_eq!(prepared.actor(), &actor());
        session.commit(prepared).unwrap();
        assert_eq!(session.revision(), 1);
        let after = session.document().clone();
        let (source, _, same_plane) = Source::read(&after, id).unwrap();
        assert_eq!(same_plane, plane);
        assert_eq!(source.u, before.u);
        assert_eq!(
            source
                .sketch
                .curves
                .iter()
                .map(|c| c.id)
                .collect::<Vec<_>>(),
            ids
        );
        assert!(source.sketch.curves.iter().all(|c| c.construction));
        if let DrawInput::Arc { clockwise, .. } = input.input {
            assert!(
                matches!(source.sketch.curves[0].kind,CurveKind::Arc {clockwise:c,..} if c==clockwise)
            );
        }
        let reopened: Document =
            serde_json::from_slice(&serde_json::to_vec(&after).unwrap()).unwrap();
        assert_eq!(after, reopened);
        session.undo(1, actor()).unwrap();
        assert_eq!(session.document(), &original);
        session.redo(2, actor()).unwrap();
        assert_eq!(session.document(), &after);
    }
}
#[test]
fn drawing_to_an_existing_fixed_point_preserves_identity_constraints_and_basis() {
    let (session, id) = setup();
    let (mut source, _, _) = Source::read(session.document(), id).unwrap();
    let point = source.add_point([10., 20.]);
    source.add_constraint(ConstraintKind::Fix { point });
    let document = session
        .document()
        .edited(&source.replace(session.document(), id).unwrap())
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let before = session.document().clone();
    let result = prepare_draw(
        &session,
        0,
        actor(),
        id,
        None,
        &request(DrawInput::Line {
            start: PointInput::Existing(point),
            end: p(40., 50.),
        }),
    )
    .unwrap();
    session.commit(result.edit.preview.unwrap()).unwrap();
    let (after, _, _) = Source::read(session.document(), id).unwrap();
    assert_eq!(after.sketch.constraints, source.sketch.constraints);
    assert_eq!(after.u, source.u);
    assert_eq!(after.sketch.points[0], source.sketch.points[0]);
    assert!(matches!(after.sketch.curves[0].kind,CurveKind::Line {start,..} if start==point));
    session.undo(1, actor()).unwrap();
    assert_eq!(session.document(), &before);
}
#[test]
fn drawing_conflicts_retain_solver_diagnostics_without_a_commit_preview() {
    let (session, id) = setup();
    let (mut source, _, _) = Source::read(session.document(), id).unwrap();
    let line = source.add_rectangle([0., 0.], [40., 20.]).unwrap();
    for value in [40., 41.] {
        source.add_constraint(ConstraintKind::Length {
            line,
            dimension: Dimension::Literal {
                value,
                unit: "mm".into(),
            },
        });
    }
    let doc = session
        .document()
        .edited(&source.replace(session.document(), id).unwrap())
        .unwrap();
    let session = Session::new(doc).unwrap();
    let before = session.document().clone();
    let draw = prepare_draw(
        &session,
        0,
        actor(),
        id,
        None,
        &request(DrawInput::Circle {
            center: p(100., 0.),
            radius_mm: 5.,
        }),
    )
    .unwrap();
    assert_eq!(draw.edit.evaluation.diagnostics.state, SolveState::Conflict);
    assert!(draw.edit.preview.is_none());
    assert!(!draw.edit.evaluation.diagnostics.conflicts.is_empty());
    assert_eq!(session.document(), &before);
}
#[test]
fn invalid_and_stale_drawing_never_changes_source_or_upgrades_legacy_sketches() {
    let (mut session, id) = setup();
    let before = session.document().clone();
    let invalid = request(DrawInput::Arc {
        center: p(0., 0.),
        start: p(10., 0.),
        end: p(0., 11.),
        clockwise: false,
    });
    assert!(prepare_draw(&session, 0, actor(), id, None, &invalid).is_err());
    assert_eq!(session.document(), &before);
    let valid = request(DrawInput::Circle {
        center: p(0., 0.),
        radius_mm: 5.,
    });
    assert!(
        matches!(prepare_draw(&session,1,actor(),id,None,&valid),Err(EditError::Command(e)) if e.code==ErrorCode::StaleRevision)
    );
    let edit = prepare_draw(&session, 0, actor(), id, None, &valid).unwrap();
    let stale = edit.edit.preview.unwrap();
    session
        .apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Other client edit".into(),
            edits: vec![Mutation::RenameDocument("Changed".into())],
        })
        .unwrap();
    let changed = session.document().clone();
    assert!(session.commit(stale).is_err());
    assert_eq!(session.document(), &changed);
    let (legacy, edits) = sketch::create(Datum::Xy, "Legacy").unwrap();
    let session = Session::new(
        Document::new("Legacy source")
            .unwrap()
            .edited(&edits)
            .unwrap(),
    )
    .unwrap();
    let before = session.document().clone();
    assert!(prepare_draw(&session, 0, actor(), legacy, None, &valid).is_err());
    assert_eq!(session.document(), &before);
}
fn snapshot(owner: EntityId, revision: u64) -> TopologySnapshot {
    let origin = Origin {
        source: owner,
        role: "cap.end".into(),
    };
    TopologySnapshot {
        owner,
        revision,
        elements: vec![TopologyElement {
            geometry: None,
            key: TopologyKey {
                kind: TopologyKind::Face,
                ordinal: 2,
            },
            origins: BTreeSet::from([origin.clone()]),
            boundary: BTreeSet::new(),
            frames: vec![AnchoredFrame {
                origin,
                frame: ReferenceFrame {
                    origin_mm: [0., 0., 18.],
                    u: [1., 0., 0.],
                    v: [0., 1., 0.],
                },
            }],
        }],
    }
}
#[test]
fn attached_draw_requires_a_current_resolved_report_and_keeps_the_face_anchor() {
    let feature = Entity::new(
        "Panel",
        EntityData::Feature {
            definition: Definition {
                type_id: "drawing.test.panel".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs: vec![],
            suppressed: false,
        },
    );
    let producer = feature.id;
    let mut session = Session::new(
        Document::new("Face drawing")
            .unwrap()
            .edited(&[Mutation::Insert(feature)])
            .unwrap(),
    )
    .unwrap();
    let current = snapshot(producer, 0);
    let created = sketch::prepare_new(
        &session,
        0,
        actor(),
        "On face",
        CreationPlane::Face {
            snapshot: &current,
            key: current.elements[0].key,
            anchor: None,
        },
        PlaneOptions::default(),
    )
    .unwrap();
    let id = created.sketch;
    let plane = created.plane;
    session.commit(created.prepared).unwrap();
    let before = session.document().clone();
    let current = snapshot(producer, 1);
    let report = inspect(&before, 1, &BTreeMap::from([(producer, current)]));
    let stale = inspect(
        &before,
        0,
        &BTreeMap::from([(producer, snapshot(producer, 0))]),
    );
    let unresolved = inspect(&before, 1, &BTreeMap::new());
    let input = request(DrawInput::Circle {
        center: p(10., 10.),
        radius_mm: 3.,
    });
    for report in [None, Some(&stale), Some(&unresolved)] {
        assert!(prepare_draw(&session, 1, actor(), id, report, &input).is_err());
        assert_eq!(session.document(), &before);
    }
    let edit = prepare_draw(&session, 1, actor(), id, Some(&report), &input).unwrap();
    let prepared = edit.edit.preview.unwrap();
    assert_eq!(prepared.document().get(plane), before.get(plane));
    assert_eq!(session.document(), &before);
    session.commit(prepared).unwrap();
    assert_eq!(session.document().get(plane), before.get(plane));
    assert_eq!(
        Source::read_evaluated(
            session.document(),
            id,
            2,
            &inspect(
                session.document(),
                2,
                &BTreeMap::from([(producer, snapshot(producer, 2))])
            )
        )
        .unwrap()
        .0
        .sketch
        .curves
        .len(),
        1
    );
    let after = session.document().clone();
    assert!(prepare_draw(&session, 2, actor(), id, Some(&report), &input).is_err());
    assert_eq!(session.document(), &after);
    session.undo(2, actor()).unwrap();
    assert_eq!(session.document(), &before);
}

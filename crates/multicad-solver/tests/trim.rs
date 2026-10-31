use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Request, Session},
    constraint_sketch::{
        Axis, ConstraintKind, CurveKind, Dimension,
        binding::{DimensionInput, DimensionRequest},
        source::Source,
    },
    document::{Document, Entity, EntityData, EntityId, Mutation},
    references::ReferenceReport,
    sketch::Datum,
    sketch_trim::{Endpoint, ExtendLine, TrimErrorKind, TrimLine, TrimPolicy, TrimRequest},
};
use multicad_solver::{self as solver, EditError, TrimEditError};
use std::collections::BTreeSet;
fn actor() -> Actor {
    Actor {
        id: "trim-client".into(),
        channel: Channel::Mcp,
    }
}
fn line(source: &mut Source, a: [f64; 2], b: [f64; 2]) -> EntityId {
    let a = source.add_point(a);
    let b = source.add_point(b);
    source.add_line(a, b)
}
fn setup() -> (Session, EntityId, EntityId, EntityId) {
    let (id, edits) = Source::create(Datum::Xy, "Trim source").unwrap();
    let doc = Document::new("Trim test").unwrap().edited(&edits).unwrap();
    let (mut source, _, _) = Source::read(&doc, id).unwrap();
    let target = line(&mut source, [0., 0.], [100., 0.]);
    source.sketch.curves[0].construction = true;
    let boundary = line(&mut source, [40., -20.], [40., 20.]);
    let unrelated = line(&mut source, [200., 0.], [219., 1.]);
    source.add_constraint(ConstraintKind::Length {
        line: unrelated,
        dimension: Dimension::Literal {
            value: 30.,
            unit: "mm".into(),
        },
    });
    (
        Session::new(doc.edited(&source.replace(&doc, id).unwrap()).unwrap()).unwrap(),
        id,
        target,
        boundary,
    )
}
fn trim(target: EntityId, boundary: EntityId, pick: f64) -> TrimRequest {
    TrimRequest::Trim(TrimLine {
        target,
        pick_mm: [pick, 0.],
        boundaries: BTreeSet::from([boundary]),
    })
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
#[test]
fn preview_cancel_commit_and_undo_upgrade_literal_source_without_baking_other_points() {
    let (mut session, id, target, boundary) = setup();
    let original = session.document().clone();
    let (authored, _, _) = Source::read(&original, id).unwrap();
    let edit = solver::prepare_trim(
        &session,
        0,
        actor(),
        id,
        None,
        &trim(target, boundary, 90.),
        TrimPolicy::default(),
    )
    .unwrap();
    assert!(edit.upgraded_source);
    assert_eq!(session.document(), &original);
    assert!(edit.edit.semantic.is_some());
    let preview = edit.edit.preview.as_ref().unwrap();
    assert_eq!(preview.actor(), &actor());
    let (candidate, _, _) = Source::read_for_evaluation(preview.document(), id, 0, None).unwrap();
    assert_eq!(
        candidate.sketch.points[..authored.sketch.points.len()],
        authored.sketch.points
    );
    assert_eq!(candidate.sketch.constraints, authored.sketch.constraints);
    assert_eq!(candidate.sketch.curves[1..], authored.sketch.curves[1..]);
    let solved = edit.edit.semantic.as_ref().unwrap().solved().unwrap();
    assert_ne!(
        solved.points[4..6],
        authored.sketch.points[4..6],
        "Unrelated rough guesses solve separately"
    );
    drop(edit);
    assert_eq!(session.document(), &original);
    assert!(!session.can_undo());
    let edit = solver::prepare_trim(
        &session,
        0,
        actor(),
        id,
        None,
        &trim(target, boundary, 90.),
        TrimPolicy::default(),
    )
    .unwrap();
    let point_ids = edit.report.created_points.clone();
    session.commit(edit.edit.preview.unwrap()).unwrap();
    let after = session.document().clone();
    let EntityData::Sketch { definition, .. } = &after.get(id).unwrap().data else {
        panic!("sketch")
    };
    assert_eq!(definition.schema_version, 2);
    assert_eq!(session.activity().last().unwrap().actor, actor());
    session.undo(1, actor()).unwrap();
    assert_eq!(session.document(), &original);
    session.redo(2, actor()).unwrap();
    assert_eq!(session.document(), &after);
    let (source, _, _) = Source::read_for_evaluation(&after, id, 3, None).unwrap();
    assert!(
        point_ids
            .iter()
            .all(|id| source.sketch.points.iter().any(|p| p.id == *id))
    );
}
#[test]
fn parameter_driven_boundary_uses_current_solved_position_and_retains_bindings_and_guesses() {
    let (mut session, id, target, boundary) = setup();
    let (mut source, _, _) = Source::read(session.document(), id).unwrap();
    let CurveKind::Line { start: origin, .. } = source.sketch.curves[0].kind else {
        panic!("line")
    };
    let CurveKind::Line { start: low, .. } = source.sketch.curves[1].kind else {
        panic!("line")
    };
    source.add_constraint(ConstraintKind::Fix { point: origin });
    source.add_constraint(ConstraintKind::Vertical { line: boundary });
    let width = source.add_constraint(ConstraintKind::Distance {
        a: origin,
        b: low,
        axis: Axis::X,
        dimension: Dimension::Literal {
            value: 40.,
            unit: "mm".into(),
        },
    });
    let wall = Entity::new(
        "wall",
        EntityData::Parameter {
            expression: "160".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let mut edits = source.replace(session.document(), id).unwrap();
    edits.push(Mutation::Insert(wall));
    session
        .apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Add wall parameter".into(),
            edits,
        })
        .unwrap();
    let binding = solver::prepare_dimension(
        &session,
        1,
        actor(),
        id,
        None,
        &DimensionRequest {
            constraint: width,
            input: DimensionInput::Expression {
                text: "=$wall".into(),
                unit: "mm".into(),
            },
            rebind: false,
        },
    )
    .unwrap();
    session.commit(binding.preview.unwrap()).unwrap();
    let before = session.document().clone();
    let (authored, _, _) = Source::read_for_evaluation(&before, id, 2, None).unwrap();
    let request = TrimRequest::Extend(ExtendLine {
        target,
        endpoint: Endpoint::End,
        boundaries: BTreeSet::from([boundary]),
    });
    let edit = solver::prepare_trim(
        &session,
        2,
        actor(),
        id,
        None,
        &request,
        TrimPolicy::default(),
    )
    .unwrap();
    assert!(!edit.upgraded_source);
    assert_eq!(session.document(), &before);
    let semantic = edit.edit.semantic.as_ref().unwrap();
    let CurveKind::Line { end, .. } = semantic
        .solved()
        .unwrap()
        .curves
        .iter()
        .find(|c| c.id == target)
        .unwrap()
        .kind
    else {
        panic!("line")
    };
    let p = semantic
        .solved()
        .unwrap()
        .points
        .iter()
        .find(|p| p.id == end)
        .unwrap()
        .position_mm;
    near(p[0], 160.);
    near(p[1], 0.);
    let prepared = edit.edit.preview.as_ref().unwrap();
    let (canonical, _, _) = Source::read_for_evaluation(prepared.document(), id, 2, None).unwrap();
    assert_eq!(
        canonical.sketch.points[..authored.sketch.points.len()],
        authored.sketch.points
    );
    assert_eq!(canonical.sketch.constraints, authored.sketch.constraints);
    assert!(
        canonical
            .sketch
            .constraints
            .iter()
            .find(|c| c.id == width)
            .unwrap()
            .kind
            .dimension()
            .unwrap()
            .expression()
            .is_some()
    );
    let EntityData::Sketch {
        inputs: before_links,
        ..
    } = &before.get(id).unwrap().data
    else {
        panic!("sketch")
    };
    let EntityData::Sketch {
        inputs: after_links,
        ..
    } = &prepared.document().get(id).unwrap().data
    else {
        panic!("sketch")
    };
    assert_eq!(before_links, after_links);
    session.commit(edit.edit.preview.unwrap()).unwrap();
    assert_eq!(session.revision(), 3);
}
#[test]
fn stale_missing_unsolved_and_affected_reference_constraints_return_no_draft_or_source_change() {
    let (mut session, id, target, boundary) = setup();
    let before = session.document().clone();
    let stale = solver::prepare_trim(
        &session,
        1,
        actor(),
        id,
        None,
        &trim(target, boundary, 90.),
        TrimPolicy::default(),
    )
    .err()
    .unwrap();
    assert!(
        matches!(stale,TrimEditError::Edit(EditError::Command(ref e)) if e.code==ErrorCode::StaleRevision)
    );
    let report = ReferenceReport {
        sketch_supports: Default::default(),
        external_results: Default::default(),
        external_errors: Default::default(),
        revision: 1,
        entries: vec![],
    };
    assert!(
        solver::prepare_trim(
            &session,
            0,
            actor(),
            id,
            Some(&report),
            &trim(target, boundary, 90.),
            TrimPolicy::default()
        )
        .is_err()
    );
    assert!(
        solver::prepare_trim(
            &session,
            0,
            actor(),
            EntityId::new(),
            None,
            &trim(target, boundary, 90.),
            TrimPolicy::default()
        )
        .is_err()
    );
    assert_eq!(session.document(), &before);
    let (mut source, _, _) = Source::read(session.document(), id).unwrap();
    let reference = source.add_constraint(ConstraintKind::Length {
        line: target,
        dimension: Dimension::Literal {
            value: 100.,
            unit: "mm".into(),
        },
    });
    source.sketch.constraints.last_mut().unwrap().reference = true;
    session
        .apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Reference dimension".into(),
            edits: source.replace(session.document(), id).unwrap(),
        })
        .unwrap();
    let before = session.document().clone();
    let error = solver::prepare_trim(
        &session,
        1,
        actor(),
        id,
        None,
        &trim(target, boundary, 90.),
        TrimPolicy::default(),
    )
    .err()
    .unwrap();
    assert!(
        matches!(error,TrimEditError::Geometry(ref e) if e.kind==TrimErrorKind::ConstraintConflict && e.involved.contains(&reference))
    );
    assert_eq!(session.document(), &before);
    source.sketch.constraints.pop();
    source.add_constraint(ConstraintKind::Length {
        line: boundary,
        dimension: Dimension::Literal {
            value: 40.,
            unit: "mm".into(),
        },
    });
    source.add_constraint(ConstraintKind::Length {
        line: boundary,
        dimension: Dimension::Literal {
            value: 50.,
            unit: "mm".into(),
        },
    });
    session
        .apply(Request {
            expected_revision: 1,
            actor: actor(),
            label: "Conflicting boundary dimensions".into(),
            edits: source.replace(session.document(), id).unwrap(),
        })
        .unwrap();
    let before = session.document().clone();
    let error = solver::prepare_trim(
        &session,
        2,
        actor(),
        id,
        None,
        &trim(target, boundary, 90.),
        TrimPolicy::default(),
    )
    .err()
    .unwrap();
    assert!(
        matches!(error,TrimEditError::Unsolved {ref diagnostics,..} if !diagnostics.conflicts.is_empty())
    );
    assert_eq!(session.document(), &before);
}

#[test]
fn shared_canonical_edit_preserves_unrelated_guesses_and_failed_solves_have_no_preview() {
    let (session, id, target, _) = setup();
    let before = session.document().clone();
    let (mut source, _, _) = Source::read(&before, id).unwrap();
    let untouched = source.sketch.points[2..].to_vec();
    let CurveKind::Line { end, .. } = source.sketch.curves[0].kind else {
        panic!("line")
    };
    source
        .sketch
        .points
        .iter_mut()
        .find(|p| p.id == end)
        .unwrap()
        .position_mm = [110., 0.];
    let edit =
        solver::prepare_canonical_edit(&session, 0, actor(), id, source.clone(), None).unwrap();
    let preview = edit.preview.as_ref().unwrap();
    let (canonical, _, _) = Source::read_for_evaluation(preview.document(), id, 0, None).unwrap();
    assert_eq!(canonical, source);
    assert_eq!(canonical.sketch.points[2..], untouched);
    assert_eq!(session.document(), &before);
    assert!(edit.semantic.is_some());
    assert_eq!(preview.actor(), &actor());
    source.add_constraint(ConstraintKind::Length {
        line: target,
        dimension: Dimension::Literal {
            value: 100.,
            unit: "mm".into(),
        },
    });
    source.add_constraint(ConstraintKind::Length {
        line: target,
        dimension: Dimension::Literal {
            value: 110.,
            unit: "mm".into(),
        },
    });
    let invalid =
        solver::prepare_canonical_edit(&session, 0, actor(), id, source.clone(), None).unwrap();
    assert!(invalid.preview.is_none());
    assert!(invalid.semantic.is_some());
    assert!(!invalid.evaluation.diagnostics.conflicts.is_empty());
    assert_eq!(session.document(), &before);
    let stale = solver::prepare_canonical_edit(&session, 1, actor(), id, source, None)
        .err()
        .unwrap();
    assert!(matches!(stale,EditError::Command(ref e) if e.code==ErrorCode::StaleRevision));
}

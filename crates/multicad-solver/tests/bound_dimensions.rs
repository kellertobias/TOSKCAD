use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Request, Session},
    constraint_sketch::{
        ConstraintKind, CurveKind, Dimension,
        binding::{DimensionInput, DimensionRequest},
        source::Source,
    },
    document::*,
    parameters,
    sketch::Datum,
    sketch_draw::{DrawInput, DrawRequest, PointInput},
};
use multicad_solver::{self as solver, EditError, evaluate_document_sketch, prepare_dimension};
fn actor() -> Actor {
    Actor {
        id: "parameter-client".into(),
        channel: Channel::Mcp,
    }
}
fn setup() -> (Session, EntityId, EntityId, EntityId) {
    let width = Entity::new(
        "width",
        EntityData::Parameter {
            expression: "80".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let (id, mut edits) = Source::create(Datum::Xy, "Driven panel").unwrap();
    edits.push(Mutation::Insert(width.clone()));
    let doc = Document::new("Parametric sketch")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let (mut source, _, _) = Source::read(&doc, id).unwrap();
    let edge = source.add_rectangle([0., 0.], [40., 20.]).unwrap();
    source.add_constraint(ConstraintKind::Fix {
        point: source.sketch.points[0].id,
    });
    let c = source.add_constraint(ConstraintKind::Length {
        line: edge,
        dimension: Dimension::Literal {
            value: 40.,
            unit: "mm".into(),
        },
    });
    source.add_constraint(ConstraintKind::Length {
        line: source.sketch.curves[1].id,
        dimension: Dimension::Literal {
            value: 20.,
            unit: "mm".into(),
        },
    });
    let doc = doc.edited(&source.replace(&doc, id).unwrap()).unwrap();
    (Session::new(doc).unwrap(), id, c, width.id)
}
fn bind(session: &mut Session, id: EntityId, c: EntityId) {
    let request = DimensionRequest {
        constraint: c,
        input: DimensionInput::Expression {
            text: "= $width".into(),
            unit: "cm".into(),
        },
        rebind: false,
    };
    let edit = prepare_dimension(session, session.revision(), actor(), id, None, &request).unwrap();
    assert!(edit.semantic.is_some());
    session.commit(edit.preview.unwrap()).unwrap();
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
fn set_parameter(session: &mut Session, id: EntityId, value: &str) {
    let mut e = session.document().get(id).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut e.data else {
        panic!("parameter")
    };
    *expression = value.into();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Set width".into(),
            edits: vec![Mutation::Replace(e)],
        })
        .unwrap();
}
#[test]
fn semantic_binding_preview_and_parameter_recompute_never_bake_coordinates_or_rhs() {
    let (mut session, id, c, width) = setup();
    let original = session.document().clone();
    let (authored, _, _) = Source::read(&original, id).unwrap();
    let edit = prepare_dimension(
        &session,
        0,
        actor(),
        id,
        None,
        &DimensionRequest {
            constraint: c,
            input: DimensionInput::Expression {
                text: "=$width".into(),
                unit: "cm".into(),
            },
            rebind: false,
        },
    )
    .unwrap();
    assert_eq!(session.document(), &original);
    let semantic = edit.semantic.as_ref().unwrap();
    assert_eq!(semantic.canonical().sketch.points, authored.sketch.points);
    assert_eq!(semantic.canonical().sketch.curves, authored.sketch.curves);
    near(semantic.resolved_rhs[&c].value, 8.);
    near(semantic.regions().unwrap().regions[0].area_mm2, 1600.);
    assert!(
        semantic
            .solved()
            .unwrap()
            .constraints
            .iter()
            .find(|x| x.id == c)
            .unwrap()
            .kind
            .dimension()
            .unwrap()
            .expression()
            .is_some()
    );
    session.commit(edit.preview.unwrap()).unwrap();
    let source = session.document().get(id).unwrap().clone();
    assert_eq!(session.activity().last().unwrap().actor, actor());
    assert!(Source::read(session.document(), id).is_err());
    let eval = evaluate_document_sketch(session.document(), 1, id, None).unwrap();
    assert!(eval.is_current(session.document(), 1));
    near(eval.regions().unwrap().regions[0].area_mm2, 1600.);
    set_parameter(&mut session, width, "120");
    assert_eq!(session.document().get(id), Some(&source));
    let next = evaluate_document_sketch(session.document(), 2, id, None).unwrap();
    near(next.regions().unwrap().regions[0].area_mm2, 2400.);
    assert!(!eval.is_current(session.document(), 2));
    session.undo(2, actor()).unwrap();
    assert_eq!(session.document().get(id), Some(&source));
    near(
        evaluate_document_sketch(session.document(), 3, id, None)
            .unwrap()
            .regions()
            .unwrap()
            .regions[0]
            .area_mm2,
        1600.,
    );
    session.undo(3, actor()).unwrap();
    assert_eq!(session.document(), &original);
}
#[test]
fn v2_draw_uses_current_snaps_and_merges_only_new_authored_ids() {
    let (mut session, id, c, _) = setup();
    bind(&mut session, id, c);
    let before =
        evaluate_document_sketch(session.document(), session.revision(), id, None).unwrap();
    let canonical = before.canonical().clone();
    let right = canonical.sketch.points[1].id;
    // Authored right corner is still x=40; evaluated corner is x=80. The new
    // arc must validate its radius at the current x=80 center and retain its ID.
    near(canonical.sketch.points[1].position_mm[0], 40.);
    near(before.solved().unwrap().points[1].position_mm[0], 80.);
    let request = DrawRequest {
        input: DrawInput::Arc {
            center: PointInput::Existing(right),
            start: PointInput::Position([90., 0.]),
            end: PointInput::Position([80., 10.]),
            clockwise: false,
        },
        construction: true,
    };
    let edit =
        solver::draw::prepare_draw(&session, session.revision(), actor(), id, None, &request)
            .unwrap();
    let preview = edit.edit.preview.unwrap();
    let (after, _, _) =
        Source::read_for_evaluation(preview.document(), id, session.revision(), None).unwrap();
    assert_eq!(
        &after.sketch.points[..canonical.sketch.points.len()],
        &canonical.sketch.points
    );
    assert_eq!(
        &after.sketch.curves[..canonical.sketch.curves.len()],
        &canonical.sketch.curves
    );
    assert_eq!(after.sketch.constraints, canonical.sketch.constraints);
    assert!(
        matches!(after.sketch.curves.last().unwrap().kind,CurveKind::Arc {center,..} if center==right)
    );
    assert_eq!(after.sketch.points.len(), canonical.sketch.points.len() + 2);
    session.commit(preview).unwrap();
    assert!(
        evaluate_document_sketch(session.document(), session.revision(), id, None)
            .unwrap()
            .geometry()
            .is_ok()
    );
}
#[test]
fn bound_identity_survives_rename_reopen_and_missing_target_never_rebinds() {
    let (mut session, id, c, width) = setup();
    bind(&mut session, id, c);
    let mut renamed = session.document().get(width).unwrap().clone();
    renamed.name = "span".into();
    let other = Entity::new(
        "width",
        EntityData::Parameter {
            expression: "900".into(),
            unit: "mm".into(),
            exposed: false,
            inputs: vec![],
        },
    );
    session
        .apply(Request {
            expected_revision: 1,
            actor: actor(),
            label: "Rename parameter".into(),
            edits: vec![Mutation::Replace(renamed), Mutation::Insert(other)],
        })
        .unwrap();
    near(
        evaluate_document_sketch(session.document(), 2, id, None)
            .unwrap()
            .regions()
            .unwrap()
            .regions[0]
            .area_mm2,
        1600.,
    );
    let wire = serde_json::to_vec(session.document()).unwrap();
    let reopened: Document = serde_json::from_slice(&wire).unwrap();
    assert_eq!(session.document(), &reopened);
    near(
        evaluate_document_sketch(&reopened, 0, id, None)
            .unwrap()
            .regions()
            .unwrap()
            .regions[0]
            .area_mm2,
        1600.,
    );
    session
        .apply(Request {
            expected_revision: 2,
            actor: actor(),
            label: "Delete bound parameter".into(),
            edits: vec![Mutation::Remove(width)],
        })
        .unwrap();
    assert!(evaluate_document_sketch(session.document(), 3, id, None).is_err());
    assert!(Source::read(session.document(), id).is_err());
    let preserved = serde_json::to_vec(session.document()).unwrap();
    let reopened: Document = serde_json::from_slice(&preserved).unwrap();
    assert_eq!(session.document(), &reopened);
}
#[test]
fn literal_replacement_keeps_semantic_version_and_unsupported_raw_consumers_fail() {
    let (mut session, id, c, _) = setup();
    bind(&mut session, id, c);
    let edit = prepare_dimension(
        &session,
        1,
        actor(),
        id,
        None,
        &DimensionRequest {
            constraint: c,
            input: DimensionInput::Literal {
                value: 100.,
                unit: "mm".into(),
            },
            rebind: false,
        },
    )
    .unwrap();
    assert!(edit.semantic.is_some());
    session.commit(edit.preview.unwrap()).unwrap();
    assert!(multicad_core::constraint_sketch::source::requires_v2(
        session.document()
    ));
    assert!(Source::read(session.document(), id).is_err());
    near(
        evaluate_document_sketch(session.document(), 2, id, None)
            .unwrap()
            .regions()
            .unwrap()
            .regions[0]
            .area_mm2,
        2000.,
    );
}
#[test]
fn stale_context_bad_units_and_numerical_conflicts_cannot_supply_current_geometry() {
    let (mut session, id, c, width) = setup();
    bind(&mut session, id, c);
    let old_parameters = session.parameters().clone();
    let source = session.document().clone();
    let bad = DimensionRequest {
        constraint: c,
        input: DimensionInput::Expression {
            text: "=$width".into(),
            unit: "deg".into(),
        },
        rebind: false,
    };
    assert!(prepare_dimension(&session, 1, actor(), id, None, &bad).is_err());
    assert_eq!(session.document(), &source);
    assert!(
        matches!(prepare_dimension(&session,0,actor(),id,None,&bad),Err(EditError::Command(e)) if e.code==ErrorCode::StaleRevision)
    );
    let (mut canonical, _, _) =
        Source::read_for_evaluation(session.document(), id, 1, None).unwrap();
    let edge = canonical.sketch.curves[0].id;
    canonical.add_constraint(ConstraintKind::Length {
        line: edge,
        dimension: Dimension::Literal {
            value: 81.,
            unit: "mm".into(),
        },
    });
    let conflict =
        solver::prepare_edit_with_report(&session, 1, actor(), id, canonical, None).unwrap();
    assert!(conflict.preview.is_none());
    let evaluation = conflict.semantic.unwrap();
    assert!(evaluation.geometry().is_err());
    assert!(evaluation.regions().is_err());
    assert_eq!(session.document(), &source);
    set_parameter(&mut session, width, "100");
    assert!(
        solver::evaluate_document_sketch_with_parameters(
            session.document(),
            2,
            id,
            None,
            &old_parameters
        )
        .is_err()
    );
    let canonical = Source::read_for_evaluation(session.document(), id, 2, None)
        .unwrap()
        .0;
    assert!(solver::evaluate(&canonical.sketch).is_err());
}
#[test]
fn parameter_dependent_sketch_is_in_the_invalidation_graph() {
    let (mut session, id, c, width) = setup();
    bind(&mut session, id, c);
    assert!(session.document().affected([width]).unwrap().contains(&id));
    let params = parameters::evaluate(session.document()).unwrap();
    near(
        params.value(width, parameters::Unit::Centimeter).unwrap(),
        8.,
    );
}

#[test]
fn attached_semantic_sketch_never_uses_absent_stale_or_unresolved_frames() {
    use multicad_core::references::*;
    use multicad_core::sketch::{self, CreationPlane, PlaneOptions};
    use std::collections::{BTreeMap, BTreeSet};
    let feature = Entity::new(
        "Support",
        EntityData::Feature {
            definition: Definition {
                type_id: "bound.test.support".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs: vec![],
            suppressed: false,
        },
    );
    let owner = feature.id;
    let snapshot = |revision| {
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
                    ordinal: 0,
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
    };
    let mut session = Session::new(
        Document::new("Attached dimensions")
            .unwrap()
            .edited(&[Mutation::Insert(feature)])
            .unwrap(),
    )
    .unwrap();
    let s = snapshot(0);
    let create = sketch::prepare_new(
        &session,
        0,
        actor(),
        "Face",
        CreationPlane::Face {
            snapshot: &s,
            key: s.elements[0].key,
            anchor: None,
        },
        PlaneOptions::default(),
    )
    .unwrap();
    let id = create.sketch;
    session.commit(create.prepared).unwrap();
    let report = inspect(
        session.document(),
        1,
        &BTreeMap::from([(owner, snapshot(1))]),
    );
    let source = Source::read_for_evaluation(session.document(), id, 1, Some(&report))
        .unwrap()
        .0;
    let mut source = source;
    source.add_rectangle([0., 0.], [40., 20.]).unwrap();
    let line = source.sketch.curves[0].id;
    source.add_constraint(ConstraintKind::Length {
        line,
        dimension: Dimension::Expression {
            expression: parameters::BoundExpression::bind(session.document(), "=60", "mm", None)
                .unwrap(),
        },
    });
    let edit =
        solver::prepare_edit_with_report(&session, 1, actor(), id, source, Some(&report)).unwrap();
    session.commit(edit.preview.unwrap()).unwrap();
    let before = session.document().clone();
    let stale = inspect(&before, 1, &BTreeMap::from([(owner, snapshot(1))]));
    let unresolved = inspect(&before, 2, &BTreeMap::new());
    for report in [None, Some(&stale), Some(&unresolved)] {
        assert!(evaluate_document_sketch(&before, 2, id, report).is_err());
    }
    let current = inspect(&before, 2, &BTreeMap::from([(owner, snapshot(2))]));
    let result = evaluate_document_sketch(&before, 2, id, Some(&current)).unwrap();
    near(result.frame.origin[2], 18.);
    assert!(result.geometry().is_ok());
    assert_eq!(session.document(), &before);
}

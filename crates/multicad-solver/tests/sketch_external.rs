//! Solver contracts use explicit analytic descriptors. Native OCCT projection
//! and dependent solid regeneration are exercised by multicad-model tests.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{ConstraintKind, Dimension, source::Source},
    document::{Definition, Document, Entity, EntityData, EntityId, Mutation},
    references::{
        self, AnalyticGeometry, GeometryIntent, Origin, TopologyElement, TopologyKey,
        TopologySnapshot,
    },
    sketch::Datum,
    sketch_draw::{DrawInput, DrawRequest, PointInput},
    sketch_external::{self, Behavior, CaptureInput, CaptureRequest, Method, OutputIds},
};
use multicad_solver::draw::prepare_draw;
use multicad_solver::*;
use std::collections::{BTreeMap, BTreeSet};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn fixture(
    geometry: AnalyticGeometry,
    method: Method,
) -> (
    Session,
    EntityId,
    CaptureRequest,
    BTreeMap<EntityId, TopologySnapshot>,
) {
    let (sketch, edits) = Source::create(Datum::Xy, "Consumer").unwrap();
    let mut document = Document::new("Solver external contracts")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let source = Entity::new(
        "Analytic descriptor fixture",
        EntityData::Feature {
            definition: Definition {
                type_id: "test.exact-descriptor".into(),
                schema_version: 1,
                data: serde_json::json!({"generation":1}),
            },
            inputs: vec![],
            suppressed: false,
        },
    );
    let owner = source.id;
    document.insert(source).unwrap();
    let snapshot = TopologySnapshot {
        owner,
        revision: 0,
        elements: vec![TopologyElement {
            key: TopologyKey {
                kind: geometry.topology_kind(),
                ordinal: 0,
            },
            origins: BTreeSet::from([Origin {
                source: owner,
                role: "analytic.edge".into(),
            }]),
            boundary: BTreeSet::new(),
            frames: vec![],
            geometry: Some(geometry),
        }],
    };
    let intent = GeometryIntent::capture(&snapshot, snapshot.elements[0].key).unwrap();
    let map = BTreeMap::from([(owner, snapshot)]);
    let request = CaptureRequest::new(
        &document,
        0,
        CaptureInput {
            sketch,
            source: owner,
            intent,
            method,
            behavior: Behavior::ReferenceOnly,
        },
        &map,
    )
    .unwrap();
    (Session::new(document).unwrap(), sketch, request, map)
}
fn capture(
    session: &mut Session,
    request: &CaptureRequest,
    map: &BTreeMap<EntityId, TopologySnapshot>,
) {
    let edit =
        prepare_external_reference(session, session.revision(), actor(), request, map).unwrap();
    session.commit(edit.edit.preview.unwrap()).unwrap();
}
fn refreshed(
    mut map: BTreeMap<EntityId, TopologySnapshot>,
    revision: u64,
) -> BTreeMap<EntityId, TopologySnapshot> {
    for snapshot in map.values_mut() {
        snapshot.revision = revision;
    }
    map
}
fn canonical(session: &Session, sketch: EntityId) -> Source {
    let EntityData::Sketch { definition, .. } = &session.document().get(sketch).unwrap().data
    else {
        panic!()
    };
    Source::from_definition(definition).unwrap()
}
#[test]
fn later_draws_do_not_serialize_external_solver_locks_and_behavior_changes_keep_all_ids() {
    let (mut session, sketch, request, map) = fixture(
        AnalyticGeometry::Line {
            a_mm: [0., 0., 0.],
            b_mm: [30., 0., 0.],
        },
        Method::Coplanar,
    );
    capture(&mut session, &request, &map);
    let before = session.document().clone();
    let map = refreshed(map, session.revision());
    let report = references::inspect(session.document(), session.revision(), &map);
    let edit = prepare_draw(
        &session,
        session.revision(),
        actor(),
        sketch,
        Some(&report),
        &DrawRequest {
            input: DrawInput::Rectangle {
                a: PointInput::Position([40., 0.]),
                b: PointInput::Position([50., 10.]),
            },
            construction: false,
        },
    )
    .unwrap();
    session.commit(edit.edit.preview.unwrap()).unwrap();
    let source = canonical(&session, sketch);
    assert_eq!(source.sketch.constraints.len(), 4);
    assert!(
        source
            .sketch
            .constraints
            .iter()
            .all(|constraint| !request.output.locks().contains(&constraint.id))
    );
    let curve = request.output.curve().unwrap();
    assert!(
        source
            .sketch
            .curves
            .iter()
            .find(|other| other.id == curve)
            .unwrap()
            .construction
    );
    let map = refreshed(map, session.revision());
    let edit = prepare_external_behavior(
        &session,
        session.revision(),
        actor(),
        sketch,
        request.reference,
        Behavior::Derived,
        &map,
    )
    .unwrap();
    session.commit(edit.preview.unwrap()).unwrap();
    assert_eq!(
        sketch_external::read(session.document(), request.reference)
            .unwrap()
            .definition
            .output,
        request.output
    );
    assert!(
        !canonical(&session, sketch)
            .sketch
            .curves
            .iter()
            .find(|other| other.id == curve)
            .unwrap()
            .construction
    );
    let map = refreshed(map, session.revision());
    let removed = prepare_remove_external_reference(
        &session,
        session.revision(),
        actor(),
        sketch,
        request.reference,
        &map,
    )
    .unwrap();
    session.commit(removed.preview.unwrap()).unwrap();
    assert!(session.document().get(request.reference).is_none());
    assert_eq!(canonical(&session, sketch).sketch.curves.len(), 4);
    assert_eq!(canonical(&session, sketch).sketch.constraints.len(), 4);
    assert_ne!(session.document(), &before);
}
#[test]
fn missing_ambiguous_stale_and_wrong_family_receipts_never_fall_back_to_authored_guesses() {
    let (mut session, sketch, request, map) = fixture(
        AnalyticGeometry::Line {
            a_mm: [0., 0., 0.],
            b_mm: [30., 0., 0.],
        },
        Method::Coplanar,
    );
    capture(&mut session, &request, &map);
    let map = refreshed(map, session.revision());
    let before = session.document().clone();
    assert!(
        evaluate_document_sketch_with_parameters(
            &before,
            session.revision(),
            sketch,
            None,
            session.parameters()
        )
        .is_err()
    );
    for case in 0..4 {
        let mut altered = map.clone();
        match case {
            0 => altered.clear(),
            1 => {
                let snapshot = altered.values_mut().next().unwrap();
                let mut duplicate = snapshot.elements[0].clone();
                duplicate.key.ordinal = 1;
                snapshot.elements.push(duplicate);
            }
            2 => altered.values_mut().next().unwrap().revision = 0,
            _ => {
                altered.values_mut().next().unwrap().elements[0].geometry =
                    Some(AnalyticGeometry::CircularArc {
                        frame: references::ReferenceFrame {
                            origin_mm: [0., 0., 0.],
                            u: [1., 0., 0.],
                            v: [0., 1., 0.],
                        },
                        radius_mm: 10.,
                        start_rad: 0.,
                        sweep_rad: std::f64::consts::TAU,
                    });
            }
        }
        let report = references::inspect(&before, session.revision(), &altered);
        assert!(
            evaluate_document_sketch_with_parameters(
                &before,
                session.revision(),
                sketch,
                Some(&report),
                session.parameters()
            )
            .is_err()
        );
    }
    assert_eq!(session.document(), &before);
}
#[test]
fn circular_radius_locks_are_derived_and_conflicting_authored_dimensions_do_not_commit() {
    let frame = references::ReferenceFrame {
        origin_mm: [0., 0., 0.],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
    };
    let (mut session, sketch, request, map) = fixture(
        AnalyticGeometry::CircularArc {
            frame,
            radius_mm: 5.,
            start_rad: 0.,
            sweep_rad: std::f64::consts::TAU,
        },
        Method::Coplanar,
    );
    capture(&mut session, &request, &map);
    let mut map = refreshed(map, session.revision());
    map.values_mut().next().unwrap().elements[0].geometry = Some(AnalyticGeometry::CircularArc {
        frame,
        radius_mm: 8.,
        start_rad: 0.,
        sweep_rad: std::f64::consts::TAU,
    });
    let report = references::inspect(session.document(), session.revision(), &map);
    let before = canonical(&session, sketch);
    let evaluated = evaluate_document_sketch_with_parameters(
        session.document(),
        session.revision(),
        sketch,
        Some(&report),
        session.parameters(),
    )
    .unwrap();
    assert!(
        matches!(evaluated.geometry().unwrap()[0],multicad_core::sketch::Geometry::Circle{radius_mm,..} if (radius_mm-8.).abs()<1e-7)
    );
    assert_eq!(canonical(&session, sketch), before);
    let mut conflicting = before;
    conflicting.add_constraint(ConstraintKind::Radius {
        curve: request.output.curve().unwrap(),
        dimension: Dimension::Literal {
            value: 11.,
            unit: "mm".into(),
        },
    });
    let edit = prepare_canonical_edit(
        &session,
        session.revision(),
        actor(),
        sketch,
        conflicting,
        Some(&report),
    )
    .unwrap();
    assert!(edit.preview.is_none());
}
#[test]
fn finite_intersection_point_is_selectable_for_constraints_and_repair_requires_a_full_consumer_solve()
 {
    let (mut session, sketch, request, map) = fixture(
        AnalyticGeometry::Line {
            a_mm: [3., 4., -10.],
            b_mm: [3., 4., 10.],
        },
        Method::Intersect,
    );
    capture(&mut session, &request, &map);
    let map = refreshed(map, session.revision());
    let source = canonical(&session, sketch);
    let OutputIds::Point { point, .. } = request.output else {
        panic!()
    };
    assert_eq!(
        source
            .sketch
            .points
            .iter()
            .find(|p| p.id == point)
            .unwrap()
            .position_mm,
        [3., 4.]
    );
    let mut replacement = map.clone();
    replacement.values_mut().next().unwrap().elements[0].geometry = Some(AnalyticGeometry::Line {
        a_mm: [8., 9., -10.],
        b_mm: [8., 9., 10.],
    });
    let selected = replacement[&request.input.source].elements[0].key;
    let before = session.document().clone();
    let edit = prepare_external_repair(
        &session,
        session.revision(),
        actor(),
        &ExternalRepairRequest {
            sketch,
            reference: request.reference,
            source: request.input.source,
            selected,
        },
        &replacement,
    )
    .unwrap();
    let evaluated = edit.semantic.unwrap();
    assert_eq!(
        evaluated
            .solved()
            .unwrap()
            .points
            .iter()
            .find(|p| p.id == point)
            .unwrap()
            .position_mm,
        [8., 9.]
    );
    assert_eq!(session.document(), &before);
    replacement.values_mut().next().unwrap().elements[0].geometry = Some(AnalyticGeometry::Line {
        a_mm: [8., 9., 10.],
        b_mm: [8., 9., 20.],
    });
    assert!(
        prepare_external_repair(
            &session,
            session.revision(),
            actor(),
            &ExternalRepairRequest {
                sketch,
                reference: request.reference,
                source: request.input.source,
                selected
            },
            &replacement
        )
        .is_err()
    );
}
#[test]
fn same_revision_source_edits_invalidate_old_report_and_stale_captures_do_not_mutate_session() {
    let (mut session, sketch, request, map) = fixture(
        AnalyticGeometry::Line {
            a_mm: [0., 0., 0.],
            b_mm: [30., 0., 0.],
        },
        Method::Coplanar,
    );
    capture(&mut session, &request, &map);
    let map = refreshed(map, session.revision());
    let old_report = references::inspect(session.document(), session.revision(), &map);
    let mut changed = session
        .document()
        .get(request.input.source)
        .unwrap()
        .clone();
    let EntityData::Feature { definition, .. } = &mut changed.data else {
        panic!()
    };
    definition.data = serde_json::json!({"generation":2});
    let candidate = session
        .document()
        .edited(&[Mutation::Replace(changed)])
        .unwrap();
    let context = SketchEvaluationContext::new(&candidate, session.revision()).unwrap();
    assert!(context.evaluate(sketch, Some(&old_report), None).is_err());
    assert!(prepare_external_reference(&session, 0, actor(), &request, &map).is_err());
    assert!(
        prepare_external_reference(&session, session.revision(), actor(), &request, &map).is_err()
    );
    let before = session.document().clone();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Removed source".into(),
            edits: vec![Mutation::Remove(request.input.source)],
        })
        .unwrap();
    let bad = references::inspect(session.document(), session.revision(), &BTreeMap::new());
    assert!(
        evaluate_document_sketch_with_parameters(
            session.document(),
            session.revision(),
            sketch,
            Some(&bad),
            session.parameters()
        )
        .is_err()
    );
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
}

//! Production public occurrence query/preparation paths, with genuine OCCT source.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::{ConstraintKind, source::Source},
    document::{Document, EntityId},
    model::{self, OccurrencePath, PrimitiveParameters, PrimitiveRequest},
    references::{AnalyticGeometry, GeometryIntent},
    sketch::Datum,
    sketch_external::*,
};
use multicad_model::Compiler;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn seed() -> (Session, EntityId, ExternalQuery) {
    let (sketch, edits) = Source::create(Datum::Xy, "Native consumer").unwrap();
    let mut session = Session::new(
        Document::new("Native occurrence references")
            .unwrap()
            .edited(&edits)
            .unwrap(),
    )
    .unwrap();
    let request = PrimitiveRequest::new(
        "Hardware",
        PrimitiveParameters::Box {
            size: [model::length(20.), model::length(10.), model::length(4.)],
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(&session, 0, actor(), &request).unwrap();
    let output = prepared.outputs[0].clone();
    session.commit(prepared.prepared).unwrap();
    let native = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let topology = native
        .solid(output.body)
        .unwrap()
        .topology_snapshot(output.body, session.revision())
        .unwrap();
    let key=topology.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Line{a_mm,b_mm})if a_mm[1].abs()<1e-7&&b_mm[1].abs()<1e-7&&a_mm[2].abs()<1e-7&&b_mm[2].abs()<1e-7&&(a_mm[0]-b_mm[0]).abs()>19.)).unwrap().key;
    let query = ExternalQuery {
        reference: EntityId::new(),
        sketch,
        source: OccurrenceSource {
            component: output.body,
            source: output.body,
            path: OccurrencePath {
                root: output.body,
                members: vec![],
            },
        },
        sketch_context: None,
        intent: GeometryIntent::capture(&topology, key).unwrap(),
        method: QueryMethod::Coplanar,
    };
    (session, sketch, query)
}
#[test]
fn public_native_capture_remove_keep_original_session_identity_and_one_undo_transaction() {
    let (mut session, sketch, query) = seed();
    let compiler = Compiler::default();
    let before = session.document().clone();
    let catalog = compiler
        .query_external_catalog(&before, session.revision(), &query, None)
        .unwrap();
    assert_eq!(catalog.results.len(), 1);
    let request =
        OccurrenceCaptureRequest::new(&catalog, &catalog.results[0].selector, Behavior::Derived)
            .unwrap();
    let prepared = compiler
        .prepare_external_capture(&session.preparation_snapshot(), actor(), &request, None)
        .unwrap();
    assert_eq!(session.document(), &before);
    assert!(
        prepared
            .report
            .external_results
            .contains_key(&query.reference)
    );
    assert!(
        !prepared
            .report
            .external_errors
            .contains_key(&query.reference)
    );
    assert!(prepared.edit.semantic.as_ref().unwrap().solved().is_some());
    let mut foreign = Session::new(before.clone()).unwrap();
    assert!(
        foreign
            .commit(prepared.edit.preview.as_ref().unwrap().clone())
            .is_err()
    );
    session.commit(prepared.edit.preview.unwrap()).unwrap();
    let after = session.document().clone();
    let mut archive = vec![];
    multicad_core::project_file::write(
        &multicad_core::project_file::ProjectFile::new(after.clone()).unwrap(),
        &mut archive,
        Default::default(),
    )
    .unwrap();
    let reopened = multicad_core::project_file::read(
        &mut std::io::Cursor::new(archive),
        Default::default(),
        &multicad_core::project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    assert_eq!(reopened, after);
    let reopened_native = compiler.compile(&reopened, 0).unwrap();
    assert!(
        reopened_native
            .snapshot()
            .references
            .external_results
            .contains_key(&query.reference)
    );
    assert!(
        multicad_solver::evaluate_document_sketch(
            &reopened,
            0,
            sketch,
            Some(&reopened_native.snapshot().references)
        )
        .unwrap()
        .solved()
        .is_some()
    );
    assert_eq!(
        occurrence_bindings(session.document(), sketch).unwrap(),
        vec![request.clone()]
    );
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &after);
    let removed = compiler
        .prepare_external_remove(
            &session.preparation_snapshot(),
            actor(),
            sketch,
            query.reference,
            None,
        )
        .unwrap();
    session.commit(removed.edit.preview.unwrap()).unwrap();
    assert!(
        occurrence_bindings(session.document(), sketch)
            .unwrap()
            .is_empty()
    );
    assert!(
        Source::read_for_evaluation(session.document(), sketch, session.revision(), None)
            .unwrap()
            .0
            .sketch
            .curves
            .is_empty()
    );
}
#[test]
fn checked_native_receipt_allows_consumer_coincident_edit_but_rejects_same_revision_upstream_change()
 {
    let (mut session, sketch, query) = seed();
    let compiler = Compiler::default();
    let catalog = compiler
        .query_external_catalog(session.document(), session.revision(), &query, None)
        .unwrap();
    let request =
        OccurrenceCaptureRequest::new(&catalog, &catalog.results[0].selector, Behavior::Derived)
            .unwrap();
    let edit = compiler
        .prepare_external_capture(&session.preparation_snapshot(), actor(), &request, None)
        .unwrap();
    session.commit(edit.edit.preview.unwrap()).unwrap();
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let mut source = Source::read_for_evaluation(
        session.document(),
        sketch,
        session.revision(),
        Some(&native.snapshot().references),
    )
    .unwrap()
    .0;
    let p = source.add_point([30., 5.]);
    source.add_constraint(ConstraintKind::Coincident {
        a: p,
        b: request.output.points[1],
    });
    let edit = multicad_solver::prepare_canonical_edit(
        &session,
        session.revision(),
        actor(),
        sketch,
        source,
        Some(&native.snapshot().references),
    )
    .unwrap();
    assert!(edit.preview.is_some());
    let solved = edit.semantic.as_ref().unwrap().solved().unwrap();
    let point = solved.points.iter().find(|x| x.id == p).unwrap();
    assert!(point.position_mm[1].abs() < 1e-7);
    assert!((point.position_mm[0] - 20.).abs() < 1e-7 || point.position_mm[0].abs() < 1e-7);
    let source_after = edit.preview.unwrap().document().clone();
    assert!(compiler.compile(&source_after, session.revision()).is_ok());
    let producer = native.snapshot().body_sources[&query.source.component];
    let changed = model::prepare_primitive_edit(
        &session,
        session.revision(),
        actor(),
        producer,
        &PrimitiveParameters::Box {
            size: [model::length(30.), model::length(10.), model::length(4.)],
            frame: Default::default(),
        },
    )
    .unwrap();
    assert!(
        multicad_solver::evaluate_document_sketch_with_parameters(
            changed.prepared.document(),
            session.revision(),
            sketch,
            Some(&native.snapshot().references),
            changed.prepared.parameters()
        )
        .is_err()
    );
    assert_eq!(session.revision(), 2);
}
#[test]
fn unrelated_broken_unconsumed_reference_blocks_native_capture_apply_without_source_changes() {
    let (mut session, sketch, old_query) = seed();
    let compiler = Compiler::default();
    let old = compiler
        .query_external_catalog(session.document(), session.revision(), &old_query, None)
        .unwrap();
    let r =
        OccurrenceCaptureRequest::new(&old, &old.results[0].selector, Behavior::Derived).unwrap();
    let prepared = compiler
        .prepare_external_capture(&session.preparation_snapshot(), actor(), &r, None)
        .unwrap();
    session.commit(prepared.edit.preview.unwrap()).unwrap();
    session
        .apply(multicad_core::commands::Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Remove referenced hardware source".into(),
            edits: vec![multicad_core::document::Mutation::Remove(
                old_query.source.component,
            )],
        })
        .unwrap();
    let box_request = PrimitiveRequest::new(
        "Independent new hardware",
        PrimitiveParameters::Box {
            size: [model::length(20.), model::length(10.), model::length(4.)],
            frame: Default::default(),
        },
    );
    let created =
        model::prepare_primitive(&session, session.revision(), actor(), &box_request).unwrap();
    let output = created.outputs[0].clone();
    session.commit(created.prepared).unwrap();
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    assert!(!native.snapshot().references.external_errors.is_empty());
    let topology = native
        .solid(output.body)
        .unwrap()
        .topology_snapshot(output.body, session.revision())
        .unwrap();
    let key=topology.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Line{a_mm,b_mm})if a_mm[2].abs()<1e-7&&b_mm[2].abs()<1e-7)).unwrap().key;
    let query = ExternalQuery {
        reference: EntityId::new(),
        sketch,
        source: OccurrenceSource {
            component: output.body,
            source: output.body,
            path: OccurrencePath {
                root: output.body,
                members: vec![],
            },
        },
        sketch_context: None,
        intent: GeometryIntent::capture(&topology, key).unwrap(),
        method: QueryMethod::Coplanar,
    };
    let before = session.document().clone();
    let revision = session.revision();
    let catalog = compiler
        .query_external_catalog(&before, revision, &query, None)
        .unwrap();
    let request =
        OccurrenceCaptureRequest::new(&catalog, &catalog.results[0].selector, Behavior::Derived)
            .unwrap();
    let error = compiler
        .prepare_external_capture(&session.preparation_snapshot(), actor(), &request, None)
        .err()
        .unwrap();
    assert!(
        error.message.contains("complete current reference report"),
        "{error}"
    );
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), revision);
}
#[test]
fn same_revision_consumer_membership_change_cannot_reuse_old_native_receipt() {
    let (mut session, sketch, mut query) = seed();
    let mut source =
        Source::read_for_evaluation(session.document(), sketch, session.revision(), None)
            .unwrap()
            .0;
    source.add_rectangle([0., 0.], [100., 50.]).unwrap();
    let edits = source.replace(session.document(), sketch).unwrap();
    session
        .apply(multicad_core::commands::Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Create receiving panel sketch".into(),
            edits,
        })
        .unwrap();
    let evaluated = multicad_solver::evaluate_document_sketch(
        session.document(),
        session.revision(),
        sketch,
        None,
    )
    .unwrap();
    let region = evaluated.regions().unwrap().regions[0].intent();
    let p = model::prepare_extrude(
        &session,
        session.revision(),
        actor(),
        &model::ExtrudeRequest::new(sketch, vec![region], Default::default()),
    )
    .unwrap();
    let receiver = p.outputs[0].clone();
    session.commit(p.prepared).unwrap();
    query.sketch_context = Some(OccurrenceContext {
        component: receiver.body,
        path: OccurrencePath {
            root: receiver.body,
            members: vec![],
        },
    });
    let compiler = Compiler::default();
    let catalog = compiler
        .query_external_catalog(session.document(), session.revision(), &query, None)
        .unwrap();
    let request = OccurrenceCaptureRequest::new(
        &catalog,
        &catalog.results[0].selector,
        Behavior::ReferenceOnly,
    )
    .unwrap();
    let p = compiler
        .prepare_external_capture(&session.preparation_snapshot(), actor(), &request, None)
        .unwrap();
    session.commit(p.edit.preview.unwrap()).unwrap();
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    assert!(
        multicad_solver::evaluate_document_sketch(
            session.document(),
            session.revision(),
            sketch,
            Some(&native.snapshot().references)
        )
        .unwrap()
        .solved()
        .is_some()
    );
    let mut body = session.document().get(receiver.body).unwrap().clone();
    let multicad_core::document::EntityData::Part { features, .. } = &mut body.data else {
        panic!()
    };
    features.clear();
    let changed = session
        .document()
        .edited(&[
            multicad_core::document::Mutation::Replace(body),
            multicad_core::document::Mutation::Remove(receiver.feature),
        ])
        .unwrap();
    assert!(validate_occurrence_query(&changed, &request.query).is_err());
    let error = multicad_solver::evaluate_document_sketch(
        &changed,
        session.revision(),
        sketch,
        Some(&native.snapshot().references),
    )
    .expect_err("same-revision old receipt must not authorize a no-longer-owned consumer frame");
    assert!(error.message.contains("own"), "{error}");
    assert!(
        native
            .snapshot()
            .references
            .external_results
            .contains_key(&query.reference)
    );
}

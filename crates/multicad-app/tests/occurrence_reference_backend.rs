//! Actual native application transport: source-local B-reps, placed instances,
//! exact result catalogues and Session-owned preparations. The included module
//! is the production app backend, not a test geometry/query implementation.
#[path = "../src/external_queries.rs"]
mod external_queries;
#[path = "../src/reference_repairs.rs"]
mod reference_repairs;
use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Request, Session},
    constraint_sketch::{ConstraintKind, source::Source},
    document::{Document, EntityId},
    model::{
        self, BodyInput, InstanceCopyRequest, InstanceCopySource, NumericTransform, OccurrencePath,
        PrimitiveParameters, PrimitiveRequest, angle, length,
    },
    project_file::{self, ContentHash, FileLimits, ProjectFile, ProjectResources},
    references::{AnalyticGeometry, GeometryIntent},
    sketch::Datum,
    sketch_external::{
        self, Behavior, ExternalCatalog, ExternalGeometry, ExternalQuery, OccurrenceCaptureRequest,
        OccurrenceSource, QueryMethod, RepairSet, RepairStamp, RepairStep,
    },
};
use multicad_model::{Compiler, GeometryBackend};
use multicad_ui::external_references::{
    ExternalPrepared, ExternalQueryBackend, ExternalQueryReply,
};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{a} != {b}");
}
fn poll(backend: &mut external_queries::Backend) -> ExternalQueryReply {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(reply) = backend.poll() {
            return reply;
        }
        assert!(
            Instant::now() < deadline,
            "Native reference backend did not settle"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
struct Fixture {
    session: Session,
    part: PrimitiveRequest,
    sketch: EntityId,
    copies: Vec<InstanceCopyRequest>,
}
fn fixture(cylinder: bool) -> Fixture {
    let (sketch, edits) = Source::create(Datum::Xy, "Native occurrence consumer").unwrap();
    let mut session = Session::new(
        Document::new("Native backend references")
            .unwrap()
            .edited(&edits)
            .unwrap(),
    )
    .unwrap();
    let part = PrimitiveRequest::new(
        "Native hardware",
        if cylinder {
            PrimitiveParameters::Cylinder {
                radius: length(5.),
                height: length(3.),
                frame: Default::default(),
            }
        } else {
            PrimitiveParameters::Box {
                size: [length(30.), length(20.), length(4.)],
                frame: Default::default(),
            }
        },
    );
    let prepared = model::prepare_primitive(&session, session.revision(), actor(), &part).unwrap();
    session.commit(prepared.prepared).unwrap();
    let copies = (0..if cylinder { 1 } else { 2 })
        .map(|index| {
            InstanceCopyRequest::new(
                InstanceCopySource::Part {
                    source: BodyInput {
                        body: part.body,
                        producer: part.feature,
                    },
                },
                NumericTransform {
                    axis: if cylinder { [0., 1., 0.] } else { [0., 0., 1.] },
                    angle: angle(90.),
                    translation: if cylinder {
                        [length(0.), length(0.), length(0.)]
                    } else {
                        [
                            length(100. + 50. * f64::from(index)),
                            length(200.),
                            length(0.),
                        ]
                    },
                    ..Default::default()
                },
            )
        })
        .collect::<Vec<_>>();
    let prepared =
        model::prepare_instance_copies(&session, session.revision(), actor(), &copies).unwrap();
    session.commit(prepared.prepared).unwrap();
    Fixture {
        session,
        part,
        sketch,
        copies,
    }
}
fn native_query(f: &Fixture, copy: usize, method: QueryMethod) -> ExternalQuery {
    query_with_backend(f, copy, method, GeometryBackend::OpenCascade)
}
fn query_with_backend(
    f: &Fixture,
    copy: usize,
    method: QueryMethod,
    backend: GeometryBackend,
) -> ExternalQuery {
    let native = Compiler::default()
        .compile_with_backend(f.session.document(), f.session.revision(), backend, None)
        .unwrap();
    let topology = native
        .solid(f.part.body)
        .unwrap()
        .topology_snapshot(f.part.body, f.session.revision())
        .unwrap();
    let selected = topology
        .elements
        .iter()
        .find(|e| match e.geometry {
            Some(AnalyticGeometry::Line { a_mm: a, b_mm: b }) => {
                a[1].abs() < 1e-8
                    && b[1].abs() < 1e-8
                    && a[2].abs() < 1e-8
                    && b[2].abs() < 1e-8
                    && (a[0] - b[0]).abs() > 29.
            }
            Some(AnalyticGeometry::CircularArc { frame, .. }) => frame.origin_mm[2].abs() < 1e-8,
            _ => false,
        })
        .expect("genuine native source edge");
    ExternalQuery {
        reference: EntityId::new(),
        sketch: f.sketch,
        source: OccurrenceSource {
            component: f.part.body,
            source: f.part.body,
            path: OccurrencePath {
                root: f.copies[copy].instance,
                members: vec![],
            },
        },
        sketch_context: None,
        intent: GeometryIntent::capture(&topology, selected.key).unwrap(),
        method,
    }
}
fn catalog(
    backend: &mut external_queries::Backend,
    session: &Session,
    query: &ExternalQuery,
) -> ExternalCatalog {
    let request = backend
        .request_catalog(
            session.preparation_snapshot(),
            ProjectResources::default(),
            query.clone(),
        )
        .unwrap();
    let ExternalQueryReply::Catalog {
        request: received,
        query: echo,
        result,
    } = poll(backend)
    else {
        panic!("catalogue reply")
    };
    assert_eq!(received, request);
    assert_eq!(&echo, query);
    let result = *result.unwrap();
    result.validate().unwrap();
    assert_eq!(result.document, session.document().id());
    assert_eq!(result.revision, session.revision());
    assert_eq!(
        result.source_hash,
        ContentHash::document(session.document()).unwrap()
    );
    assert_eq!(result.query_hash, query.hash().unwrap());
    result
}
fn prepare(
    backend: &mut external_queries::Backend,
    session: &Session,
    input: &OccurrenceCaptureRequest,
    repair: bool,
) -> Result<Box<ExternalPrepared>, String> {
    let request = backend
        .request_prepare(
            session.preparation_snapshot(),
            ProjectResources::default(),
            actor(),
            input.clone(),
            repair,
        )
        .unwrap();
    let ExternalQueryReply::Prepared {
        request: received,
        source_hash,
        input: echo,
        repair: mode,
        result,
    } = poll(backend)
    else {
        panic!("prepared reply")
    };
    assert_eq!(received, request);
    assert_eq!(
        source_hash,
        ContentHash::document(session.document()).unwrap()
    );
    assert_eq!(&echo, input);
    assert_eq!(mode, repair);
    result
}
fn line_x(catalog: &ExternalCatalog, x: f64) {
    assert_eq!(catalog.results.len(), 1);
    let ExternalGeometry::Line { a_mm: a, b_mm: b } = catalog.results[0].geometry else {
        panic!("exact finite line")
    };
    near(a[0], x);
    near(b[0], x);
    near(a[1].min(b[1]), 200.);
    near(a[1].max(b[1]), 230.);
}

#[test]
fn rust_worker_captures_a_placed_line_that_rebuilds_its_consuming_extrusion_after_reopen() {
    let mut f = fixture(false);
    let backend = GeometryBackend::Rust;
    let compiler = Compiler::default();
    let mut worker = external_queries::Backend::with_geometry_backend(backend);
    let query = query_with_backend(&f, 0, QueryMethod::Project, backend);
    let discovered = catalog(&mut worker, &f.session, &query);
    line_x(&discovered, 100.);
    let input = OccurrenceCaptureRequest::new(
        &discovered,
        &discovered.results[0].selector,
        Behavior::Derived,
    )
    .unwrap();
    let prepared = prepare(&mut worker, &f.session, &input, false).unwrap();
    assert!(
        prepared
            .report
            .external_results
            .contains_key(&input.query.reference)
    );
    f.session.commit(prepared.edit.preview.unwrap()).unwrap();

    let ExternalGeometry::Line { a_mm, b_mm } = discovered.results[0].geometry else {
        panic!("Expected the exact projected line");
    };
    let [mut low, mut high] = [input.output.points[0], input.output.points[1]];
    if a_mm[1] > b_mm[1] {
        std::mem::swap(&mut low, &mut high);
    }
    let mut source =
        Source::read_for_evaluation(f.session.document(), f.sketch, f.session.revision(), None)
            .unwrap()
            .0;
    let right_low = source.add_point([110., 200.]);
    let right_high = source.add_point([110., 230.]);
    for point in [right_low, right_high] {
        source.add_constraint(ConstraintKind::Fix { point });
    }
    source.add_line(low, right_low);
    source.add_line(right_low, right_high);
    source.add_line(right_high, high);
    let edits = source.replace(f.session.document(), f.sketch).unwrap();
    f.session
        .apply(Request {
            expected_revision: f.session.revision(),
            actor: actor(),
            label: "Close the outline using the projected edge".into(),
            edits,
        })
        .unwrap();
    let model = compiler
        .compile_with_backend(f.session.document(), f.session.revision(), backend, None)
        .unwrap();
    let evaluated = multicad_solver::evaluate_document_sketch(
        f.session.document(),
        f.session.revision(),
        f.sketch,
        Some(&model.snapshot().references),
    )
    .unwrap();
    let region = evaluated.regions().unwrap().regions[0].intent();
    let extrusion = model::ExtrudeRequest::new(f.sketch, vec![region], Default::default());
    let prepared =
        model::prepare_extrude(&f.session, f.session.revision(), actor(), &extrusion).unwrap();
    let output = prepared.outputs[0].clone();
    f.session.commit(prepared.prepared).unwrap();

    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(f.session.document().clone()).unwrap(),
        &mut bytes,
        FileLimits::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let mut session = Session::new(reopened.project.document).unwrap();
    let rebuild = |session: &Session| {
        compiler
            .compile_with_backend(session.document(), session.revision(), backend, None)
            .unwrap()
    };
    let model = rebuild(&session);
    near(model.snapshot().shapes[&output.feature].volume_mm3, 3000.);
    assert!(
        model
            .snapshot()
            .references
            .external_results
            .contains_key(&input.query.reference)
    );

    let mut placement = f.copies[0].parameters.clone();
    placement.translation[0] = length(98.);
    let edit = model::prepare_instance_placement_edit(
        &session,
        session.revision(),
        actor(),
        f.copies[0].driver,
        &placement,
    )
    .unwrap();
    session.commit(edit.prepared).unwrap();
    let model = rebuild(&session);
    near(model.snapshot().shapes[&output.feature].volume_mm3, 3600.);
    line_x(&catalog(&mut worker, &session, &query), 98.);

    let mut second = f.copies[1].parameters.clone();
    second.translation[0] = length(103.);
    let edit = model::prepare_instance_placement_edit(
        &session,
        session.revision(),
        actor(),
        f.copies[1].driver,
        &second,
    )
    .unwrap();
    session.commit(edit.prepared).unwrap();
    let mut replacement = query;
    replacement.source.path.root = f.copies[1].instance;
    let replacement = catalog(&mut worker, &session, &replacement);
    line_x(&replacement, 103.);
    let repair = OccurrenceCaptureRequest::repair(
        &replacement,
        &replacement.results[0].selector,
        Behavior::Derived,
        &input,
    )
    .unwrap();
    assert_eq!(repair.output, input.output);
    let before = session.document().clone();
    let resources = "no resources".to_owned();
    let set = RepairSet {
        stamp: RepairStamp::current(&before, session.revision(), Some(resources.clone())).unwrap(),
        steps: vec![RepairStep::RebindOccurrence {
            request: Box::new(repair),
        }],
    };
    let checked = reference_repairs::check_with_geometry_backend(
        backend,
        session.preparation_snapshot(),
        set,
        resources,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert!(checked.can_apply(), "{:?}", checked.blockers);
    assert_eq!(session.document(), &before);
    session.commit(checked.prepared.unwrap()).unwrap();
    near(
        rebuild(&session).snapshot().shapes[&output.feature].volume_mm3,
        2100.,
    );
}

#[test]
fn rust_worker_captures_a_rotated_rim_contact_that_follows_its_source_after_reopen() {
    let f = fixture(true);
    let before = f.session.document().clone();
    let backend = GeometryBackend::Rust;
    let rust = query_with_backend(&f, 0, QueryMethod::Intersect, backend);
    let mut worker = external_queries::Backend::with_geometry_backend(backend);
    let found = catalog(&mut worker, &f.session, &rust);
    let point = |choice: &sketch_external::ExternalChoice| {
        let ExternalGeometry::Point { position_mm } = choice.geometry else {
            panic!("Expected a finite rim contact");
        };
        position_mm
    };
    assert_eq!(found.results.len(), 2);
    let mut ys = found
        .results
        .iter()
        .map(|c| point(c)[1])
        .collect::<Vec<_>>();
    ys.sort_by(f64::total_cmp);
    near(ys[0], -5.);
    near(ys[1], 5.);
    assert!(
        found
            .results
            .iter()
            .all(|c| point(c)[0].abs() < 1e-7 && !c.tangent)
    );
    assert_ne!(
        found.results[0].selector.branch,
        found.results[1].selector.branch
    );

    if multicad_kernel::OCCT_LINKED {
        let mut native_worker = external_queries::Backend::new();
        let native = catalog(
            &mut native_worker,
            &f.session,
            &native_query(&f, 0, QueryMethod::Intersect),
        );
        assert_eq!(native.results.len(), found.results.len());
        for rust in &found.results {
            assert!(native.results.iter().any(|reference| {
                let [a, b] = [point(rust), point(reference)];
                (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-7
            }));
        }
    }

    let selected = found.results.iter().find(|c| point(c)[1] > 0.).unwrap();
    let request =
        OccurrenceCaptureRequest::new(&found, &selected.selector, Behavior::Derived).unwrap();
    let prepared = prepare(&mut worker, &f.session, &request, false).unwrap();
    assert_eq!(f.session.document(), &before);
    let mut session = f.session;
    session.commit(prepared.edit.preview.unwrap()).unwrap();
    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        FileLimits::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let mut session = Session::new(reopened.project.document).unwrap();
    assert_eq!(
        sketch_external::read_occurrence(session.document(), request.query.reference).unwrap(),
        request
    );
    let assert_receipt = |session: &Session, expected_y: f64| {
        let model = Compiler::default()
            .compile_with_backend(session.document(), session.revision(), backend, None)
            .unwrap();
        assert!(model.snapshot().references.external_errors.is_empty());
        let receipt = &model.snapshot().references.external_results[&request.query.reference];
        assert_eq!(receipt.selector, request.selector);
        let ExternalGeometry::Point { position_mm } = receipt.geometry else {
            panic!("Expected a resolved sketch point");
        };
        near(position_mm[0], 0.);
        near(position_mm[1], expected_y);
    };
    assert_receipt(&session, 5.);

    let mut moved = f.copies[0].parameters.clone();
    moved.translation[1] = length(2.);
    moved.translation[2] = length(1.);
    let edit = model::prepare_instance_placement_edit(
        &session,
        session.revision(),
        actor(),
        f.copies[0].driver,
        &moved,
    )
    .unwrap();
    session.commit(edit.prepared).unwrap();
    assert_receipt(&session, 2. + 24_f64.sqrt());
}
#[test]
fn rust_worker_captures_an_oblique_rim_as_an_exact_rational_reference() {
    let mut f = fixture(true);
    let backend = GeometryBackend::Rust;
    let mut placement = f.copies[0].parameters.clone();
    placement.angle = angle(45.);
    let edit = model::prepare_instance_placement_edit(
        &f.session,
        f.session.revision(),
        actor(),
        f.copies[0].driver,
        &placement,
    )
    .unwrap();
    f.session.commit(edit.prepared).unwrap();
    let query = query_with_backend(&f, 0, QueryMethod::Project, backend);
    let mut worker = external_queries::Backend::with_geometry_backend(backend);
    let found = catalog(&mut worker, &f.session, &query);
    assert_eq!(found.results.len(), 1);
    let ExternalGeometry::BSpline { poles_mm, basis } = &found.results[0].geometry else {
        panic!("Expected an exact rational ellipse");
    };
    assert_eq!(basis.degree, 2);
    assert_eq!(basis.multiplicities, [3, 2, 2, 2, 3]);
    assert_eq!(poles_mm[0], poles_mm[8]);
    for i in 0..=32 {
        let [x, y] = basis.evaluate(poles_mm, f64::from(i) / 32.).unwrap().point;
        near(2. * x.powi(2) + y.powi(2), 25.);
    }
    let request =
        OccurrenceCaptureRequest::new(&found, &found.results[0].selector, Behavior::ReferenceOnly)
            .unwrap();
    let prepared = prepare(&mut worker, &f.session, &request, false).unwrap();
    f.session.commit(prepared.edit.preview.unwrap()).unwrap();
    let rebuilt = Compiler::default()
        .compile_with_backend(f.session.document(), f.session.revision(), backend, None)
        .unwrap();
    let receipt = &rebuilt.snapshot().references.external_results[&request.query.reference];
    assert_eq!(receipt.geometry, found.results[0].geometry);
    let source = Source::read_for_evaluation(
        f.session.document(),
        f.sketch,
        f.session.revision(),
        Some(&rebuilt.snapshot().references),
    )
    .unwrap()
    .0;
    assert!(
        source
            .sketch
            .curves
            .iter()
            .any(|curve| Some(curve.id) == request.output.curve && curve.construction)
    );
}

#[test]
fn actual_worker_capture_repair_remove_preserve_source_and_original_session_ownership() {
    let mut f = fixture(false);
    let source = f.session.document().clone();
    let revision = f.session.revision();
    let mut backend = external_queries::Backend::new();
    let first = native_query(&f, 0, QueryMethod::Coplanar);
    let discovered = catalog(&mut backend, &f.session, &first);
    line_x(&discovered, 100.);
    let input = OccurrenceCaptureRequest::new(
        &discovered,
        &discovered.results[0].selector,
        Behavior::Derived,
    )
    .unwrap();
    let prepared = prepare(&mut backend, &f.session, &input, false).unwrap();
    assert_eq!(prepared.reference, input.query.reference);
    let preview = prepared.edit.preview.as_ref().unwrap();
    f.session.validate_preview(preview).unwrap();
    assert!(
        prepared
            .report
            .external_results
            .contains_key(&input.query.reference)
    );
    let foreign = Session::new(source.clone()).unwrap();
    assert_eq!(
        foreign.validate_preview(preview).unwrap_err().code,
        ErrorCode::ForeignPreview
    );
    assert_eq!(f.session.document(), &source);
    assert_eq!(f.session.revision(), revision);
    let stale = preview.clone();
    f.session.commit(prepared.edit.preview.unwrap()).unwrap();
    assert_eq!(
        f.session.validate_preview(&stale).unwrap_err().code,
        ErrorCode::StaleRevision
    );
    let stored =
        sketch_external::read_occurrence(f.session.document(), input.query.reference).unwrap();
    assert_eq!(stored.query.source.path, input.query.source.path);
    assert_eq!(stored.output, input.output);
    let native = Compiler::default()
        .compile(f.session.document(), f.session.revision())
        .unwrap();
    near(native.snapshot().shapes[&f.part.feature].volume_mm3, 2400.);
    assert!(native.snapshot().references.external_errors.is_empty());
    let mut replacement = native_query(&f, 1, QueryMethod::Coplanar);
    replacement.reference = input.query.reference;
    let discovered = catalog(&mut backend, &f.session, &replacement);
    line_x(&discovered, 150.);
    let mut repaired = OccurrenceCaptureRequest::new(
        &discovered,
        &discovered.results[0].selector,
        Behavior::Derived,
    )
    .unwrap();
    repaired.output = input.output.clone();
    let before = f.session.document().clone();
    let checked = prepare(&mut backend, &f.session, &repaired, true).unwrap();
    assert_eq!(f.session.document(), &before);
    f.session.commit(checked.edit.preview.unwrap()).unwrap();
    assert_eq!(
        sketch_external::read_occurrence(f.session.document(), input.query.reference).unwrap(),
        repaired
    );
    let before = f.session.document().clone();
    let request = backend
        .request_remove(
            f.session.preparation_snapshot(),
            ProjectResources::default(),
            actor(),
            f.sketch,
            input.query.reference,
        )
        .unwrap();
    let ExternalQueryReply::Removed {
        request: received,
        source_hash,
        sketch,
        reference,
        result,
    } = poll(&mut backend)
    else {
        panic!("removed reply")
    };
    assert_eq!(request, received);
    assert_eq!(sketch, f.sketch);
    assert_eq!(reference, input.query.reference);
    assert_eq!(source_hash, ContentHash::document(&before).unwrap());
    assert_eq!(f.session.document(), &before);
    let checked = result.unwrap();
    f.session
        .validate_preview(checked.edit.preview.as_ref().unwrap())
        .unwrap();
    f.session.commit(checked.edit.preview.unwrap()).unwrap();
    assert!(
        sketch_external::occurrence_bindings(f.session.document(), f.sketch)
            .unwrap()
            .is_empty()
    );
    f.session.undo(f.session.revision(), actor()).unwrap();
    assert_eq!(f.session.document(), &before);
    f.session.redo(f.session.revision(), actor()).unwrap();
    near(
        Compiler::default()
            .compile(f.session.document(), f.session.revision())
            .unwrap()
            .snapshot()
            .shapes[&f.part.feature]
            .volume_mm3,
        2400.,
    );
}
#[test]
fn actual_worker_empty_tangent_singular_and_changed_branch_fail_without_source_mutation() {
    let mut f = fixture(true);
    let mut backend = external_queries::Backend::new();
    let query = native_query(&f, 0, QueryMethod::Intersect);
    let discovered = catalog(&mut backend, &f.session, &query);
    assert_eq!(discovered.results.len(), 2);
    assert!(discovered.results.iter().all(|r| !r.tangent));
    let mut contacts = discovered
        .results
        .iter()
        .map(|r| match r.geometry {
            ExternalGeometry::Point { position_mm } => position_mm,
            _ => panic!("Exact finite circle/plane contacts must be points"),
        })
        .collect::<Vec<_>>();
    contacts.sort_by(|a, b| a[1].total_cmp(&b[1]));
    near(contacts[0][0], 0.);
    near(contacts[1][0], 0.);
    near(contacts[0][1], -5.);
    near(contacts[1][1], 5.);
    let capture = OccurrenceCaptureRequest::new(
        &discovered,
        &discovered.results[0].selector,
        Behavior::ReferenceOnly,
    )
    .unwrap();
    let mut singular = query.clone();
    singular.method = QueryMethod::Project;
    backend
        .request_catalog(
            f.session.preparation_snapshot(),
            Default::default(),
            singular,
        )
        .unwrap();
    let ExternalQueryReply::Catalog { result, .. } = poll(&mut backend) else {
        panic!()
    };
    assert!(result.unwrap_err().contains("singular"));
    let mut damaged = capture.clone();
    damaged.selector.expected_count = 1;
    let before = f.session.document().clone();
    assert!(
        prepare(&mut backend, &f.session, &damaged, false)
            .err()
            .expect("Changed native count must fail")
            .contains("count changed")
    );
    assert_eq!(f.session.document(), &before);
    let mut placement = f.copies[0].parameters.clone();
    placement.translation[2] = length(5.);
    let edit = model::prepare_instance_placement_edit(
        &f.session,
        f.session.revision(),
        actor(),
        f.copies[0].driver,
        &placement,
    )
    .unwrap();
    f.session.commit(edit.prepared).unwrap();
    let tangent = catalog(&mut backend, &f.session, &query);
    assert_eq!(tangent.results.len(), 1);
    assert!(tangent.results[0].tangent);
    assert!(
        OccurrenceCaptureRequest::new(&tangent, &tangent.results[0].selector, Behavior::Derived)
            .is_err()
    );
    let before = f.session.document().clone();
    assert!(
        prepare(&mut backend, &f.session, &capture, false)
            .err()
            .expect("Obsolete native branch must fail")
            .contains("count changed")
    );
    assert_eq!(f.session.document(), &before);
    placement.translation[2] = length(6.);
    let edit = model::prepare_instance_placement_edit(
        &f.session,
        f.session.revision(),
        actor(),
        f.copies[0].driver,
        &placement,
    )
    .unwrap();
    f.session.commit(edit.prepared).unwrap();
    let empty = catalog(&mut backend, &f.session, &query);
    assert!(empty.results.is_empty());
    assert!(!empty.diagnostics.is_empty());
    let before = f.session.document().clone();
    let revision = f.session.revision();
    assert!(
        prepare(&mut backend, &f.session, &capture, false)
            .err()
            .expect("Obsolete native branch must fail")
            .contains("count changed")
    );
    assert_eq!(f.session.document(), &before);
    assert_eq!(f.session.revision(), revision);
}

#[test]
fn actual_worker_rejects_native_preparation_with_an_unconsumed_broken_reference_and_allows_its_removal()
 {
    use multicad_core::commands::Request;
    let mut f = fixture(true);
    let mut backend = external_queries::Backend::new();
    let original = native_query(&f, 0, QueryMethod::Intersect);
    let discovered = catalog(&mut backend, &f.session, &original);
    let captured = OccurrenceCaptureRequest::new(
        &discovered,
        &discovered.results[0].selector,
        Behavior::ReferenceOnly,
    )
    .unwrap();
    let prepared = prepare(&mut backend, &f.session, &captured, false).unwrap();
    f.session.commit(prepared.edit.preview.unwrap()).unwrap();
    let mut placement = f.copies[0].parameters.clone();
    placement.translation[2] = length(6.);
    let edit = model::prepare_instance_placement_edit(
        &f.session,
        f.session.revision(),
        actor(),
        f.copies[0].driver,
        &placement,
    )
    .unwrap();
    f.session.commit(edit.prepared).unwrap();
    assert_eq!(
        sketch_external::read_occurrence(f.session.document(), captured.query.reference).unwrap(),
        captured,
        "Broken source remains canonical and explicitly repairable"
    );
    let (second, edits) = Source::create(Datum::Yz, "Independent valid consumer").unwrap();
    f.session
        .apply(Request {
            expected_revision: f.session.revision(),
            actor: actor(),
            label: "Create separate consumer".into(),
            edits,
        })
        .unwrap();
    let mut valid = original;
    valid.reference = EntityId::new();
    valid.sketch = second;
    valid.method = QueryMethod::Coplanar;
    let discovered = catalog(&mut backend, &f.session, &valid);
    assert_eq!(discovered.results.len(), 1);
    assert!(matches!(
        discovered.results[0].geometry,
        ExternalGeometry::Circle { .. }
    ));
    let candidate = OccurrenceCaptureRequest::new(
        &discovered,
        &discovered.results[0].selector,
        Behavior::ReferenceOnly,
    )
    .unwrap();
    let source = f.session.document().clone();
    let revision = f.session.revision();
    let error = prepare(&mut backend, &f.session, &candidate, false)
        .err()
        .expect(
            "A full-scope native snapshot with unresolved references cannot authorize preparation",
        );
    assert!(error.to_lowercase().contains("reference"));
    assert_eq!(f.session.document(), &source);
    assert_eq!(f.session.revision(), revision);
    backend
        .request_remove(
            f.session.preparation_snapshot(),
            Default::default(),
            actor(),
            f.sketch,
            captured.query.reference,
        )
        .unwrap();
    let ExternalQueryReply::Removed { result, .. } = poll(&mut backend) else {
        panic!("native removal reply")
    };
    let recovered = result.unwrap();
    assert_eq!(f.session.document(), &source);
    f.session
        .validate_preview(recovered.edit.preview.as_ref().unwrap())
        .unwrap();
    f.session.commit(recovered.edit.preview.unwrap()).unwrap();
    assert!(
        sketch_external::occurrence_bindings(f.session.document(), f.sketch)
            .unwrap()
            .is_empty()
    );
    let native = Compiler::default()
        .compile(f.session.document(), f.session.revision())
        .unwrap();
    assert!(native.snapshot().references.external_errors.is_empty());
    near(
        native.snapshot().shapes[&f.part.feature].volume_mm3,
        75. * std::f64::consts::PI,
    );
}

//! Canonical ownership/receipt contracts. Genuine BRep queries are model tests.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{ConstraintKind, source::Source},
    document::{Document, EntityId, Mutation},
    model::{self, OccurrencePath, PrimitiveParameters, PrimitiveRequest},
    project_file::{self, ContentHash, ProjectFile},
    references::{
        self, AnalyticGeometry, Origin, TopologyElement, TopologyKey, TopologyKind,
        TopologySnapshot,
    },
    sketch::Datum,
    sketch_external::*,
};
use std::collections::{BTreeMap, BTreeSet};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn fixture() -> (Session, EntityId, ExternalCatalog) {
    let (sketch, edits) = Source::create(Datum::Xy, "Consumer").unwrap();
    let mut s = Session::new(
        Document::new("Occurrence contracts")
            .unwrap()
            .edited(&edits)
            .unwrap(),
    )
    .unwrap();
    let p = PrimitiveRequest::new(
        "Hardware",
        PrimitiveParameters::Box {
            size: [model::length(20.), model::length(10.), model::length(4.)],
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(&s, 0, actor(), &p).unwrap();
    let output = prepared.outputs[0].clone();
    s.commit(prepared.prepared).unwrap();
    let origin = Origin {
        source: output.feature,
        role: "box.edge".into(),
    };
    let key = TopologyKey {
        kind: TopologyKind::Edge,
        ordinal: 0,
    };
    let topology = TopologySnapshot {
        owner: output.body,
        revision: s.revision(),
        elements: vec![TopologyElement {
            key,
            origins: BTreeSet::from([origin.clone()]),
            boundary: Default::default(),
            frames: vec![],
            geometry: Some(AnalyticGeometry::Line {
                a_mm: [0., 0., 0.],
                b_mm: [20., 0., 0.],
            }),
        }],
    };
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
        intent: references::GeometryIntent::capture(&topology, key).unwrap(),
        method: QueryMethod::Coplanar,
    };
    let catalog = ExternalCatalog {
        document: s.document().id(),
        revision: s.revision(),
        source_hash: ContentHash::document(s.document()).unwrap(),
        query_hash: query.hash().unwrap(),
        query,
        sketch_world_pose: Default::default(),
        results: vec![ExternalChoice {
            selector: ResultSelector {
                support: vec![origin],
                family: ExternalGeometryFamily::Line,
                branch: None,
                expected_count: 1,
            },
            geometry: ExternalGeometry::Line {
                a_mm: [0., 0.],
                b_mm: [20., 0.],
            },
            source_parameter: None,
            source_interval: Some([0., 20.]),
            tangent: false,
        }],
        coincident: true,
        diagnostics: vec![],
    };
    (s, sketch, catalog)
}
fn capture(session: &mut Session, catalog: &ExternalCatalog) -> OccurrenceCaptureRequest {
    let request =
        OccurrenceCaptureRequest::new(catalog, &catalog.results[0].selector, Behavior::Derived)
            .unwrap();
    let edits = occurrence_capture_edits(
        session.document(),
        session.revision(),
        &request,
        catalog,
        false,
    )
    .unwrap();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Capture occurrence reference".into(),
            edits,
        })
        .unwrap();
    request
}
#[test]
fn source_binding_links_identity_history_and_broken_paths_roundtrip_without_rebinding() {
    let (mut s, sketch, c) = fixture();
    let before = s.document().clone();
    let r = capture(&mut s, &c);
    assert_eq!(read_occurrence(s.document(), r.query.reference).unwrap(), r);
    assert_eq!(
        occurrence_bindings(s.document(), sketch).unwrap(),
        vec![r.clone()]
    );
    assert!(bindings(s.document(), sketch).unwrap().is_empty());
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(s.revision(), actor()).unwrap();
    let source = s.document().clone();
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(source.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert_eq!(reopened.document, source);
    let broken = source
        .edited(&[Mutation::Remove(r.query.source.component)])
        .unwrap();
    assert_eq!(read_occurrence(&broken, r.query.reference).unwrap(), r);
    assert!(validate_occurrence_query(&broken, &r.query).is_err());
    assert!(!broken.reference_diagnostics().is_empty());
}
#[test]
fn mandatory_native_receipt_is_closure_bound_and_protected_outputs_are_read_only() {
    let (mut s, sketch, c) = fixture();
    let r = capture(&mut s, &c);
    let revision = s.revision();
    let mut report = references::inspect(s.document(), revision, &BTreeMap::new());
    assert!(report.external_errors.contains_key(&r.query.reference));
    assert!(!report.all_resolved());
    let (source, frame, _) =
        Source::read_for_evaluation(s.document(), sketch, revision, Some(&report)).unwrap();
    let mut seed = source.sketch.clone();
    assert!(
        apply_to_solver(
            s.document(),
            revision,
            sketch,
            frame,
            Some(&report),
            &mut seed
        )
        .is_err()
    );
    report.external_errors.remove(&r.query.reference);
    let projection = external_pose_projection(s.document(), r.query.reference)
        .unwrap()
        .unwrap();
    report.external_results.insert(
        r.query.reference,
        ExternalResultReceipt {
            document: s.document().id(),
            revision,
            source_hash: ContentHash::document(s.document()).unwrap(),
            query_hash: r.query.hash().unwrap(),
            dependency_hash: references::dependency_hash(s.document(), r.query.reference).unwrap(),
            plane_hash: frame_hash(frame).unwrap(),
            pose_hash: projection.fingerprint,
            selector: r.selector.clone(),
            geometry: c.results[0].geometry.clone(),
        },
    );
    apply_to_solver(
        s.document(),
        revision,
        sketch,
        frame,
        Some(&report),
        &mut seed,
    )
    .unwrap();
    assert_eq!(seed.constraints.len(), 2);
    let mut next = source.clone();
    let p = next.add_point([40., 30.]);
    next.add_constraint(ConstraintKind::Fix { point: p });
    let doc = s
        .document()
        .edited(&next.replace(s.document(), sketch).unwrap())
        .unwrap();
    let mut next_seed = next.sketch.clone();
    apply_to_solver(&doc, revision, sketch, frame, Some(&report), &mut next_seed).unwrap();
    let mut bad = source;
    bad.sketch
        .points
        .iter_mut()
        .find(|p| p.id == r.output.points[0])
        .unwrap()
        .position_mm = [2., 3.];
    assert!(bad.replace(s.document(), sketch).is_err());
}
#[test]
fn unrelated_consumer_context_foreign_producer_and_implicit_dependent_removal_reject() {
    let (mut s, sketch, c) = fixture();
    let r = capture(&mut s, &c);
    let mut invalid = r.query.clone();
    invalid.source.source = sketch;
    assert!(validate_occurrence_query(s.document(), &invalid).is_err());
    invalid = r.query.clone();
    invalid.sketch_context = Some(OccurrenceContext {
        component: r.query.source.component,
        path: r.query.source.path.clone(),
    });
    assert!(validate_occurrence_query(s.document(), &invalid).is_err());
    let (mut source, _, _) =
        Source::read_for_evaluation(s.document(), sketch, s.revision(), None).unwrap();
    let p = source.add_point([30., 0.]);
    source.add_line(r.output.points[1], p);
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Dependent authored line".into(),
        edits: source.replace(s.document(), sketch).unwrap(),
    })
    .unwrap();
    assert!(remove_occurrence_edits(s.document(), sketch, r.query.reference).is_err());
}

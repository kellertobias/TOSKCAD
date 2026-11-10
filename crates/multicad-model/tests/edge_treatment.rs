use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, Mutation},
    model::{
        self, BodyInput, EdgeTreatmentParameters, EdgeTreatmentProfile, EdgeTreatmentRequest,
        PrimitiveParameters, PrimitiveRequest, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{AnalyticGeometry, GeometryIntent, Origin, TopologyKind},
};
use multicad_model::{Compiler, Model};
use std::collections::BTreeSet;
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}")
}
fn compile(session: &Session) -> Model {
    Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap()
}
fn fixture() -> (Session, BodyInput, GeometryIntent) {
    let mut document = Document::new("Native edge feature").unwrap();
    for (name, value) in [("width", "30"), ("radius", "2")] {
        document
            .insert(Entity::new(
                name,
                EntityData::Parameter {
                    expression: value.into(),
                    unit: "mm".into(),
                    exposed: true,
                    inputs: vec![],
                },
            ))
            .unwrap();
    }
    let mut session = Session::new(document).unwrap();
    let width = Dimension::Expression {
        expression: BoundExpression::bind(session.document(), "=$width", "mm", None).unwrap(),
    };
    let primitive = PrimitiveRequest::new(
        "Stock",
        PrimitiveParameters::Box {
            size: [width, length(20.), length(10.)],
            frame: Default::default(),
        },
    );
    session
        .commit(
            model::prepare_primitive(&session, session.revision(), actor(), &primitive)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let instance = model::InstanceCopyRequest::new(
        model::InstanceCopySource::Part {
            source: BodyInput {
                body: primitive.body,
                producer: primitive.feature,
            },
        },
        model::NumericTransform {
            translation: [length(80.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    session
        .commit(
            model::prepare_instance_copies(
                &session,
                session.revision(),
                actor(),
                std::slice::from_ref(&instance),
            )
            .unwrap()
            .prepared,
        )
        .unwrap();
    let native = compile(&session);
    let topology = &native.snapshot().shapes[&primitive.feature].topology;
    let edge=topology.elements.iter().find(|element|matches!(element.geometry,Some(AnalyticGeometry::Line {a_mm,b_mm}) if element.key.kind==TopologyKind::Edge && a_mm[0]==30. && b_mm[0]==30. && a_mm[1]==20. && b_mm[1]==20.)).expect("native vertical stock edge");
    let capture = GeometryIntent::capture(topology, edge.key).unwrap();
    (
        session,
        BodyInput {
            body: primitive.body,
            producer: primitive.feature,
        },
        capture,
    )
}
fn parameters(session: &Session) -> EdgeTreatmentParameters {
    EdgeTreatmentParameters {
        profile: EdgeTreatmentProfile::Fillet,
        size: Dimension::Expression {
            expression: BoundExpression::bind(session.document(), "=$radius", "mm", None).unwrap(),
        },
    }
}

#[test]
fn native_fillet_switches_to_chamfer_and_rebuilds_bound_geometry_through_undo_and_reopen() {
    let (mut session, input, capture) = fixture();
    let original = session.document().clone();
    let source = compile(&session);
    let request =
        EdgeTreatmentRequest::new(input.clone(), parameters(&session), vec![capture.clone()]);
    let preview =
        model::prepare_edge_treatment(&session, session.revision(), actor(), &request).unwrap();
    let fillet = Compiler::default()
        .compile(preview.prepared.document(), session.revision())
        .unwrap();
    near(
        fillet.solid(input.body).unwrap().volume_mm3().unwrap(),
        6000. - (4. - std::f64::consts::PI) * 10.,
    );
    assert!(fillet.snapshot().references.all_resolved());
    assert_eq!(fillet.snapshot().occurrences.len(), 1);
    for instance in fillet.snapshot().occurrences.keys() {
        near(
            fillet.solid(*instance).unwrap().volume_mm3().unwrap(),
            6000. - (4. - std::f64::consts::PI) * 10.,
        );
    }
    near(
        source.solid(input.body).unwrap().volume_mm3().unwrap(),
        6000.,
    );
    assert_eq!(session.document(), &original);
    session.commit(preview.prepared).unwrap();
    let applied = session.document().clone();
    let mut chamfer = parameters(&session);
    chamfer.profile = EdgeTreatmentProfile::Chamfer;
    let edit = model::prepare_edge_treatment_edit(
        &session,
        session.revision(),
        actor(),
        request.feature,
        &chamfer,
        std::slice::from_ref(&capture),
    )
    .unwrap();
    let native = Compiler::default()
        .compile(edit.prepared.document(), session.revision())
        .unwrap();
    near(
        native.solid(input.body).unwrap().volume_mm3().unwrap(),
        5980.,
    );
    session.commit(edit.prepared).unwrap();
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &applied);
    session.redo(session.revision(), actor()).unwrap();
    near(
        compile(&session)
            .solid(input.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        5980.,
    );
    let mut width = session
        .document()
        .entities()
        .find(|e| e.name == "width")
        .unwrap()
        .clone();
    let EntityData::Parameter { expression, .. } = &mut width.data else {
        panic!()
    };
    *expression = "40".into();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Change stock width".into(),
            edits: vec![Mutation::Replace(width)],
        })
        .unwrap();
    let changed = compile(&session);
    near(
        changed.solid(input.body).unwrap().volume_mm3().unwrap(),
        7980.,
    );
    assert!(changed.snapshot().references.all_resolved());
    for instance in changed.snapshot().occurrences.keys() {
        near(
            changed.solid(*instance).unwrap().volume_mm3().unwrap(),
            7980.,
        );
    }
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let reopened = Compiler::default()
        .compile(&opened.project.document, 0)
        .unwrap();
    near(
        reopened.solid(input.body).unwrap().volume_mm3().unwrap(),
        7980.,
    );
    assert_eq!(
        model::read_edge_treatment(&opened.project.document, request.feature)
            .unwrap()
            .1,
        vec![capture]
    );
    let step = std::env::temp_dir().join(format!("multicad-edge-{}.step", input.body));
    reopened
        .solid(input.body)
        .unwrap()
        .write_step(&step)
        .unwrap();
    #[cfg(feature = "occt")]
    let readback = multicad_kernel::Solid::read_step(&step, Default::default()).unwrap();
    #[cfg(not(feature = "occt"))]
    let readback = multicad_kernel::Solid::import_rust_step(
        &std::fs::read_to_string(&step).unwrap(),
        request.feature,
        Default::default(),
    )
    .unwrap();
    near(readback.volume_mm3().unwrap(), 7980.);
    std::fs::remove_file(step).unwrap();
}

#[test]
fn failed_native_geometry_missing_and_ambiguous_edges_preserve_the_previous_solid_and_source() {
    let (session, input, capture) = fixture();
    let original = session.document().clone();
    let valid = compile(&session);
    let mut request = EdgeTreatmentRequest::new(
        input.clone(),
        EdgeTreatmentParameters {
            profile: EdgeTreatmentProfile::Fillet,
            size: length(1e6),
        },
        vec![capture.clone()],
    );
    let invalid =
        model::prepare_edge_treatment(&session, session.revision(), actor(), &request).unwrap();
    assert!(
        Compiler::default()
            .compile(invalid.prepared.document(), session.revision())
            .is_err()
    );
    request.parameters.size = length(2.);
    request.edges[0].origins = BTreeSet::from([Origin {
        source: input.producer,
        role: "missing-selected-edge".into(),
    }]);
    let missing =
        model::prepare_edge_treatment(&session, session.revision(), actor(), &request).unwrap();
    let error = Compiler::default()
        .compile(missing.prepared.document(), session.revision())
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("needs repair"), "{error}");
    let snapshot = &valid.snapshot().shapes[&input.producer].topology;
    let shared = snapshot
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.origins.iter().any(|o| o.role == "cap.end"))
        .unwrap()
        .origins
        .clone();
    request.edges[0] = capture;
    request.edges[0].origins = shared;
    request.edges[0].boundary = None;
    let ambiguous =
        model::prepare_edge_treatment(&session, session.revision(), actor(), &request).unwrap();
    let error = Compiler::default()
        .compile(ambiguous.prepared.document(), session.revision())
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("Ambiguous"), "{error}");
    assert_eq!(session.document(), &original);
    near(
        valid.solid(input.body).unwrap().volume_mm3().unwrap(),
        6000.,
    );
}
#[test]
fn symmetric_box_chamfer_preserves_the_linked_body_and_undo() {
    let (mut session, input, edge) = fixture();
    let mut request = EdgeTreatmentRequest::new(
        input.clone(),
        EdgeTreatmentParameters {
            profile: EdgeTreatmentProfile::Chamfer,
            size: length(2.),
        },
        vec![edge.clone()],
    );
    let original = session.document().clone();
    let invalid = {
        request.parameters.size = length(20.);
        model::prepare_edge_treatment(&session, session.revision(), actor(), &request).unwrap()
    };
    assert!(
        Compiler::default()
            .compile(invalid.prepared.document(), session.revision())
            .is_err()
    );
    assert_eq!(session.document(), &original);

    request.parameters.size = length(2.);
    let prepared =
        model::prepare_edge_treatment(&session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let model = compile(&session);
    near(
        model.solid(input.body).unwrap().volume_mm3().unwrap(),
        5980.,
    );
    assert_eq!(model.snapshot().occurrences.len(), 1);
    for occurrence in model.snapshot().occurrences.keys() {
        near(
            model.solid(*occurrence).unwrap().volume_mm3().unwrap(),
            5980.,
        );
    }
    let generated = Origin {
        source: request.feature,
        role: format!("edge/{}", edge.id),
    };
    assert!(
        model.snapshot().shapes[&request.feature]
            .topology
            .elements
            .iter()
            .any(|element| {
                element.key.kind == TopologyKind::Face && element.origins.contains(&generated)
            })
    );
    session.undo(session.revision(), actor()).unwrap();
    near(
        compile(&session)
            .solid(input.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        6000.,
    );
}

#[test]
fn sided_distance_and_bound_angle_rebuild_the_same_explicit_edge() {
    let (mut session, input, edge) = fixture();
    let before = session.document().clone();
    let base = compile(&session);
    let topology = &base.snapshot().shapes[&input.producer].topology;
    let face=topology.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Plane {frame}) if e.key.kind==TopologyKind::Face && frame.normal()[0]>0.99)).unwrap();
    let face = GeometryIntent::capture(topology, face.key).unwrap();
    let mut request = EdgeTreatmentRequest::new(
        input.clone(),
        EdgeTreatmentParameters {
            profile: EdgeTreatmentProfile::Chamfer,
            size: length(2.),
        },
        vec![edge.clone(), face.clone()],
    );
    request.chamfer = Some(model::ChamferSettings::TwoDistances {
        second: length(3.),
        sides: vec![model::EdgeSide {
            edge: edge.id,
            face: face.id,
        }],
    });
    let candidate =
        model::prepare_edge_treatment(&session, session.revision(), actor(), &request).unwrap();
    let native = Compiler::default()
        .compile(candidate.prepared.document(), session.revision())
        .unwrap();
    near(
        native.solid(input.body).unwrap().volume_mm3().unwrap(),
        5970.,
    );
    assert_eq!(session.document(), &before);
    session.commit(candidate.prepared).unwrap();
    let mut parameter = Entity::new(
        "bevel",
        EntityData::Parameter {
            expression: "45".into(),
            unit: "deg".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Add bevel angle".into(),
            edits: vec![Mutation::Insert(parameter.clone())],
        })
        .unwrap();
    let mut edit = EdgeTreatmentRequest::existing(session.document(), request.feature).unwrap();
    edit.chamfer = Some(model::ChamferSettings::DistanceAngle {
        angle: Dimension::Expression {
            expression: BoundExpression::bind(session.document(), "=$bevel", "deg", None).unwrap(),
        },
        sides: vec![model::EdgeSide {
            edge: edge.id,
            face: face.id,
        }],
    });
    session
        .commit(
            model::prepare_edge_treatment_edit_request(
                &session,
                session.revision(),
                actor(),
                &edit,
            )
            .unwrap()
            .prepared,
        )
        .unwrap();
    near(
        compile(&session)
            .solid(input.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        5980.,
    );
    let EntityData::Parameter { expression, .. } = &mut parameter.data else {
        panic!()
    };
    *expression = "30".into();
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Change bevel angle".into(),
            edits: vec![Mutation::Replace(parameter)],
        })
        .unwrap();
    let native = compile(&session);
    let generated = native.snapshot().shapes[&request.feature]
        .topology
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face
                && e.origins
                    .iter()
                    .any(|o| o.source == request.feature && o.role == format!("edge/{}", edge.id))
        })
        .unwrap();
    let Some(AnalyticGeometry::Plane { frame }) = generated.geometry else {
        panic!()
    };
    near(frame.normal()[0].abs().acos(), std::f64::consts::PI / 6.);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let reopened = Compiler::default()
        .compile(&opened.project.document, 0)
        .unwrap();
    near(
        reopened.solid(input.body).unwrap().volume_mm3().unwrap(),
        native.solid(input.body).unwrap().volume_mm3().unwrap(),
    );
    assert_eq!(
        model::read_edge_treatment(&opened.project.document, request.feature)
            .unwrap()
            .1,
        vec![edge, face]
    );
}

#[test]
fn preserved_bevel_ancestry_survives_two_exact_cuts() {
    use multicad_model::GeometryBackend;
    let (mut session, input, edge) = fixture();
    let request = EdgeTreatmentRequest::new(
        input.clone(),
        EdgeTreatmentParameters {
            profile: EdgeTreatmentProfile::Chamfer,
            size: length(2.),
        },
        vec![edge.clone()],
    );
    let prepared =
        model::prepare_edge_treatment(&session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let mut target = BodyInput {
        producer: request.feature,
        ..input
    };
    for x in [5., 9.] {
        let tool = PrimitiveRequest::new(
            "Cutting box",
            PrimitiveParameters::Box {
                size: [length(2.), length(2.), length(2.)],
                frame: model::PrimitiveFrame {
                    origin: [length(x), length(x), length(8.)],
                    ..Default::default()
                },
            },
        );
        let prepared =
            model::prepare_primitive(&session, session.revision(), actor(), &tool).unwrap();
        session.commit(prepared.prepared).unwrap();
        let cut = model::BooleanRequest::new(
            model::Operation::Cut,
            target.clone(),
            vec![BodyInput {
                body: tool.body,
                producer: tool.feature,
            }],
        );
        let prepared = model::prepare_boolean(&session, session.revision(), actor(), &cut).unwrap();
        session.commit(prepared.prepared).unwrap();
        target.producer = cut.feature;
    }
    let compiled = Compiler::default()
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    let generated = Origin {
        source: request.feature,
        role: format!("edge/{}", edge.id),
    };
    let shape = &compiled.snapshot().shapes[&target.producer];
    near(shape.volume_mm3, 5964.);
    assert!(
        shape.topology.elements.iter().any(|face| {
            face.key.kind == TopologyKind::Face && face.origins.contains(&generated)
        })
    );
}

#[test]
fn rust_box_fillet_preserves_planar_references_and_analytic_step_after_edit() {
    use multicad_core::references::{self, Resolution};
    use multicad_model::GeometryBackend;

    let (mut session, input, edge) = fixture();
    let compile_rust = |session: &Session| {
        Compiler::default()
            .compile_with_backend(
                session.document(),
                session.revision(),
                GeometryBackend::Rust,
                None,
            )
            .unwrap()
    };
    let base = compile_rust(&session);
    let stock = &base.snapshot().shapes[&input.producer].topology;
    let original_faces: Vec<_> = stock
        .elements
        .iter()
        .filter(|element| element.key.kind == TopologyKind::Face)
        .map(|element| GeometryIntent::capture(stock, element.key).unwrap())
        .collect();
    let mut fillet =
        EdgeTreatmentRequest::new(input.clone(), parameters(&session), vec![edge.clone()]);
    let original = session.document().clone();
    fillet.parameters.size = length(20.);
    let oversized =
        model::prepare_edge_treatment(&session, session.revision(), actor(), &fillet).unwrap();
    assert!(
        Compiler::default()
            .compile_with_backend(
                oversized.prepared.document(),
                session.revision() + 1,
                GeometryBackend::Rust,
                None,
            )
            .is_err()
    );
    assert_eq!(session.document(), &original);
    fillet.parameters = parameters(&session);
    let valid =
        model::prepare_edge_treatment(&session, session.revision(), actor(), &fillet).unwrap();
    session.commit(valid.prepared).unwrap();

    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    session = Session::new(opened.project.document).unwrap();
    let generated = Origin {
        source: fillet.feature,
        role: format!("edge/{}", edge.id),
    };
    let temporary = tempfile::tempdir().unwrap();
    for (width, radius) in [(30., 2.), (40., 3.)] {
        if radius == 3. {
            let edits = [("width", "40"), ("radius", "3")].map(|(name, value)| {
                let mut parameter = session
                    .document()
                    .entities()
                    .find(|entity| entity.name == name)
                    .unwrap()
                    .clone();
                let EntityData::Parameter { expression, .. } = &mut parameter.data else {
                    unreachable!();
                };
                *expression = value.into();
                Mutation::Replace(parameter)
            });
            session
                .apply(Request {
                    expected_revision: session.revision(),
                    actor: actor(),
                    label: "Change stock and blend radius".into(),
                    edits: edits.into(),
                })
                .unwrap();
        }
        let model = compile_rust(&session);
        assert_eq!(model.snapshot().bodies[&input.body], fillet.feature);
        assert!(model.snapshot().references.all_resolved());
        let shape = &model.snapshot().shapes[&fillet.feature];
        let expected = width * 200. - (1. - std::f64::consts::FRAC_PI_4) * radius * radius * 10.;
        near(shape.volume_mm3, expected);
        assert_eq!(shape.solid_count, 1);
        assert_eq!(shape.bounds.min, [0.; 3]);
        assert_eq!(shape.bounds.max, [width, 20., 10.]);
        for occurrence in model.snapshot().occurrences.keys() {
            near(
                model.solid(*occurrence).unwrap().volume_mm3().unwrap(),
                expected,
            );
        }
        let current_stock = &model.snapshot().shapes[&input.producer].topology;
        for intent in &original_faces {
            let Resolution::Resolved { key, .. } =
                references::resolve(intent, &shape.topology, session.revision())
            else {
                panic!("missing stock face {intent:?}");
            };
            let source = current_stock
                .elements
                .iter()
                .find(|face| face.key.kind == TopologyKind::Face && face.origins == intent.origins)
                .unwrap();
            let treated = shape
                .topology
                .elements
                .iter()
                .find(|face| face.key == key)
                .unwrap();
            assert!(!source.frames.is_empty());
            assert!(
                source
                    .frames
                    .iter()
                    .all(|frame| treated.frames.contains(frame))
            );
        }
        let blends: Vec<_> = shape
            .topology
            .elements
            .iter()
            .filter(|face| face.key.kind == TopologyKind::Face && face.origins.contains(&generated))
            .collect();
        assert_eq!(blends.len(), 1);
        assert!(blends[0].geometry.is_none());
        assert!(blends[0].frames.is_empty());
        let solid = model.solid(fillet.feature).unwrap();
        let faces = solid.faces().unwrap();
        near(
            faces[blends[0].key.ordinal as usize].area_mm2,
            std::f64::consts::FRAC_PI_2 * radius * 10.,
        );
        let path = temporary.path().join(format!("box-fillet-{radius}.step"));
        solid.write_step(&path).unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("CYLINDRICAL_SURFACE")
        );
        let reimported = multicad_kernel::Solid::import_rust_step(
            &std::fs::read_to_string(&path).unwrap(),
            fillet.feature,
            Default::default(),
        )
        .unwrap();
        near(reimported.volume_mm3().unwrap(), expected);
        #[cfg(feature = "occt")]
        {
            let native = Compiler::default()
                .compile_with_backend(
                    session.document(),
                    session.revision(),
                    GeometryBackend::OpenCascade,
                    None,
                )
                .unwrap();
            let native_solid = native.solid(fillet.feature).unwrap();
            near(native_solid.volume_mm3().unwrap(), expected);
            let area: f64 = faces.iter().map(|face| face.area_mm2).sum();
            near(
                native_solid
                    .faces()
                    .unwrap()
                    .iter()
                    .map(|face| face.area_mm2)
                    .sum(),
                area,
            );
            let readback = multicad_kernel::Solid::read_step(&path, Default::default()).unwrap();
            near(readback.volume_mm3().unwrap(), expected);
            near(
                readback
                    .faces()
                    .unwrap()
                    .iter()
                    .map(|face| face.area_mm2)
                    .sum(),
                area,
            );
        }
    }
}

#[test]
fn rust_box_fillet_accepts_each_complete_edge_and_preserves_the_other_edges() {
    use multicad_core::references::{self, Resolution};
    use multicad_model::GeometryBackend;

    let (session, input, _) = fixture();
    let compile = |document: &Document, revision| {
        Compiler::default()
            .compile_with_backend(document, revision, GeometryBackend::Rust, None)
            .unwrap()
    };
    let base = compile(session.document(), session.revision());
    let stock = &base.snapshot().shapes[&input.producer].topology;
    let edges: Vec<_> = stock
        .elements
        .iter()
        .filter_map(|element| {
            if let Some(AnalyticGeometry::Line { a_mm, b_mm }) = element.geometry {
                Some((
                    GeometryIntent::capture(stock, element.key).unwrap(),
                    a_mm,
                    b_mm,
                ))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(edges.len(), 12);
    for (edge, a, b) in &edges {
        let length = (0..3).map(|axis| (a[axis] - b[axis]).abs()).sum::<f64>();
        let request =
            EdgeTreatmentRequest::new(input.clone(), parameters(&session), vec![edge.clone()]);
        let prepared =
            model::prepare_edge_treatment(&session, session.revision(), actor(), &request).unwrap();
        let revision = session.revision() + 1;
        let result = compile(prepared.prepared.document(), revision);
        let shape = &result.snapshot().shapes[&request.feature];
        near(
            shape.volume_mm3,
            6000. - (4. - std::f64::consts::PI) * length,
        );
        for (candidate, ..) in &edges {
            assert_eq!(
                matches!(
                    references::resolve(candidate, &shape.topology, revision),
                    Resolution::Resolved { .. }
                ),
                candidate.id != edge.id,
                "edge {edge:?}, candidate {candidate:?}",
            );
        }
    }
    let (first, a, b) = &edges[0];
    let across = (0..3).find(|&axis| a[axis] != b[axis]).unwrap();
    let (perpendicular, ..) = edges
        .iter()
        .find(|(_, a, b)| a[across] == b[across])
        .unwrap();
    let multiple = EdgeTreatmentRequest::new(
        input,
        parameters(&session),
        vec![first.clone(), perpendicular.clone()],
    );
    let prepared =
        model::prepare_edge_treatment(&session, session.revision(), actor(), &multiple).unwrap();
    let result = Compiler::default().compile_with_backend(
        prepared.prepared.document(),
        session.revision() + 1,
        GeometryBackend::Rust,
        None,
    );
    let failure = result
        .err()
        .expect("perpendicular blends require shared corner surfaces");
    assert!(
        failure
            .to_string()
            .contains("complete parallel straight edges")
    );
}

#[test]
fn rust_box_fillet_keeps_parallel_corner_origins_and_round_trips_step() {
    use multicad_core::references::{self, Resolution};
    use multicad_model::GeometryBackend;

    let (session, input, _) = fixture();
    let compile = |document: &Document, revision| {
        Compiler::default().compile_with_backend(document, revision, GeometryBackend::Rust, None)
    };
    let base = compile(session.document(), session.revision()).unwrap();
    let stock = &base.snapshot().shapes[&input.producer].topology;
    let corner = |x, y| {
        let edge = stock
            .elements
            .iter()
            .find(|element| {
                matches!(element.geometry, Some(AnalyticGeometry::Line { a_mm, b_mm })
                if a_mm[..2] == [x, y] && b_mm[..2] == [x, y])
            })
            .unwrap();
        GeometryIntent::capture(stock, edge.key).unwrap()
    };
    let corners = [
        corner(0., 0.),
        corner(0., 20.),
        corner(30., 20.),
        corner(30., 0.),
    ];
    let revision = session.revision() + 1;
    let prepare = |request: &EdgeTreatmentRequest| {
        model::prepare_edge_treatment(&session, session.revision(), actor(), request)
            .unwrap()
            .prepared
    };

    let temporary = tempfile::tempdir().unwrap();
    for (case, (selection, radius)) in [
        (&[0, 1][..], 2.),
        (&[0, 2][..], 15.),
        (&[1, 2, 3][..], 2.),
        (&[0, 1, 2, 3][..], 2.),
    ]
    .into_iter()
    .enumerate()
    {
        let mut request = EdgeTreatmentRequest::new(
            input.clone(),
            parameters(&session),
            selection.iter().map(|&i| corners[i].clone()).collect(),
        );
        request.parameters.size = length(radius);
        let prepared = prepare(&request);
        let model = compile(prepared.document(), revision).unwrap();
        let shape = &model.snapshot().shapes[&request.feature];
        let expected = 6000.
            - selection.len() as f64 * (1. - std::f64::consts::FRAC_PI_4) * radius * radius * 10.;
        near(shape.volume_mm3, expected);
        assert_eq!(shape.solid_count, 1);
        assert_eq!(shape.bounds.min, [0.; 3]);
        assert_eq!(shape.bounds.max, [30., 20., 10.]);

        for element in &stock.elements {
            let intent = GeometryIntent::capture(stock, element.key).unwrap();
            let removed = match element.geometry {
                Some(AnalyticGeometry::Point { position_mm }) => {
                    let corner_index = match (position_mm[0] == 0., position_mm[1] == 0.) {
                        (true, true) => 0,
                        (true, false) => 1,
                        (false, false) => 2,
                        (false, true) => 3,
                    };
                    selection.contains(&corner_index)
                }
                _ => request
                    .edges
                    .iter()
                    .any(|edge| edge.origins == intent.origins),
            };
            assert_eq!(
                matches!(
                    references::resolve(&intent, &shape.topology, revision),
                    Resolution::Resolved { .. }
                ),
                !removed,
                "case {case}, stock element {:?}",
                element.key,
            );
        }
        let solid = model.solid(request.feature).unwrap();
        let faces = solid.faces().unwrap();
        assert_eq!(faces.len(), 6 + selection.len());
        for edge in &request.edges {
            let origin = Origin {
                source: request.feature,
                role: format!("edge/{}", edge.id),
            };
            let [rounded] = shape
                .topology
                .elements
                .iter()
                .filter(|element| {
                    element.key.kind == TopologyKind::Face && element.origins.contains(&origin)
                })
                .collect::<Vec<_>>()[..]
            else {
                panic!("one fillet face expected for {origin:?}");
            };
            near(
                faces[rounded.key.ordinal as usize].area_mm2,
                std::f64::consts::FRAC_PI_2 * radius * 10.,
            );
        }
        solid
            .manufacturing_mesh_with_receipt(Default::default())
            .unwrap();
        let path = temporary
            .path()
            .join(format!("parallel-box-fillet-{case}.step"));
        solid.write_step(&path).unwrap();
        let step = std::fs::read_to_string(&path).unwrap();
        assert_eq!(step.matches("CYLINDRICAL_SURFACE").count(), selection.len());
        let readback =
            multicad_kernel::Solid::import_rust_step(&step, request.feature, Default::default())
                .unwrap();
        near(readback.volume_mm3().unwrap(), expected);
        assert_eq!(readback.bounds().unwrap(), shape.bounds);
        assert_eq!(readback.solid_count().unwrap(), 1);

        if selection.len() == 4 {
            let mut bytes = Vec::new();
            project_file::write(
                &ProjectFile::new(prepared.document().clone()).unwrap(),
                &mut bytes,
                Default::default(),
            )
            .unwrap();
            let opened = project_file::read(
                &mut std::io::Cursor::new(bytes),
                Default::default(),
                &project_file::supported_capabilities(),
            )
            .unwrap();
            let reopened = compile(&opened.project.document, revision).unwrap();
            assert_eq!(
                reopened.snapshot().shapes[&request.feature].topology,
                shape.topology
            );
            request.edges.reverse();
            let reordered = compile(prepare(&request).document(), revision).unwrap();
            assert_eq!(
                reordered.snapshot().shapes[&request.feature].topology,
                shape.topology
            );
        }
    }

    let mut touching =
        EdgeTreatmentRequest::new(input, parameters(&session), corners[..2].to_vec());
    touching.parameters.size = length(10.);
    let original = session.document().clone();
    let failure = compile(prepare(&touching).document(), revision)
        .err()
        .expect("a shared side cannot collapse");
    assert!(failure.to_string().contains("minimum feature size"));
    assert_eq!(session.document(), &original);
    assert_eq!(session.revision() + 1, revision);
}

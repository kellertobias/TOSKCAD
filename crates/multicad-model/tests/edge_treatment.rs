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
    let readback = multicad_kernel::Solid::read_step(&step, Default::default()).unwrap();
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
fn sided_distance_and_bound_angle_rebuild_the_same_explicit_native_edge() {
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

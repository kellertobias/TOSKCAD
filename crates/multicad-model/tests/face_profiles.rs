//! CAD-146: Extrude and Revolve directly from planar model faces. Every
//! check reads genuine native solids (volumes and bounds), never meshes.
#[allow(dead_code)]
mod common;
use common::{HEIGHT, INNER_AREA, SharedProfiles, WIDTH, actor, near, point_near};
use multicad_core::{
    constraint_sketch::Dimension,
    document::{Entity, EntityData, EntityId, Mutation},
    model::{
        self, BodyInput, Extent, ExtrudeBooleanRequest, ExtrudeParameters, ExtrudeRequest,
        FaceInput, Operation, PrimitiveParameters, PrimitiveRequest, RevolveParameters,
        RevolveRequest, Termination, angle, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{self, Origin, Resolution, TopologyKind},
};
use multicad_model::{Compiler, Model};

const FACE_AREA: f64 = WIDTH * HEIGHT - INNER_AREA;

fn compile(f: &SharedProfiles) -> Model {
    Compiler::default()
        .compile(f.session.document(), f.session.revision())
        .unwrap()
}
/// The outer body's generated top cap (z = 10): a rectangle with one hole.
fn top_face(f: &SharedProfiles, model: &Model) -> FaceInput {
    let parent = f.outer().feature;
    let snapshot = &model.snapshot().shapes[&parent].topology;
    let cap = Origin {
        source: parent,
        role: "cap.end".into(),
    };
    let element = snapshot
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&cap))
        .expect("Named generated top cap");
    model::capture_profile_face(snapshot, element.key, None).unwrap()
}
fn distance(value: f64) -> ExtrudeParameters {
    ExtrudeParameters {
        extent: Extent::OneSided {
            end: Termination::Distance {
                distance: length(value),
            },
        },
        ..Default::default()
    }
}
fn commit_extrude(f: &mut SharedProfiles, request: &ExtrudeRequest) {
    let prepared =
        model::prepare_extrude(&f.session, f.session.revision(), actor(), request).unwrap();
    f.session.commit(prepared.prepared).unwrap();
}

#[test]
fn face_profile_new_body_uses_exact_outer_boundary_and_hole_in_both_directions() {
    let mut f = SharedProfiles::new(Default::default());
    let first = compile(&f);
    let face = top_face(&f, &first);
    let request = ExtrudeRequest::from_faces(vec![face.clone()], distance(5.));
    commit_extrude(&mut f, &request);
    let feature = request.outputs[0].feature;
    let source = model::read_extrude(f.session.document(), feature).unwrap();
    assert_eq!(source.profile.face(), Some(face.intent.id));
    let model = compile(&f);
    let shape = &model.snapshot().shapes[&feature];
    near(shape.volume_mm3, FACE_AREA * 5.);
    point_near(shape.bounds.min, [0., 0., 10.]);
    point_near(shape.bounds.max, [WIDTH, HEIGHT, 15.]);
    assert_eq!(model.snapshot().bodies[&request.outputs[0].body], feature);
    // The hole is genuinely open: the new body has as many side faces as
    // the source face has boundary edges (4 + 1) plus two caps.
    let faces = model.solid(feature).unwrap().faces().unwrap();
    assert_eq!(faces.len(), 7);

    // A negative distance sweeps into the source body with the same magnitude.
    let prepared = model::prepare_extrude_edit(
        &f.session,
        f.session.revision(),
        actor(),
        feature,
        &distance(-4.),
    )
    .unwrap();
    f.session.commit(prepared.prepared).unwrap();
    let model = compile(&f);
    let shape = &model.snapshot().shapes[&feature];
    near(shape.volume_mm3, FACE_AREA * 4.);
    point_near(shape.bounds.min, [0., 0., 6.]);
    point_near(shape.bounds.max, [WIDTH, HEIGHT, 10.]);
    // Undo and redo restore the exact face-profile feature.
    f.session.undo(f.session.revision(), actor()).unwrap();
    near(
        compile(&f).snapshot().shapes[&feature].volume_mm3,
        FACE_AREA * 5.,
    );
    f.session.redo(f.session.revision(), actor()).unwrap();
    near(
        compile(&f).snapshot().shapes[&feature].volume_mm3,
        FACE_AREA * 4.,
    );
}

#[test]
fn face_profile_join_and_cut_change_the_target_body_volume() {
    for (operation, signed, expected) in [
        (Operation::Join, 5., FACE_AREA * 15.),
        (Operation::Cut, -4., FACE_AREA * 6.),
    ] {
        let mut f = SharedProfiles::new(Default::default());
        let first = compile(&f);
        let face = top_face(&f, &first);
        let target = BodyInput {
            body: f.outer().body,
            producer: f.outer().feature,
        };
        let request = ExtrudeBooleanRequest::new(
            ExtrudeRequest::from_faces(vec![face], distance(signed)),
            operation,
            vec![target.clone()],
            false,
        );
        let prepared =
            model::prepare_extrude_boolean(&f.session, f.session.revision(), actor(), &request)
                .unwrap();
        f.session.commit(prepared.prepared).unwrap();
        let model = compile(&f);
        let head = model.snapshot().bodies[&target.body];
        assert_eq!(head, request.booleans[0].feature);
        let shape = &model.snapshot().shapes[&head];
        near(shape.volume_mm3, expected);
        let top = if operation == Operation::Join {
            15.
        } else {
            6.
        };
        point_near(shape.bounds.min, [0., 0., 0.]);
        point_near(shape.bounds.max, [WIDTH, HEIGHT, top]);
    }
}

#[test]
fn face_profile_revolve_sweeps_the_exact_face_about_its_frame_axis() {
    let mut f = SharedProfiles::new(Default::default());
    let first = compile(&f);
    let face = top_face(&f, &first);
    // Axis: the face frame's U axis through its origin (the y = 0 edge).
    let parameters = RevolveParameters {
        axis_origin: [length(0.), length(0.)],
        axis_angle: angle(0.),
        angle: angle(90.),
    };
    let request = RevolveRequest::from_faces(vec![face.clone()], parameters);
    let prepared =
        model::prepare_revolve(&f.session, f.session.revision(), actor(), &request).unwrap();
    f.session.commit(prepared.prepared).unwrap();
    let feature = request.outputs[0].feature;
    let source = model::read_revolve(f.session.document(), feature).unwrap();
    assert_eq!(source.profile.face(), Some(face.intent.id));
    let model = compile(&f);
    let shape = &model.snapshot().shapes[&feature];
    // Pappus: angle × first moment of the face about the axis (y = 0).
    let moment = WIDTH * HEIGHT * HEIGHT / 2. - INNER_AREA * 20.;
    near(shape.volume_mm3, std::f64::consts::FRAC_PI_2 * moment);
    point_near(shape.bounds.min, [0., 0., 10.]);
    point_near(shape.bounds.max, [WIDTH, HEIGHT, 10. + HEIGHT]);
    // A reopened revolve edit keeps the profile face link.
    let edited = RevolveParameters {
        angle: angle(45.),
        ..request.parameters.clone()
    };
    let prepared =
        model::prepare_revolve_edit(&f.session, f.session.revision(), actor(), feature, &edited)
            .unwrap();
    f.session.commit(prepared.prepared).unwrap();
    near(
        compile(&f).snapshot().shapes[&feature].volume_mm3,
        std::f64::consts::FRAC_PI_4 * moment,
    );
}

#[test]
fn face_profile_follows_a_parameter_edit_and_survives_save_and_reopen() {
    let mut f = SharedProfiles::new(Default::default());
    let parameter = Entity::new(
        "base_height",
        EntityData::Parameter {
            expression: "10".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    f.apply(vec![Mutation::Insert(parameter.clone())]);
    let bound = ExtrudeParameters {
        extent: Extent::OneSided {
            end: Termination::Distance {
                distance: Dimension::Expression {
                    expression: BoundExpression::bind(
                        f.session.document(),
                        "=$base_height",
                        "mm",
                        None,
                    )
                    .unwrap(),
                },
            },
        },
        ..Default::default()
    };
    let prepared = model::prepare_extrude_edit(
        &f.session,
        f.session.revision(),
        actor(),
        f.outer().feature,
        &bound,
    )
    .unwrap();
    f.session.commit(prepared.prepared).unwrap();
    let first = compile(&f);
    let face = top_face(&f, &first);
    let sketch_only_document = f.session.document().clone();
    let request = ExtrudeRequest::from_faces(vec![face], distance(5.));
    commit_extrude(&mut f, &request);
    let feature = request.outputs[0].feature;

    // Save and reopen: the capability is required and older readers refuse.
    let project = ProjectFile::new(f.session.document().clone()).unwrap();
    assert!(
        project
            .capabilities()
            .contains(model::FACE_PROFILE_CAPABILITY)
    );
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert_eq!(&opened.document, f.session.document());
    let reopened = Compiler::default().compile(&opened.document, 0).unwrap();
    near(
        reopened.snapshot().shapes[&feature].volume_mm3,
        FACE_AREA * 5.,
    );
    let mut older = project_file::supported_capabilities();
    older.remove(model::FACE_PROFILE_CAPABILITY);
    assert!(project_file::read(&mut bytes.as_slice(), Default::default(), &older).is_err());
    // Sketch-only documents do not require the new capability.
    let sketch_only = ProjectFile::new(sketch_only_document).unwrap();
    assert!(
        !sketch_only
            .capabilities()
            .contains(model::FACE_PROFILE_CAPABILITY)
    );

    // Moving the face through the parameter rebuilds the face extrusion on
    // the moved face without rewriting its saved reference.
    let saved = f.session.document().get(feature).unwrap().clone();
    let mut edited = parameter.clone();
    let EntityData::Parameter { expression, .. } = &mut edited.data else {
        panic!()
    };
    *expression = "20".into();
    f.apply(vec![Mutation::Replace(edited)]);
    let moved = compile(&f);
    let shape = &moved.snapshot().shapes[&feature];
    near(shape.volume_mm3, FACE_AREA * 5.);
    point_near(shape.bounds.min, [0., 0., 20.]);
    point_near(shape.bounds.max, [WIDTH, HEIGHT, 25.]);
    assert_eq!(f.session.document().get(feature).unwrap(), &saved);
}

#[test]
fn broken_face_reference_reports_a_repair_diagnostic_instead_of_retargeting() {
    let mut f = SharedProfiles::new(Default::default());
    let first = compile(&f);
    let face = top_face(&f, &first);
    let request = ExtrudeRequest::from_faces(vec![face], distance(5.));
    commit_extrude(&mut f, &request);
    let feature = request.outputs[0].feature;
    let mut entity = f.session.document().get(feature).unwrap().clone();
    let EntityData::Feature { inputs, .. } = &mut entity.data else {
        panic!()
    };
    let link = inputs
        .iter_mut()
        .find(|l| l.role == model::PROFILE_FACE_ROLE)
        .unwrap();
    let intent = link.geometry.as_mut().unwrap();
    let anchor = intent.attachment.as_ref().unwrap().anchor.clone();
    intent.origins = std::collections::BTreeSet::from([
        anchor,
        Origin {
            source: f.outer().feature,
            role: "missing.face".into(),
        },
    ]);
    let reference = intent.id;
    f.apply(vec![Mutation::Replace(entity)]);
    let error = Compiler::default()
        .compile(f.session.document(), f.session.revision())
        .err()
        .expect("A missing profile face must not fall back to another face");
    assert_eq!(error.entity, Some(feature));
    assert!(error.message.contains("unresolved"), "{}", error.message);
    assert!(error.message.contains("repair"), "{}", error.message);
    // The shared reference report names the broken reference as Missing.
    let parents = Compiler::default()
        .compile_sources(
            f.session.document(),
            f.session.revision(),
            &[f.outer().feature],
        )
        .unwrap();
    let topologies = parents
        .snapshot()
        .shapes
        .iter()
        .map(|(id, shape)| (*id, shape.topology.clone()))
        .collect();
    let report = references::inspect(f.session.document(), f.session.revision(), &topologies);
    let diagnostic = report
        .get(references::ReferenceAddress {
            owner: feature,
            reference,
        })
        .unwrap();
    assert_eq!(diagnostic.role, model::PROFILE_FACE_ROLE);
    assert_eq!(diagnostic.resolution, Resolution::Missing);
}

#[test]
fn curved_and_ambiguous_faces_are_refused_without_a_source_change() {
    let mut f = SharedProfiles::new(Default::default());
    let request = PrimitiveRequest::new(
        "Cylinder",
        PrimitiveParameters::Cylinder {
            radius: length(10.),
            height: length(10.),
            frame: Default::default(),
        },
    );
    let prepared =
        model::prepare_primitive(&f.session, f.session.revision(), actor(), &request).unwrap();
    f.session.commit(prepared.prepared).unwrap();
    let revision = f.session.revision();
    let original = f.session.document().clone();
    let model = compile(&f);
    let snapshot = &model.snapshot().shapes[&request.feature].topology;
    let solid = model.solid(request.feature).unwrap();
    let curved = solid
        .faces()
        .unwrap()
        .into_iter()
        .find(|face| face.plane.is_none())
        .unwrap();
    let key = snapshot
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == curved.index)
        .unwrap()
        .key;
    let message = model::capture_profile_face(snapshot, key, None).unwrap_err();
    assert!(
        message.contains("curved") || message.contains("planar"),
        "{message}"
    );
    // Not a face at all.
    let edge = snapshot
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Edge)
        .unwrap()
        .key;
    assert!(model::capture_profile_face(snapshot, edge, None).is_err());
    // A face input without a planar attachment is refused by preparation.
    let mut face = top_face(&f, &model);
    face.intent.attachment = None;
    let unanchored = ExtrudeRequest::from_faces(vec![face.clone()], distance(5.));
    assert!(model::prepare_extrude(&f.session, revision, actor(), &unanchored).is_err());
    // The same face twice is refused.
    let face = top_face(&f, &model);
    let mut twice = face.clone();
    twice.intent.id = EntityId::new();
    let duplicate = ExtrudeRequest::from_faces(vec![face, twice], distance(5.));
    assert!(model::prepare_extrude(&f.session, revision, actor(), &duplicate).is_err());
    assert_eq!(f.session.revision(), revision);
    assert_eq!(f.session.document(), &original);
}

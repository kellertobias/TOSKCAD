use super::*;
use crate::modeling::native_fixture::screw_connection as fixture;
use crate::modeling::{native_fixture, screw_connection::ScrewConnectionPanel};
use std::time::{Duration, Instant};

fn controller() -> ModelToolController {
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(native_fixture::NativeBackend::new()));
    controller
}
fn begin(
    controller: &mut ModelToolController,
    session: &Session,
    request: model::ScrewConnectionRequest,
) {
    controller
        .begin_form(
            session,
            &Default::default(),
            Channel::Ui,
            ModelForm::ScrewConnection(Box::new(
                ScrewConnectionPanel::from_request(session.document(), request, false).unwrap(),
            )),
        )
        .unwrap();
}
fn wait(controller: &mut ModelToolController, session: &Session) {
    let deadline = Instant::now() + Duration::from_secs(180);
    while controller.pending() {
        controller.poll(session, &Default::default());
        assert!(
            Instant::now() < deadline,
            "native connection timed out: {:?}",
            controller.error()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn native_connection_preview_cancel_apply_history_and_reopen_keep_every_owned_identity() {
    successful_lifecycle(fixture::fixture());
}

#[test]
fn native_tapped_connection_preview_cancel_apply_history_and_reopen_keep_every_owned_identity() {
    successful_lifecycle(fixture::tapped_fixture());
}

fn successful_lifecycle(
    (document, request, _): (Document, model::ScrewConnectionRequest, EntityId),
) {
    let mut session = Session::new(document.clone()).unwrap();
    let connection = request.connection;
    let instances = request
        .hardware
        .iter()
        .map(|role| role.instance)
        .collect::<Vec<_>>();
    let mut controller = controller();
    begin(&mut controller, &session, request.clone());
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    wait(&mut controller, &session);
    assert!(
        controller.can_apply(&session, &Default::default()),
        "{:?}",
        controller.error()
    );
    let candidate = controller.candidate.as_ref().unwrap();
    let scene = candidate.scene.as_ref().unwrap();
    assert_eq!(
        scene
            .model_evaluation()
            .unwrap()
            .screw_connection_checks
            .len(),
        1
    );
    let mut missing = (**scene.model_evaluation().unwrap()).clone();
    missing.screw_connection_checks.clear();
    let stripped = Scene::new(
        scene.stamp(),
        scene.geometries().values().cloned().collect(),
        scene.instances().to_vec(),
    )
    .unwrap()
    .with_model_evaluation(Arc::new(missing))
    .unwrap();
    let refusal = candidate
        .proof
        .validate(candidate.prepared.document(), &stripped, &candidate.outputs)
        .expect_err("geometry and poses alone cannot authorize Apply");
    assert!(
        refusal.contains("scoped geometry alone cannot authorize Apply"),
        "Missing-receipt control must fail at the full connection gate: {refusal}"
    );
    controller.cancel();
    assert_eq!(session.document(), &document);
    assert_eq!(session.revision(), 0);
    begin(&mut controller, &session, request);
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    wait(&mut controller, &session);
    assert!(
        controller.can_apply(&session, &Default::default()),
        "{:?}",
        controller.error()
    );
    controller.apply(&mut session, &Default::default()).unwrap();
    assert_eq!(session.revision(), 1);
    let applied = session.document().clone();
    for instance in &instances {
        assert_eq!(
            ModelToolController::screw_connection_for(&applied, *instance),
            Some(connection)
        );
    }
    session.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &document);
    session.redo(2, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &applied);
    let file = multicad_core::project_file::ProjectFile::new(applied.clone()).unwrap();
    let mut bytes = vec![];
    multicad_core::project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let reopened = multicad_core::project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &multicad_core::project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(reopened.project.document, applied);
    let mut reopened = Session::new(reopened.project.document).unwrap();
    controller
        .begin_existing(&reopened, &Default::default(), instances[0], Channel::Ui)
        .unwrap();
    controller.cancel();
    assert_eq!(reopened.revision(), 0);
    assert_eq!(reopened.document(), &applied);
    // Keep the mutable Session in the same operator lifecycle as the live tool.
    assert!(reopened.undo(0, Actor::local(Channel::Ui)).is_err());
}

#[test]
fn native_unmet_connection_engagement_never_enables_apply_or_publishes_partial_parts() {
    for (document, mut request, _) in [fixture::fixture(), fixture::tapped_fixture()] {
        request.parameters.minimum_engagement = model::length(4.);
        let session = Session::new(document.clone()).unwrap();
        let mut controller = controller();
        begin(&mut controller, &session, request);
        controller
            .preview(&session, &Default::default(), Channel::Ui)
            .unwrap();
        wait(&mut controller, &session);
        assert!(!controller.can_apply(&session, &Default::default()));
        assert!(controller.error().is_some());
        assert_eq!(session.document(), &document);
        assert_eq!(session.revision(), 0);
    }
}

#[test]
fn native_captured_grip_and_receiver_entry_follow_a_parameter_change_without_baking_sources() {
    use multicad_core::{
        document::{Entity, Mutation},
        parameters::BoundExpression,
        references::AnalyticGeometry,
    };
    let (document, mut request, stock) = fixture::fixture();
    let height = Entity::new(
        "receiving_height",
        EntityData::Parameter {
            expression: "4.1".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let document = document
        .edited(&[Mutation::Insert(height.clone())])
        .unwrap();
    let mut setup = Session::new(document).unwrap();
    let mut primitive = model::read_primitive(setup.document(), stock)
        .unwrap()
        .parameters;
    let model::PrimitiveParameters::Box { size, .. } = &mut primitive else {
        panic!("native rectangular receiving stock")
    };
    size[2] = multicad_core::constraint_sketch::Dimension::Expression {
        expression: BoundExpression::bind(setup.document(), "=$receiving_height", "mm", None)
            .unwrap(),
    };
    setup
        .commit(
            model::prepare_primitive_edit(&setup, 0, Actor::local(Channel::Ui), stock, &primitive)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let original = multicad_model::Compiler::default()
        .compile(setup.document(), setup.revision())
        .unwrap();
    let topology = &original.snapshot().shapes[&stock].topology;
    let bottom = topology.elements.iter().find(|element| matches!(element.geometry, Some(AnalyticGeometry::Plane { frame }) if frame.normal()[2] < -0.99) && !element.frames.is_empty()).unwrap();
    let end = model::capture_profile_face(topology, bottom.key, None).unwrap();
    request.parameters.end_reference = Some(model::ScrewConnectionEndReference {
        receiver: request.axis.source.receiver.clone(),
        occurrence: request.axis.source.occurrence.clone(),
        face: end,
    });
    request.parameters.grip = model::length(0.);
    request.receivers[0].source.entry_reference = Some(request.axis.face.clone());
    request.receivers[0].source.station = model::length(0.);
    let connection = request.connection;
    let screw = request
        .hardware
        .iter()
        .find(|role| role.role == model::ScrewConnectionHardwareRole::Screw)
        .unwrap()
        .instance;
    let nut = request
        .hardware
        .iter()
        .find(|role| role.role == model::ScrewConnectionHardwareRole::Nut)
        .unwrap()
        .instance;
    let mut session = Session::new(setup.document().clone()).unwrap();
    let mut controller = controller();
    begin(&mut controller, &session, request);
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    wait(&mut controller, &session);
    assert!(
        controller.can_apply(&session, &Default::default()),
        "{:?}",
        controller.error()
    );
    let first = controller
        .candidate
        .as_ref()
        .unwrap()
        .scene
        .as_ref()
        .unwrap()
        .clone();
    let pose = |scene: &Scene, instance| {
        scene
            .assembly_evaluation()
            .unwrap()
            .leaves
            .iter()
            .find(|leaf| leaf.path.root == instance)
            .unwrap()
            .pose
    };
    let first_screw = pose(&first, screw);
    let first_nut = pose(&first, nut);
    assert!(
        (first.model_evaluation().unwrap().screw_connection_checks[0].grip_mm - 4.1).abs() < 1e-7
    );
    controller.apply(&mut session, &Default::default()).unwrap();
    let retained = model::read_screw_connection_request(session.document(), connection).unwrap();
    let mut changed = session.document().get(height.id).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut changed.data else {
        unreachable!()
    };
    *expression = "4.35".into();
    session
        .apply(multicad_core::commands::Request {
            expected_revision: 1,
            actor: Actor::local(Channel::Ui),
            label: "Change receiving height by half a pitch".into(),
            edits: vec![Mutation::Replace(changed)],
        })
        .unwrap();
    assert_eq!(
        model::read_screw_connection_request(session.document(), connection).unwrap(),
        retained,
        "Parameter updates must not rewrite authored hardware or capture sources"
    );
    let before = session.document().clone();
    controller
        .begin_existing(&session, &Default::default(), connection, Channel::Ui)
        .unwrap();
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    wait(&mut controller, &session);
    assert!(
        controller.can_apply(&session, &Default::default()),
        "{:?}",
        controller.error()
    );
    let second = controller
        .candidate
        .as_ref()
        .unwrap()
        .scene
        .as_ref()
        .unwrap();
    let check = &second.model_evaluation().unwrap().screw_connection_checks[0];
    assert!((check.grip_mm - 4.35).abs() < 1e-7 && check.native_component_collision_checked);
    let second_screw = pose(second, screw);
    let second_nut = pose(second, nut);
    assert!((second_screw.translation_mm[2] - first_screw.translation_mm[2] - 0.25).abs() < 1e-7);
    assert!(
        second_nut
            .translation_mm
            .iter()
            .zip(first_nut.translation_mm)
            .all(|(a, b)| (a - b).abs() < 1e-7),
        "Far-face hardware must remain on the fixed bottom plane"
    );
    let alignment = first_nut
        .vector([1., 0., 0.])
        .iter()
        .zip(second_nut.vector([1., 0., 0.]))
        .map(|(a, b)| a * b)
        .sum::<f64>();
    assert!(
        alignment < -0.999999,
        "Half-pitch source change requires the actual matching half-turn, observed dot {alignment}"
    );
    controller.cancel();
    assert_eq!(session.document(), &before);
    let file = multicad_core::project_file::ProjectFile::new(before.clone()).unwrap();
    let mut bytes = vec![];
    multicad_core::project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let reopened = multicad_core::project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &multicad_core::project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(reopened.project.document, before);
}

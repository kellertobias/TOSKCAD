use super::*;
use crate::modeling::{PlacementMode, native_fixture};
use multicad_core::{
    document::Document,
    model::{self, BodyInput, NumericTransform, PrimitiveParameters, PrimitiveRequest},
};
use multicad_model::Compiler;
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn fixture() -> (Session, PrimitiveRequest, PrimitiveRequest, EntityId) {
    let mut session = Session::new(Document::new("Native placement UI").unwrap()).unwrap();
    let a = PrimitiveRequest::new(
        "A",
        PrimitiveParameters::Box {
            size: [length(2.), length(3.), length(4.)],
            frame: Default::default(),
        },
    );
    let b = PrimitiveRequest::new("B", a.parameters.clone());
    for request in [&a, &b] {
        let prepared =
            model::prepare_primitive(&session, session.revision(), actor(), request).unwrap();
        session.commit(prepared.prepared).unwrap();
    }
    let copy = model::InstanceCopyRequest::new(
        model::InstanceCopySource::Part {
            source: BodyInput {
                body: b.body,
                producer: b.feature,
            },
        },
        Default::default(),
    );
    let prepared = model::prepare_instance_copies(
        &session,
        session.revision(),
        actor(),
        std::slice::from_ref(&copy),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    (session, a, b, copy.instance)
}
fn canonical(session: &Session) -> Arc<Scene> {
    let model = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    native_fixture::scene(session.document(), model.snapshot(), 0).unwrap()
}
fn wait(controller: &mut ModelToolController, session: &Session) {
    let until = Instant::now() + Duration::from_secs(3);
    while controller.pending() && Instant::now() < until {
        controller.poll(session, &Default::default());
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!controller.pending(), "native worker did not reply");
}
#[test]
fn current_native_mixed_move_preview_has_exact_output_and_instance_proof_one_apply_and_undo() {
    let (mut session, a, b, instance) = fixture();
    let original = session.document().clone();
    let scene = canonical(&session);
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(native_fixture::NativeBackend::new()));
    controller
        .begin_placement(
            &session,
            &Default::default(),
            &scene,
            &BTreeSet::from([a.body, instance]),
            PlacementMode::Move,
            Channel::Ui,
        )
        .unwrap();
    let ModelRequest::Placement(mut command) = controller
        .form
        .as_ref()
        .unwrap()
        .request(session.document(), controller.capabilities())
        .unwrap()
    else {
        panic!()
    };
    *command = command.with_parameters(NumericTransform {
        translation: [length(10.), length(20.), length(0.)],
        angle: model::angle(90.),
        ..Default::default()
    });
    // The form's immutable identities survive repeated source-neutral requests.
    let stamp = controller.stamp().unwrap();
    controller
        .submit_final(
            &session,
            &Default::default(),
            Channel::CommandLine,
            stamp,
            &ModelRequest::Placement(command.clone()),
        )
        .unwrap();
    assert_eq!(session.document(), &original);
    assert!(!controller.can_apply(&session, &Default::default()));
    wait(&mut controller, &session);
    assert!(
        controller.can_apply(&session, &Default::default()),
        "{:?}",
        controller.error()
    );
    let candidate = controller.candidate.as_ref().unwrap();
    assert_eq!(candidate.instances, vec![instance]);
    let output = candidate.outputs[0].feature;
    let preview = candidate.scene.as_ref().unwrap();
    let actual = preview
        .reference_instances()
        .iter()
        .find(|i| i.id == instance)
        .unwrap();
    assert_eq!(actual.transform.point([0.; 3]), [10., 20., 0.]);
    assert_eq!(actual.geometry, b.feature);
    let outputs = controller.apply(&mut session, &Default::default()).unwrap();
    assert_eq!(outputs[0].feature, output);
    assert_eq!(
        session.activity().last().unwrap().actor.channel,
        Channel::CommandLine
    );
    let native = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    assert_eq!(native.snapshot().shapes[&output].bounds.min, [7., 20., 0.]);
    assert_eq!(
        native.snapshot().occurrences[&instance].bounds.min,
        [7., 20., 0.]
    );
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &original);
}
#[test]
fn instance_only_copy_and_saved_driver_edit_have_real_native_proof_and_do_not_rewrite_part() {
    let (mut session, _a, b, instance) = fixture();
    let original = session.document().clone();
    let scene = canonical(&session);
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(native_fixture::NativeBackend::new()));
    controller
        .begin_placement(
            &session,
            &Default::default(),
            &scene,
            &BTreeSet::from([instance]),
            PlacementMode::Copy,
            Channel::Ui,
        )
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
    let copied = controller.candidate_instances()[0];
    assert_ne!(copied, instance);
    assert!(
        controller
            .apply(&mut session, &Default::default())
            .unwrap()
            .is_empty()
    );
    assert_eq!(session.document().get(b.body), original.get(b.body));
    let Some(EntityData::Instance {
        placement_source: Some(driver),
        ..
    }) = session.document().get(copied).map(|e| &e.data)
    else {
        panic!()
    };
    let driver = *driver;
    controller
        .begin_existing(&session, &Default::default(), driver, Channel::Ui)
        .unwrap();
    assert!(controller.editing());
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    wait(&mut controller, &session);
    assert!(
        controller.can_apply(&session, &Default::default()),
        "{:?}",
        controller.error()
    );
    controller.cancel();
    assert_eq!(
        model::read_instance_placement(session.document(), driver)
            .unwrap()
            .owner,
        copied
    );
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &original);
}
#[test]
fn occurrence_proof_rejects_forged_pose_missing_instance_and_stale_source() {
    let (session, _a, _b, instance) = fixture();
    let scene = canonical(&session);
    assert!(proof::placements(session.document(), session.revision(), &scene).is_ok());
    let mut instances = scene.reference_instances().to_vec();
    instances
        .iter_mut()
        .find(|i| i.id == instance)
        .unwrap()
        .transform = crate::viewport::Transform::from_rigid_pose(model::RigidPose {
        translation_mm: [1., 0., 0.],
        ..Default::default()
    })
    .unwrap();
    let wrong = Scene::new(
        scene.stamp(),
        scene.geometries().values().cloned().collect(),
        instances,
    )
    .unwrap()
    .with_reference_aliases(
        Compiler::default()
            .compile(session.document(), session.revision())
            .unwrap()
            .snapshot()
            .body_sources
            .clone(),
    )
    .unwrap();
    assert!(
        proof::placements(session.document(), session.revision(), &wrong)
            .unwrap_err()
            .contains("incorrect Instance pose")
    );
    let instances = scene
        .reference_instances()
        .iter()
        .filter(|i| i.id != instance)
        .cloned()
        .collect();
    let missing = Scene::new(
        scene.stamp(),
        scene.geometries().values().cloned().collect(),
        instances,
    )
    .unwrap();
    assert!(proof::placements(session.document(), session.revision(), &missing).is_err());
    assert!(!ModelToolController::placement_available(
        session.document(),
        session.revision() + 1,
        &scene,
        &BTreeSet::from([instance])
    ));
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(native_fixture::NativeBackend::new()));
    controller
        .begin_placement(
            &session,
            &Default::default(),
            &scene,
            &BTreeSet::from([instance]),
            PlacementMode::Move,
            Channel::Ui,
        )
        .unwrap();
    controller.cancel();
    assert!(!controller.active());
}
#[test]
fn referenced_alignment_native_preview_rebuilds_saved_driver_and_missing_target_never_enables_apply()
 {
    use multicad_core::references::{Attachment, GeometryIntent, Origin, TopologyKind};
    let (mut session, a, b, instance) = fixture();
    let original = session.document().clone();
    let scene = canonical(&session);
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(native_fixture::NativeBackend::new()));
    controller
        .begin_alignment(
            &session,
            &Default::default(),
            &scene,
            &BTreeSet::from([instance]),
            Channel::Ui,
        )
        .unwrap();
    let input = |producer: EntityId, source: EntityId| {
        let snapshot = scene.geometry_snapshot(producer).unwrap();
        let face = snapshot
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Face && e.frames.len() == 1)
            .unwrap();
        let mut intent = GeometryIntent::capture(&snapshot, face.key).unwrap();
        intent.attachment = Some(Attachment {
            anchor: face.frames[0].origin.clone(),
            offset_mm: [0.; 3],
            angle_rad: 0.,
        });
        multicad_core::model::AlignmentInput { source, intent }
    };
    let source = input(b.feature, b.body);
    let target = input(a.feature, a.body);
    let parameters = model::AlignmentParameters {
        source: model::AlignmentAnchor::Geometry {
            reference: source.intent.id,
        },
        target: model::AlignmentAnchor::Geometry {
            reference: target.intent.id,
        },
        offset: [length(25.), length(0.), length(0.)],
        ..Default::default()
    };
    let request = model::InstanceAlignmentRequest::new(instance, parameters, vec![source, target]);
    controller
        .submit_final(
            &session,
            &Default::default(),
            Channel::Ui,
            controller.stamp().unwrap(),
            &ModelRequest::Alignment(Box::new(crate::modeling::AlignmentCommand::Instance(
                request.clone(),
            ))),
        )
        .unwrap();
    wait(&mut controller, &session);
    assert!(
        controller.can_apply(&session, &Default::default()),
        "{:?}",
        controller.error()
    );
    assert_eq!(session.document(), &original);
    controller.apply(&mut session, &Default::default()).unwrap();
    controller
        .begin_existing(&session, &Default::default(), request.driver, Channel::Ui)
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
    controller.cancel();
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &original);
    let scene = canonical(&session);
    controller
        .begin_alignment(
            &session,
            &Default::default(),
            &scene,
            &BTreeSet::from([instance]),
            Channel::Ui,
        )
        .unwrap();
    let mut missing = request;
    missing.driver = EntityId::new();
    let target = &mut missing.references[1].intent;
    let absent = Origin {
        source: EntityId::new(),
        role: "missing geometry".into(),
    };
    target.origins = BTreeSet::from([absent.clone()]);
    target.boundary = None;
    target.attachment = Some(Attachment {
        anchor: absent,
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    controller
        .submit_final(
            &session,
            &Default::default(),
            Channel::Ui,
            controller.stamp().unwrap(),
            &ModelRequest::Alignment(Box::new(crate::modeling::AlignmentCommand::Instance(
                missing,
            ))),
        )
        .unwrap();
    wait(&mut controller, &session);
    assert!(!controller.can_apply(&session, &Default::default()));
    assert!(controller.error().is_some());
    assert_eq!(session.document(), &original);
}
#[test]
fn placement_edit_cannot_substitute_another_driver_or_use_creation_scope() {
    let (session, _a, _b, instance) = fixture();
    let Some(EntityData::Instance {
        placement_source: Some(driver),
        ..
    }) = session.document().get(instance).map(|e| &e.data)
    else {
        panic!()
    };
    let original = session.document().clone();
    let request =
        ModelRequest::Placement(Box::new(crate::modeling::PlacementCommand::EditInstance {
            driver: *driver,
            instance,
            parameters: Default::default(),
        }));
    assert!(
        request
            .prepare_edit(&session, actor(), EntityId::new())
            .err()
            .unwrap()
            .contains("producer identity")
    );
    assert!(
        request
            .prepare(&session, actor())
            .err()
            .unwrap()
            .contains("Reopen")
    );
    assert_eq!(session.document(), &original);
}

//! Candidate publication and transactional lifecycle regressions. The scripted
//! provider controls receipt order; native compilation correctness is verified
//! separately by the production model crate.
use super::*;
use crate::viewport::{Geometry, Instance, SceneReply, Transform};
use multicad_core::{
    constraint_sketch::source::Source,
    document::{Document, Mutation},
    model::BodyInput,
    sketch::Datum,
    sketch_regions,
};
use multicad_kernel::{GeometryPolicy, MeshOptions, Solid};
use std::{collections::VecDeque, sync::Mutex};

#[derive(Default)]
struct State {
    requests: Vec<(Document, ProjectResources, SceneStamp)>,
    replies: VecDeque<SceneReply>,
    reject: bool,
    cancellations: usize,
    evaluations: Vec<multicad_core::model::EvaluationOptions>,
    fast_hardware_preview: bool,
}
struct ScriptedBackend(Arc<Mutex<State>>);
impl SceneBackend for ScriptedBackend {
    fn supports_fast_hardware_preview(&self) -> bool {
        self.0.lock().unwrap().fast_hardware_preview
    }
    fn request_evaluation(
        &mut self,
        document: &Document,
        resources: &ProjectResources,
        stamp: SceneStamp,
        options: &multicad_core::model::EvaluationOptions,
    ) -> Result<(), String> {
        options.validate().map_err(|error| error.to_string())?;
        self.0.lock().unwrap().evaluations.push(options.clone());
        self.request_project(document, resources, stamp)
    }
    fn request(&mut self, document: &Document, stamp: SceneStamp) -> Result<(), String> {
        self.request_project(document, &Default::default(), stamp)
    }
    fn request_project(
        &mut self,
        document: &Document,
        resources: &ProjectResources,
        stamp: SceneStamp,
    ) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        state
            .requests
            .push((document.clone(), resources.clone(), stamp));
        if state.reject {
            Err("Native request queue unavailable".into())
        } else {
            Ok(())
        }
    }
    fn cancel(&mut self) {
        self.0.lock().unwrap().cancellations += 1;
    }
    fn poll(&mut self) -> Option<SceneReply> {
        self.0.lock().unwrap().replies.pop_front()
    }
}
fn fixture() -> (Session, EntityId, Vec<RegionIntent>) {
    let (sketch, edits) = Source::create(Datum::Xy, "Shared exact profiles").unwrap();
    let document = Document::new("Candidate preview lifecycle")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
    source.add_rectangle([0., 0.], [30., 20.]).unwrap();
    source.add_rectangle([40., 0.], [60., 20.]).unwrap();
    let profiles = sketch_regions::extract(&source)
        .regions
        .iter()
        .map(|region| region.intent())
        .collect();
    let document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    (Session::new(document).unwrap(), sketch, profiles)
}
fn controller(
    session: &Session,
    sketch: EntityId,
    profiles: Vec<RegionIntent>,
) -> (ModelToolController, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State::default()));
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(ScriptedBackend(state.clone())));
    controller
        .begin(session, &Default::default(), sketch, profiles, Channel::Ui)
        .unwrap();
    (controller, state)
}
fn native_scene(document: &Document, stamp: SceneStamp) -> Arc<Scene> {
    let mut geometries = vec![];
    let mut instances = vec![];
    for entity in document.entities() {
        if let EntityData::Part { features, .. } = &entity.data {
            let feature = *features.last().unwrap();
            let solid =
                Solid::box_named([30., 20., 10.], feature, GeometryPolicy::default()).unwrap();
            let mesh = solid.mesh(MeshOptions::default()).unwrap();
            geometries.push(Arc::new(
                Geometry::new(
                    feature,
                    mesh.vertices.as_chunks::<3>().0.to_vec(),
                    mesh.indices,
                    mesh.face_indices,
                    solid
                        .topology_snapshot(feature, stamp.revision)
                        .unwrap()
                        .elements,
                )
                .unwrap(),
            ));
            instances.push(Instance {
                id: entity.id,
                geometry: feature,
                transform: Transform::default(),
                color: [180, 190, 205, 255],
            });
        }
    }
    Arc::new(Scene::new(stamp, geometries, instances).unwrap())
}
fn reply(state: &Arc<Mutex<State>>, index: usize) -> Arc<Scene> {
    let (document, _, stamp) = state.lock().unwrap().requests[index].clone();
    let scene = native_scene(&document, stamp);
    state.lock().unwrap().replies.push_back(SceneReply {
        stamp,
        result: Ok(scene.clone()),
    });
    scene
}
#[test]
fn automatic_extrude_debounces_edits_and_accepts_only_the_confirmed_stamp() {
    let (session, sketch, profiles) = fixture();
    let before = session.document().clone();
    let resources = ProjectResources::default();
    let (mut controller, state) = controller(&session, sketch, profiles);
    let ctx = egui::Context::default();
    let tick = |controller: &mut ModelToolController, time| {
        ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                controller.tick_auto_preview(ui, &session, &resources, false);
            },
        )
        .drop_without_applying_deltas();
    };
    let distance = |controller: &mut ModelToolController, value| {
        let Some(ModelForm::Extrude(panel)) = &mut controller.form else {
            panic!("Expected an extrusion draft");
        };
        panel.set_gizmo_distance(value);
    };
    tick(&mut controller, 0.);
    distance(&mut controller, 24.);
    tick(&mut controller, 0.03);
    tick(&mut controller, 0.08);
    assert!(state.lock().unwrap().requests.is_empty());
    assert!(controller.arm_confirmation(&session, &resources));
    tick(&mut controller, 0.11);
    assert_eq!(state.lock().unwrap().requests.len(), 1);
    assert!(!controller.confirmation_ready(&session, &resources));

    distance(&mut controller, 32.);
    tick(&mut controller, 0.13);
    tick(&mut controller, 0.21);
    assert_eq!(state.lock().unwrap().requests.len(), 2);
    reply(&state, 0);
    controller.poll(&session, &resources);
    assert!(!controller.can_apply(&session, &resources));
    assert!(controller.arm_confirmation(&session, &resources));
    reply(&state, 1);
    controller.poll(&session, &resources);
    assert!(controller.confirmation_ready(&session, &resources));

    distance(&mut controller, 0.);
    tick(&mut controller, 0.23);
    assert!(!controller.can_apply(&session, &resources));
    tick(&mut controller, 0.31);
    assert_eq!(state.lock().unwrap().requests.len(), 2);
    assert!(controller.error().is_some());
    assert!(!controller.confirmation_ready(&session, &resources));
    controller.cancel();
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), 0);
}

#[test]
fn hardware_opens_immediately_and_only_the_latest_edit_can_confirm() {
    use multicad_core::model::ModelEvaluationInfo;

    for (fast_hardware_preview, delay) in [(false, 0.30), (true, 0.075)] {
        let mut session = Session::new(Document::new("Hardware preview").unwrap()).unwrap();
        let before = session.document().clone();
        let resources = ProjectResources::default();
        let state = Arc::new(Mutex::new(State {
            fast_hardware_preview,
            ..Default::default()
        }));
        let mut controller = ModelToolController::default();
        controller.set_backend(Box::new(ScriptedBackend(state.clone())));
        controller
            .begin_mechanical_part(&session, &resources, Channel::Ui)
            .unwrap();
        let ctx = egui::Context::default();
        let tick = |controller: &mut ModelToolController, session: &Session, time| {
            ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    controller.tick_auto_preview(ui, session, &resources, false);
                },
            )
            .drop_without_applying_deltas();
        };
        let first_cancellations = state.lock().unwrap().cancellations;
        assert!(controller.pending());
        assert_eq!(state.lock().unwrap().requests.len(), 1);
        tick(&mut controller, &session, 0.);
        assert_eq!(state.lock().unwrap().cancellations, first_cancellations);
        assert_eq!(state.lock().unwrap().requests.len(), 1);

        let edit = |controller: &mut ModelToolController, session: &Session, diameter| {
            let ModelRequest::MechanicalPart(mut request) = controller
                .form
                .as_ref()
                .unwrap()
                .request(session.document(), controller.capabilities())
                .unwrap()
            else {
                panic!("Expected a hardware draft");
            };
            let model::MechanicalPartParameters::Washer {
                outside_diameter, ..
            } = &mut request.parameters
            else {
                panic!("Expected the default washer");
            };
            *outside_diameter = length(diameter);
            let panel =
                MechanicalPartPanel::new(*request, session.parameters().clone(), false).unwrap();
            controller.form = Some(ModelForm::MechanicalPart(Box::new(panel)));
        };
        edit(&mut controller, &session, 8.);
        tick(&mut controller, &session, 0.02);
        assert!(controller.arm_confirmation(&session, &resources));
        edit(&mut controller, &session, 9.);
        tick(&mut controller, &session, 0.04);
        assert!(controller.keyboard_confirm.is_none());
        tick(&mut controller, &session, 0.04 + delay - 0.002);
        assert_eq!(state.lock().unwrap().requests.len(), 1);
        assert!(controller.arm_confirmation(&session, &resources));
        tick(&mut controller, &session, 0.04 + delay + 0.002);
        assert_eq!(state.lock().unwrap().requests.len(), 2);
        assert_eq!(session.document(), &before);

        let (document, _, stamp) = state.lock().unwrap().requests[1].clone();
        reply(&state, 0);
        controller.poll(&session, &resources);
        assert!(!controller.confirmation_ready(&session, &resources));
        let scene = native_scene(&document, stamp)
            .as_ref()
            .clone()
            .with_model_evaluation(Arc::new(ModelEvaluationInfo {
                source_hash: ContentHash::document(&document).unwrap(),
                screw_connection_checks: vec![],
                consumed_bodies: Default::default(),
                cut_targets: vec![],
            }))
            .unwrap();
        state.lock().unwrap().replies.push_back(SceneReply {
            stamp,
            result: Ok(Arc::new(scene)),
        });
        controller.poll(&session, &resources);
        assert!(
            controller.confirmation_ready(&session, &resources),
            "{:?}",
            controller.error()
        );
        let outputs = controller.apply(&mut session, &resources).unwrap();
        let source = model::read_mechanical_part(session.document(), outputs[0].feature).unwrap();
        let model::MechanicalPartParameters::Washer {
            outside_diameter, ..
        } = source.parameters
        else {
            panic!("Expected the final washer");
        };
        assert_eq!(outside_diameter, length(9.));
        assert_eq!(session.revision(), 1);
    }
}
#[test]
fn abandoning_pending_native_preview_notifies_its_owner_and_rejects_late_replies() {
    let (session, sketch, profiles) = fixture();
    let before = session.document().clone();
    let (mut controller, state) = controller(&session, sketch, profiles);
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    assert!(controller.pending());
    let cancellations = state.lock().unwrap().cancellations;
    // Changing a form invalidates its pending native request without committing.
    controller.invalidate();
    assert_eq!(state.lock().unwrap().cancellations, cancellations + 1);
    assert!(!controller.pending());
    assert!(controller.active());
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    assert!(controller.pending());
    let cancellations = state.lock().unwrap().cancellations;
    controller.cancel();
    assert_eq!(state.lock().unwrap().cancellations, cancellations + 1);
    assert!(!controller.active());
    // A misbehaving provider can still deliver old replies; neither is accepted.
    reply(&state, 0);
    reply(&state, 1);
    controller.poll(&session, &Default::default());
    assert!(!controller.pending());
    assert!(!controller.can_apply(&session, &Default::default()));
    assert!(
        controller
            .display_scene(&session, &Default::default())
            .is_none()
    );
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), 0);
    assert!(!session.can_undo());
}
#[test]
fn begin_preview_and_cancel_never_mutate_source_or_canonical_display_lane() {
    let (session, sketch, profiles) = fixture();
    let before = session.document().clone();
    let (mut controller, state) = controller(&session, sketch, profiles);
    assert!(controller.active());
    assert_eq!(session.document(), &before);
    controller
        .preview(&session, &Default::default(), Channel::CommandLine)
        .unwrap();
    assert!(controller.pending());
    assert!(!controller.can_apply(&session, &Default::default()));
    let sent = state.lock().unwrap().requests[0].0.clone();
    assert_ne!(sent, before);
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), 0);
    assert!(!session.can_undo());
    reply(&state, 0);
    controller.poll(&session, &Default::default());
    assert!(controller.can_apply(&session, &Default::default()));
    assert!(
        controller
            .display_scene(&session, &Default::default())
            .is_some()
    );
    controller.cancel();
    assert!(!controller.active());
    assert!(
        controller
            .display_scene(&session, &Default::default())
            .is_none()
    );
    assert_eq!(session.document(), &before);
    assert!(!session.can_undo());
}
#[test]
fn only_matching_success_can_apply_exact_transaction_with_one_undo_and_handoff() {
    let (mut session, sketch, profiles) = fixture();
    let before = session.document().clone();
    let (mut controller, state) = controller(&session, sketch, profiles);
    controller
        .preview(&session, &Default::default(), Channel::CommandLine)
        .unwrap();
    assert!(controller.apply(&mut session, &Default::default()).is_err());
    // A premature Apply must not destroy the pending candidate.
    reply(&state, 0);
    controller.poll(&session, &Default::default());
    let preview = controller
        .display_scene(&session, &Default::default())
        .unwrap();
    assert!(
        controller
            .committed_preview(&session, &Default::default())
            .is_none()
    );
    let sent = state.lock().unwrap().requests[0].0.clone();
    let outputs = controller.apply(&mut session, &Default::default()).unwrap();
    assert_eq!(outputs.len(), 2);
    assert_eq!(session.document(), &sent);
    assert_eq!(session.revision(), 1);
    assert!(!controller.active());
    assert!(Arc::ptr_eq(
        &preview,
        controller
            .committed_preview(&session, &Default::default())
            .unwrap()
    ));
    assert_eq!(
        session.activity().last().unwrap().actor.channel,
        Channel::CommandLine
    );
    assert!(Arc::ptr_eq(
        &preview,
        &controller
            .display_scene(&session, &Default::default())
            .unwrap()
    ));
    controller.canonical_ready(&session);
    assert!(
        controller
            .display_scene(&session, &Default::default())
            .is_none()
    );
    session.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(2, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &sent);
}
#[test]
fn stale_reply_cancelled_tool_and_foreign_session_never_enable_apply() {
    let (session, sketch, profiles) = fixture();
    let (mut controller, state) = controller(&session, sketch, profiles.clone());
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    let first = state.lock().unwrap().requests[0].2;
    controller.cancel();
    controller
        .begin(&session, &Default::default(), sketch, profiles, Channel::Ui)
        .unwrap();
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    assert!(state.lock().unwrap().requests[1].2.generation > first.generation);
    reply(&state, 0);
    controller.poll(&session, &Default::default());
    assert!(controller.pending());
    assert!(!controller.can_apply(&session, &Default::default()));
    reply(&state, 1);
    controller.poll(&session, &Default::default());
    assert!(controller.can_apply(&session, &Default::default()));
    let mut foreign = Session::new(session.document().clone()).unwrap();
    assert!(controller.apply(&mut foreign, &Default::default()).is_err());
    assert_eq!(foreign.revision(), 0);
    controller.poll(&foreign, &Default::default());
    assert!(!controller.can_apply(&foreign, &Default::default()));
}
#[test]
fn source_revision_and_resource_changes_invalidate_native_receipts() {
    for resource_change in [false, true] {
        let (mut session, sketch, profiles) = fixture();
        let (mut controller, state) = controller(&session, sketch, profiles);
        controller
            .preview(&session, &Default::default(), Channel::Ui)
            .unwrap();
        reply(&state, 0);
        let mut resources = ProjectResources::default();
        if resource_change {
            resources
                .metadata
                .insert("changed".into(), serde_json::json!(true));
        } else {
            let mut entity = session.document().get(sketch).unwrap().clone();
            entity.name = "Externally renamed sketch".into();
            session
                .apply(Request {
                    expected_revision: 0,
                    actor: Actor::local(Channel::Mcp),
                    label: "External source edit".into(),
                    edits: vec![Mutation::Replace(entity)],
                })
                .unwrap();
        }
        let before = session.document().clone();
        let cancellations = state.lock().unwrap().cancellations;
        controller.poll(&session, &resources);
        assert_eq!(state.lock().unwrap().cancellations, cancellations + 1);
        assert!(!controller.can_apply(&session, &resources));
        assert!(controller.display_scene(&session, &resources).is_none());
        assert!(controller.apply(&mut session, &resources).is_err());
        assert_eq!(session.document(), &before);
    }
}
#[test]
fn wrong_scope_empty_mesh_success_and_native_failure_cannot_be_applied() {
    for kind in [0, 1, 2, 3] {
        let (mut session, sketch, profiles) = fixture();
        let before = session.document().clone();
        let (mut controller, state) = controller(&session, sketch, profiles);
        controller
            .preview(&session, &Default::default(), Channel::Ui)
            .unwrap();
        let (document, _, stamp) = state.lock().unwrap().requests[0].clone();
        let result = match kind {
            0 => Ok(native_scene(
                &document,
                SceneStamp {
                    generation: stamp.generation + 1,
                    ..stamp
                },
            )),
            1 => Ok(Arc::new(Scene::new(stamp, vec![], vec![]).unwrap())),
            2 => Err("Native exact profile is open".into()),
            _ => {
                let full = native_scene(&document, stamp);
                Ok(Arc::new(
                    Scene::new(
                        stamp,
                        full.geometries().values().cloned().collect(),
                        full.reference_instances().to_vec(),
                    )
                    .unwrap()
                    .with_source_scope(vec![sketch])
                    .unwrap(),
                ))
            }
        };
        state
            .lock()
            .unwrap()
            .replies
            .push_back(SceneReply { stamp, result });
        controller.poll(&session, &Default::default());
        assert!(!controller.can_apply(&session, &Default::default()));
        assert!(controller.error().is_some());
        assert!(controller.apply(&mut session, &Default::default()).is_err());
        assert_eq!(session.document(), &before);
    }
}
#[test]
fn candidate_hash_and_form_stamp_are_checked_independently_of_reply_scope() {
    for corrupt_hash in [true, false] {
        let (session, sketch, profiles) = fixture();
        let (mut controller, state) = controller(&session, sketch, profiles);
        controller
            .preview(&session, &Default::default(), Channel::Ui)
            .unwrap();
        if corrupt_hash {
            controller.candidate.as_mut().unwrap().content = ContentHash::of(b"wrong candidate");
        } else {
            let request = controller
                .form
                .as_ref()
                .unwrap()
                .request(session.document(), controller.capabilities())
                .unwrap();
            let ModelRequest::Extrude(request) = request else {
                panic!("extrusion form")
            };
            let mut panel = ExtrudePanel::default();
            panel.begin_request(*request).unwrap();
            controller.form = Some(ModelForm::Extrude(Box::new(panel)));
        }
        reply(&state, 0);
        controller.poll(&session, &Default::default());
        assert!(!controller.can_apply(&session, &Default::default()));
        assert!(
            controller
                .display_scene(&session, &Default::default())
                .is_none()
        );
    }
}

#[test]
fn primitives_use_the_same_native_gate_without_any_sketch() {
    for kind in [
        ModelingTool::Box,
        ModelingTool::Cylinder,
        ModelingTool::Sphere,
    ] {
        let mut session = Session::new(Document::new("No sketch required").unwrap()).unwrap();
        let before = session.document().clone();
        let state = Arc::new(Mutex::new(State::default()));
        let mut controller = ModelToolController::default();
        controller.set_backend(Box::new(ScriptedBackend(state.clone())));
        controller
            .begin_primitive(&session, &Default::default(), kind, Channel::Ui)
            .unwrap();
        assert_eq!(controller.label(), kind.label());
        assert!(controller.active());
        assert_eq!(session.document(), &before);
        assert!(controller.pending(), "Opening requests the native preview");
        assert!(!controller.can_apply(&session, &Default::default()));
        let first = state.lock().unwrap().requests[0].0.clone();
        controller
            .preview(&session, &Default::default(), Channel::Ui)
            .unwrap();
        assert_eq!(
            state.lock().unwrap().requests[1].0,
            first,
            "Repeated previews preserve primitive identities and names"
        );
        reply(&state, 0);
        controller.poll(&session, &Default::default());
        assert!(!controller.can_apply(&session, &Default::default()));
        reply(&state, 1);
        controller.poll(&session, &Default::default());
        let output = controller.apply(&mut session, &Default::default()).unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(session.document(), &first);
        assert_eq!(session.revision(), 1);
        let source =
            multicad_core::model::read_primitive(session.document(), output[0].feature).unwrap();
        assert!(matches!(
            (kind, source.parameters),
            (ModelingTool::Box, PrimitiveParameters::Box { .. })
                | (ModelingTool::Cylinder, PrimitiveParameters::Cylinder { .. })
                | (ModelingTool::Sphere, PrimitiveParameters::Sphere { .. })
        ));
        session.undo(1, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(session.document(), &before);
    }
}
#[test]
fn primitive_drag_replaces_the_native_preview_with_current_dimensions() {
    let session = Session::new(Document::new("Primitive drag").unwrap()).unwrap();
    let resources = ProjectResources::default();
    let state = Arc::new(Mutex::new(State::default()));
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(ScriptedBackend(state.clone())));
    controller
        .begin_primitive(&session, &resources, ModelingTool::Box, Channel::Ui)
        .unwrap();

    let context = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.));
    let camera = crate::camera::Camera {
        yaw: 0.,
        pitch: std::f32::consts::FRAC_PI_2,
        scale: 2.,
        target: [50., 30., 0.],
        perspective: false,
    };
    let project = |point| {
        let [x, y, _] = camera.project(point, rect);
        egui::pos2(x, y)
    };
    let frame = |controller: &mut ModelToolController, events| {
        let mut owned = false;
        let output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(rect),
                events,
                ..Default::default()
            },
            |ui| {
                ui.interact(
                    rect,
                    ui.id().with("viewport"),
                    egui::Sense::click_and_drag(),
                );
                owned = controller.primitive_viewport(ui, &session, &resources, camera, rect, true);
            },
        );
        output.drop_without_applying_deltas();
        owned
    };
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: egui::Modifiers::NONE,
    };

    frame(&mut controller, vec![]);
    let width_handle = project([100., 60., 20.]);
    let new_width = project([135., 60., 20.]);
    assert!(frame(
        &mut controller,
        vec![
            egui::Event::PointerMoved(width_handle),
            button(width_handle, true)
        ],
    ));
    assert!(frame(
        &mut controller,
        vec![egui::Event::PointerMoved(new_width)]
    ));
    reply(&state, 0);
    controller.poll(&session, &resources);
    assert!(!controller.can_apply(&session, &resources));
    assert!(frame(&mut controller, vec![button(new_width, false)]));
    assert_eq!(state.lock().unwrap().requests.len(), 2);
    reply(&state, 1);
    controller.poll(&session, &resources);
    assert!(controller.can_apply(&session, &resources));

    let origin = project([0.; 3]);
    let place_handle = origin + egui::vec2(75., 0.);
    let placed = place_handle + egui::vec2(25., 0.);
    assert!(frame(
        &mut controller,
        vec![
            egui::Event::PointerMoved(place_handle),
            button(place_handle, true)
        ],
    ));
    assert!(frame(
        &mut controller,
        vec![egui::Event::PointerMoved(placed)]
    ));
    assert!(!controller.can_apply(&session, &resources));
    assert!(frame(&mut controller, vec![button(placed, false)]));
    let current = state.lock().unwrap().requests[2].0.clone();
    let feature = current
        .entities()
        .find(|entity| multicad_core::model::read_primitive(&current, entity.id).is_ok())
        .unwrap();
    let primitive = multicad_core::model::read_primitive(&current, feature.id).unwrap();
    let model::ResolvedPrimitive::Box { size_mm, frame } =
        primitive.parameters.resolve(session.parameters()).unwrap()
    else {
        panic!("dragged primitive must remain a box");
    };
    assert!((size_mm[0] - 135.).abs() < 0.001, "{size_mm:?}");
    assert_eq!(size_mm[1..], [60., 20.]);
    let [x, y, _] = camera.project(frame.origin.map(|v| v as f32), rect);
    assert!(egui::pos2(x, y).distance(origin + (placed - place_handle)) < 0.001);

    reply(&state, 2);
    controller.poll(&session, &resources);
    assert!(controller.can_apply(&session, &resources));
    controller.cancel();
    assert_eq!(session.revision(), 0);
    assert!(session.document().entities().next().is_none());
}

#[test]
fn generated_names_are_unique_against_committed_bodies_and_stable_during_preview() {
    let (mut session, sketch, profiles) = fixture();
    let (mut controller, state) = controller(&session, sketch, profiles.clone());
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    reply(&state, 0);
    controller.poll(&session, &Default::default());
    let first = controller.apply(&mut session, &Default::default()).unwrap();
    let old_names = first
        .iter()
        .map(|body| session.document().get(body.body).unwrap().name.clone())
        .collect::<Vec<_>>();
    controller
        .begin(&session, &Default::default(), sketch, profiles, Channel::Ui)
        .unwrap();
    let ModelRequest::Extrude(request) = controller
        .form
        .as_ref()
        .unwrap()
        .request(session.document(), controller.capabilities())
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(old_names, ["Extruded part 1", "Extruded part 2"]);
    assert_eq!(
        request
            .outputs
            .iter()
            .map(|body| body.name.as_str())
            .collect::<Vec<_>>(),
        ["Extruded part 3", "Extruded part 4"]
    );
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    let sent = state.lock().unwrap().requests[1].0.clone();
    for output in &request.outputs {
        assert_eq!(sent.get(output.body).unwrap().name, output.name);
    }
}
#[test]
fn revolve_creation_uses_checked_receipt_and_preserves_profiles_and_output_identities() {
    let (mut session, sketch, profiles) = fixture();
    let before = session.document().clone();
    let state = Arc::new(Mutex::new(State::default()));
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(ScriptedBackend(state.clone())));
    controller
        .begin_revolve(
            &session,
            &Default::default(),
            sketch,
            profiles.clone(),
            Channel::Ui,
        )
        .unwrap();
    // New revolves require an explicit axis; choose the sketch V axis.
    controller.choose_revolve_axis([0., 0.], 90.);
    let ModelRequest::Revolve(request) = controller
        .form
        .as_ref()
        .unwrap()
        .request(session.document(), controller.capabilities())
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        request
            .outputs
            .iter()
            .map(|out| out.region.clone())
            .collect::<Vec<_>>(),
        profiles
    );
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    assert!(controller.apply(&mut session, &Default::default()).is_err());
    reply(&state, 0);
    controller.poll(&session, &Default::default());
    let output = controller.apply(&mut session, &Default::default()).unwrap();
    assert_eq!(output.len(), 2);
    for out in output {
        let source = multicad_core::model::read_revolve(session.document(), out.feature).unwrap();
        assert_eq!(source.output.body, out.body);
        assert_eq!(source.profile.sketch_id(), Some(sketch));
    }
    assert_eq!(session.document().get(sketch), before.get(sketch));
    assert_eq!(session.revision(), 1);
    session.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(session.document(), &before);
}

fn boolean_fixture() -> (Session, Vec<CreatedBody>, Arc<Scene>) {
    let mut session = Session::new(Document::new("Boolean receipt membership").unwrap()).unwrap();
    let mut bodies = vec![];
    for index in 0..3 {
        let request = PrimitiveRequest::new(
            format!("Body {index}"),
            PrimitiveParameters::Box {
                size: [length(30.), length(20.), length(10.)],
                frame: PrimitiveFrame::default(),
            },
        );
        let prepared = multicad_core::model::prepare_primitive(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            &request,
        )
        .unwrap();
        bodies.extend(prepared.outputs.clone());
        session.commit(prepared.prepared).unwrap();
    }
    let scene = native_scene(
        session.document(),
        SceneStamp {
            document: session.document().id(),
            revision: session.revision(),
            generation: 1,
        },
    );
    (session, bodies, scene)
}
#[test]
fn boolean_batch_proof_requires_target_heads_and_exact_tool_membership_before_one_commit() {
    use multicad_core::model::{BooleanRequest, Operation};
    use std::collections::BTreeSet;
    for keep in [false, true] {
        for wrong_membership in [false, true] {
            let (mut session, bodies, input_scene) = boolean_fixture();
            let before = session.document().clone();
            let state = Arc::new(Mutex::new(State::default()));
            let mut controller = ModelToolController::default();
            controller.set_backend(Box::new(ScriptedBackend(state.clone())));
            controller
                .begin_boolean(
                    &session,
                    &Default::default(),
                    &input_scene,
                    &BTreeSet::from([bodies[0].body, bodies[1].body]),
                    Channel::Ui,
                )
                .unwrap();
            let request = ModelRequest::Booleans(
                bodies[..2]
                    .iter()
                    .map(|body| {
                        let mut request = BooleanRequest::new(
                            Operation::Cut,
                            BodyInput {
                                body: body.body,
                                producer: body.feature,
                            },
                            vec![BodyInput {
                                body: bodies[2].body,
                                producer: bodies[2].feature,
                            }],
                        );
                        request.keep_tools = keep;
                        request
                    })
                    .collect(),
            );
            controller
                .submit(
                    &session,
                    &Default::default(),
                    Channel::Ui,
                    controller.stamp().unwrap(),
                    &request,
                )
                .unwrap();
            let (document, _, stamp) = state.lock().unwrap().requests[0].clone();
            let full = native_scene(&document, stamp);
            let remove_tool = if wrong_membership { keep } else { !keep };
            let instances = full
                .instances()
                .iter()
                .filter(|instance| !remove_tool || instance.id != bodies[2].body)
                .cloned()
                .collect();
            let scene = Arc::new(
                Scene::new(
                    stamp,
                    full.geometries().values().cloned().collect(),
                    instances,
                )
                .unwrap(),
            );
            state.lock().unwrap().replies.push_back(SceneReply {
                stamp,
                result: Ok(scene),
            });
            controller.poll(&session, &Default::default());
            if wrong_membership {
                assert!(!controller.can_apply(&session, &Default::default()));
                assert!(controller.apply(&mut session, &Default::default()).is_err());
                assert_eq!(session.document(), &before);
            } else {
                let revision = session.revision();
                let output = controller.apply(&mut session, &Default::default()).unwrap();
                assert_eq!(output.len(), 2);
                assert_eq!(session.revision(), revision + 1);
                assert_eq!(
                    session.document().get(bodies[2].body),
                    before.get(bodies[2].body)
                );
                session
                    .undo(session.revision(), Actor::local(Channel::Ui))
                    .unwrap();
                assert_eq!(session.document(), &before);
            }
        }
    }
}
#[test]
fn consumption_proof_tracks_the_current_producer_version_instead_of_hiding_future_heads() {
    use multicad_core::{
        document::Entity,
        model::{BodyOutput, BooleanRequest, Operation, PrimitiveDefinition},
    };
    let (session, bodies, _) = boolean_fixture();
    let request = BooleanRequest::new(
        Operation::Cut,
        BodyInput {
            body: bodies[0].body,
            producer: bodies[0].feature,
        },
        vec![BodyInput {
            body: bodies[2].body,
            producer: bodies[2].feature,
        }],
    );
    let request = ModelRequest::Booleans(vec![request]);
    let prepared = request
        .prepare(&session, Actor::local(Channel::Ui))
        .unwrap();
    let proof = request.tool_proof(prepared.prepared.document()).unwrap();
    assert!(proof[0].consumed);
    assert_eq!(proof[0].input.producer, bodies[2].feature);
    let parameters = PrimitiveParameters::Box {
        size: [length(35.), length(25.), length(15.)],
        frame: PrimitiveFrame::default(),
    };
    let feature = Entity::new(
        "Later current tool producer",
        EntityData::Feature {
            definition: PrimitiveDefinition {
                output: BodyOutput {
                    body: bodies[2].body,
                },
                parameters,
            }
            .definition()
            .unwrap(),
            inputs: vec![],
            suppressed: false,
        },
    );
    let mut body = prepared
        .prepared
        .document()
        .get(bodies[2].body)
        .unwrap()
        .clone();
    let EntityData::Part { features, .. } = &mut body.data else {
        panic!()
    };
    features.push(feature.id);
    let later = prepared
        .prepared
        .document()
        .edited(&[Mutation::Insert(feature.clone()), Mutation::Replace(body)])
        .unwrap();
    let proof = request.tool_proof(&later).unwrap();
    assert!(!proof[0].consumed);
    assert_eq!(proof[0].input.producer, feature.id);
}

#[path = "conversion_tests.rs"]
mod conversion_tests;
#[path = "edit_tests.rs"]
mod edit_tests;
#[path = "group_tests.rs"]
mod group_tests;
#[path = "operation_tests.rs"]
mod operation_tests;

#[test]
fn split_requires_both_native_outputs_and_original_input_consumption_before_atomic_apply() {
    use multicad_core::model::{self, ModelEvaluationInfo};
    use std::collections::BTreeSet;
    for keep_tools in [false, true] {
        let (mut session, bodies, raw) = boolean_fixture();
        let original = session.document().clone();
        let current = Arc::new(
            Scene::new(
                raw.stamp(),
                raw.geometries().values().cloned().collect(),
                raw.instances().to_vec(),
            )
            .unwrap()
            .with_model_evaluation(Arc::new(ModelEvaluationInfo {
                screw_connection_checks: vec![],
                source_hash: ContentHash::document(session.document()).unwrap(),
                consumed_bodies: BTreeSet::new(),
                cut_targets: vec![],
            }))
            .unwrap(),
        );
        let state = Arc::new(Mutex::new(State::default()));
        let mut controller = ModelToolController::default();
        controller.set_backend(Box::new(ScriptedBackend(state.clone())));
        controller
            .begin_split(
                &session,
                &Default::default(),
                &current,
                &BTreeSet::from([bodies[0].body]),
                Channel::Ui,
            )
            .unwrap();
        let request = model::SplitRequest::new(
            BodyInput {
                body: bodies[0].body,
                producer: bodies[0].feature,
            },
            BodyInput {
                body: bodies[1].body,
                producer: bodies[1].feature,
            },
            keep_tools,
        );
        let request = ModelRequest::Split(Box::new(request));
        let form = controller.stamp().unwrap();
        controller
            .submit(
                &session,
                &Default::default(),
                Channel::CommandLine,
                form,
                &request,
            )
            .unwrap();
        let (candidate, _, stamp) = state.lock().unwrap().requests[0].clone();
        let make_scene = |stamp, consume_target| {
            let raw = native_scene(&candidate, stamp);
            let mut consumed = BTreeSet::new();
            if consume_target {
                consumed.insert(bodies[0].body);
            }
            if !keep_tools {
                consumed.insert(bodies[1].body);
            }
            Arc::new(
                Scene::new(
                    stamp,
                    raw.geometries().values().cloned().collect(),
                    raw.instances()
                        .iter()
                        .filter(|instance| !consumed.contains(&instance.id))
                        .cloned()
                        .collect(),
                )
                .unwrap()
                .with_model_evaluation(Arc::new(ModelEvaluationInfo {
                    screw_connection_checks: vec![],
                    source_hash: ContentHash::document(&candidate).unwrap(),
                    consumed_bodies: consumed,
                    cut_targets: vec![],
                }))
                .unwrap(),
            )
        };
        // All requested new bodies are present, but the original target is
        // incorrectly still a final output. Metadata alone cannot excuse it.
        state.lock().unwrap().replies.push_back(SceneReply {
            stamp,
            result: Ok(make_scene(stamp, false)),
        });
        controller.poll(&session, &Default::default());
        assert!(!controller.can_apply(&session, &Default::default()));
        assert_eq!(session.document(), &original);
        controller
            .submit(
                &session,
                &Default::default(),
                Channel::CommandLine,
                form,
                &request,
            )
            .unwrap();
        let stamp = state.lock().unwrap().requests[1].2;
        state.lock().unwrap().replies.push_back(SceneReply {
            stamp,
            result: Ok(make_scene(stamp, true)),
        });
        controller.poll(&session, &Default::default());
        assert!(
            controller.can_apply(&session, &Default::default()),
            "{:?}",
            controller.error()
        );
        let outputs = controller.apply(&mut session, &Default::default()).unwrap();
        assert_eq!(outputs.len(), 2);
        assert_ne!(outputs[0].body, outputs[1].body);
        assert!(
            outputs
                .iter()
                .all(|output| original.get(output.body).is_none())
        );
        assert_eq!(session.document(), &candidate);
        assert_eq!(session.revision(), 4);
        assert_eq!(
            session.activity().last().unwrap().actor.channel,
            Channel::CommandLine
        );
        session.undo(4, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(session.document(), &original);
        session.redo(5, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(session.document(), &candidate);
    }
}

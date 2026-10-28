//! Candidate publication and transactional lifecycle regressions. The scripted
//! provider controls receipt order; native compilation correctness is verified
//! separately by the production model crate.
use super::*;
use crate::viewport::{Geometry, Instance, SceneReply, Transform};
use multicad_core::{
    constraint_sketch::source::Source,
    document::{Document, Mutation},
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
}
struct ScriptedBackend(Arc<Mutex<State>>);
impl SceneBackend for ScriptedBackend {
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
    let sent = state.lock().unwrap().requests[0].0.clone();
    let outputs = controller.apply(&mut session, &Default::default()).unwrap();
    assert_eq!(outputs.len(), 2);
    assert_eq!(session.document(), &sent);
    assert_eq!(session.revision(), 1);
    assert!(!controller.active());
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
        controller.poll(&session, &resources);
        assert!(!controller.can_apply(&session, &resources));
        assert!(controller.display_scene(&session, &resources).is_none());
        assert!(controller.apply(&mut session, &resources).is_err());
        assert_eq!(session.document(), &before);
    }
}
#[test]
fn wrong_scope_empty_mesh_success_and_native_failure_cannot_be_applied() {
    for kind in [0, 1, 2] {
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
            _ => Err("Native exact profile is open".into()),
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
    for kind in [ModelingTool::Box, ModelingTool::Cylinder] {
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
        controller
            .preview(&session, &Default::default(), Channel::Ui)
            .unwrap();
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
        ));
        session.undo(1, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(session.document(), &before);
    }
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

use super::*;
use crate::viewport::{
    Geometry as DisplayGeometry, Instance, Scene, SceneBackend, SceneFeed, SceneReply, SceneStamp,
    Transform,
};
use multicad_core::{model, project_file::ContentHash};
use multicad_kernel::{GeometryPolicy, MeshOptions, Solid};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Default)]
struct Backend {
    reply: Option<SceneReply>,
    missing_info: bool,
    wrong_hash: bool,
}
impl SceneBackend for Backend {
    fn request(&mut self, document: &Document, stamp: SceneStamp) -> Result<(), String> {
        let feature=document.entities().find(|entity|matches!(&entity.data,EntityData::Feature{definition,..} if definition.type_id==model::PRIMITIVE_TYPE)).unwrap();
        let source = model::read_primitive(document, feature.id).unwrap();
        let solid =
            Solid::box_named([40., 30., 20.], feature.id, GeometryPolicy::default()).unwrap();
        let mesh = solid.mesh(MeshOptions::default()).unwrap();
        let geometry = DisplayGeometry::new(
            feature.id,
            mesh.vertices.as_chunks::<3>().0.to_vec(),
            mesh.indices,
            mesh.face_indices,
            solid
                .topology_snapshot(feature.id, stamp.revision)
                .unwrap()
                .elements,
        )
        .unwrap();
        let mut scene = Scene::new(
            stamp,
            vec![Arc::new(geometry)],
            vec![Instance {
                id: source.output.body,
                geometry: feature.id,
                transform: Transform::default(),
                color: [140, 180, 210, 255],
            }],
        )
        .unwrap();
        if !self.missing_info {
            scene = scene
                .with_model_evaluation(Arc::new(model::ModelEvaluationInfo {
                    screw_connection_checks: vec![],
                    source_hash: if self.wrong_hash {
                        ContentHash::of(b"different source")
                    } else {
                        ContentHash::document(document).unwrap()
                    },
                    consumed_bodies: BTreeSet::new(),
                    cut_targets: vec![],
                }))
                .unwrap();
        }
        self.reply = Some(SceneReply {
            stamp,
            result: Ok(Arc::new(scene)),
        });
        Ok(())
    }
    fn poll(&mut self) -> Option<SceneReply> {
        self.reply.take()
    }
}
fn fixture() -> (Session, model::PrimitiveRequest) {
    let mut session = Session::new(Document::new("Feature Inspector").unwrap()).unwrap();
    let request = model::PrimitiveRequest::new(
        "Editable box",
        model::PrimitiveParameters::Box {
            size: [model::length(40.), model::length(30.), model::length(20.)],
            frame: Default::default(),
        },
    );
    let prepared =
        model::prepare_primitive(&session, 0, Actor::local(Channel::Ui), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    (session, request)
}
fn frame(
    context: &egui::Context,
    editor: &mut SketchEditor,
    session: &mut Session,
    feed: &mut SceneFeed,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200., 900.))),
            events,
            ..Default::default()
        },
        |ui| editor.show(ui, session, false, feed),
    );
    output.textures_delta.clear();
    output
}
fn click(
    context: &egui::Context,
    editor: &mut SketchEditor,
    session: &mut Session,
    feed: &mut SceneFeed,
    label: &str,
) {
    let output = frame(context, editor, session, feed, vec![]);
    let position = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing Inspector action {label}"));
    for pressed in [true, false] {
        frame(
            context,
            editor,
            session,
            feed,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
}
#[test]
fn inspector_opens_saved_feature_controls_and_preview_cancel_preserves_source() {
    let (mut session, request) = fixture();
    let before = session.document().clone();
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::<Backend>::default());
    feed.tick(session.document(), session.revision());
    let mut editor = SketchEditor::default();
    editor.set_model_backend(Box::<Backend>::default());
    editor.tree_selection = BTreeSet::from([request.body]);
    assert_eq!(
        editor.selected_applied_feature(session.document()),
        Some((request.feature, true))
    );
    assert!(editor.feature_edit_ready(&session, &feed, request.feature));
    let context = egui::Context::default();
    click(
        &context,
        &mut editor,
        &mut session,
        &mut feed,
        "Edit latest feature",
    );
    assert!(editor.model.editing());
    assert!(editor.has_pending_edits());
    click(&context, &mut editor, &mut session, &mut feed, "Preview");
    frame(&context, &mut editor, &mut session, &mut feed, vec![]);
    assert!(editor.model_can_apply(&session, &Default::default()));
    click(&context, &mut editor, &mut session, &mut feed, "Cancel");
    assert!(!editor.model_active());
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), 1);
    assert!(editor.message.contains("cancelled"));
    editor.tree_selection = BTreeSet::from([request.feature]);
    assert_eq!(
        editor.selected_applied_feature(session.document()),
        Some((request.feature, false))
    );
    click(
        &context,
        &mut editor,
        &mut session,
        &mut feed,
        "Edit feature",
    );
    assert!(editor.model.editing());
    editor.cancel_model();
    // A changed source revision cannot reuse the former current scene.
    let mut renamed = session.document().get(request.body).unwrap().clone();
    renamed.name = "Renamed box".into();
    let prepared = session
        .prepare(multicad_core::commands::Request {
            expected_revision: 1,
            actor: Actor::local(Channel::Ui),
            label: "Unrelated edit".into(),
            edits: vec![multicad_core::document::Mutation::Replace(renamed)],
        })
        .unwrap();
    session.commit(prepared).unwrap();
    assert!(editor.feature_edit_ready(&session, &feed, request.feature));
    editor
        .begin_feature_edit(
            &session,
            &Default::default(),
            &feed,
            request.feature,
            Channel::Ui,
        )
        .unwrap();
    assert!(editor.model_active());
    assert!(
        !editor.model_can_apply(&session, &Default::default()),
        "The stale canonical scene cannot authorize an edit"
    );
    editor
        .preview_model(&session, &Default::default(), Channel::Ui)
        .unwrap();
    editor.poll_model(&session, &Default::default());
    assert!(
        editor.model_can_apply(&session, &Default::default()),
        "A separate fresh candidate can validate the saved source controls"
    );
    editor.cancel_model();
}
#[test]
fn source_controls_open_without_native_context_but_fresh_preview_metadata_is_mandatory() {
    let (session, request) = fixture();
    let mut editor = SketchEditor::default();
    for missing_info in [true, false] {
        editor.set_model_backend(Box::new(Backend {
            missing_info,
            wrong_hash: true,
            ..Default::default()
        }));
        let mut feed = SceneFeed::default();
        feed.set_backend(Box::new(Backend {
            missing_info,
            wrong_hash: true,
            ..Default::default()
        }));
        feed.tick(session.document(), session.revision());
        assert!(editor.feature_edit_ready(&session, &feed, request.feature));
        editor
            .begin_feature_edit(
                &session,
                &Default::default(),
                &feed,
                request.feature,
                Channel::Ui,
            )
            .unwrap();
        assert!(editor.model_active());
        assert!(!editor.model_can_apply(&session, &Default::default()));
        editor
            .preview_model(&session, &Default::default(), Channel::Ui)
            .unwrap();
        editor.poll_model(&session, &Default::default());
        assert!(
            !editor.model_can_apply(&session, &Default::default()),
            "Missing or foreign candidate metadata must still reject Apply"
        );
        editor.cancel_model();
    }
}

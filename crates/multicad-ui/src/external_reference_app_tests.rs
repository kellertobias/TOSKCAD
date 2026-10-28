use super::*;
use crate::{
    external_references::{
        ExternalTool,
        tests::{fixture, scene},
    },
    viewport::{Scene, SceneBackend, SceneReply, SceneStamp},
};
use std::sync::Arc;
struct Storage;
impl DocumentStorage for Storage {
    fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
        Err("File IO is outside this UI fixture".into())
    }
    fn load(&mut self, _: &std::path::Path) -> Result<Document, String> {
        Err("File IO is outside this UI fixture".into())
    }
}
struct Backend {
    scene: Arc<Scene>,
    reply: Option<SceneReply>,
}
impl SceneBackend for Backend {
    fn request(&mut self, document: &Document, stamp: SceneStamp) -> Result<(), String> {
        if self.scene.model_evaluation().unwrap().source_hash
            != multicad_core::project_file::ContentHash::document(document)?
        {
            return Err("Test native scene belongs to different source".into());
        }
        let scene = Scene::new(
            stamp,
            self.scene.geometries().values().cloned().collect(),
            self.scene.reference_instances().to_vec(),
        )?
        .with_model_evaluation(self.scene.model_evaluation().unwrap().clone())?;
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
fn frame(context: &egui::Context, app: &mut Application, events: Vec<egui::Event>) {
    context
        .run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1360., 900.),
                )),
                events,
                ..Default::default()
            },
            |ui| app.show(ui),
        )
        .drop_without_applying_deltas();
}
#[test]
fn ribbon_command_pointer_preview_apply_and_undo_use_the_shared_external_transaction() {
    let mut fixture = fixture();
    let document = fixture.session.document().clone();
    let EntityData::Sketch { plane, .. } = document.get(fixture.sketch).unwrap().data else {
        panic!()
    };
    let mut plane_entity = document.get(plane).unwrap().clone();
    let EntityData::Plane { origin_mm, .. } = &mut plane_entity.data else {
        panic!()
    };
    origin_mm[2] = 10.;
    let document = document.edited(&[Mutation::Replace(plane_entity)]).unwrap();
    fixture.session = Session::new(document.clone()).unwrap();
    let native = scene(&fixture, 1);
    let mut app = Application::with_document(Box::new(Storage), document.clone()).unwrap();
    app.current_mut().scene.set_backend(Box::new(Backend {
        scene: native,
        reply: None,
    }));
    app.mode = Mode::Sketch;
    app.current_mut()
        .sketch
        .open_at(&document, fixture.sketch, 0)
        .unwrap();
    let context = egui::Context::default();
    for _ in 0..4 {
        frame(&context, &mut app, vec![]);
    }
    assert!(app.enabled(Command::External(ExternalTool::Project)));
    app.invoke(
        Command::External(ExternalTool::Project),
        &[],
        Channel::CommandLine,
    )
    .unwrap();
    assert_eq!(
        app.registry.workspace(Mode::Sketch).tabs[app.tab].title,
        "References"
    );
    assert!(!app.enabled(Command::Preview));
    app.current_mut().sketch.request_fit();
    frame(&context, &mut app, vec![]);
    assert_eq!(
        app.current().sketch.camera.target,
        [15., 10., 5.],
        "Fit uses current native source bounds while selecting a reference, not the empty consumer sketch"
    );
    app.current_mut().sketch.camera = crate::camera::Camera {
        yaw: 0.,
        pitch: 1.0,
        scale: 10.,
        target: [15., 10., 10.],
        perspective: false,
    };
    frame(&context, &mut app, vec![]);
    let rect = app.current().sketch.viewport_rect.unwrap();
    let position = app.current().sketch.camera.project([15., 0., 10.], rect);
    let point = egui::pos2(position[0], position[1]);
    for pressed in [true, false] {
        frame(
            &context,
            &mut app,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    assert_eq!(app.current().session.document(), &document);
    app.invoke(Command::Preview, &[], Channel::CommandLine)
        .unwrap();
    assert!(app.current().sketch.external_pending());
    assert!(app.enabled(Command::Apply));
    assert!(!app.enabled(Command::Save));
    assert!(
        app.invoke(Command::Mode(Mode::Model), &[], Channel::Ui)
            .is_err()
    );
    app.invoke(Command::Apply, &[], Channel::CommandLine)
        .unwrap();
    assert_eq!(app.current().session.revision(), 1);
    assert_eq!(
        multicad_core::sketch_external::bindings(app.current().session.document(), fixture.sketch)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        app.current()
            .session
            .activity()
            .last()
            .unwrap()
            .actor
            .channel,
        Channel::CommandLine
    );
    app.invoke(Command::Undo, &[], Channel::CommandLine)
        .unwrap();
    assert_eq!(app.current().session.document(), &document);
    assert!(
        app.invoke(
            Command::External(ExternalTool::Project),
            &["invalid"],
            Channel::CommandLine
        )
        .is_err()
    );
}

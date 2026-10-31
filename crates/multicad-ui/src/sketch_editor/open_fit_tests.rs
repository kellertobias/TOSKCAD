//! CAD-175: opening a modeled project frames the evaluated model once.
use super::*;
use crate::{modeling::native_fixture::NativeBackend, viewport::SceneFeed};

fn frame(
    context: &egui::Context,
    editor: &mut SketchEditor,
    session: &mut Session,
    feed: &mut SceneFeed,
    events: Vec<egui::Event>,
) {
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400., 1000.))),
            events,
            ..Default::default()
        },
        |ui| editor.show(ui, session, false, feed),
    );
    output.textures_delta.clear();
}
/// Run frames until the native scene for the current revision is visible.
fn settle(
    context: &egui::Context,
    editor: &mut SketchEditor,
    session: &mut Session,
    feed: &mut SceneFeed,
) {
    for _ in 0..600 {
        feed.tick(session.document(), session.revision());
        frame(context, editor, session, feed, vec![]);
        if feed.is_current(session.document().id(), session.revision()) {
            frame(context, editor, session, feed, vec![]);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("native scene did not settle: {:?}", feed.error());
}
fn on_screen(editor: &SketchEditor, bounds: [[f64; 3]; 2]) -> bool {
    let rect = editor.viewport_rect.expect("viewport laid out");
    (0..8).all(|corner| {
        let p = editor.camera.project(
            std::array::from_fn(|i| bounds[(corner >> i) & 1][i] as f32),
            rect,
        );
        rect.contains(egui::pos2(p[0], p[1]))
    })
}

#[test]
fn reopened_cabinet_is_framed_once_after_evaluation() {
    let (mut session, ..) = super::cabinet_edit_tests::cabinet();
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::new(NativeBackend::new()));
    let mut editor = SketchEditor::default();
    let context = egui::Context::default();
    settle(&context, &mut editor, &mut session, &mut feed);
    let bounds = feed.visible().unwrap().world_bounds().unwrap();
    assert!(
        on_screen(&editor, bounds),
        "whole cabinet visible without Fit: {:?}",
        editor.camera
    );
}

#[test]
fn later_scenes_never_override_user_navigation() {
    let (mut session, ..) = super::cabinet_edit_tests::cabinet();
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::new(NativeBackend::new()));
    let mut editor = SketchEditor::default();
    let context = egui::Context::default();
    settle(&context, &mut editor, &mut session, &mut feed);
    editor.camera.zoom(600.);
    editor.camera.orbit(Vec2::new(40., 10.));
    let navigated = editor.camera;
    frame(&context, &mut editor, &mut session, &mut feed, vec![]);
    // A new source revision publishes a new scene; the view is kept.
    let prepared = session
        .prepare(multicad_core::commands::Request {
            expected_revision: session.revision(),
            actor: Actor::local(Channel::Ui),
            label: "Rename".into(),
            edits: vec![multicad_core::document::Mutation::RenameDocument(
                "Renamed".into(),
            )],
        })
        .unwrap();
    session.commit(prepared).unwrap();
    settle(&context, &mut editor, &mut session, &mut feed);
    assert_eq!(editor.camera, navigated);
}

#[test]
fn home_frames_the_whole_model_and_fit_keeps_the_orientation() {
    let (mut session, ..) = super::cabinet_edit_tests::cabinet();
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::new(NativeBackend::new()));
    let mut editor = SketchEditor::default();
    let context = egui::Context::default();
    settle(&context, &mut editor, &mut session, &mut feed);
    let bounds = feed.visible().unwrap().world_bounds().unwrap();
    editor.camera.orbit(Vec2::new(120., -30.));
    editor.camera.zoom(900.);
    editor.home_view();
    frame(&context, &mut editor, &mut session, &mut feed, vec![]);
    let default = crate::camera::Camera::default();
    assert_eq!(
        (editor.camera.yaw, editor.camera.pitch),
        (default.yaw, default.pitch)
    );
    assert!(
        on_screen(&editor, bounds),
        "Home frames the cabinet: {:?}",
        editor.camera
    );
    editor.camera.orbit(Vec2::new(80., 20.));
    editor.camera.zoom(900.);
    let (yaw, pitch) = (editor.camera.yaw, editor.camera.pitch);
    assert!(!on_screen(&editor, bounds));
    editor.request_fit();
    frame(&context, &mut editor, &mut session, &mut feed, vec![]);
    assert_eq!((editor.camera.yaw, editor.camera.pitch), (yaw, pitch));
    assert!(
        on_screen(&editor, bounds),
        "Fit frames it in the current orientation"
    );
}

#[test]
fn empty_model_keeps_the_bounded_default_view() {
    let mut session = Session::new(Document::new("Empty").unwrap()).unwrap();
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::new(NativeBackend::new()));
    let mut editor = SketchEditor::default();
    let context = egui::Context::default();
    settle(&context, &mut editor, &mut session, &mut feed);
    // Nothing to frame: no automatic zoom, only the bounded default scale.
    assert_eq!(editor.camera.scale, crate::camera::Camera::default().scale);
    assert!(editor.camera.target.iter().all(|v| v.is_finite()));
    editor.home_view();
    frame(&context, &mut editor, &mut session, &mut feed, vec![]);
    let home = editor.camera;
    assert_eq!(home.target, [0.; 3]);
    assert_eq!(home.scale, crate::camera::Camera::default().scale);
}

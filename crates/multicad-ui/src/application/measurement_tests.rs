use super::*;
struct MemoryStorage;
impl crate::storage::DocumentStorage for MemoryStorage {
    fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
        Err("Not used".into())
    }
    fn load(&mut self, _: &std::path::Path) -> Result<Document, String> {
        Err("Not used".into())
    }
}
#[test]
fn global_measure_command_opens_from_every_mode_and_restores_it_without_source_edits() {
    for mode in Mode::ALL {
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        app.mode = mode;
        let before = app.current().session.document().clone();
        let rev = app.current().session.revision();
        app.invoke(Command::Measure, &[], Channel::Ui).unwrap();
        assert!(app.current().sketch.measurement.active());
        assert_eq!(app.mode, Mode::Model);
        assert!(app.invoke(Command::NewSketch, &[], Channel::Ui).is_err());
        assert!(
            app.invoke(
                Command::Modeling(crate::modeling::ModelingTool::Box),
                &[],
                Channel::Ui
            )
            .is_err()
        );
        app.invoke(Command::Measure, &[], Channel::Ui).unwrap();
        assert_eq!(app.mode, mode);
        assert!(!app.current().sketch.measurement.active());
        assert_eq!(app.current().session.document(), &before);
        assert_eq!(app.current().session.revision(), rev);
    }
}
#[test]
fn measuring_preserves_parameter_draft_and_cannot_apply_or_discard_it() {
    let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
    app.current_mut().editor.rows[0].name = "width".into();
    app.current_mut().editor.rows[0].expression = "600".into();
    assert!(app.current().editor.dirty());
    let before = app.current().session.document().clone();
    app.invoke(Command::Measure, &[], Channel::Ui).unwrap();
    assert!(app.invoke(Command::Apply, &[], Channel::Ui).is_err());
    assert!(app.invoke(Command::Discard, &[], Channel::Ui).is_err());
    app.invoke(Command::Measure, &[], Channel::Ui).unwrap();
    assert_eq!(app.mode, Mode::Parameters);
    assert!(app.current().editor.dirty());
    assert_eq!(app.current().editor.rows[0].name, "width");
    assert_eq!(app.current().session.document(), &before);
}
#[test]
fn native_measure_button_is_available_beside_library_and_uses_the_shared_command() {
    let ctx = egui::Context::default();
    crate::theme::configure(&ctx);
    let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
    app.mode = Mode::Model;
    let draw = |app: &mut Application, events| {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1360., 860.),
                )),
                events,
                ..Default::default()
            },
            |ui| app.show(ui),
        )
    };
    draw(&mut app, vec![]).drop_without_applying_deltas();
    let out = draw(&mut app, vec![]);
    let p = out
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == "Measure" => {
                Some(t.pos + t.galley.size() * 0.5)
            }
            _ => None,
        })
        .expect("Measure button missing");
    out.drop_without_applying_deltas();
    let before = app.current().session.document().clone();
    for pressed in [true, false] {
        draw(
            &mut app,
            vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        )
        .drop_without_applying_deltas();
    }
    assert!(app.current().sketch.measurement.active());
    assert_eq!(app.current().session.document(), &before);
    assert_eq!(app.mode, Mode::Model);
}
#[test]
fn close_and_file_switch_restore_workspace_tab_and_do_not_leave_hidden_measurement_input() {
    let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
    app.mode = Mode::Model;
    app.tab = 2;
    app.invoke(Command::Measure, &[], Channel::Ui).unwrap();
    app.invoke(Command::Measure, &[], Channel::Ui).unwrap();
    assert_eq!((app.mode, app.tab), (Mode::Model, 2));
    app.invoke(Command::Measure, &[], Channel::Ui).unwrap();
    app.invoke(Command::New, &[], Channel::Ui).unwrap();
    assert_eq!(app.projects.len(), 2);
    assert!(!app.projects[0].sketch.measurement.active());
    assert!(app.projects[0].measure_return_view.is_none());
    app.activate_project(0);
    assert!(!app.current().sketch.measurement.active());
    assert!(app.enabled(Command::Parameter));
}

use super::*;
use crate::file_picker::{FilePicker, Reply, Request as PickerRequest};
use std::sync::{Arc, Mutex};
struct Storage;
impl DocumentStorage for Storage {
    fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
        panic!("navigation cannot save")
    }
    fn load(&mut self, _: &std::path::Path) -> Result<Document, String> {
        Err("unused".into())
    }
}
struct Picker(Arc<Mutex<Vec<PickerRequest>>>);
impl FilePicker for Picker {
    fn start(&mut self, request: PickerRequest) -> Result<(), String> {
        self.0.lock().unwrap().push(request);
        Ok(())
    }
    fn poll(&mut self) -> Option<Reply> {
        None
    }
}
fn app() -> Application {
    let mut app = Application::new(Box::new(Storage)).unwrap();
    app.pending_files.clear();
    app
}
fn frame(ctx: &egui::Context, app: &mut Application, events: Vec<egui::Event>) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 700.),
            )),
            focused: true,
            events,
            ..Default::default()
        },
        |ui| app.show(ui),
    )
}
fn button(out: &egui::FullOutput, label: &str) -> egui::Rect {
    out.platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find_map(|(_, n)| {
            let r = n.bounds()?;
            (n.label() == Some(label) && n.role() == egui::accesskit::Role::Button).then(|| {
                egui::Rect::from_min_max(
                    egui::pos2(r.x0 as f32, r.y0 as f32),
                    egui::pos2(r.x1 as f32, r.y1 as f32),
                )
            })
        })
        .unwrap_or_else(|| panic!("missing button {label}"))
}
fn click(ctx: &egui::Context, app: &mut Application, label: &str) {
    let out = frame(ctx, app, vec![]);
    let pos = button(&out, label).center();
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        frame(
            ctx,
            app,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        )
        .drop_without_applying_deltas();
    }
}
fn context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::configure(&ctx);
    ctx
}
#[test]
fn project_omits_the_modeling_actions_shown_in_other_modes() {
    assert_eq!(
        Mode::ALL.map(Mode::title),
        ["Project", "Model", "Motion", "Drawings", "Export"]
    );
    for mode in Mode::ALL {
        let mut app = app();
        app.mode = mode;
        let source = app.current().session.document().clone();
        let ctx = context();
        for _ in 0..3 {
            frame(&ctx, &mut app, vec![]).drop_without_applying_deltas();
        }
        let out = frame(&ctx, &mut app, vec![]);
        if mode == Mode::Parameters {
            let tree = out.platform_output.accesskit_update.as_ref().unwrap();
            for label in ["Add Sketch", "Import", "Library", "Measure", "MCP history"] {
                assert!(
                    !tree
                        .nodes
                        .iter()
                        .any(|(_, node)| node.label() == Some(label)
                            && node.role() == egui::accesskit::Role::Button)
                );
            }
            out.drop_without_applying_deltas();
            continue;
        }
        let row = egui::PanelState::load(&ctx, egui::Id::new("global-actions"))
            .unwrap()
            .outer_rect;
        let mut previous = row.left();
        for label in ["Add Sketch", "Import", "Library", "Measure", "MCP history"] {
            let rect = button(&out, label);
            assert!(
                row.contains_rect(rect),
                "{mode:?} {label} {rect:?} outside {row:?}"
            );
            assert!(rect.left() >= previous);
            previous = rect.right();
        }
        out.drop_without_applying_deltas();
        click(&ctx, &mut app, "Add Sketch");
        assert_eq!(app.mode.top_level(), Mode::Model);
        assert!(app.current().sketch.picking);
        assert!(!app.current().sketch.editing);
        assert_eq!(app.current().session.document(), &source);
        app.invoke(Command::AbortSketch, &[], Channel::Ui).unwrap();
        assert_eq!(app.current().session.document(), &source);
    }
}
#[test]
fn import_format_selection_and_os_picker_cancellation_preserve_mode_document_and_history() {
    use crate::file_picker::Kind;
    for (label, kind) in [
        ("STEP (.step, .stp)", Kind::ImportStep),
        ("KiCad board (.kicad_pcb)", Kind::ImportKicad),
        ("SVG / DXF artwork", Kind::ImportArtwork),
        ("Canonical JSON snapshot", Kind::ImportSource),
    ] {
        let mut app = app();
        app.mode = Mode::Drawings;
        app.set_mechanical_import_backend(crate::mechanical_import::tests::backend());
        let requests = Arc::new(Mutex::new(Vec::new()));
        app.set_file_picker(Box::new(Picker(requests.clone())));
        let source = app.current().session.document().clone();
        let ctx = context();
        app.invoke(Command::Import, &[], Channel::Ui).unwrap();
        for _ in 0..3 {
            frame(&ctx, &mut app, vec![]).drop_without_applying_deltas();
        }
        click(&ctx, &mut app, label);
        assert!(!app.import_open);
        assert_eq!(
            app.mode,
            Mode::Drawings,
            "choosing a format does not change workspace"
        );
        app.start_native_picker().unwrap();
        let request = requests.lock().unwrap().last().unwrap().clone();
        assert_eq!(request.kind, kind);
        app.accept_picker_reply(Reply {
            id: request.id,
            result: Ok(None),
        });
        assert!(app.file_action.is_none());
        assert_eq!(app.mode, Mode::Drawings);
        assert_eq!(app.current().session.document(), &source);
        assert_eq!(app.current().session.revision(), 0);
        assert!(!app.current().session.can_undo());
    }
    let mut app = app();
    app.mode = Mode::Motion;
    let source = app.current().session.document().clone();
    let ctx = context();
    app.invoke(Command::Import, &[], Channel::Ui).unwrap();
    for _ in 0..3 {
        frame(&ctx, &mut app, vec![]).drop_without_applying_deltas();
    }
    click(&ctx, &mut app, "Cancel");
    assert!(!app.import_open);
    assert_eq!(app.mode, Mode::Motion);
    assert_eq!(app.current().session.document(), &source);
}
#[test]
fn legacy_sketch_recall_enters_model_without_stale_editing_tools_and_motion_reuses_assembly_catalog()
 {
    let mut app = app();
    app.current_mut().view = crate::recent_projects::RecentView {
        mode: Mode::Sketch,
        workspace: "Edit".into(),
        ..Default::default()
    };
    app.restore_current_view();
    assert_eq!(app.mode, Mode::Model);
    assert_eq!(app.tab, 0);
    app.invoke(Command::NewSketch, &["xy"], Channel::Ui)
        .unwrap();
    app.current_mut().view = crate::recent_projects::RecentView {
        mode: Mode::Sketch,
        workspace: "Edit".into(),
        ..Default::default()
    };
    app.restore_current_view();
    assert_eq!(
        app.mode,
        Mode::Sketch,
        "a live open editor survives a project-tab return"
    );
    assert_eq!(app.mode.top_level(), Mode::Model);
    app.invoke(Command::FinishSketch, &[], Channel::Ui).unwrap();
    assert!(app.registry.tool("engraving").is_none());
    assert_eq!(
        app.registry.resolve("sketch").unwrap().action,
        Command::NewSketch
    );
    assert_eq!(
        app.registry.resolve("import").unwrap().action,
        Command::Import
    );
    let motion = app.registry.workspace(Mode::Motion);
    for tool in crate::assembly::AssemblyTool::ALL {
        assert_eq!(
            motion.tabs[0].tools.contains(&tool.id()),
            tool.motion_workspace()
        );
    }
    assert!(
        !app.registry
            .workspace(Mode::Model)
            .tabs
            .iter()
            .any(|tab| tab.title == "Import")
    );
}

#[test]
fn an_existing_sketch_opens_from_model_into_contextual_editing_without_source_or_history_changes() {
    let mut app = app();
    app.invoke(Command::NewSketch, &["xy"], Channel::Ui)
        .unwrap();
    app.invoke(
        Command::Draw(crate::sketch_editor::DrawTool::Rectangle),
        &["0", "0", "40", "20"],
        Channel::Ui,
    )
    .unwrap();
    let id = app.current().sketch.selected.unwrap();
    app.invoke(Command::FinishSketch, &[], Channel::Ui).unwrap();
    assert_eq!(app.mode, Mode::Model);
    let source = app.current().session.document().clone();
    let revision = app.current().session.revision();
    let ctx = context();
    for _ in 0..3 {
        frame(&ctx, &mut app, vec![]).drop_without_applying_deltas();
    }
    click(&ctx, &mut app, "Open selected sketch");
    assert_eq!(app.mode, Mode::Sketch);
    assert_eq!(app.mode.top_level(), Mode::Model);
    assert!(app.current().sketch.editing);
    assert_eq!(app.current().sketch.selected, Some(id));
    assert_eq!(app.current().session.document(), &source);
    assert_eq!(app.current().session.revision(), revision);
    let out = frame(&ctx, &mut app, vec![]);
    button(&out, "Finish sketch");
    button(&out, "Abort");
    out.drop_without_applying_deltas();
}

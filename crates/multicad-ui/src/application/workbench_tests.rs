//! CAD-162: starting a specialized tool keeps its originating workbench
//! through configuration, Apply and Cancel; intentional top-level
//! transitions such as Finish Sketch → Model stay intact.
use super::*;
struct Storage;
impl DocumentStorage for Storage {
    fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
        Ok(())
    }
    fn load(&mut self, _: &std::path::Path) -> Result<Document, String> {
        Err("unused".into())
    }
}
struct Backend;
impl crate::viewport::SceneBackend for Backend {
    fn request(&mut self, _: &Document, _: crate::viewport::SceneStamp) -> Result<(), String> {
        Ok(())
    }
    fn poll(&mut self) -> Option<crate::viewport::SceneReply> {
        None
    }
}
fn app() -> Application {
    let mut app = Application::new(Box::new(Storage)).unwrap();
    app.pending_files.clear();
    app.set_model_backends(std::sync::Arc::new(|| Box::new(Backend)));
    app
}
fn tab_index(app: &Application, mode: Mode, title: &str) -> usize {
    app.registry
        .workspace(mode)
        .tabs
        .iter()
        .position(|tab| tab.title == title)
        .unwrap()
}
fn action(app: &Application, id: &str) -> Command {
    app.registry.tool(id).unwrap().action
}
#[test]
fn placement_keeps_mechanical_while_motion_tools_route_to_motion_without_edits() {
    for id in [
        "place-component",
        "relationship",
        "joint-motion",
        "interference",
        "motion-range",
    ] {
        let mut app = app();
        app.mode = Mode::Model;
        let mechanical = tab_index(&app, Mode::Model, "Mechanical");
        app.tab = mechanical;
        let before = app.current().session.document().clone();
        app.invoke(action(&app, id), &[], Channel::Ui).unwrap();
        let destination = if id == "place-component" {
            (Mode::Model, mechanical)
        } else {
            (Mode::Motion, 0)
        };
        assert_eq!((app.mode, app.tab), destination, "{id}");
        app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
        assert_eq!(app.current().session.document(), &before, "{id}");
        assert_eq!(app.current().session.revision(), 0, "{id}");
        assert_eq!((app.mode, app.tab), destination, "{id}");
    }
}
/// Window/Door additionally need an applied wall; they share this path.
const SPECIALIZED: [(&str, &str); 11] = [
    ("Architecture", "wall"),
    ("Architecture", "level"),
    ("Architecture", "plant"),
    ("Architecture", "equipment"),
    ("Wood", "panel"),
    ("Wood", "drill"),
    ("Sheet metal", "sheet"),
    ("Weld", "tube"),
    ("Mechanical", "thread"),
    ("Mechanical", "gear"),
    ("FDM", "fdm-aid"),
];
#[test]
fn specialized_model_tools_keep_their_workbench_through_cancel_and_restart() {
    for (title, id) in SPECIALIZED {
        let mut app = app();
        app.mode = Mode::Model;
        let tab = tab_index(&app, Mode::Model, title);
        app.tab = tab;
        let command = action(&app, id);
        app.invoke(command, &[], Channel::Ui)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!((app.mode, app.tab), (Mode::Model, tab), "{id} on open");
        assert!(app.current().sketch.model_active(), "{id} form open");
        assert!(
            app.tool_active(command),
            "{id} highlighted while configured"
        );
        // Another tool on the same tab is not highlighted.
        for (_, other) in SPECIALIZED.iter().filter(|(t, o)| *t == title && *o != id) {
            assert!(!app.tool_active(action(&app, other)), "{other} vs {id}");
        }
        app.invoke(Command::Discard, &[], Channel::Ui)
            .unwrap_or_else(|e| panic!("{id} cancel: {e}"));
        assert!(!app.current().sketch.model_active(), "{id} cancelled");
        assert_eq!((app.mode, app.tab), (Mode::Model, tab), "{id} after Cancel");
        assert!(!app.tool_active(command), "no stale highlight after Cancel");
        // A repeated operation starts on the same workbench without navigation.
        app.invoke(command, &[], Channel::Ui).unwrap();
        assert_eq!((app.mode, app.tab), (Mode::Model, tab), "{id} repeated");
        // Apply keeps the workbench whether it commits or reports invalid input.
        let _ = app.invoke(Command::Apply, &[], Channel::Ui);
        assert_eq!((app.mode, app.tab), (Mode::Model, tab), "{id} after Apply");
    }
}
#[test]
fn search_or_command_entry_opens_the_tools_workbench_instead_of_common() {
    let mut app = app();
    app.mode = Mode::Model;
    app.tab = 0;
    app.invoke(action(&app, "panel"), &[], Channel::CommandLine)
        .unwrap();
    assert_eq!(app.tab, tab_index(&app, Mode::Model, "Wood"));
    app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
    // Sketch → Room offers Wall too; the Model workbench offering it opens.
    app.mode = Mode::Sketch;
    app.tab = tab_index(&app, Mode::Sketch, "Room");
    app.invoke(action(&app, "wall"), &[], Channel::Ui).unwrap();
    assert_eq!(
        (app.mode, app.tab),
        (Mode::Model, tab_index(&app, Mode::Model, "Architecture"))
    );
    app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
    // Common tools still open on Common.
    app.tab = tab_index(&app, Mode::Model, "Wood");
    app.invoke(action(&app, "box"), &[], Channel::Ui).unwrap();
    assert_eq!(app.tab, 0);
}
#[test]
fn drawing_annotate_and_drafting_tools_keep_their_workbench() {
    let mut app = app();
    app.invoke(Command::Mode(Mode::Drawings), &[], Channel::Ui)
        .unwrap();
    app.invoke(action(&app, "drawing-new"), &[], Channel::Ui)
        .unwrap();
    assert_eq!(app.tab, 0);
    for (title, id) in [("Annotate", "drawing-note"), ("Drafting", "draft-line")] {
        let tab = tab_index(&app, Mode::Drawings, title);
        app.tab = tab;
        let command = action(&app, id);
        app.invoke(command, &[], Channel::Ui)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!((app.mode, app.tab), (Mode::Drawings, tab), "{id} on open");
        assert!(app.tool_active(command), "{id} highlighted");
        app.invoke(action(&app, "page-discard"), &[], Channel::Ui)
            .unwrap();
        assert_eq!(app.tab, tab, "{id} after page Discard");
        assert!(!app.tool_active(command));
        app.invoke(command, &[], Channel::Ui).unwrap();
        // Text note and Sheet line first wait for placement on the paper.
        let drawings = &mut app.current_mut().drawings;
        if let Some(text) = drawings.pending_note_text() {
            *text = "Note".into();
            drawings.finish_placement().unwrap();
        } else {
            drawings.placement_click([60., 60.], false).unwrap();
            drawings.placement_click([90., 60.], false).unwrap();
        }
        assert!(app.current().drawings.dirty(), "{id} placed");
        app.invoke(Command::Apply, &[], Channel::Ui).unwrap();
        assert_eq!(
            (app.mode, app.tab),
            (Mode::Drawings, tab),
            "{id} after Apply"
        );
        app.invoke(command, &[], Channel::Ui).unwrap();
        app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
        assert_eq!(app.tab, tab, "{id} after Cancel");
    }
}
#[test]
fn finish_sketch_still_moves_to_model_common_and_drafts_still_guard_tools() {
    let mut app = app();
    app.mode = Mode::Sketch;
    app.invoke(Command::NewSketch, &["xy"], Channel::Ui)
        .unwrap();
    app.tab = tab_index(&app, Mode::Sketch, "Edit");
    app.invoke(Command::FinishSketch, &[], Channel::Ui).unwrap();
    assert_eq!((app.mode, app.tab), (Mode::Model, 0));
    // A pending parameter draft still blocks specialized tools.
    app.mode = Mode::Model;
    app.tab = tab_index(&app, Mode::Model, "Wood");
    let p = app.current_mut();
    p.editor.rows.last_mut().unwrap().name = "width".into();
    assert!(app.current().editor.dirty());
    assert!(app.invoke(action(&app, "panel"), &[], Channel::Ui).is_err());
    assert_eq!(app.tab, tab_index(&app, Mode::Model, "Wood"));
}

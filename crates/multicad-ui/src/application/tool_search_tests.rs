//! CAD-157: the tool finder discovers export workflows by output format,
//! shows where each result lives and keeps the pending-draft guards.
use super::*;
use multicad_core::exchange::{ExportError, PrintKey, PrintReply, PrintRequest};
use std::sync::Arc;
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
impl crate::print::PrintBackend for Backend {
    fn preview(
        &mut self,
        _: &Document,
        _: u64,
        _: &PrintRequest,
        _: Actor,
    ) -> Result<EntityId, ExportError> {
        Ok(EntityId::new())
    }
    fn write(
        &mut self,
        _: &Document,
        _: u64,
        _: &PrintRequest,
        _: &PrintKey,
        _: &std::path::Path,
        _: Actor,
    ) -> Result<EntityId, ExportError> {
        Ok(EntityId::new())
    }
    fn cancel(&mut self, _: EntityId) -> bool {
        true
    }
    fn poll(&mut self) -> Option<PrintReply> {
        None
    }
    fn pending(&self) -> bool {
        false
    }
}
fn app() -> Application {
    let mut app = Application::new(Box::new(Storage)).unwrap();
    app.pending_files.clear();
    app.set_print_backends(Arc::new(|| Box::new(Backend)))
        .unwrap();
    app
}
fn frame(ctx: &egui::Context, app: &mut Application, events: Vec<egui::Event>) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1360., 860.),
            )),
            events,
            ..Default::default()
        },
        |ui| app.show(ui),
    );
    output.textures_delta.clear();
    output
}
fn enter() -> Vec<egui::Event> {
    [true, false]
        .map(|pressed| egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        })
        .to_vec()
}
fn search(ctx: &egui::Context, app: &mut Application, query: &str) -> Vec<String> {
    frame(ctx, app, vec![]).drop_without_applying_deltas();
    app.focus_search = true;
    app.find.clear();
    frame(ctx, app, vec![]).drop_without_applying_deltas();
    frame(ctx, app, vec![egui::Event::Text(query.into())]).drop_without_applying_deltas();
    let output = frame(ctx, app, vec![]);
    let texts = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.text().to_string()),
            _ => None,
        })
        .collect();
    output.drop_without_applying_deltas();
    texts
}
#[test]
fn stl_and_3mf_find_print_packaging_and_enter_opens_export_print() {
    for query in ["STL", "3mf"] {
        let ctx = egui::Context::default();
        let mut app = app();
        app.mode = Mode::Model;
        let shown = search(&ctx, &mut app, query);
        assert!(!shown.iter().any(|t| t == "No matching tools"), "{query}");
        assert!(
            shown.iter().any(|t| t == "Preview package"),
            "{query}: {shown:?}"
        );
        assert!(
            shown.iter().any(|t| t == "Export › Print"),
            "{query} context"
        );
        frame(&ctx, &mut app, enter()).drop_without_applying_deltas();
        assert_eq!(app.mode, Mode::Export, "{query}");
        assert!(app.print_tab(), "{query} opens Export → Print");
        assert!(!app.message_error, "{}", app.message);
        assert!(!app.find_open);
    }
}
#[test]
fn format_search_keeps_pending_draft_guards() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.mode = Mode::Model;
    app.current_mut().editor.rows[0].name = "width".into();
    let shown = search(&ctx, &mut app, "stl");
    assert!(shown.iter().any(|t| t == "Preview package"));
    frame(&ctx, &mut app, enter()).drop_without_applying_deltas();
    assert_eq!(app.mode, Mode::Model, "disabled results do not run");
    assert_eq!(app.current().editor.rows[0].name, "width");
    assert_eq!(app.current().session.revision(), 0);
}
#[test]
fn every_named_format_routes_to_an_implemented_export_workflow_with_context() {
    let app = app();
    for format in [
        "STL", "3MF", "STEP", "stp", "OBJ", "PDF", "DXF", "GLB", "gltf",
    ] {
        let results = app.registry.search(format);
        assert!(
            results.iter().any(|tool| tool.id == "export-set"),
            "{format} reaches export sets"
        );
        for tool in &results {
            assert!(
                !matches!(tool.action, Command::Extension(_)) || tool.id.starts_with("import-"),
                "{format} offers placeholder {}",
                tool.id
            );
        }
        let note = crate::tool_search::note(format, "export-set").unwrap();
        assert!(
            note.starts_with(&format!("{}:", format.to_uppercase()))
                || matches!(format, "stp" | "gltf")
        );
    }
    assert_eq!(
        app.registry
            .location(Command::Export(crate::exchange::ExportTool::NewSet)),
        Some((Mode::Export, "Common"))
    );
    assert_eq!(
        app.registry
            .location(Command::Print(crate::print::PrintTool::Preview)),
        Some((Mode::Export, "Print"))
    );
    assert!(
        app.registry
            .search("export")
            .iter()
            .any(|tool| tool.id == "print-preview"),
        "searching export surfaces print packaging"
    );
    assert!(
        crate::tool_search::note("stl", "print-preview")
            .unwrap()
            .contains("STL per Part")
    );
    // The retired placeholders no longer appear as disabled STEP/OBJ, DXF/PDF.
    assert!(app.registry.tool("export-3d").is_none());
    assert!(app.registry.tool("export-2d").is_none());
}
#[test]
fn choosing_an_export_set_route_opens_export_common() {
    let mut app = app();
    app.mode = Mode::Model;
    let tool = *app
        .registry
        .search("step")
        .into_iter()
        .find(|tool| tool.id == "export-set")
        .unwrap();
    assert!(app.enabled(tool.action));
    app.execute(tool.action, &[], Channel::Ui);
    assert!(!app.message_error, "{}", app.message);
    assert_eq!((app.mode, app.tab), (Mode::Export, 0));
}

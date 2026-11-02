use super::*;
use crate::{architecture_navigation::Category, modeling::ModelingTool};
struct Memory;
impl DocumentStorage for Memory {
    fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
        Err("read-only test".into())
    }
    fn load(&mut self, _: &std::path::Path) -> Result<Document, String> {
        Err("read-only test".into())
    }
}
fn app() -> Application {
    let mut app = Application::new(Box::new(Memory)).unwrap();
    app.mode = Mode::Model;
    app
}
fn context(density: f32) -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::configure(&ctx);
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("architecture-test-density"), density));
    ctx
}
fn paint(
    app: &mut Application,
    ctx: &egui::Context,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let density = ctx
        .data(|data| data.get_temp::<f32>(egui::Id::new("architecture-test-density")))
        .unwrap_or(1.);
    let mut input = egui::RawInput {
        focused: true,
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        events,
        ..Default::default()
    };
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .native_pixels_per_point = Some(density);
    let mut output = ctx.run_ui(input, |ui| app.show(ui));
    assert_eq!(output.pixels_per_point, density);
    output.textures_delta.clear();
    output
}
fn bounds(out: &egui::FullOutput, label: &str) -> egui::Rect {
    let tree = out
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("accessible native controls");
    tree.nodes
        .iter()
        .filter_map(|(_, node)| {
            (node.label() == Some(label))
                .then(|| node.bounds())
                .flatten()
        })
        .map(|r| {
            egui::Rect::from_min_max(
                egui::pos2(r.x0 as f32, r.y0 as f32),
                egui::pos2(r.x1 as f32, r.y1 as f32),
            )
        })
        .max_by(|a, b| a.height().total_cmp(&b.height()))
        .unwrap_or_else(|| panic!("Missing accessible control {label}"))
}
fn click(
    app: &mut Application,
    ctx: &egui::Context,
    size: egui::Vec2,
    pos: egui::Pos2,
) -> egui::FullOutput {
    for pressed in [true, false] {
        paint(
            app,
            ctx,
            size,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )
        .drop_without_applying_deltas();
    }
    paint(app, ctx, size, vec![])
}
#[test]
fn actual_category_clicks_lift_parent_controls_and_back_close_restore_one_row_without_placement() {
    for (size, density) in [
        (egui::vec2(1100., 700.), 1.),
        (egui::vec2(1360., 860.), 1.),
        (egui::vec2(1360., 860.), 2.),
    ] {
        let ctx = context(density);
        let mut app = app();
        let source = app.current().session.document().clone();
        paint(&mut app, &ctx, size, vec![]).drop_without_applying_deltas();
        let out = paint(&mut app, &ctx, size, vec![]);
        let architecture = bounds(&out, "Architecture").center();
        let root = click(&mut app, &ctx, size, architecture);
        let wall = bounds(&root, "Wall");
        let electrical = bounds(&root, "Electrical");
        assert!(wall.height() > 70.);
        bounds(&root, "Door");
        bounds(&root, "Window");
        bounds(&root, "Cutout");
        bounds(&root, "Levels");
        bounds(&root, "Stairs");
        let mut nested = click(&mut app, &ctx, size, electrical.center());
        assert_eq!(
            app.active_architecture_category(),
            Some(Category::Electrical)
        );
        let compact_wall = bounds(&nested, "Wall");
        assert!(compact_wall.height() <= crate::architecture_navigation::PARENT_CUE_HEIGHT);
        assert!(compact_wall.center().y < wall.center().y);
        for label in ["Switch", "Socket", "Junction box", "Back"] {
            let r = bounds(&nested, label);
            assert!(r.min.x >= 0. && r.max.x <= size.x);
            assert!(r.min.y >= compact_wall.max.y);
        }
        for category in [
            Category::Water,
            Category::Heating,
            Category::Furniture,
            Category::Landscaping,
            Category::Electrical,
        ] {
            let target = bounds(&nested, category.label()).center();
            nested = click(&mut app, &ctx, size, target);
            assert_eq!(app.active_architecture_category(), Some(category));
            assert_eq!(app.current().session.document(), &source);
            assert_eq!(app.current().session.revision(), 0);
            assert!(!app.current().sketch.model_interaction_active());
            let r = bounds(&nested, "Close");
            assert!(r.max.x <= size.x && r.height() <= 24.);
        }
        let back = bounds(&nested, "Back").center();
        let restored = click(&mut app, &ctx, size, back);
        assert_eq!(app.active_architecture_category(), None);
        assert!((bounds(&restored, "Wall").center().y - wall.center().y).abs() < 1.);
        let again = bounds(&restored, "Electrical").center();
        let nested = click(&mut app, &ctx, size, again);
        let close = bounds(&nested, "Close").center();
        let root = click(&mut app, &ctx, size, close);
        assert_eq!(app.active_architecture_category(), None);
        assert!((bounds(&root, "Wall").center().y - wall.center().y).abs() < 1.);
        assert_eq!(app.current().session.document(), &source);
        assert_eq!(app.current().session.revision(), 0);
    }
}
#[test]
fn category_context_is_per_project_and_saved_subpaths_restore_without_running_tools() {
    let mut app = app();
    let first = app.current().session.document().clone();
    app.invoke(
        Command::ArchitectureCategory(Category::Furniture),
        &[],
        Channel::CommandLine,
    )
    .unwrap();
    app.remember_current_view();
    assert_eq!(app.current().view.workspace, "Architecture / Furniture");
    app.projects
        .push(Project::new(Document::new("Second project").unwrap()).unwrap());
    app.activate_project(1);
    assert_eq!(app.current().architecture_category, None);
    app.mode = Mode::Model;
    app.invoke(
        Command::ArchitectureCategory(Category::Water),
        &[],
        Channel::Ui,
    )
    .unwrap();
    app.activate_project(0);
    assert_eq!(
        app.active_architecture_category(),
        Some(Category::Furniture)
    );
    assert_eq!(app.current().session.document(), &first);
    assert!(!app.current().sketch.model_interaction_active());
    for category in Category::ALL {
        app.current_mut().architecture_category = None;
        app.current_mut().view = crate::recent_projects::RecentView {
            mode: Mode::Model,
            workspace: category.workspace().into(),
        };
        app.restore_current_view();
        assert_eq!(app.active_architecture_category(), Some(category));
        assert_eq!(app.current().session.document(), &first);
    }
    app.current_mut().view.workspace = "Architecture / future category".into();
    app.restore_current_view();
    assert_eq!(app.active_architecture_category(), None);
    assert_eq!(
        app.registry.workspace(app.mode).tabs[app.tab].title,
        "Architecture"
    );
    assert!(
        app.invoke(
            Command::ArchitectureCategory(Category::Electrical),
            &["unexpected"],
            Channel::CommandLine
        )
        .is_err()
    );
    assert_eq!(app.active_architecture_category(), None);
    assert_eq!(app.current().session.document(), &first);
}
#[test]
fn preset_commands_open_the_chosen_native_form_and_navigation_preserves_it_until_cancel() {
    use crate::modeling::native_fixture::NativeBackend;
    use multicad_core::equipment::Kind;
    let mut app = app();
    let ctx = context(1.);
    let size = egui::vec2(1360., 860.);
    let before = app.current().session.document().clone();
    app.set_model_backends(std::sync::Arc::new(|| Box::new(NativeBackend::new())));
    let preset = app.registry.parse_line("junction-box").unwrap().0;
    assert_eq!(preset, Command::EquipmentPreset(Kind::JunctionBox));
    assert!(app.enabled(preset));
    app.invoke(preset, &[], Channel::CommandLine).unwrap();
    assert!(app.current().sketch.model_active());
    assert!(app.tool_active(preset));
    assert_eq!(
        app.active_architecture_category(),
        Some(Category::Electrical)
    );
    paint(&mut app, &ctx, size, vec![]).drop_without_applying_deltas();
    let out = paint(&mut app, &ctx, size, vec![]);
    assert!(
        out.platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .any(|(_, node)| node.value() == Some(Kind::JunctionBox.preset().name)),
        "The actual inspector name must use the chosen family"
    );
    let back = bounds(&out, "Back").center();
    let root = click(&mut app, &ctx, size, back);
    assert!(app.current().sketch.model_active());
    assert!(app.tool_active(preset));
    assert_eq!(app.current().session.document(), &before);
    let water = bounds(&root, "Water").center();
    click(&mut app, &ctx, size, water).drop_without_applying_deltas();
    assert!(app.current().sketch.model_active());
    assert_eq!(app.active_architecture_category(), Some(Category::Water));
    assert_eq!(app.current().session.revision(), 0);
    app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
    assert!(!app.current().sketch.model_interaction_active());
    assert!(!app.tool_active(preset));
    assert_eq!(app.current().session.document(), &before);
    let stairs = app.registry.parse_line("stairs").unwrap().0;
    assert_eq!(stairs, Command::Modeling(ModelingTool::Stair));
    app.invoke(stairs, &[], Channel::CommandLine).unwrap();
    assert!(app.current().sketch.model_active());
    assert_eq!(app.active_architecture_category(), None);
    assert_eq!(
        app.registry.workspace(app.mode).tabs[app.tab].title,
        "Architecture"
    );
    assert_eq!(app.current().session.document(), &before);
    app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
    assert_eq!(app.current().session.document(), &before);
}

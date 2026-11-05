use super::*;
use crate::{architecture_navigation::Category, modeling::ModelingTool};
#[test]
fn saved_architecture_views_restore_known_world_and_missing_levels_without_changing_source() {
    use crate::{architecture_view::Recall, recent_projects::RecentView};
    let (session, levels, _, _) = crate::architecture_view::tests::building();
    let source = session.document().clone();
    for level in [Some(levels[1]), None, Some(EntityId::new())] {
        let mut app = app();
        *app.current_mut() = Project::new(source.clone()).unwrap();
        app.current_mut()
            .sketch
            .restore_architecture_view(Recall { plan: true, level });
        app.tab = app
            .registry
            .workspace(Mode::Model)
            .tabs
            .iter()
            .position(|tab| tab.title == "Architecture")
            .unwrap();
        app.current_mut().architecture_category = Some(Category::Electrical);
        app.remember_current_view();
        let saved = app.current().view.clone();
        assert_eq!(saved.architecture, Some(Recall { plan: true, level }));
        *app.current_mut() = Project::new(source.clone()).unwrap();
        app.current_mut().view = saved;
        app.restore_current_view();
        let ctx = context(1.);
        paint(&mut app, &ctx, egui::vec2(1360., 860.), vec![]);
        assert_eq!(
            app.current().sketch.architecture_recall(),
            Recall { plan: true, level }
        );
        assert_eq!(
            app.active_architecture_category(),
            Some(Category::Electrical)
        );
        assert_eq!(app.current().session.document(), &source);
        assert_eq!(app.current().session.revision(), 0);
    }
    let mut app = app();
    *app.current_mut() = Project::new(source.clone()).unwrap();
    app.current_mut().view = RecentView {
        mode: Mode::Model,
        workspace: "Architecture".into(),
        architecture: None,
    };
    app.restore_current_view();
    paint(&mut app, &context(1.), egui::vec2(1360., 860.), vec![]);
    assert_eq!(
        app.current().sketch.architecture_recall(),
        Recall {
            plan: false,
            level: Some(levels[0])
        }
    );
    assert_eq!(app.current().session.document(), &source);
}

#[test]
fn architecture_plan_and_level_buttons_are_source_neutral_at_compact_and_native_scales() {
    for (size, density) in [
        (egui::vec2(1100., 700.), 1.),
        (egui::vec2(1360., 860.), 1.),
        (egui::vec2(1360., 860.), 2.),
    ] {
        let (session, levels, bodies, _) = crate::architecture_view::tests::building();
        let mut app = app();
        *app.current_mut() = Project::new(session.document().clone()).unwrap();
        app.tab = app
            .registry
            .workspace(Mode::Model)
            .tabs
            .iter()
            .position(|tab| tab.title == "Architecture")
            .unwrap();
        let ctx = context(density);
        let before = app.current().session.document().clone();
        app.set_scene_backends(std::sync::Arc::new(|| {
            Box::new(crate::modeling::native_fixture::NativeBackend::new())
        }));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !app.current().scene.is_current(
            app.current().session.document().id(),
            app.current().session.revision(),
        ) {
            paint(&mut app, &ctx, size, vec![]).drop_without_applying_deltas();
            assert!(
                std::time::Instant::now() < deadline,
                "native level scene timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        for _ in 0..3 {
            paint(&mut app, &ctx, size, vec![]);
        }
        let mut out = paint(&mut app, &ctx, size, vec![]);
        for _ in 0..3 {
            let position = bounds(&out, "2D plan").center();
            out = click(&mut app, &ctx, size, position);
            assert!(app.current().sketch.architecture_recall().plan);
            let position = bounds(&out, "Upper · 3000 mm").center();
            out = click(&mut app, &ctx, size, position);
            assert_eq!(
                app.current().sketch.architecture_recall().level,
                Some(levels[1])
            );
            assert_eq!(app.current().sketch.camera.target[2], 3000.);
            assert!(!app.current().sketch.camera.perspective);
            out = click(
                &mut app,
                &ctx,
                size,
                bounds(&out, "Floor slab · Upper floor").center(),
            );
            assert_eq!(app.current().sketch.selected_solid(), Some(bodies[1]));
            let position = bounds(&out, "3D model").center();
            out = click(&mut app, &ctx, size, position);
            assert!(!app.current().sketch.architecture_recall().plan);
            assert_eq!(
                app.current().sketch.architecture_recall().level,
                Some(levels[1])
            );
            assert_eq!(app.current().sketch.selected_solid(), Some(bodies[1]));
        }
        assert_eq!(app.current().session.document(), &before);
        assert_eq!(app.current().session.revision(), 0);
    }
}
#[test]
fn planting_entry_from_upper_plan_explicitly_selects_world_without_reassigning_source() {
    for (size, density, entry) in [
        (egui::vec2(1100., 700.), 1., 0),
        (egui::vec2(1360., 860.), 2., 0),
        (egui::vec2(1100., 700.), 1., 1),
        (egui::vec2(1360., 860.), 2., 1),
        (egui::vec2(1100., 700.), 1., 2),
    ] {
        let (session, levels, _, _) = crate::architecture_view::tests::building();
        let source = session.document().clone();
        let mut app = app();
        *app.current_mut() = Project::new(source.clone()).unwrap();
        app.set_model_backends(std::sync::Arc::new(|| {
            Box::new(crate::modeling::native_fixture::NativeBackend::new())
        }));
        if entry == 0 {
            app.tab = app
                .registry
                .workspace(Mode::Model)
                .tabs
                .iter()
                .position(|tab| tab.title == "Architecture")
                .unwrap();
        }
        if entry != 2 {
            app.current_mut()
                .sketch
                .restore_architecture_view(crate::architecture_view::Recall {
                    plan: true,
                    level: Some(levels[1]),
                });
        }
        let ctx = context(density);
        if entry == 0 {
            let out = paint(&mut app, &ctx, size, vec![]);
            let out = click(&mut app, &ctx, size, bounds(&out, "Landscaping").center());
            click(&mut app, &ctx, size, bounds(&out, "Planting").center())
                .drop_without_applying_deltas();
        } else {
            paint(&mut app, &ctx, size, vec![]).drop_without_applying_deltas();
            assert_eq!(
                app.registry.workspace(Mode::Model).tabs[app.tab].title,
                "Common"
            );
            app.invoke(Command::Modeling(ModelingTool::Plant), &[], Channel::Ui)
                .unwrap();
            paint(&mut app, &ctx, size, vec![]).drop_without_applying_deltas();
        }
        assert_eq!(
            app.current().sketch.architecture_recall(),
            crate::architecture_view::Recall {
                plan: entry != 2,
                level: None
            }
        );
        if entry != 2 {
            assert_eq!(app.current().sketch.camera.target[2], 0.);
        }
        assert!(app.current().sketch.model_active());
        app.current_mut().sketch.cancel_model();
        assert_eq!(app.current().session.document(), &source);
    }
}
struct Memory;
#[test]
fn pending_model_keeps_its_level_when_the_user_views_another_floor() {
    let (session, levels, _, _) = crate::architecture_view::tests::building();
    let source = session.document().clone();
    let mut app = app();
    *app.current_mut() = Project::new(source.clone()).unwrap();
    app.set_model_backends(std::sync::Arc::new(|| {
        Box::new(crate::modeling::native_fixture::NativeBackend::new())
    }));
    app.tab = app
        .registry
        .workspace(Mode::Model)
        .tabs
        .iter()
        .position(|tab| tab.title == "Architecture")
        .unwrap();
    app.current_mut()
        .sketch
        .restore_architecture_view(crate::architecture_view::Recall {
            plan: true,
            level: Some(levels[0]),
        });
    let ctx = context(1.);
    let size = egui::vec2(1100., 700.);
    paint(&mut app, &ctx, size, vec![]).drop_without_applying_deltas();
    app.invoke(Command::Modeling(ModelingTool::Wall), &[], Channel::Ui)
        .unwrap();
    let out = paint(&mut app, &ctx, size, vec![]);
    click(
        &mut app,
        &ctx,
        size,
        bounds(&out, "Upper · 3000 mm").center(),
    )
    .drop_without_applying_deltas();
    assert_eq!(
        app.current().sketch.architecture_recall().level,
        Some(levels[1])
    );
    assert_eq!(app.current().sketch.camera.target[2], 3000.);
    assert!(app.current().sketch.model_active());
    assert_eq!(app.current().session.document(), &source);
    app.invoke(Command::Preview, &[], Channel::Ui).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !app
        .current()
        .sketch
        .model_can_apply(&app.current().session, &app.current().resources)
    {
        paint(&mut app, &ctx, size, vec![]).drop_without_applying_deltas();
        assert!(
            std::time::Instant::now() < deadline,
            "native wall preview timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(app.current().session.document(), &source);
    app.invoke(Command::Apply, &[], Channel::Ui).unwrap();
    let wall = multicad_core::model::walls(app.current().session.document())
        .next()
        .unwrap();
    assert_eq!(
        multicad_core::model::read_wall(app.current().session.document(), wall.id)
            .unwrap()
            .level,
        Some(levels[0])
    );
    app.invoke(Command::Undo, &[], Channel::Ui).unwrap();
    assert_eq!(app.current().session.document(), &source);
}
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
            ..Default::default()
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

#[test]
fn saved_feature_edit_does_not_highlight_the_previous_creation_tool() {
    use crate::modeling::native_fixture::NativeBackend;
    let bytes = include_bytes!("../../../../examples/apartment.tcd");
    let project = multicad_core::project_file::read(
        &mut &bytes[..],
        multicad_core::project_file::FileLimits::default(),
        &multicad_core::project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let before = project.document;
    let feature = multicad_core::model::walls(&before).next().unwrap().id;
    let mut app = app();
    *app.current_mut() = Project::new(before.clone()).unwrap();
    app.set_model_backends(std::sync::Arc::new(|| Box::new(NativeBackend::new())));
    let ctx = context(1.);
    let size = egui::vec2(1100., 700.);
    let window = Command::Modeling(ModelingTool::Window);
    let wall = Command::Modeling(ModelingTool::Wall);
    app.invoke(window, &[], Channel::Ui).unwrap();
    assert!(app.tool_active(window));
    app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
    let project = app.current_mut();
    project
        .sketch
        .begin_feature_edit(
            &project.session,
            &project.resources,
            &project.scene,
            feature,
            Channel::Ui,
        )
        .unwrap();
    assert_eq!(app.current().sketch.model_label(), "Feature edit");
    for _ in 0..3 {
        paint(&mut app, &ctx, size, vec![]).drop_without_applying_deltas();
    }
    let output = paint(&mut app, &ctx, size, vec![]);
    let nodes = &output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes;
    for label in ["Window", "Wall"] {
        let selected = nodes
            .iter()
            .find_map(|(_, node)| {
                (node.label() == Some(label))
                    .then(|| node.toggled())
                    .flatten()
            })
            .unwrap();
        assert_eq!(selected, egui::accesskit::Toggled::False, "{label}");
    }
    output.drop_without_applying_deltas();
    assert_eq!(app.current().session.document(), &before);
    app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
    assert_eq!(app.current().session.document(), &before);
    assert_eq!(app.current().session.revision(), 0);
    app.invoke(wall, &[], Channel::Ui).unwrap();
    assert!(app.tool_active(wall));
    assert!(!app.tool_active(window));
}

#[test]
fn library_browser_highlight_does_not_revive_a_cancelled_equipment_preset() {
    use crate::modeling::native_fixture::NativeBackend;
    use multicad_core::equipment::Kind;
    let mut app = app();
    app.set_model_backends(std::sync::Arc::new(|| Box::new(NativeBackend::new())));
    let ctx = context(1.);
    let size = egui::vec2(1360., 860.);
    let before = app.current().session.document().clone();
    let dishwasher = Command::EquipmentPreset(Kind::Dishwasher);
    let library = Command::Extension("library");
    let assert_selected = |out: &egui::FullOutput, label: &str, selected: bool| {
        let nodes = &out.platform_output.accesskit_update.as_ref().unwrap().nodes;
        let actual = nodes
            .iter()
            .find_map(|(_, node)| {
                (node.label() == Some(label))
                    .then(|| node.toggled())
                    .flatten()
            })
            .unwrap_or_else(|| panic!("Missing selectable ribbon control {label}"));
        assert_eq!(actual, egui::accesskit::Toggled::from(selected), "{label}");
    };
    app.invoke(dishwasher, &[], Channel::Ui).unwrap();
    for _ in 0..3 {
        paint(&mut app, &ctx, size, vec![]).drop_without_applying_deltas();
    }
    let out = paint(&mut app, &ctx, size, vec![]);
    assert_selected(&out, "Dishwasher", true);
    out.drop_without_applying_deltas();
    app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
    app.invoke(library, &[], Channel::Ui).unwrap();
    for _ in 0..2 {
        paint(&mut app, &ctx, size, vec![]).drop_without_applying_deltas();
    }
    let out = paint(&mut app, &ctx, size, vec![]);
    assert_selected(&out, "Dishwasher", false);
    assert_selected(&out, "Symbol library", true);
    out.drop_without_applying_deltas();
    assert_eq!(app.current().session.document(), &before);
    assert_eq!(app.current().session.revision(), 0);
    app.invoke(
        Command::ArchitectureCategory(Category::Water),
        &[],
        Channel::Ui,
    )
    .unwrap();
    let out = paint(&mut app, &ctx, size, vec![]);
    assert_selected(&out, "Symbol library", true);
    out.drop_without_applying_deltas();
    app.invoke(library, &[], Channel::Ui).unwrap();
    let out = paint(&mut app, &ctx, size, vec![]);
    assert_selected(&out, "Symbol library", false);
    out.drop_without_applying_deltas();
    assert!(!app.tool_active(dishwasher));
    assert!(!app.current().sketch.symbol_active());
    app.invoke(dishwasher, &[], Channel::Ui).unwrap();
    let out = paint(&mut app, &ctx, size, vec![]);
    assert_selected(&out, "Dishwasher", true);
    assert_selected(&out, "Symbol library", false);
    out.drop_without_applying_deltas();
    app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
    assert_eq!(app.current().session.document(), &before);
    assert_eq!(app.current().session.revision(), 0);
}

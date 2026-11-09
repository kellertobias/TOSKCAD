use super::*;
#[path = "../../../../multicad-app/examples/parameter_workflow_review.rs"]
#[allow(dead_code)]
mod workflow;
use multicad_model::Compiler;
mod materials;
struct Storage;
impl DocumentStorage for Storage {
    fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
        Err("No file writes in Project shell tests".into())
    }
    fn load(&mut self, _: &std::path::Path) -> Result<Document, String> {
        Err("unused".into())
    }
}
fn app() -> (Application, EntityId) {
    let fixture = workflow::seed().unwrap();
    let mut app =
        Application::with_document(Box::new(Storage), fixture.session.document().clone()).unwrap();
    app.pending_files.clear();
    app.tab = PARAMETERS_TAB;
    (app, fixture.output.body)
}
fn frame(
    ctx: &egui::Context,
    app: &mut Application,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            focused: true,
            events,
            ..Default::default()
        },
        |ui| app.show(ui),
    )
}
fn button(output: &egui::FullOutput, label: &str) -> (egui::Rect, bool) {
    output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find_map(|(_, node)| {
            let b = node.bounds()?;
            (node.label() == Some(label) && b.y1 - b.y0 >= 30.).then_some((
                egui::Rect::from_min_max(
                    egui::pos2(b.x0 as f32, b.y0 as f32),
                    egui::pos2(b.x1 as f32, b.y1 as f32),
                ),
                !node.is_disabled(),
            ))
        })
        .unwrap_or_else(|| panic!("Missing Project action {label}"))
}
fn click(ctx: &egui::Context, app: &mut Application, size: egui::Vec2, label: &str) {
    let out = frame(ctx, app, size, vec![]);
    let (rect, enabled) = button(&out, label);
    assert!(enabled, "{label} disabled");
    out.drop_without_applying_deltas();
    click_center(ctx, app, size, rect.center());
}
fn click_center(ctx: &egui::Context, app: &mut Application, size: egui::Vec2, pos: egui::Pos2) {
    for pressed in [true, false] {
        frame(
            ctx,
            app,
            size,
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
#[test]
fn project_and_model_share_full_width_workspace_tab_geometry() {
    for size in [egui::vec2(1024., 600.), egui::vec2(1360., 860.)] {
        let (mut app, _) = app();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::configure(&ctx);
        let mut bars = Vec::new();
        for (mode, label) in [(Mode::Parameters, "Parameters"), (Mode::Model, "Common")] {
            app.mode = mode;
            let output = frame(&ctx, &mut app, size, vec![]);
            let (tab, enabled) = button(&output, label);
            assert!(enabled);
            let bar = output
                .shapes
                .iter()
                .find_map(|shape| {
                    let egui::Shape::Rect(rect) = &shape.shape else {
                        return None;
                    };
                    (rect.fill == mode.color() && rect.rect.width() >= size.x - 1.)
                        .then_some(rect.rect)
                })
                .expect("full-width mode accent");
            assert!(bar.contains_rect(tab));
            assert_eq!(tab.height(), 30.);
            bars.push(bar);
            output.drop_without_applying_deltas();
        }
        assert_eq!(bars[0], bars[1]);
    }
}
#[test]
fn model_loading_is_visible_without_inspector_and_reuses_the_current_receipt() {
    use crate::viewport::{Scene, SceneBackend, SceneReply, SceneStamp};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct State {
        requests: Vec<SceneStamp>,
        reply: Option<SceneReply>,
    }
    struct Backend(Arc<Mutex<State>>);
    impl SceneBackend for Backend {
        fn request(&mut self, _: &Document, stamp: SceneStamp) -> Result<(), String> {
            self.0.lock().unwrap().requests.push(stamp);
            Ok(())
        }
        fn poll(&mut self) -> Option<SceneReply> {
            self.0.lock().unwrap().reply.take()
        }
    }
    let mut app =
        Application::with_document(Box::new(Storage), Document::new("Delayed model").unwrap())
            .unwrap();
    let state = Arc::new(Mutex::new(State::default()));
    app.current_mut()
        .scene
        .set_backend(Box::new(Backend(state.clone())));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::configure(&ctx);
    let size = egui::vec2(1024., 600.);
    for label in ["Settings", "Inspector"] {
        let out = frame(&ctx, &mut app, size, vec![]);
        let bounds = out
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find_map(|(_, node)| {
                (node.label() == Some(label))
                    .then(|| node.bounds())
                    .flatten()
            })
            .unwrap();
        let pos = egui::pos2(
            ((bounds.x0 + bounds.x1) * 0.5) as f32,
            ((bounds.y0 + bounds.y1) * 0.5) as f32,
        );
        out.drop_without_applying_deltas();
        click_center(&ctx, &mut app, size, pos);
    }
    let visible = |app: &mut Application, expected: &str| {
        let out = frame(&ctx, app, size, vec![]);
        let found = out.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == expected)
        });
        out.drop_without_applying_deltas();
        found
    };
    click(&ctx, &mut app, size, "Start Modeling");
    assert_eq!(app.mode, Mode::Model);
    assert!(app.current().scene.pending());
    assert!(visible(&mut app, "Loading model…"));
    let first = state.lock().unwrap().requests[0];
    state.lock().unwrap().reply = Some(SceneReply {
        stamp: first,
        result: Err("Restore the referenced project before rebuilding.".into()),
    });
    assert!(visible(&mut app, "Model could not load"));
    assert!(!visible(&mut app, "Loading model…"));
    click(&ctx, &mut app, size, "Retry model load");
    assert!(visible(&mut app, "Loading model…"));

    let second = state.lock().unwrap().requests[1];
    let current = Arc::new(Scene::new(second, vec![], vec![]).unwrap());
    state.lock().unwrap().reply = Some(SceneReply {
        stamp: second,
        result: Ok(current.clone()),
    });
    assert!(!visible(&mut app, "Loading model…"));
    click(&ctx, &mut app, size, "Project");
    assert_eq!(app.mode, Mode::Parameters);
    click(&ctx, &mut app, size, "Start Modeling");
    assert!(!visible(&mut app, "Loading model…"));
    assert!(Arc::ptr_eq(
        app.current().scene.visible().unwrap(),
        &current
    ));
    assert_eq!(state.lock().unwrap().requests.len(), 2);

    let project = app.current_mut();
    let primitive = multicad_core::model::PrimitiveRequest::new(
        "New geometry",
        multicad_core::model::PrimitiveParameters::Box {
            size: [10.; 3].map(multicad_core::model::length),
            frame: Default::default(),
        },
    );
    let prepared = multicad_core::model::prepare_primitive(
        &project.session,
        project.session.revision(),
        Actor::local(Channel::Ui),
        &primitive,
    )
    .unwrap();
    project.session.commit(prepared.prepared).unwrap();
    assert!(visible(&mut app, "Updating model…"));
    let changed = state.lock().unwrap().requests[2];
    assert_ne!(changed.revision, second.revision);
    state.lock().unwrap().reply = Some(SceneReply {
        stamp: second,
        result: Ok(current),
    });
    assert!(visible(&mut app, "Updating model…"));
    let latest = Arc::new(Scene::new(changed, vec![], vec![]).unwrap());
    state.lock().unwrap().reply = Some(SceneReply {
        stamp: changed,
        result: Ok(latest.clone()),
    });
    assert!(!visible(&mut app, "Updating model…"));
    assert!(Arc::ptr_eq(app.current().scene.visible().unwrap(), &latest));
}
#[test]
fn general_commits_details_without_an_apply_button_and_starts_modeling() {
    let (mut app, _) = app();
    app.tab = 0;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::configure(&ctx);
    let size = egui::vec2(1024., 670.);
    let out = frame(&ctx, &mut app, size, vec![]);
    let metadata = egui::PanelState::load(&ctx, egui::Id::new("project-details"))
        .unwrap()
        .outer_rect;
    assert!(metadata.left().abs() < 1.);
    let (start, enabled) = button(&out, "Start Modeling");
    assert!(enabled && metadata.contains_rect(start));
    for label in ["Project Contents", "No saved poses", "No drawings yet."] {
        assert!(
            out.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label)
            }),
            "{label}"
        );
    }
    let access = out.platform_output.accesskit_update.as_ref().unwrap();
    assert!(
        !access
            .nodes
            .iter()
            .any(|(_, node)| matches!(node.label(), Some("Apply and Rebuild" | "Discard Edit")))
    );
    out.drop_without_applying_deltas();
    app.current_mut().editor.info.name = "Workshop cabinet".into();
    frame(&ctx, &mut app, size, vec![]).drop_without_applying_deltas();
    assert!(!app.current().editor.info_dirty());
    assert_eq!(app.current().session.document().name(), "Workshop cabinet");
    assert!(app.current().session.can_undo());
    click(&ctx, &mut app, size, "Start Modeling");
    assert_eq!(app.mode, Mode::Model);
    assert_eq!(app.current().session.document().name(), "Workshop cabinet");
}
#[test]
fn parameters_put_actions_in_a_ribbon_and_keep_the_table_below_it() {
    for (size, density) in [
        (egui::vec2(1024., 600.), 1.),
        (egui::vec2(1024., 600.), 2.),
        (egui::vec2(1360., 860.), 1.),
        (egui::vec2(1360., 860.), 2.),
    ] {
        let (mut app, _) = app();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        ctx.set_pixels_per_point(density);
        crate::theme::configure(&ctx);
        for _ in 0..3 {
            frame(&ctx, &mut app, size, vec![]).drop_without_applying_deltas();
        }
        let out = frame(&ctx, &mut app, size, vec![]);
        let header = egui::PanelState::load(&ctx, egui::Id::new("project-tabs"))
            .unwrap()
            .outer_rect;
        let ribbon = egui::PanelState::load(&ctx, egui::Id::new("project-parameter-ribbon"))
            .unwrap()
            .outer_rect;
        assert!(egui::PanelState::load(&ctx, egui::Id::new("project-actions")).is_none());
        assert!(egui::PanelState::load(&ctx, egui::Id::new("parameters-metadata")).is_none());
        assert!((ribbon.top() - header.bottom()).abs() <= 1.);
        let mut previous = 0.;
        for label in [
            "Apply and Rebuild",
            "Discard Edit",
            "Preview",
            "Undo",
            "Redo",
        ] {
            let (rect, enabled) = button(&out, label);
            assert!(!enabled, "clean baseline has no {label} action");
            assert!(
                ribbon.contains_rect(rect),
                "{label}: {rect:?} outside {ribbon:?}"
            );
            assert!(
                rect.left() >= previous,
                "{label} follows the preceding tool"
            );
            previous = rect.right();
        }
        assert!(out.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text)
                if text.galley.text() == "Parameters" && text.pos.y >= ribbon.bottom())
        }));
        fn search_x(shape: &egui::Shape) -> Option<f32> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == "Search parameters…" => {
                    Some(text.pos.x)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(search_x),
                _ => None,
            }
        }
        let x = out
            .shapes
            .iter()
            .find_map(|shape| search_x(&shape.shape))
            .expect("search text is visible");
        assert!(x >= size.x - 260., "search text at {x} in {size:?}");
        out.drop_without_applying_deltas();
    }
}
#[test]
fn project_buttons_keep_preview_apply_discard_and_history_transactions_on_the_real_panel() {
    let (mut app, body) = app();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::configure(&ctx);
    let size = egui::vec2(1024., 600.);
    for _ in 0..3 {
        frame(&ctx, &mut app, size, vec![]).drop_without_applying_deltas();
    }
    let before = app.current().session.document().clone();
    let preview = app.current().editor.info.preview.clone();
    app.current_mut()
        .editor
        .rows
        .iter_mut()
        .find(|row| row.name == "width")
        .unwrap()
        .expression = "160".into();
    app.current_mut().editor.info.name = "Project action review".into();
    for _ in 0..2 {
        frame(&ctx, &mut app, size, vec![]).drop_without_applying_deltas();
    }
    let out = frame(&ctx, &mut app, size, vec![]);
    assert!(button(&out, "Apply and Rebuild").1);
    assert!(button(&out, "Discard Edit").1);
    assert!(button(&out, "Preview").1);
    assert!(!button(&out, "Undo").1);
    out.drop_without_applying_deltas();
    click(&ctx, &mut app, size, "Preview");
    assert_eq!(app.current().session.document(), &before);
    assert_eq!(app.current().editor.info.preview, preview);
    click(&ctx, &mut app, size, "Apply and Rebuild");
    let changed = app.current().session.document().clone();
    assert_ne!(changed, before);
    assert_eq!(app.current().session.revision(), 1);
    let model = Compiler::default().compile(&changed, 1).unwrap();
    assert!((model.solid(body).unwrap().volume_mm3().unwrap() - 160. * 80. * 8.).abs() < 0.01);
    assert_eq!(
        multicad_core::project_file::info::read(&changed)
            .unwrap()
            .name,
        "Project action review"
    );
    click(&ctx, &mut app, size, "Undo");
    assert_eq!(app.current().session.document(), &before);
    click(&ctx, &mut app, size, "Redo");
    assert_eq!(app.current().session.document(), &changed);
    app.current_mut()
        .editor
        .rows
        .iter_mut()
        .find(|row| row.name == "width")
        .unwrap()
        .expression = "=$missing_driver".into();
    for _ in 0..2 {
        frame(&ctx, &mut app, size, vec![]).drop_without_applying_deltas();
    }
    let out = frame(&ctx, &mut app, size, vec![]);
    assert!(!button(&out, "Apply and Rebuild").1);
    assert!(button(&out, "Discard Edit").1);
    let mut diagnostic_found = false;
    for shape in &out.shapes {
        if let egui::Shape::Text(text) = &shape.shape
            && text.galley.text().contains("missing_driver")
            && text.galley.text() != "=$missing_driver"
        {
            let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
            assert!(shape.clip_rect.contains_rect(rect));
            assert!(rect.bottom() < 350.);
            diagnostic_found = true;
        }
    }
    assert!(
        diagnostic_found,
        "the compact table keeps invalid formula diagnostics visible"
    );
    out.drop_without_applying_deltas();
    click(&ctx, &mut app, size, "Discard Edit");
    assert_eq!(app.current().session.document(), &changed);
    assert!(!app.current().editor.dirty());
}

#[test]
fn export_and_quick_3mf_use_one_project_tab_at_both_window_sizes() {
    for size in [egui::vec2(1024., 600.), egui::vec2(1360., 860.)] {
        let (mut app, _) = app();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::configure(&ctx);
        click(&ctx, &mut app, size, "Export");
        assert_eq!(app.mode.top_level(), Mode::Parameters);
        assert_eq!(app.project_tab_index(), PROJECT_EXPORT_TAB);
        let output = frame(&ctx, &mut app, size, vec![]);
        let nodes = &output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes;
        let has = |label| nodes.iter().any(|(_, node)| node.label() == Some(label));
        for label in [
            "General",
            "Parameters",
            "Materials",
            "Add export",
            "Add 3D print plate",
            "Quick 3MF package",
        ] {
            assert!(has(label), "Missing {label}");
        }
        assert_eq!(
            nodes
                .iter()
                .filter(|(_, node)| node.label() == Some("Export"))
                .count(),
            1
        );
        for label in [
            "Sketch",
            "Measure",
            "Library",
            "Print",
            "Render",
            "Common",
            "Save set",
            "Discard edits",
        ] {
            assert!(!has(label), "Unexpected {label}");
        }
        output.drop_without_applying_deltas();

        app.invoke(
            Command::Export(crate::exchange::ExportTool::NewSet),
            &[],
            Channel::Ui,
        )
        .unwrap();
        let output = frame(&ctx, &mut app, size, vec![]);
        let ribbon = egui::PanelState::load(&ctx, egui::Id::new("project-export-ribbon"))
            .unwrap()
            .outer_rect;
        for label in ["Save set", "Discard edits"] {
            let (bounds, enabled) = button(&output, label);
            assert!(
                enabled && bounds.top() > ribbon.bottom(),
                "{label} belongs to the inspector"
            );
            assert_eq!(
                output
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .filter(|(_, node)| node.label() == Some(label))
                    .count(),
                1
            );
        }
        output.drop_without_applying_deltas();
        app.invoke(
            Command::Export(crate::exchange::ExportTool::Discard),
            &[],
            Channel::Ui,
        )
        .unwrap();

        app.invoke(
            Command::Print(crate::print::PrintTool::Cancel),
            &[],
            Channel::Ui,
        )
        .unwrap();
        assert!(app.print_tab());
        assert_eq!(app.project_tab_index(), PROJECT_EXPORT_TAB);
        click(&ctx, &mut app, size, "General");
        assert_eq!(app.mode, Mode::Parameters);
        assert_eq!(app.project_tab_index(), 0);
    }
}

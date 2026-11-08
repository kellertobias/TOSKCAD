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
    for pressed in [true, false] {
        frame(
            ctx,
            app,
            size,
            vec![
                egui::Event::PointerMoved(rect.center()),
                egui::Event::PointerButton {
                    pos: rect.center(),
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
fn general_keeps_metadata_on_the_left_and_preserves_the_draft_between_tabs() {
    let (mut app, _) = app();
    app.tab = 0;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::configure(&ctx);
    let size = egui::vec2(1024., 600.);
    let out = frame(&ctx, &mut app, size, vec![]);
    let metadata = egui::PanelState::load(&ctx, egui::Id::new("parameters-metadata"))
        .unwrap()
        .outer_rect;
    assert!(metadata.left().abs() < 1.);
    assert!(egui::PanelState::load(&ctx, egui::Id::new("project-material-ribbon")).is_none());
    assert!(button(&out, "Start Modeling").1);
    for label in [
        "Project contents",
        "No saved poses yet.",
        "No drawings yet.",
    ] {
        assert!(out.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label)
        }));
    }
    out.drop_without_applying_deltas();

    let source = app.current().session.document().clone();
    app.current_mut().editor.info.name = "Pending Project title".into();
    click(&ctx, &mut app, size, "Parameters");
    assert_eq!(app.tab, PARAMETERS_TAB);
    assert!(app.current().editor.info_dirty());
    click(&ctx, &mut app, size, "General");
    assert_eq!(app.tab, 0);
    assert_eq!(app.current().editor.info.name, "Pending Project title");
    assert_eq!(app.current().session.document(), &source);
    let out = frame(&ctx, &mut app, size, vec![]);
    assert!(!button(&out, "Start Modeling").1);
    out.drop_without_applying_deltas();
    app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
    click(&ctx, &mut app, size, "Start Modeling");
    assert_eq!(app.mode, Mode::Model);
    assert_eq!(app.current().session.document(), &source);
}
#[test]
fn parameters_keep_left_actions_and_a_readable_table_without_a_ribbon() {
    for (size, density) in [
        (egui::vec2(1024., 600.), 1.),
        (egui::vec2(1024., 600.), 2.),
        (egui::vec2(1100., 700.), 1.),
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
        assert!(egui::PanelState::load(&ctx, egui::Id::new("ribbon")).is_none());
        let header = egui::PanelState::load(&ctx, egui::Id::new("project-tabs"))
            .unwrap()
            .outer_rect;
        let left = egui::PanelState::load(&ctx, egui::Id::new("project-actions"))
            .unwrap()
            .outer_rect;
        assert!(egui::PanelState::load(&ctx, egui::Id::new("parameters-metadata")).is_none());
        assert!(left.top() <= header.bottom() + 1.);
        let mut previous = header.bottom();
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
                rect.left() >= left.left() + 6. && rect.right() <= left.right() - 6.,
                "{label}: {rect:?} outside {left:?}"
            );
            assert!(rect.top() > previous);
            assert!(rect.bottom() < size.y - 36.);
            assert!((rect.height() - 34.).abs() < 1.);
            previous = rect.bottom();
        }
        let mut heading = None;
        for shape in &out.shapes {
            if let egui::Shape::Text(text) = &shape.shape {
                if text.galley.text() == "Parameters" {
                    heading = Some(text.pos);
                }
                assert_ne!(text.galley.text(), "Parts");
            }
            if let egui::Shape::Rect(rect) = &shape.shape {
                assert!(
                    !(rect.fill == Color32::BLACK
                        && rect.rect.width() > size.x - 8.
                        && rect.rect.height() > 60.
                        && rect.rect.top() >= header.bottom() - 1.),
                    "Project retained a black ribbon band"
                );
            }
        }
        let heading = heading.expect("parameter table heading");
        assert!(heading.x >= left.right() + 12.);
        assert!(heading.y < header.bottom() + 40.);
        out.drop_without_applying_deltas();
        if size.x <= 1100. {
            frame(
                &ctx,
                &mut app,
                size,
                vec![
                    egui::Event::PointerMoved(egui::pos2(
                        (left.right() + size.x) * 0.5,
                        header.bottom() + 100.,
                    )),
                    egui::Event::MouseWheel {
                        phase: egui::TouchPhase::Move,
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(-1200., 0.),
                        modifiers: Default::default(),
                    },
                ],
            )
            .drop_without_applying_deltas();
            for _ in 0..16 {
                frame(&ctx, &mut app, size, vec![]).drop_without_applying_deltas();
            }
            let out = frame(&ctx, &mut app, size, vec![]);
            assert!(
                out.shapes.iter().any(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Row" => {
                        let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                        shape.clip_rect.contains_rect(rect) && rect.right() < size.x
                    }
                    _ => false,
                }),
                "horizontal scrolling must make the final table column usable at the native minimum"
            );
            out.drop_without_applying_deltas();
        }
        // Returning from a ribbon workspace must also reclaim its vertical band.
        app.mode = Mode::Model;
        frame(&ctx, &mut app, size, vec![]).drop_without_applying_deltas();
        app.mode = Mode::Parameters;
        let out = frame(&ctx, &mut app, size, vec![]);
        let (rect, _) = button(&out, "Apply and Rebuild");
        assert!(rect.top() < header.bottom() + 60.);
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

//! Full application transition from target acquisition to a model Inspector.
use super::*;
use crate::{
    modeling::{ModelingTool, native_fixture::NativeBackend},
    viewport::BackendFactory,
};
use multicad_core::model::{self, PrimitiveParameters, PrimitiveRequest};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
struct Storage;
impl DocumentStorage for Storage {
    fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
        Err("unused".into())
    }
    fn load(&mut self, _: &std::path::Path) -> Result<Document, String> {
        Err("unused".into())
    }
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
            events,
            focused: true,
            ..Default::default()
        },
        |ui| app.show(ui),
    )
}
fn click(ctx: &egui::Context, app: &mut Application, size: egui::Vec2, label: &str) {
    let out = frame(ctx, app, size, vec![]);
    let position = out
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find_map(|(_, n)| {
            (n.label() == Some(label))
                .then(|| n.bounds())
                .flatten()
                .map(|b| egui::pos2(((b.x0 + b.x1) * 0.5) as f32, ((b.y0 + b.y1) * 0.5) as f32))
        })
        .or_else(|| {
            out.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == label => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
        })
        .unwrap_or_else(|| panic!("Missing {label}"));
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        frame(
            ctx,
            app,
            size,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
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
fn model_enter_and_escape_respect_text_and_open_dropdowns() {
    let size = egui::vec2(1360., 860.);
    let mut app = Application::with_document(
        Box::new(Storage),
        Document::new("Staged Model keys").unwrap(),
    )
    .unwrap();
    app.set_model_backends(Arc::new(|| Box::new(NativeBackend::new())));
    app.mode = Mode::Model;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::configure(&ctx);
    app.invoke(Command::Modeling(ModelingTool::Box), &[], Channel::Ui)
        .unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        frame(&ctx, &mut app, size, vec![]).drop_without_applying_deltas();
        let p = app.current();
        if p.sketch.model_can_apply(&p.session, &p.resources) {
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    let key = |app: &mut Application, key| {
        for pressed in [true, false] {
            frame(
                &ctx,
                app,
                size,
                vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            )
            .drop_without_applying_deltas();
        }
    };
    click(&ctx, &mut app, size, "XY");
    assert!(egui::Popup::is_any_open(&ctx));
    key(&mut app, egui::Key::Escape);
    assert!(!egui::Popup::is_any_open(&ctx));
    assert!(app.current().sketch.model_active());
    assert_eq!(app.current().session.revision(), 0);

    let output = frame(&ctx, &mut app, size, vec![]);
    let nodes = &output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes;
    let bounds = nodes
        .iter()
        .find(|(_, node)| node.value() == Some("Box 1"))
        .and_then(|(_, node)| node.bounds())
        .expect("Body name text field");
    output.drop_without_applying_deltas();
    let pos = egui::pos2(
        (bounds.x0 + bounds.x1) as f32 * 0.5,
        (bounds.y0 + bounds.y1) as f32 * 0.5,
    );
    let focus_name = |app: &mut Application| {
        for pressed in [true, false] {
            frame(
                &ctx,
                app,
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
    };
    focus_name(&mut app);
    assert!(ctx.text_edit_focused());
    key(&mut app, egui::Key::Enter);
    assert!(app.current().sketch.model_active());
    assert_eq!(app.current().session.revision(), 0);
    focus_name(&mut app);
    key(&mut app, egui::Key::Escape);
    assert!(app.current().sketch.model_active());

    key(&mut app, egui::Key::Enter);
    assert_eq!(app.current().session.revision(), 1);
    assert!(!app.current().sketch.model_active());
    key(&mut app, egui::Key::Enter);
    assert_eq!(app.current().session.revision(), 1);
    let applied = app.current().session.document().clone();
    app.invoke(Command::Modeling(ModelingTool::Box), &[], Channel::Ui)
        .unwrap();
    key(&mut app, egui::Key::Escape);
    assert!(!app.current().sketch.model_active());
    assert_eq!(app.current().session.document(), &applied);
}

#[test]
fn compact_model_navigation_and_project_switcher_preserve_context() {
    let size = egui::vec2(1024., 600.);
    let mut seed = Session::new(Document::new("Selection navigation").unwrap()).unwrap();
    let mut bodies = Vec::new();
    for (name, x) in [("Near block", 0.), ("Far block", 250.)] {
        let stock = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: [model::length(60.), model::length(40.), model::length(12.)],
                frame: model::PrimitiveFrame {
                    origin: [model::length(x), model::length(0.), model::length(0.)],
                    ..Default::default()
                },
            },
        );
        seed.commit(
            model::prepare_primitive(&seed, seed.revision(), Actor::local(Channel::Ui), &stock)
                .unwrap()
                .prepared,
        )
        .unwrap();
        bodies.push(stock.body);
    }
    let original = seed.document().clone();
    let mut app = Application::with_document(Box::new(Storage), original.clone()).unwrap();
    app.pending_files.clear();
    app.set_scene_backends(Arc::new(|| Box::new(NativeBackend::new())));
    app.mode = Mode::Model;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::configure(&ctx);

    let until = Instant::now() + Duration::from_secs(5);
    while !app.current().scene.is_current(original.id(), 0) {
        frame(&ctx, &mut app, size, vec![]).drop_without_applying_deltas();
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    let controls = |app: &mut Application, expected| {
        let output = frame(&ctx, app, size, vec![]);
        let nodes = &output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes;
        for name in ["Fit selection", "Clear selection"] {
            let mut buttons = nodes.iter().filter(|(_, node)| node.label() == Some(name));
            let (_, button) = buttons.next().unwrap_or_else(|| panic!("Missing {name}"));
            assert!(buttons.next().is_none(), "only one {name} control");
            assert_eq!(!button.is_disabled(), expected, "{name}");
            let bounds = button.bounds().unwrap();
            assert!(bounds.y0 > 500. && bounds.y1 < 600., "{bounds:?}");
        }
        output.drop_without_applying_deltas();
    };
    controls(&mut app, false);
    click(&ctx, &mut app, size, "Near block");
    controls(&mut app, true);
    assert_eq!(app.current().sketch.selected_solid(), Some(bodies[0]));
    click(&ctx, &mut app, size, "Fit selection");
    let target = app.current().sketch.camera.target;
    assert!(
        target
            .into_iter()
            .zip([30., 20., 6.])
            .all(|(x, y)| (x - y).abs() < 0.01)
    );
    click(&ctx, &mut app, size, "Clear selection");
    assert!(app.current().sketch.selected_solid().is_none());
    controls(&mut app, false);

    click(&ctx, &mut app, size, "Near block");
    app.current_mut().editor.rows[0].unit = "cm".into();
    app.invoke(Command::New, &[], Channel::Ui).unwrap();
    app.current_mut().editor.rows[0].unit = "in".into();
    click(
        &ctx,
        &mut app,
        size,
        "Switch project. Active project Untitled 2. 2 open",
    );
    click(&ctx, &mut app, size, "Selection navigation  •");
    assert_eq!(app.active, 0);
    assert_eq!(app.current().editor.rows[0].unit, "cm");
    assert_eq!(app.current().sketch.selected_solid(), Some(bodies[0]));
    assert_eq!(app.current().session.document(), &original);
    click(
        &ctx,
        &mut app,
        size,
        "Switch project. Active project Selection navigation. 2 open",
    );
    click(&ctx, &mut app, size, "Untitled 2  •");
    assert_eq!(app.active, 1);
    assert_eq!(app.current().editor.rows[0].unit, "in");
    assert!(app.current().sketch.selected_solid().is_none());
}

#[test]
fn tool_first_scale_footer_is_painted_and_clickable_after_target_selection() {
    for (width, height, density) in [
        (1360., 420., 1.),
        (1024., 600., 2.),
        (1360., 860., 1.),
        (1360., 860., 2.),
        (1360., 1000., 2.),
    ] {
        let size = egui::vec2(width, height);
        let mut seed = Session::new(Document::new("Scale footer transition").unwrap()).unwrap();
        let stock = PrimitiveRequest::new(
            "Source block",
            PrimitiveParameters::Box {
                size: [model::length(60.), model::length(40.), model::length(12.)],
                frame: Default::default(),
            },
        );
        seed.commit(
            model::prepare_primitive(&seed, seed.revision(), Actor::local(Channel::Ui), &stock)
                .unwrap()
                .prepared,
        )
        .unwrap();
        let mut app =
            Application::with_document(Box::new(Storage), seed.document().clone()).unwrap();
        app.pending_files.clear();
        let factory: BackendFactory = Arc::new(|| Box::new(NativeBackend::new()));
        app.set_scene_backends(factory.clone());
        app.set_model_backends(factory);
        app.mode = Mode::Model;
        let original = app.current().session.document().clone();
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(density);
        ctx.enable_accesskit();
        crate::theme::configure(&ctx);
        let until = Instant::now() + Duration::from_secs(5);
        while !app.current().scene.is_current(original.id(), 0) {
            frame(&ctx, &mut app, size, vec![]).drop_without_applying_deltas();
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        // Preserve the physical-review path through a different native tool
        // before acquiring Scale targets in this same Inspector/scroll context.
        app.invoke(Command::Modeling(ModelingTool::Cylinder), &[], Channel::Ui)
            .unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            let out = frame(&ctx, &mut app, size, vec![]);
            let ready = out
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Apply") && !n.is_disabled());
            let texts = out
                .shapes
                .iter()
                .filter_map(|s| {
                    if let egui::Shape::Text(t) = &s.shape {
                        Some(t.galley.text().to_owned())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            out.drop_without_applying_deltas();
            if ready {
                break;
            }
            assert!(
                Instant::now() < until,
                "Cylinder Preview failed at height {height}: {texts:?}"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        if width == 1024. {
            let open_width = app.current().sketch.viewport_rect.unwrap().width();
            click(&ctx, &mut app, size, "Settings");
            click(&ctx, &mut app, size, "Model Tree");
            click(&ctx, &mut app, size, "Settings");
            click(&ctx, &mut app, size, "Inspector");
            frame(&ctx, &mut app, size, vec![]).drop_without_applying_deltas();
            assert!(app.current().sketch.viewport_rect.unwrap().width() > open_width + 300.);
            assert!(app.current().sketch.model_interaction_active());
            assert_eq!(app.current().session.document(), &original);

            let roomy = egui::vec2(1360., 860.);
            click(&ctx, &mut app, roomy, "Settings");
            click(&ctx, &mut app, roomy, "Inspector");
            click(&ctx, &mut app, size, "Settings");
            click(&ctx, &mut app, size, "Model Tree");
            let out = frame(&ctx, &mut app, size, vec![]);
            assert!(
                out.platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .any(|(_, n)| n.label() == Some("Apply") && !n.is_disabled()),
                "the existing native preview must still be ready after reopening the Inspector"
            );
            assert!((app.current().sketch.viewport_rect.unwrap().width() - open_width).abs() < 1.);
            out.drop_without_applying_deltas();
        }
        click(&ctx, &mut app, size, "Cancel");
        assert!(!app.current().sketch.model_interaction_active());
        assert_eq!(app.current().session.document(), &original);
        app.invoke(Command::Modeling(ModelingTool::Scale), &[], Channel::Ui)
            .unwrap();
        assert!(app.current().sketch.awaiting_model_targets());
        let out = frame(&ctx, &mut app, size, vec![]);
        let row = out
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "Source block" => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("native Part tree row");
        out.drop_without_applying_deltas();
        for pressed in [true, false] {
            frame(
                &ctx,
                &mut app,
                size,
                vec![
                    egui::Event::PointerMoved(row),
                    egui::Event::PointerButton {
                        pos: row,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            )
            .drop_without_applying_deltas();
        }
        assert!(!app.current().sketch.awaiting_model_targets());
        click(&ctx, &mut app, size, "Fit selection");
        let mut cancel = None;
        for _ in 0..3 {
            let out = frame(&ctx, &mut app, size, vec![]);
            let mut missing = vec![];
            for label in ["Preview", "Apply", "Cancel"] {
                let painted = out
                    .shapes
                    .iter()
                    .enumerate()
                    .find_map(|(index, s)| match &s.shape {
                        egui::Shape::Text(t) if t.galley.text() == label => Some((
                            egui::Rect::from_min_size(t.pos, t.galley.size()),
                            s.clip_rect,
                            index,
                        )),
                        _ => None,
                    });
                if let Some((rect, clip, index)) = painted.filter(|(r, c, _)| c.contains_rect(*r)) {
                    let occlusion =
                        out.shapes
                            .iter()
                            .skip(index + 1)
                            .find_map(|s| match &s.shape {
                                egui::Shape::Rect(r)
                                    if r.fill.a() == 255
                                        && r.rect.contains(rect.center())
                                        && s.clip_rect.contains(rect.center()) =>
                                {
                                    Some((r.rect, r.fill))
                                }
                                _ => None,
                            });
                    assert!(
                        occlusion.is_none(),
                        "height {height}: {label} overpainted by {occlusion:?}"
                    );
                    if label == "Cancel" {
                        cancel = Some(rect.center());
                    }
                    assert!(clip.bottom() > height - 100.);
                } else {
                    missing.push((label, painted));
                }
            }
            out.drop_without_applying_deltas();
            assert!(
                missing.is_empty(),
                "height {height}: clipped or missing fixed actions {missing:?}"
            );
        }
        for pressed in [true, false] {
            let pos = cancel.unwrap();
            frame(
                &ctx,
                &mut app,
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
        assert!(!app.current().sketch.model_interaction_active());
        assert_eq!(app.current().session.document(), &original);
        assert_eq!(app.current().session.revision(), 0);
    }
}

/// Room → Door from the ribbon command: refused without a wall, then a door
/// is previewed natively and applied as one transaction on the wall.
#[test]
fn room_door_tool_previews_and_applies_one_wall_transaction() {
    use multicad_core::model::{
        BodyOutput, Justification, NewType, NewTypeKind, WallChange, WallDefinition, WallRequest,
        WallTypeDefinition,
    };
    let size = egui::vec2(1360., 860.);
    let mut empty =
        Application::with_document(Box::new(Storage), Document::new("No walls").unwrap()).unwrap();
    empty.pending_files.clear();
    let factory: BackendFactory = Arc::new(|| Box::new(NativeBackend::new()));
    empty.set_scene_backends(factory.clone());
    empty.set_model_backends(factory.clone());
    empty.mode = Mode::Sketch;
    let refused = empty
        .invoke(Command::Modeling(ModelingTool::Door), &[], Channel::Ui)
        .unwrap_err();
    assert!(refused.contains("Draw a wall first"), "{refused}");

    let mut seed = Session::new(Document::new("Room door").unwrap()).unwrap();
    let ty = NewType::new(
        "Block 200",
        NewTypeKind::Wall(WallTypeDefinition {
            thickness: model::length(200.),
            height: model::length(2500.),
            justification: Justification::Center,
            material: None,
        }),
    );
    let request = WallRequest::new(
        "Wall 1",
        WallDefinition {
            layer_placement: None,
            output: BodyOutput {
                body: EntityId::new(),
            },
            wall_type: ty.id,
            path: vec![
                [model::length(0.), model::length(0.)],
                [model::length(4000.), model::length(0.)],
            ],
            closed: false,
            level: None,
            base_z: model::length(0.),
            thickness: None,
            height: None,
            justification: None,
            openings: vec![],
            cutouts: vec![],
        },
    );
    let wall = request.feature;
    seed.commit(
        model::prepare_wall_change(
            &seed,
            0,
            Actor::local(Channel::Ui),
            &[ty],
            &WallChange::Create(&request),
        )
        .unwrap()
        .prepared,
    )
    .unwrap();
    let mut app = Application::with_document(Box::new(Storage), seed.document().clone()).unwrap();
    app.pending_files.clear();
    app.set_scene_backends(factory.clone());
    app.set_model_backends(factory);
    app.mode = Mode::Sketch;
    let original = app.current().session.document().clone();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::configure(&ctx);
    let until = Instant::now() + Duration::from_secs(5);
    while !app.current().scene.is_current(original.id(), 0) {
        frame(&ctx, &mut app, size, vec![]).drop_without_applying_deltas();
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(app.enabled(Command::Modeling(ModelingTool::Door)));
    app.invoke(Command::Modeling(ModelingTool::Door), &[], Channel::Ui)
        .unwrap();
    assert!(app.current().sketch.model_interaction_active());
    click(&ctx, &mut app, size, "Preview");
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let out = frame(&ctx, &mut app, size, vec![]);
        let ready = out
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .any(|(_, n)| n.label() == Some("Apply") && !n.is_disabled());
        out.drop_without_applying_deltas();
        if ready {
            break;
        }
        assert!(Instant::now() < until, "Door Preview never became ready");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(app.current().session.document(), &original);
    click(&ctx, &mut app, size, "Apply");
    let session = &app.current().session;
    assert_eq!(
        session.revision(),
        1,
        "one transaction on the reopened document"
    );
    let placed = model::read_wall(session.document(), wall).unwrap();
    assert_eq!(placed.openings.len(), 1);
    assert_eq!(placed.openings[0].offset, model::length(500.));
    assert_eq!(
        session.activity().last().map(|a| a.label.as_str()),
        Some("Place door")
    );
}

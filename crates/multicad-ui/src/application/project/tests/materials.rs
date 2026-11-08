use super::*;
use multicad_core::materials::{LibraryDefinition, StockKind, read};
use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, Instant},
};

fn native_app() -> (Application, egui::Context, egui::Vec2) {
    let (mut app, _) = app();
    app.set_model_backends(Arc::new(|| {
        Box::new(crate::modeling::native_fixture::NativeBackend::new())
    }));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::configure(&ctx);
    let size = egui::vec2(1360., 860.);
    for _ in 0..3 {
        frame(&ctx, &mut app, size, vec![]).drop_without_applying_deltas();
    }
    (app, ctx, size)
}
fn preview_and_apply(ctx: &egui::Context, app: &mut Application, size: egui::Vec2) {
    assert!(
        !app.enabled(Command::Apply),
        "native preview must gate Apply"
    );
    let before = app.current().session.document().clone();
    click(ctx, app, size, "Preview");
    let deadline = Instant::now() + Duration::from_secs(15);
    while !app.enabled(Command::Apply) {
        assert!(
            Instant::now() < deadline,
            "Project native preview timed out: {}",
            app.message
        );
        frame(ctx, app, size, vec![]).drop_without_applying_deltas();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(app.current().session.document(), &before);
    assert!(
        app.message.contains("preview ready"),
        "Completion status stayed busy: {}",
        app.message
    );
    click(ctx, app, size, "Apply and Rebuild");
    assert!(!app.current().sketch.material_library().dirty());
}
#[test]
fn project_buttons_create_one_shared_material_sheet_and_profile_through_native_preview() {
    let (mut app, ctx, size) = native_app();
    let before_ids: BTreeSet<_> = app
        .current()
        .session
        .document()
        .entities()
        .map(|e| e.id)
        .collect();
    let revision = app.current().session.revision();
    click(&ctx, &mut app, size, "Materials");
    assert_eq!(app.mode, Mode::Parameters);
    assert_eq!(
        app.registry.workspace(app.mode).tabs[app.tab].title,
        "Materials"
    );
    assert!(egui::PanelState::load(&ctx, egui::Id::new("ribbon")).is_none());
    assert!(egui::PanelState::load(&ctx, egui::Id::new("parameters-metadata")).is_some());
    for label in ["New material", "New sheet stock", "New profile stock"] {
        click(&ctx, &mut app, size, label);
        assert!(app.current().pending_edits());
        assert!(!app.enabled(Command::Save));
        assert!(!app.enabled(Command::Undo));
        assert!(!app.enabled(Command::Mode(Mode::Model)));
        preview_and_apply(&ctx, &mut app, size);
    }
    assert_eq!(app.current().session.revision(), revision + 3);
    let created: Vec<_> = app
        .current()
        .session
        .document()
        .entities()
        .filter(|e| !before_ids.contains(&e.id))
        .collect();
    assert_eq!(
        created.len(),
        3,
        "one shared material, one sheet and one profile; no duplicate catalogs"
    );
    let material = created
        .iter()
        .find(|e| matches!(e.data, EntityData::Material { .. }))
        .unwrap()
        .id;
    let mut kinds = BTreeSet::new();
    let mut profile = None;
    for e in created {
        if let Some(LibraryDefinition::Stock(stock)) = read(e).unwrap() {
            assert_eq!(stock.material, material);
            match stock.specification {
                StockKind::Sheet { thickness_mm, .. } => {
                    assert_eq!(thickness_mm, 1.);
                    kinds.insert("sheet");
                }
                StockKind::Profile {
                    width_mm,
                    height_mm,
                    wall_mm,
                    ..
                } => {
                    assert_eq!((width_mm, height_mm, wall_mm), (30., 30., Some(2.)));
                    kinds.insert("profile");
                    profile = Some(e.id);
                }
                _ => panic!("wrong Project stock kind"),
            }
        }
    }
    assert_eq!(kinds, BTreeSet::from(["sheet", "profile"]));
    let profile = profile.unwrap();
    click(&ctx, &mut app, size, "Undo");
    assert!(app.current().session.document().get(profile).is_none());
    click(&ctx, &mut app, size, "Redo");
    assert!(app.current().session.document().get(profile).is_some());
}
#[test]
fn project_material_draft_discard_is_neutral_and_cannot_be_hidden_by_mode_or_parameter_commands() {
    let (mut app, ctx, size) = native_app();
    let before = app.current().session.document().clone();
    click(&ctx, &mut app, size, "Materials");
    click(&ctx, &mut app, size, "New sheet stock");
    assert!(
        app.invoke(Command::Mode(Mode::Model), &[], Channel::Ui)
            .is_err()
    );
    assert!(
        app.invoke(Command::Parameter, &["test", "mm", "20"], Channel::Ui)
            .is_err()
    );
    assert_eq!(app.mode, Mode::Parameters);
    assert_eq!(app.tab, 1);
    assert_eq!(app.current().session.document(), &before);
    click(&ctx, &mut app, size, "Discard Edit");
    assert_eq!(app.current().session.document(), &before);
    assert!(!app.current().pending_edits());
    click(&ctx, &mut app, size, "Parameters");
    assert_eq!(app.tab, 0);
}
#[test]
fn legacy_model_materials_recall_routes_to_the_project_catalog_without_source_changes() {
    let (mut app, _, _) = native_app();
    let before = app.current().session.document().clone();
    app.current_mut().view = crate::recent_projects::RecentView {
        mode: Mode::Model,
        workspace: "Materials".into(),
        ..Default::default()
    };
    app.restore_current_view();
    assert_eq!(app.mode, Mode::Parameters);
    assert_eq!(
        app.registry.workspace(app.mode).tabs[app.tab].title,
        "Materials"
    );
    assert!(
        !app.registry
            .workspace(Mode::Model)
            .tabs
            .iter()
            .any(|t| t.title == "Materials")
    );
    assert_eq!(app.current().session.document(), &before);
    assert_eq!(
        app.registry.parse_line("materials").unwrap().0,
        Command::Extension("materials")
    );
}

#[test]
fn compact_material_chooser_keeps_selected_definition_and_draft_through_resize() {
    let (mut app, ctx, _) = native_app();
    let material = Entity::new(
        "Birch ply definition",
        EntityData::Material {
            density_kg_m3: Some(700.),
            color_rgba: [160, 170, 180, 255],
            section_hatch: None,
        },
    );
    let revision = app.current().session.revision();
    app.current_mut()
        .session
        .apply(multicad_core::commands::Request {
            expected_revision: revision,
            actor: Actor::local(Channel::Ui),
            label: "Add material fixture".into(),
            edits: vec![multicad_core::document::Mutation::Insert(material.clone())],
        })
        .unwrap();
    let before = app.current().session.document().clone();
    let size = egui::vec2(1024., 600.);
    click(&ctx, &mut app, size, "Materials");

    let click_at = |app: &mut Application, point: egui::Pos2| {
        for pressed in [true, false] {
            frame(
                &ctx,
                app,
                size,
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            )
            .drop_without_applying_deltas();
        }
    };
    let painted = |app: &mut Application, label| {
        let out = frame(&ctx, app, size, vec![]);
        let rect = out
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                    shape.clip_rect.contains_rect(rect).then_some(rect)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing unclipped compact Material control: {label}"));
        out.drop_without_applying_deltas();
        rect
    };
    let picker = painted(&mut app, "Choose a definition");
    click_at(&mut app, picker.center());
    let entry = painted(&mut app, material.name.as_str());
    click_at(&mut app, entry.center());

    let value = painted(&mut app, material.name.as_str());
    assert!(value.left() > 184. && value.right() < size.x - 300.);
    let out = frame(&ctx, &mut app, size, vec![]);
    let name = out
        .shapes
        .iter()
        .filter_map(|shape| {
            let egui::Shape::Text(text) = &shape.shape else {
                return None;
            };
            (text.galley.text() == material.name).then(|| {
                egui::pos2(
                    text.pos.x + text.galley.size().x + 6.,
                    text.pos.y + text.galley.size().y * 0.5,
                )
            })
        })
        .max_by(|a, b| a.y.total_cmp(&b.y))
        .expect("compact Material Name field");
    out.drop_without_applying_deltas();
    click_at(&mut app, name);
    if !ctx.text_edit_focused() {
        // Dismiss the chooser overlay before focusing the form below it.
        click_at(&mut app, name);
    }
    assert!(ctx.text_edit_focused());
    for (frame_size, key, text) in [
        (size, egui::Key::Space, " "),
        (size, egui::Key::Q, "q"),
        (egui::vec2(1360., 860.), egui::Key::R, "r"),
        (size, egui::Key::E, "e"),
    ] {
        let out = frame(
            &ctx,
            &mut app,
            frame_size,
            vec![
                egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                },
                egui::Event::Text(text.into()),
            ],
        );
        out.drop_without_applying_deltas();
        assert!(
            ctx.text_edit_focused(),
            "the Name field lost focus while typing {text:?}"
        );
        assert!(!app.current().sketch.material_library().pending());
    }
    painted(&mut app, "Birch ply definition qre");
    let label = painted(&mut app, "Density (kg/m³)");
    click_at(&mut app, label.center());
    assert!(app.current().sketch.material_library().dirty());
    assert_eq!(app.current().session.document(), &before);

    for size in [egui::vec2(1360., 860.), size] {
        frame(&ctx, &mut app, size, vec![]).drop_without_applying_deltas();
        assert!(app.current().sketch.material_library().dirty());
        assert_eq!(app.current().session.document(), &before);
    }
    click(&ctx, &mut app, size, "Discard Edit");
    assert!(!app.current().sketch.material_library().dirty());
    assert_eq!(app.current().session.document(), &before);
}

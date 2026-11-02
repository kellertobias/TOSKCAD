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

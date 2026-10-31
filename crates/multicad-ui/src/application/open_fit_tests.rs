//! CAD-175: a project opened into Model frames its evaluated model once.
use super::*;
use crate::modeling::native_fixture::NativeBackend;
use egui::Vec2;
use std::{path::Path, sync::Arc};

struct NoFiles;
impl DocumentStorage for NoFiles {
    fn save(&mut self, _: &Path, _: &Document) -> Result<(), String> {
        Ok(())
    }
    fn load(&mut self, _: &Path) -> Result<Document, String> {
        Err("No file".into())
    }
    fn save_project(
        &mut self,
        _: &Path,
        _: &ProjectFile,
        _: crate::storage::SaveMode,
    ) -> Result<crate::storage::SavedProject, crate::storage::StorageError> {
        Err("No file".to_string().into())
    }
}

fn frame(ctx: &egui::Context, app: &mut Application, size: Vec2, events: Vec<egui::Event>) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| app.show(ui),
    );
    output.textures_delta.clear();
}
fn settle(ctx: &egui::Context, app: &mut Application, size: Vec2) {
    for _ in 0..600 {
        frame(ctx, app, size, vec![]);
        let p = app.current();
        if p.scene
            .is_current(p.session.document().id(), p.session.revision())
        {
            for _ in 0..3 {
                frame(ctx, app, size, vec![]);
            }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("native scene did not settle");
}
fn opened(document: Document) -> (egui::Context, Application) {
    let mut app = Application::with_document(Box::new(NoFiles), document).unwrap();
    app.set_scene_backends(Arc::new(|| Box::new(NativeBackend::new())));
    app.mode = Mode::Model;
    (egui::Context::default(), app)
}
fn on_screen(app: &Application, bounds: [[f64; 3]; 2]) -> bool {
    let sketch = &app.current().sketch;
    let rect = sketch.viewport_rect.expect("viewport laid out");
    (0..8).all(|corner| {
        let p = sketch.camera.project(
            std::array::from_fn(|i| bounds[(corner >> i) & 1][i] as f32),
            rect,
        );
        rect.contains(egui::pos2(p[0], p[1]))
    })
}

fn cabinet_document() -> Document {
    use multicad_core::{
        commands::{Actor, Channel},
        document::{Entity, EntityData, EntityId},
        materials::{Grain, LibraryDefinition, Stock, StockKind, prepare_library},
        model,
    };
    let mut s = Session::new(Document::new("Reopened cabinet").unwrap()).unwrap();
    let birch = Entity::new(
        "Birch",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [210, 180, 140, 255],
            section_hatch: None,
        },
    );
    let ply = LibraryDefinition::Stock(Stock {
        material: birch.id,
        specification: StockKind::Panel {
            thickness_mm: 18.,
            length_mm: 2500.,
            width_mm: 1250.,
            grain: Grain::AlongLength,
        },
    })
    .entity(EntityId::new(), "Ply 18")
    .unwrap();
    let ui = Actor::local(Channel::Ui);
    s.commit(prepare_library(&s, 0, ui.clone(), &[birch, ply.clone()]).unwrap())
        .unwrap();
    let request = model::CarcassRequest::new(
        "Cabinet 1",
        model::CarcassDefinition {
            cabinet: None,
            origin: [model::length(0.), model::length(0.), model::length(0.)],
            rotation: model::angle(0.),
            width: model::length(600.),
            height: model::length(720.),
            depth: model::length(560.),
            stock: ply.id,
            construction: Default::default(),
            back: model::Back::None,
            shelves: model::Shelves {
                layout: model::ShelfLayout::Even { count: 1 },
                setback: model::length(20.),
                clearance: model::length(1.),
            },
            front_band: None,
            group: None,
        },
    );
    s.commit(
        model::prepare_carcass(&s, 1, ui, &request)
            .unwrap()
            .model
            .prepared,
    )
    .unwrap();
    s.document().clone()
}

#[test]
fn project_opened_into_parameters_is_framed_when_model_is_shown_and_follows_startup_resize() {
    let (ctx, mut app) = opened(cabinet_document());
    app.mode = Mode::Parameters;
    let small = Vec2::new(1100., 700.);
    settle(&ctx, &mut app, small);
    app.mode = Mode::Model;
    for _ in 0..3 {
        frame(&ctx, &mut app, small, vec![]);
    }
    let bounds = {
        let p = app.current();
        p.scene.visible().unwrap().world_bounds().unwrap()
    };
    assert!(on_screen(&app, bounds), "{:?}", app.current().sketch.camera);
    let first = app.current().sketch.camera;
    // The window settles at a larger size before the user navigates.
    let large = Vec2::new(1700., 1050.);
    for _ in 0..3 {
        frame(&ctx, &mut app, large, vec![]);
    }
    assert!(on_screen(&app, bounds));
    assert!(app.current().sketch.camera.scale > first.scale);
}

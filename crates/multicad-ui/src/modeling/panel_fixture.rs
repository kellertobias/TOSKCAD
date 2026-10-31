//! Panel fixtures for viewport placement tests: stock, a flat panel, a
//! carcass, and a click in a top view of the production native scene.
use crate::{
    camera::Camera,
    egui,
    modeling::{TerminationViewport, native_fixture},
};
use multicad_core::{
    commands::Session,
    document::{Document, EntityId},
    model::{self, length},
};
use multicad_core::{
    commands::{Actor, Channel},
    document::{Entity, EntityData},
    materials::{Grain, LibraryDefinition, Stock, StockKind, prepare_library},
    model::{
        Back, BodyOutput, CarcassDefinition, CarcassRequest, Construction, GrainAxis,
        PanelDefinition, PanelEdges, PanelPlane, PanelRequest, PanelRole, PanelSource, ShelfLayout,
        Shelves, angle,
    },
};
use multicad_model::Compiler;

pub(crate) fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn stocked() -> (Session, EntityId) {
    let mut s = Session::new(Document::new("Machining placement").unwrap()).unwrap();
    let birch = Entity::new(
        "Birch",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [210, 180, 140, 255],
            section_hatch: None,
        },
    );
    let stock = LibraryDefinition::Stock(Stock {
        material: birch.id,
        specification: StockKind::Panel {
            thickness_mm: 18.,
            length_mm: 2500.,
            width_mm: 1250.,
            grain: Grain::None,
        },
    })
    .entity(EntityId::new(), "Ply 18")
    .unwrap();
    let id = stock.id;
    s.commit(prepare_library(&s, 0, actor(), &[birch, stock]).unwrap())
        .unwrap();
    (s, id)
}
/// One 600 × 400 × 18 panel on XY.
pub(crate) fn flat_panel() -> (Session, EntityId) {
    let (mut s, stock) = stocked();
    let request = PanelRequest::new(
        "Side",
        PanelDefinition {
            output: BodyOutput {
                body: EntityId::new(),
            },
            source: PanelSource::Rectangle {
                stock,
                plane: PanelPlane::Xy,
                origin: [length(0.), length(0.), length(0.)],
                length: length(600.),
                width: length(400.),
            },
            grain: GrainAxis::U,
            edges: PanelEdges::default(),
            oversize: None,
        },
    );
    let p = model::prepare_panel(&s, s.revision(), actor(), &request).unwrap();
    s.commit(p.prepared).unwrap();
    (s, request.body)
}
/// A 600 × 720 × 400 carcass; returns its left and right sides.
pub(crate) fn carcass() -> (Session, EntityId, EntityId) {
    let (mut s, stock) = stocked();
    let created = model::prepare_carcass(
        &s,
        s.revision(),
        actor(),
        &CarcassRequest::new(
            "Cabinet",
            CarcassDefinition {
                origin: [length(0.), length(0.), length(0.)],
                rotation: angle(0.),
                width: length(600.),
                height: length(720.),
                depth: length(400.),
                stock,
                construction: Construction::SidesFullHeight,
                back: Back::None,
                shelves: Shelves {
                    layout: ShelfLayout::Even { count: 0 },
                    setback: length(10.),
                    clearance: length(1.),
                },
                front_band: None,
                group: None,
            },
        ),
    )
    .unwrap();
    let side = |role| {
        created
            .panels
            .iter()
            .find(|(r, _)| *r == role)
            .unwrap()
            .1
            .body
    };
    let (l, r) = (side(PanelRole::LeftSide), side(PanelRole::RightSide));
    s.commit(created.model.prepared).unwrap();
    (s, l, r)
}
/// Hover and click a world point in a top view of the production scene,
/// handing the viewport to a form.
pub(crate) fn click(
    s: &Session,
    point: [f32; 3],
    form: impl FnOnce(TerminationViewport<'_>) -> bool,
) -> bool {
    let native = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    let scene = native_fixture::scene(s.document(), native.snapshot(), 0).unwrap();
    let camera = Camera {
        yaw: 0.,
        pitch: std::f32::consts::FRAC_PI_2,
        scale: 0.5,
        target: [300., 200., 0.],
        perspective: false,
    };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600., 600.));
    let p = camera.project(point, rect);
    let ctx = egui::Context::default();
    let mut changed = false;
    let mut form = Some(form);
    ctx.run_ui(Default::default(), |ui| {
        let Some(form) = form.take() else { return };
        changed = form(TerminationViewport {
            scene: Some(&scene),
            report: None,
            camera,
            rect,
            pointer: Some(egui::pos2(p[0], p[1])),
            clicked: true,
            painter: ui.painter(),
            section: None,
        });
    })
    .drop_without_applying_deltas();
    changed
}

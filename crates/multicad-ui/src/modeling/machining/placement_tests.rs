//! Visual placement and the System 32 tool, driven through the production
//! native scene and the same form values the Inspector edits.
use super::*;
use crate::{
    camera::Camera,
    modeling::{TerminationViewport, native_fixture, panel_face},
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

fn actor() -> Actor {
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
pub(super) fn flat_panel() -> (Session, EntityId) {
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
pub(super) fn carcass() -> (Session, EntityId, EntityId) {
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
/// Hover and click a world point in a top view of the production scene.
fn click(form: &mut MachiningPanel, s: &Session, point: [f32; 3]) -> bool {
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
    ctx.run_ui(Default::default(), |ui| {
        changed = form.viewport(
            s.document(),
            s.revision(),
            TerminationViewport {
                scene: Some(&scene),
                report: None,
                camera,
                rect,
                pointer: Some(egui::pos2(p[0], p[1])),
                clicked: true,
                painter: ui.painter(),
                section: None,
            },
        );
    })
    .drop_without_applying_deltas();
    changed
}

#[test]
fn clicking_a_visible_panel_face_adds_the_target_and_places_and_moves_holes() {
    let (s, side) = flat_panel();
    let mut form = MachiningPanel::new("Cable".into());
    form.kind = Kind::Drill;
    form.holes.clear();
    form.through = true;
    // The top face at z = 18, clicked near (123.4, 57.6): snapped to whole mm.
    assert!(click(&mut form, &s, [123.4, 57.6, 18.]));
    assert_eq!(form.targets, vec![side]);
    assert_eq!(form.side, Side::Top);
    assert_eq!(form.hole_points(), vec![[123., 58.]]);
    // Near the center lines the point snaps onto them.
    assert!(click(&mut form, &s, [298., 203., 18.]));
    assert_eq!(form.hole_points(), vec![[123., 58.], [300., 200.]]);
    // Clicking an existing hole selects it; the next click moves it.
    assert!(click(&mut form, &s, [123., 58., 18.]));
    assert_eq!(form.selected_hole, Some(0));
    assert!(click(&mut form, &s, [450., 100., 18.]));
    assert_eq!(form.hole_points(), vec![[450., 100.], [300., 200.]]);
    // The Inspector values build the same exact cut.
    let prepared = form
        .request(s.document(), true)
        .unwrap()
        .prepare(&s, actor())
        .unwrap();
    let model = Compiler::default()
        .compile(prepared.prepared.document(), 2)
        .unwrap();
    let v = model
        .solid(prepared.outputs[0].feature)
        .unwrap()
        .volume_mm3()
        .unwrap();
    let hole = std::f64::consts::PI * 2.5 * 2.5 * 18.;
    assert!((v - (600. * 400. * 18. - 2. * hole)).abs() < 1e-3);
}

#[test]
fn a_hole_outside_the_face_is_refused_with_the_panel_named() {
    let (s, side) = flat_panel();
    let mut form = MachiningPanel::new("Edge hole".into());
    form.kind = Kind::Drill;
    form.diameter = field(20.);
    form.through = true;
    form.holes.clear();
    assert!(click(&mut form, &s, [3., 200., 18.]));
    // Snapped onto the edge: a 20 mm hole there leaves the face.
    assert_eq!(form.hole_points(), vec![[0., 200.]]);
    let parameters = multicad_core::parameters::evaluate(s.document()).unwrap();
    let resolved = form.resolved(s.document(), &parameters);
    assert_eq!(resolved[0].0, side);
    let error = resolved[0].1.as_ref().unwrap_err();
    assert!(
        error.contains("leaves the 600 × 400 mm face of Side"),
        "{error}"
    );
    // A pocket click sets its center.
    form.kind = Kind::Pocket;
    assert!(click(&mut form, &s, [200., 100., 18.]));
    assert_eq!(form.center[0].text, "200");
    assert_eq!(form.center[1].text, "100");
}

#[test]
fn system32_tool_targets_both_sides_places_rows_and_reopens() {
    let (mut s, left, right) = carcass();
    // Selecting one side seeds both, so opposite rows stay aligned.
    let (panels, refused) = panel_face::selected_panels(s.document(), [left]);
    assert!(refused.is_empty());
    let targets = panel_face::with_opposite_sides(s.document(), panels);
    assert_eq!(targets, vec![left, right]);
    let mut form = MachiningPanel::system32("Shelf pins".into()).with_targets(targets);
    assert_eq!(form.label(), "System 32 shelf pins");
    assert_eq!(form.side, Side::Inside);
    // A click on the right side's inner face (x = 582) 50 mm from the front
    // moves row 1 there; the face stays "inside" for both sides.
    let parameters = multicad_core::parameters::evaluate(s.document()).unwrap();
    let hit = panel_face::locate(s.document(), &parameters, right, [582., 50.2, 300.]).unwrap();
    assert_eq!(hit.face, MachiningFace::Bottom);
    let at = panel_face::snap(hit.at, hit.frame.size, 1., 5.);
    assert!(form.click(s.document(), &parameters, hit, at, |_, _| f32::MAX));
    assert_eq!(form.pin_rows[0].1.text, "50");
    assert_eq!(form.side, Side::Inside);
    // An edge click is explained, not applied.
    let edge = panel_face::locate(s.document(), &parameters, left, [9., 0., 300.]).unwrap();
    assert!(matches!(edge.face, MachiningFace::Edge { .. }));
    form.click(s.document(), &parameters, edge, edge.at, |_, _| f32::MAX);
    assert!(form.pick_note.as_deref().unwrap().contains("inside face"));
    assert_eq!(form.side, Side::Inside);

    let resolved = form.resolved(s.document(), &parameters);
    let faces: Vec<_> = resolved
        .iter()
        .map(|(_, r)| r.as_ref().unwrap().face)
        .collect();
    assert_eq!(faces, vec![MachiningFace::Top, MachiningFace::Bottom]);
    assert!(
        resolved
            .iter()
            .all(|(_, r)| r.as_ref().unwrap().holes.len() == 34)
    );
    let prepared = form
        .request(s.document(), true)
        .unwrap()
        .prepare(&s, actor())
        .unwrap();
    let cut = prepared.outputs[0].feature;
    s.commit(prepared.prepared).unwrap();
    let edit = MachiningPanel::existing(s.document(), cut).unwrap();
    assert_eq!(edit.kind, Kind::ShelfPins);
    assert_eq!(edit.side, Side::Inside);
    assert_eq!(edit.targets, vec![left, right]);
    assert_eq!(edit.pin_rows[0].1.text, "50");
    assert_eq!(edit.pin_rows[1].0, RowEdge::End);
    // Targets are fixed on an applied operation: another panel is refused.
    let mut edit = edit;
    let parameters = multicad_core::parameters::evaluate(s.document()).unwrap();
    let top = model::carcass_panels(
        s.document(),
        model::read_panel(
            s.document(),
            model::panel_feature(s.document(), left).unwrap(),
        )
        .map(|p| match p.source {
            PanelSource::Carcass { carcass, .. } => carcass,
            _ => unreachable!(),
        })
        .unwrap(),
    )
    .unwrap()
    .into_iter()
    .find(|(r, ..)| *r == PanelRole::Top)
    .unwrap()
    .2;
    let hit = panel_face::locate(s.document(), &parameters, top, [300., 200., 720.]).unwrap();
    assert!(!edit.click(s.document(), &parameters, hit, hit.at, |_, _| f32::MAX));
    assert!(edit.pick_note.as_deref().unwrap().contains("fixed"));
}

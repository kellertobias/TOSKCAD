//! CAD-163: selecting a generated cabinet's group exposes Edit cabinet, which
//! reopens the saved carcass instead of starting Cabinet 2.
use super::*;
use crate::viewport::{SceneBackend, SceneFeed, SceneReply, SceneStamp};
use multicad_core::{
    document::Entity,
    materials::{Grain, LibraryDefinition, Stock, StockKind, prepare_library},
    model,
};
use std::collections::BTreeSet;

/// Accepts requests and never replies: the edit path must work source-only.
#[derive(Default)]
struct Silent;
impl SceneBackend for Silent {
    fn request(&mut self, _: &Document, _: SceneStamp) -> Result<(), String> {
        Ok(())
    }
    fn poll(&mut self) -> Option<SceneReply> {
        None
    }
}

pub(super) fn cabinet() -> (Session, EntityId, EntityId) {
    let mut s = Session::new(Document::new("Cabinet edit").unwrap()).unwrap();
    let birch = Entity::new(
        "Birch",
        EntityData::Material {
            density_kg_m3: None,
            color_rgba: [210, 180, 140, 255],
            section_hatch: None,
        },
    );
    let stock = |t: f64, name: &str| {
        LibraryDefinition::Stock(Stock {
            material: birch.id,
            specification: StockKind::Panel {
                thickness_mm: t,
                length_mm: 2500.,
                width_mm: 1250.,
                grain: Grain::AlongLength,
            },
        })
        .entity(EntityId::new(), name)
        .unwrap()
    };
    let (ply18, ply6) = (stock(18., "Ply 18"), stock(6., "Ply 6"));
    let entities = [birch.clone(), ply18.clone(), ply6.clone()];
    s.commit(prepare_library(&s, 0, Actor::local(Channel::Ui), &entities).unwrap())
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
            stock: ply18.id,
            construction: Default::default(),
            back: model::Back::Inset {
                stock: ply6.id,
                recess: model::length(10.),
            },
            shelves: model::Shelves {
                layout: model::ShelfLayout::Even { count: 1 },
                setback: model::length(20.),
                clearance: model::length(1.),
            },
            front_band: None,
            group: None,
        },
    );
    let p = model::prepare_carcass(&s, 1, Actor::local(Channel::Ui), &request).unwrap();
    s.commit(p.model.prepared).unwrap();
    let group = model::read_carcass(s.document(), request.carcass)
        .unwrap()
        .group
        .unwrap();
    (s, request.carcass, group)
}

fn texts(output: &egui::FullOutput) -> Vec<(String, Pos2)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, Pos2)>) {
        match shape {
            egui::Shape::Text(t) => {
                out.push((t.galley.text().to_owned(), t.pos + t.galley.size() * 0.5))
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = vec![];
    output.shapes.iter().for_each(|s| walk(&s.shape, &mut out));
    out
}
fn frame(
    context: &egui::Context,
    editor: &mut SketchEditor,
    session: &mut Session,
    feed: &mut SceneFeed,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400., 1000.))),
            events,
            ..Default::default()
        },
        |ui| editor.show(ui, session, false, feed),
    );
    output.textures_delta.clear();
    output
}

#[test]
fn selected_cabinet_group_offers_edit_cabinet_and_opens_the_saved_carcass() {
    let (mut session, carcass, group) = cabinet();
    let before = session.document().clone();
    let mut feed = SceneFeed::default();
    feed.set_backend(Box::<Silent>::default());
    feed.tick(session.document(), session.revision());
    let mut editor = SketchEditor::default();
    editor.set_model_backend(Box::<Silent>::default());
    editor.tree_selection = BTreeSet::from([group]);
    // A group has no applied feature of its own; the cabinet action finds it.
    assert_eq!(editor.selected_applied_feature(session.document()), None);
    let target = editor.selected_carcass(session.document()).unwrap();
    assert_eq!(target.carcass, carcass);
    let context = egui::Context::default();
    let label = "Edit cabinet \u{201c}Cabinet 1\u{201d}";
    let output = frame(&context, &mut editor, &mut session, &mut feed, vec![]);
    let position = texts(&output)
        .into_iter()
        .find(|(t, _)| t == label)
        .map(|(_, p)| p)
        .unwrap_or_else(|| panic!("Missing Inspector action {label}"));
    for pressed in [true, false] {
        frame(
            &context,
            &mut editor,
            &mut session,
            &mut feed,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    assert!(editor.model_active(), "{}", editor.message);
    // Frame the 600 × 720 × 560 mm cabinet so the guide labels are on screen.
    editor.camera.target = [300., 280., 360.];
    editor.camera.scale = 0.4;
    let output = frame(&context, &mut editor, &mut session, &mut feed, vec![]);
    let shown: Vec<_> = texts(&output).into_iter().map(|(t, _)| t).collect();
    assert!(
        shown.iter().any(|t| t.starts_with("Edit the carcass")),
        "the carcass edit form opened: {shown:?}"
    );
    assert!(shown.iter().any(|t| t.starts_with("Keeps 6 panel(s)")));
    // The viewport draws the saved envelope dimensions while editing.
    for dimension in ["Width 600 mm", "Height 720 mm", "Depth 560 mm"] {
        assert!(
            shown.iter().any(|t| t == dimension),
            "{dimension}: {shown:?}"
        );
    }
    editor.cancel_model();
    assert_eq!(session.document(), &before);
    assert_eq!(model::carcasses(session.document()).count(), 1);
}

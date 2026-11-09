use super::*;
use crate::{
    modeling::{ModelingTool, native_fixture::NativeBackend},
    viewport::{PickFilter, PickOptions, SceneFeed},
};
use multicad_core::{
    model::{self, PrimitiveParameters, PrimitiveRequest, length},
    references::{AnalyticGeometry, TopologyKind},
};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};
struct Host {
    session: Session,
    editor: SketchEditor,
    feed: SceneFeed,
    ctx: egui::Context,
    body: EntityId,
    producer: EntityId,
}

#[test]
fn sided_chamfer_face_click_does_not_replace_the_edge_selection() {
    let mut h = Host::new();
    let original = h.session.document().clone();
    h.editor.tree_selection = BTreeSet::from([h.body]);
    h.editor
        .activate_model_tool(
            &h.session,
            &Default::default(),
            &h.feed,
            ModelingTool::EdgeTreatment,
            Channel::Ui,
        )
        .unwrap();
    h.frame(vec![]);
    let (point, edge_key) = h.edges()[0];
    h.click(point, false);
    let selected = h.editor.model.edge_treatment_edges().unwrap().to_vec();
    h.click_label("Chamfer");
    h.click_label("Symmetric distances");
    h.click_label("Two distances");
    h.click_label("Choose side face for edge 1");
    assert_eq!(
        h.editor.model.edge_treatment_pick_kind(),
        Some(TopologyKind::Face)
    );
    let scene = h.editor.edge_source_scene(&h.session, &h.feed).unwrap();
    let snapshot = scene.geometry_snapshot(h.producer).unwrap();
    let edge = snapshot
        .elements
        .iter()
        .find(|e| e.key == edge_key)
        .unwrap();
    let Some(AnalyticGeometry::Line { a_mm, .. }) = edge.geometry else {
        panic!("Expected a vertical edge")
    };
    let rect = h.editor.viewport_rect.unwrap();
    let point = [[0., 10., 5.], [30., 10., 5.], [15., 0., 5.], [15., 20., 5.]]
        .into_iter()
        .filter(|p| p[0] == a_mm[0] || p[1] == a_mm[1])
        .find_map(|p| {
            let p = h.editor.camera.project(p.map(|v| v as f32), rect);
            let p = Pos2::new(p[0], p[1]);
            scene
                .pick_screen_with(
                    h.editor.camera,
                    rect,
                    p,
                    PickOptions {
                        filter: PickFilter::Faces,
                        include_hidden: false,
                        section: None,
                    },
                )
                .map(|_| p)
        })
        .expect("An adjacent native side face must be visible");
    h.click(point, false);
    assert_eq!(h.editor.model.edge_treatment_edges().unwrap(), selected);
    assert_eq!(h.session.document(), &original);
    if h.editor.model.edge_treatment_picking() {
        h.click_label("Finish picking edges");
    }
    h.ready();
    h.click_label("Apply");
    let native = multicad_model::Compiler::default()
        .compile(h.session.document(), h.session.revision())
        .unwrap();
    let volume = native.solid(h.body).unwrap().volume_mm3().unwrap();
    assert!(volume > 0. && volume < 6000.);
    let EntityData::Part { features, .. } = &h.session.document().get(h.body).unwrap().data else {
        panic!()
    };
    let (saved, _) =
        model::read_edge_treatment(h.session.document(), *features.last().unwrap()).unwrap();
    assert!(matches!(
        saved.chamfer,
        Some(model::ChamferSettings::TwoDistances { .. })
    ));
}

#[test]
fn shell_tool_first_face_selection_shift_toggle_and_native_apply_are_source_bound() {
    use crate::modeling::AdvancedKind;
    let mut h = Host::new();
    let original = h.session.document().clone();
    let revision = h.session.revision();
    h.editor
        .activate_model_tool(
            &h.session,
            &Default::default(),
            &h.feed,
            ModelingTool::Advanced(AdvancedKind::Shell),
            Channel::Ui,
        )
        .unwrap();
    assert!(h.editor.awaiting_model_targets());
    h.click_label("Select Body Stock");
    assert!(h.editor.model.advanced_active());
    assert_eq!(h.editor.model.advanced_input().unwrap().body, h.body);
    assert!(h.editor.model.advanced_picking());
    let source = h.editor.advanced_source_scene(&h.session, &h.feed).unwrap();
    assert!(!source.is_full_model());
    assert_eq!(source.instances()[0].geometry, h.producer);
    let rect = h.editor.viewport_rect.unwrap();
    let point = h.editor.camera.project([15., 10., 10.], rect);
    let point = Pos2::new(point[0], point[1]);
    h.click(point, false);
    let capture = h.editor.model.advanced_captures().unwrap()[0].clone();
    assert_eq!(capture.kind, TopologyKind::Face);
    h.click(point, true);
    assert!(h.editor.model.advanced_captures().unwrap().is_empty());
    h.click(point, false);
    h.click_label("Finish picking faces");
    h.ready();
    assert_eq!(h.session.document(), &original);
    h.click_label("Apply");
    assert_eq!(h.session.revision(), revision + 1);
    let native = multicad_model::Compiler::default()
        .compile(h.session.document(), h.session.revision())
        .unwrap();
    let solid = native.solid(h.body).unwrap();
    assert_eq!(solid.solid_count().unwrap(), 1);
    let volume = solid.volume_mm3().unwrap();
    assert!(
        volume > 0. && volume < 6000.,
        "Shell must change exact solid geometry: {volume}"
    );
    let feature = match &h.session.document().get(h.body).unwrap().data {
        EntityData::Part { features, .. } => *features.last().unwrap(),
        _ => panic!("Expected the original Part"),
    };
    let applied = h.session.document().clone();
    h.settle();
    h.editor
        .begin_feature_edit(
            &h.session,
            &Default::default(),
            &h.feed,
            feature,
            Channel::Ui,
        )
        .unwrap();
    h.frame(vec![]);
    assert_eq!(
        h.editor.model.advanced_input().unwrap().producer,
        h.producer
    );
    h.click_label("Cancel");
    assert_eq!(h.session.document(), &applied);
    h.session
        .undo(h.session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    assert_eq!(h.session.document(), &original);
}

#[test]
fn sweep_and_loft_open_tool_first_without_selecting_an_unrelated_solid() {
    use crate::modeling::AdvancedKind;
    let mut h = Host::new();
    let original = h.session.document().clone();
    let revision = h.session.revision();
    for kind in [AdvancedKind::Sweep, AdvancedKind::Loft] {
        h.editor
            .activate_model_tool(
                &h.session,
                &Default::default(),
                &h.feed,
                ModelingTool::Advanced(kind),
                Channel::Ui,
            )
            .unwrap();
        h.frame(vec![]);
        assert!(h.editor.model.advanced_active());
        assert!(!h.editor.awaiting_model_targets());
        assert!(h.editor.model.advanced_input().is_none());
        assert!(!h.editor.model_can_apply(&h.session, &Default::default()));
        h.click_label("Cancel");
        assert_eq!(h.session.document(), &original);
        assert_eq!(h.session.revision(), revision);
    }
}
impl Host {
    fn new() -> Self {
        let mut session =
            Session::new(Document::new("Native Part edge acquisition").unwrap()).unwrap();
        let request = PrimitiveRequest::new(
            "Stock",
            PrimitiveParameters::Box {
                size: [length(30.), length(20.), length(10.)],
                frame: Default::default(),
            },
        );
        session
            .commit(
                model::prepare_primitive(&session, 0, Actor::local(Channel::Ui), &request)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let mut feed = SceneFeed::default();
        feed.set_backend(Box::new(NativeBackend::new()));
        let until = Instant::now() + Duration::from_secs(5);
        while !feed.is_current(session.document().id(), session.revision())
            && Instant::now() < until
        {
            feed.tick(session.document(), session.revision());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(feed.is_current(session.document().id(), session.revision()));
        let mut editor = SketchEditor::default();
        editor.set_model_backend(Box::new(NativeBackend::new()));
        editor.grid = false;
        let mut host = Self {
            session,
            editor,
            feed,
            ctx: Default::default(),
            body: request.body,
            producer: request.feature,
        };
        host.ctx.enable_accesskit();
        for _ in 0..5 {
            host.frame(vec![]);
        }
        host.editor.camera = Camera {
            yaw: -0.6,
            pitch: 0.6,
            scale: 6.,
            target: [15., 10., 5.],
            perspective: false,
        };
        host.frame(vec![]);
        host
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.render(events).drop_without_applying_deltas();
    }
    fn render(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1360., 860.))),
                events,
                ..Default::default()
            },
            |ui| {
                self.editor.show_project(
                    ui,
                    &mut self.session,
                    false,
                    &mut self.feed,
                    &Default::default(),
                )
            },
        )
    }
    fn click_label(&mut self, label: &str) {
        self.frame(vec![]);
        let out = self.render(vec![]);
        let pos = out
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .or_else(|| {
                out.platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .find(|(_, node)| node.label() == Some(label))
                    .and_then(|(_, node)| node.bounds())
                    .map(|bounds| {
                        Pos2::new(
                            ((bounds.x0 + bounds.x1) / 2.) as f32,
                            ((bounds.y0 + bounds.y1) / 2.) as f32,
                        )
                    })
            })
            .unwrap_or_else(|| panic!("Missing {label}"));
        out.drop_without_applying_deltas();
        self.click(pos, false);
    }
    fn ready(&mut self) {
        let until = Instant::now() + Duration::from_secs(5);
        while !self
            .editor
            .model_can_apply(&self.session, &Default::default())
            && Instant::now() < until
        {
            self.frame(vec![]);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            self.editor
                .model_can_apply(&self.session, &Default::default()),
            "{:?}",
            self.editor.model.error()
        );
    }
    fn settle(&mut self) {
        let until = Instant::now() + Duration::from_secs(5);
        while !self
            .feed
            .is_current(self.session.document().id(), self.session.revision())
            && Instant::now() < until
        {
            self.feed
                .tick(self.session.document(), self.session.revision());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            self.feed
                .is_current(self.session.document().id(), self.session.revision()),
            "{:?}",
            self.feed.error()
        );
        self.frame(vec![]);
    }
    fn click(&mut self, pos: Pos2, shift: bool) {
        let modifiers = egui::Modifiers {
            shift,
            ..Default::default()
        };
        self.frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers,
            },
        ]);
        self.frame(vec![
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            },
            egui::Event::ModifiersChanged(egui::Modifiers::NONE),
        ]);
    }
    fn edges(&self) -> Vec<(Pos2, multicad_core::references::TopologyKey)> {
        let scene = self
            .editor
            .edge_source_scene(&self.session, &self.feed)
            .unwrap();
        let rect = self.editor.viewport_rect.unwrap();
        scene
            .geometry_snapshot(self.producer)
            .unwrap()
            .elements
            .iter()
            .filter_map(|element| {
                let Some(AnalyticGeometry::Line { a_mm, b_mm }) = element.geometry else {
                    return None;
                };
                if element.key.kind != TopologyKind::Edge
                    || a_mm[0] != b_mm[0]
                    || a_mm[1] != b_mm[1]
                {
                    return None;
                }
                let p = self.editor.camera.project(
                    std::array::from_fn(|i| ((a_mm[i] + b_mm[i]) / 2.) as f32),
                    rect,
                );
                let p = Pos2::new(p[0], p[1]);
                let hit = scene.pick_screen_with(
                    self.editor.camera,
                    rect,
                    p,
                    PickOptions {
                        filter: PickFilter::Edges,
                        include_hidden: false,
                        section: None,
                    },
                )?;
                (hit.key == element.key && rect.shrink(16.).contains(p)).then_some((p, element.key))
            })
            .collect()
    }
}

#[test]
fn actual_inspector_native_fillet_then_saved_chamfer_apply_and_cancel_use_the_frozen_input() {
    let mut h = Host::new();
    let original = h.session.document().clone();
    let revision = h.session.revision();
    h.editor.tree_selection = BTreeSet::from([h.body]);
    h.editor
        .activate_model_tool(
            &h.session,
            &Default::default(),
            &h.feed,
            ModelingTool::EdgeTreatment,
            Channel::Ui,
        )
        .unwrap();
    h.frame(vec![]);
    let point = h.edges()[0].0;
    h.click(point, false);
    h.click_label("Finish picking edges");
    assert!(!h.editor.model.edge_treatment_picking());
    h.ready();
    assert_eq!(h.session.document(), &original);
    h.click_label("Apply");
    assert_eq!(h.session.revision(), revision + 1);
    assert!(!h.editor.model_active());
    let feature = match &h.session.document().get(h.body).unwrap().data {
        EntityData::Part { features, .. } => *features.last().unwrap(),
        _ => panic!(),
    };
    let saved = model::read_edge_treatment(h.session.document(), feature).unwrap();
    assert_eq!(
        saved.0.parameters.profile,
        model::EdgeTreatmentProfile::Fillet
    );
    let native = multicad_model::Compiler::default()
        .compile(h.session.document(), h.session.revision())
        .unwrap();
    assert!(
        (native.solid(h.body).unwrap().volume_mm3().unwrap()
            - (6000. - (4. - std::f64::consts::PI) * 10.))
            .abs()
            < 1e-5
    );
    h.settle();
    let basis = crate::modeling::object_gizmo_basis(
        h.session.document(),
        h.session.revision(),
        h.feed.visible().unwrap(),
        &BTreeSet::from([h.body]),
    )
    .unwrap();
    assert_eq!(basis.basis, crate::modeling::GizmoBasis::default());
    let applied = h.session.document().clone();
    h.editor
        .begin_feature_edit(
            &h.session,
            &Default::default(),
            &h.feed,
            feature,
            Channel::Ui,
        )
        .unwrap();
    h.frame(vec![]);
    assert!(!h.editor.model.edge_treatment_picking());
    h.click_label("Select / reselect edges");
    assert!(h.editor.model.edge_treatment_picking());
    assert_eq!(
        h.editor
            .edge_source_scene(&h.session, &h.feed)
            .unwrap()
            .instances()[0]
            .geometry,
        h.producer
    );
    h.click_label("Cancel");
    assert_eq!(h.session.document(), &applied);
    assert_eq!(h.session.revision(), revision + 1);
    h.editor
        .begin_feature_edit(
            &h.session,
            &Default::default(),
            &h.feed,
            feature,
            Channel::Ui,
        )
        .unwrap();
    h.frame(vec![]);
    h.click_label("Chamfer");
    h.ready();
    h.click_label("Apply");
    assert_eq!(h.session.revision(), revision + 2);
    let edited = model::read_edge_treatment(h.session.document(), feature).unwrap();
    assert_eq!(edited.0.input.producer, h.producer);
    assert_eq!(
        edited.0.parameters.profile,
        model::EdgeTreatmentProfile::Chamfer
    );
    assert_eq!(edited.1, saved.1);
    let native = multicad_model::Compiler::default()
        .compile(h.session.document(), h.session.revision())
        .unwrap();
    assert!((native.solid(h.body).unwrap().volume_mm3().unwrap() - 5980.).abs() < 1e-5);
    h.session
        .undo(h.session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    assert_eq!(h.session.document(), &applied);
}
#[test]
fn native_tool_first_part_edges_shift_toggle_and_cancel_preserve_canonical_source() {
    let mut h = Host::new();
    let original = h.session.document().clone();
    let revision = h.session.revision();
    h.editor
        .activate_model_tool(
            &h.session,
            &Default::default(),
            &h.feed,
            ModelingTool::EdgeTreatment,
            Channel::Ui,
        )
        .unwrap();
    assert!(h.editor.awaiting_model_targets());
    assert!(h.editor.message.contains("Part"));
    let rect = h.editor.viewport_rect.unwrap();
    let point = h.editor.camera.project([15., 10., 10.], rect);
    h.click(Pos2::new(point[0], point[1]), false);
    assert!(h.editor.model.edge_treatment_active());
    assert!(h.editor.model.edge_treatment_edges().unwrap().is_empty());
    assert_eq!(h.editor.model.edge_treatment_input().unwrap().body, h.body);
    let source = h.editor.edge_source_scene(&h.session, &h.feed).unwrap();
    assert_eq!(source.instances().len(), 1);
    assert_eq!(source.instances()[0].geometry, h.producer);
    assert!(!source.is_full_model());
    let edges = h.edges();
    assert!(edges.len() >= 2, "need two visible native vertical edges");
    h.click(edges[0].0, false);
    let first = h.editor.model.edge_treatment_edges().unwrap()[0].clone();
    h.click(edges[1].0, true);
    assert_eq!(h.editor.model.edge_treatment_edges().unwrap().len(), 2);
    assert_eq!(h.editor.model.edge_treatment_edges().unwrap()[0], first);
    h.click(edges[0].0, true);
    let second = h.editor.model.edge_treatment_edges().unwrap()[0].clone();
    assert_ne!(second.id, first.id);
    h.click(edges[1].0, true);
    assert!(h.editor.model.edge_treatment_edges().unwrap().is_empty());
    assert!(!h.editor.model_can_apply(&h.session, &Default::default()));
    h.editor.cancel_model();
    assert_eq!(h.session.document(), &original);
    assert_eq!(h.session.revision(), revision);
    assert!(!h.editor.model_interaction_active());
    h.editor.tree_selection = BTreeSet::from([h.body]);
    h.editor
        .activate_model_tool(
            &h.session,
            &Default::default(),
            &h.feed,
            ModelingTool::EdgeTreatment,
            Channel::Ui,
        )
        .unwrap();
    assert!(h.editor.model.edge_treatment_active());
    h.editor.cancel_model();
    assert_eq!(h.session.document(), &original);
}

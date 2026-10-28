//! Assembly commands through the actual SketchEditor ownership and native feed.
use super::*;
use crate::{
    assembly::AssemblyTool,
    modeling::native_fixture::NativeBackend,
    viewport::{Ray, SceneFeed},
};
use multicad_core::model::{self, *};
use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, Instant},
};
struct Host {
    session: Session,
    editor: SketchEditor,
    feed: SceneFeed,
    ctx: egui::Context,
    part: EntityId,
    instance: EntityId,
    joint: EntityId,
}
impl Host {
    fn new() -> Self {
        let mut s = Session::new(Document::new("Assembly host").unwrap()).unwrap();
        let p = PrimitiveRequest::new(
            "Panel",
            PrimitiveParameters::Box {
                size: [length(30.), length(20.), length(10.)],
                frame: Default::default(),
            },
        );
        s.commit(
            prepare_primitive(&s, 0, Actor::local(Channel::Ui), &p)
                .unwrap()
                .prepared,
        )
        .unwrap();
        let i = ComponentPlacementRequest::new(
            p.body,
            NumericTransform {
                translation: [length(60.), length(0.), length(0.)],
                ..Default::default()
            },
        );
        s.commit(
            prepare_place_component(&s, s.revision(), Actor::local(Channel::Ui), &i)
                .unwrap()
                .prepared,
        )
        .unwrap();
        let j = JointRequest::new(
            i.instance,
            JointParameters {
                kind: JointKind::Slider,
                target: JointAnchor::Frame {
                    frame: Box::new(PrimitiveFrame {
                        origin: [length(60.), length(0.), length(0.)],
                        ..Default::default()
                    }),
                },
                ..Default::default()
            },
            vec![],
        );
        s.commit(
            prepare_joint(&s, s.revision(), Actor::local(Channel::Ui), &j)
                .unwrap()
                .prepared,
        )
        .unwrap();
        let mut feed = SceneFeed::default();
        feed.set_backend(Box::new(NativeBackend::new()));
        let until = Instant::now() + Duration::from_secs(5);
        while !feed.is_current(s.document().id(), s.revision()) && Instant::now() < until {
            feed.tick(s.document(), s.revision());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            feed.is_current(s.document().id(), s.revision()),
            "{:?}",
            feed.error()
        );
        let mut editor = SketchEditor::default();
        editor.set_model_backend(Box::new(NativeBackend::new()));
        editor.set_assembly_backend(Box::new(NativeBackend::new()));
        Self {
            session: s,
            editor,
            feed,
            ctx: Default::default(),
            part: p.body,
            instance: i.instance,
            joint: j.driver,
        }
    }
    fn start(&mut self, tool: AssemblyTool) {
        self.editor
            .begin_assembly_tool(
                &self.session,
                &Default::default(),
                &self.feed,
                tool,
                Channel::Ui,
            )
            .unwrap();
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
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
    fn click(&mut self, label: &str) {
        self.frame(vec![]).drop_without_applying_deltas();
        let out = self.frame(vec![]);
        let pos = out
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == label => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing {label}"));
        out.drop_without_applying_deltas();
        self.click_at(pos);
    }
    fn click_at(&mut self, pos: Pos2) {
        for pressed in [true, false] {
            self.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ])
            .drop_without_applying_deltas();
        }
    }
}
#[test]
fn assembly_host_tool_first_all_modes_cancel_without_source_or_history_edits() {
    let mut h = Host::new();
    let source = h.session.document().clone();
    let revision = h.session.revision();
    for tool in AssemblyTool::ALL {
        h.start(tool);
        assert!(h.editor.model_active());
        assert_eq!(h.editor.has_pending_edits(), tool.authoring());
        h.frame(vec![]).drop_without_applying_deltas();
        assert!(!h.editor.model_can_apply(&h.session, &Default::default()));
        h.editor.cancel_model();
        assert!(!h.editor.model_interaction_active());
        assert_eq!(h.session.document(), &source);
        assert_eq!(h.session.revision(), revision);
    }
    h.editor.measurement.open();
    assert!(
        h.editor
            .begin_assembly_tool(
                &h.session,
                &Default::default(),
                &h.feed,
                AssemblyTool::Place,
                Channel::Ui
            )
            .is_err()
    );
    assert!(h.editor.measurement.active());
}
#[test]
fn assembly_host_selected_instance_edits_the_saved_joint_and_fixed_footer_applies_once() {
    let mut h = Host::new();
    let source = h.session.document().clone();
    let revision = h.session.revision();
    h.editor.tree_selection = BTreeSet::from([h.instance]);
    assert_eq!(
        h.editor.assembly_edit_actions(h.session.document()),
        vec![(h.joint, "Edit relationship")]
    );
    h.click("Edit relationship");
    assert!(h.editor.assembly.active());
    h.click("Moving local anchor");
    h.click("Pick face");
    assert_eq!(
        h.editor.assembly.anchor_pick().unwrap().instance,
        h.instance
    );
    let rect = h.editor.viewport_rect.unwrap();
    let pixel = h.editor.camera.project([75., 10., 10.], rect);
    let pos = Pos2::new(pixel[0], pixel[1]);
    assert_eq!(
        h.feed
            .visible()
            .unwrap()
            .pick_screen_with(
                h.editor.camera,
                rect,
                pos,
                crate::viewport::PickOptions {
                    filter: crate::viewport::PickFilter::Faces,
                    include_hidden: false,
                    section: None
                }
            )
            .unwrap()
            .instance,
        h.instance
    );
    h.click_at(pos);
    assert!(
        h.editor.assembly.anchor_pick().is_none(),
        "{}",
        h.editor.message
    );
    h.click("Preview");
    let until = Instant::now() + Duration::from_secs(5);
    while !h.editor.model_can_apply(&h.session, &Default::default()) && Instant::now() < until {
        h.frame(vec![]).drop_without_applying_deltas();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(h.editor.model_can_apply(&h.session, &Default::default()));
    h.click("Apply");
    assert_eq!(h.session.revision(), revision + 1);
    assert!(!h.editor.model_active());
    assert_eq!(h.editor.tree_selection, BTreeSet::from([h.instance]));
    assert_eq!(
        h.editor.assembly_edit_actions(h.session.document()),
        vec![(h.joint, "Edit relationship")]
    );
    assert_eq!(h.session.document().get(h.part), source.get(h.part));
    assert!(model::read_joint(h.session.document(), h.joint).is_ok());
    h.session
        .undo(h.session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    assert_eq!(h.session.document(), &source);
}
#[test]
fn assembly_host_explode_is_cached_source_neutral_and_cannot_authorize_geometry_picks() {
    let mut h = Host::new();
    let source = h.session.document().clone();
    h.start(AssemblyTool::Explode);
    let canonical = h.feed.visible().unwrap().clone();
    let first = h
        .editor
        .assembly_exploded_scene(&h.session, &Default::default(), &canonical)
        .unwrap()
        .unwrap();
    let second = h
        .editor
        .assembly_exploded_scene(&h.session, &Default::default(), &canonical)
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert!(!first.is_full_model());
    assert!(first.assembly_evaluation().is_none());
    assert!(
        first
            .pick_face(Ray::new([75., 10., 100.], [0., 0., -1.]).unwrap())
            .is_none()
    );
    assert!(!h.editor.has_pending_edits());
    assert_eq!(h.session.document(), &source);
    h.editor.cancel_model();
    assert!(
        h.editor
            .assembly_exploded_scene(&h.session, &Default::default(), &canonical)
            .unwrap()
            .is_none()
    );
}

#[test]
fn assembly_host_nested_member_anchor_pick_uses_its_native_source_not_outer_copy_ids() {
    let mut h = Host::new();
    let definition = AssemblyRequest::new("Reusable module", vec![h.instance], vec![h.part]);
    h.session
        .commit(
            prepare_assembly(
                &h.session,
                h.session.revision(),
                Actor::local(Channel::Ui),
                &definition,
            )
            .unwrap()
            .prepared,
        )
        .unwrap();
    let outer = ComponentPlacementRequest::new(
        definition.assembly,
        NumericTransform {
            translation: [length(1000.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    h.session
        .commit(
            prepare_place_component(
                &h.session,
                h.session.revision(),
                Actor::local(Channel::Ui),
                &outer,
            )
            .unwrap()
            .prepared,
        )
        .unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    while !h
        .feed
        .is_current(h.session.document().id(), h.session.revision())
        && Instant::now() < until
    {
        h.feed.tick(h.session.document(), h.session.revision());
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        h.feed
            .is_current(h.session.document().id(), h.session.revision())
    );
    assert_eq!(h.feed.visible().unwrap().instances()[0].id, outer.instance);
    let baseline = h.session.document().clone();
    h.editor.tree_selection = BTreeSet::from([h.instance]);
    h.click("Edit relationship");
    h.click("Moving local anchor");
    h.click("Pick face");
    let source = h
        .editor
        .assembly_anchor_scene(&h.session, &h.feed)
        .unwrap()
        .unwrap();
    assert!(!source.is_full_model());
    assert!(source.model_evaluation().is_none());
    assert_eq!(source.instances()[0].id, h.instance);
    let rect = h.editor.viewport_rect.unwrap();
    let pixel = h.editor.camera.project([75., 10., 10.], rect);
    let pos = Pos2::new(pixel[0], pixel[1]);
    assert_eq!(
        source
            .pick_screen_with(
                h.editor.camera,
                rect,
                pos,
                crate::viewport::PickOptions {
                    filter: crate::viewport::PickFilter::Faces,
                    include_hidden: false,
                    section: None
                }
            )
            .unwrap()
            .instance,
        h.instance
    );
    h.click_at(pos);
    assert!(
        h.editor.assembly.anchor_pick().is_none(),
        "{}",
        h.editor.message
    );
    h.click("Preview");
    let until = Instant::now() + Duration::from_secs(5);
    while !h.editor.model_can_apply(&h.session, &Default::default()) && Instant::now() < until {
        h.frame(vec![]).drop_without_applying_deltas();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(h.editor.model_can_apply(&h.session, &Default::default()));
    h.click("Apply");
    let (saved, _) = model::read_joint(h.session.document(), h.joint).unwrap();
    assert!(matches!(
        saved.parameters.moving,
        JointAnchor::Geometry { .. }
    ));
    assert_eq!(h.session.document().get(h.part), baseline.get(h.part));
    assert_eq!(
        h.session.document().get(outer.instance),
        baseline.get(outer.instance)
    );
    assert_eq!(h.editor.tree_selection, BTreeSet::from([h.instance]));
}

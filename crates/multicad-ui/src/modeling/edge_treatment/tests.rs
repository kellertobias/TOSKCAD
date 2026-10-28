//! Native input topology and typed source-intent checks. Source preparation here
//! does not establish that an edge-treated output is geometrically valid.
use super::*;
use crate::{
    egui::{Context, Event, Modifiers, PointerButton, Pos2, Rect},
    inspector_actions::{self, Owner},
    modeling::{ModelToolController, native_fixture},
};
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Entity, EntityData},
    model::{PrimitiveParameters, PrimitiveRequest},
    parameters::BoundExpression,
};
use multicad_model::Compiler;
use std::sync::Arc;
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn fixture() -> (
    Session,
    Arc<Scene>,
    EntityId,
    Vec<GeometryIntent>,
    GeometryIntent,
) {
    let mut d = Document::new("Edge Inspector source").unwrap();
    d.insert(Entity::new(
        "radius",
        EntityData::Parameter {
            expression: "1.5".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    ))
    .unwrap();
    let mut s = Session::new(d).unwrap();
    let mut source = None;
    let mut other = None;
    for name in ["Stock", "Other stock"] {
        let p = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: [model::length(20.), model::length(16.), model::length(12.)],
                frame: Default::default(),
            },
        );
        s.commit(
            model::prepare_primitive(&s, s.revision(), actor(), &p)
                .unwrap()
                .prepared,
        )
        .unwrap();
        if source.is_none() {
            source = Some(p)
        } else {
            other = Some(p)
        }
    }
    let source = source.unwrap();
    let other = other.unwrap();
    let native = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    let scene = native_fixture::scene(s.document(), native.snapshot(), 0).unwrap();
    let snapshot = scene.geometry_snapshot(source.feature).unwrap();
    let edges = snapshot
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Edge)
        .take(3)
        .map(|e| GeometryIntent::capture(&snapshot, e.key).unwrap())
        .collect::<Vec<_>>();
    let snapshot = scene.geometry_snapshot(other.feature).unwrap();
    let foreign = GeometryIntent::capture(
        &snapshot,
        snapshot
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Edge)
            .unwrap()
            .key,
    )
    .unwrap();
    (s, scene, source.body, edges, foreign)
}
fn frame(
    ctx: &Context,
    p: &mut EdgeTreatmentPanel,
    s: &Session,
    events: Vec<Event>,
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(260., 300.))),
            focused: true,
            events,
            ..Default::default()
        },
        |ui| {
            inspector_actions::scope(
                ui,
                Some(Owner::Model),
                crate::registry::Mode::Model.color(),
                |ui| {
                    p.show(ui, s.document(), true, PreviewState::Idle);
                },
            );
        },
    )
}
fn click(ctx: &Context, p: &mut EdgeTreatmentPanel, s: &Session, label: &str) {
    frame(ctx, p, s, vec![]).drop_without_applying_deltas();
    let out = frame(ctx, p, s, vec![]);
    let b = out
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some(label))
        .and_then(|(_, n)| n.bounds())
        .unwrap_or_else(|| panic!("Missing {label}"));
    let pos = Pos2::new(((b.x0 + b.x1) * 0.5) as f32, ((b.y0 + b.y1) * 0.5) as f32);
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        frame(
            ctx,
            p,
            s,
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        )
        .drop_without_applying_deltas();
    }
}
#[test]
fn empty_edges_are_editable_but_no_request_and_frozen_native_captures_are_checked() {
    let (s, scene, body, edges, foreign) = fixture();
    let before = s.document().clone();
    let mut p = EdgeTreatmentPanel::new(s.document(), s.revision(), &scene, body, vec![]).unwrap();
    assert!(p.picking());
    assert!(p.request(s.document(), true).is_err());
    assert!(p.set_edges(&scene, &[foreign]).is_err());
    let mut duplicate = edges[0].clone();
    duplicate.id = EntityId::new();
    assert!(p.set_edges(&scene, &[edges[0].clone(), duplicate]).is_err());
    let mut wrong = edges[0].clone();
    wrong.kind = TopologyKind::Face;
    assert!(p.set_edges(&scene, &[wrong]).is_err());
    assert!(p.set_edges(&scene, &edges[..2]).unwrap());
    assert!(!p.set_edges(&scene, &edges[..2]).unwrap());
    assert!(p.request(s.document(), true).is_ok());
    let partial = scene.isolated_source(p.input.producer).unwrap();
    assert!(EdgeTreatmentPanel::new(s.document(), s.revision(), &partial, body, edges).is_err());
    assert_eq!(s.document(), &before);
}
#[test]
fn actual_profile_toggle_retains_feature_captures_and_bound_radius_with_pinned_actions() {
    let (s, scene, body, edges, _) = fixture();
    let mut p = EdgeTreatmentPanel::new(
        s.document(),
        s.revision(),
        &scene,
        body,
        edges[..2].to_vec(),
    )
    .unwrap();
    p.size.text = "=$radius".into();
    let before = s.document().clone();
    let initial = p.request(s.document(), true).unwrap();
    let ctx = Context::default();
    ctx.enable_accesskit();
    click(&ctx, &mut p, &s, "Chamfer");
    assert_eq!(p.profile, EdgeTreatmentProfile::Chamfer);
    let command = p.request(s.document(), true).unwrap();
    let (EdgeTreatmentCommand::Create(a), EdgeTreatmentCommand::Create(b)) = (initial, &command)
    else {
        panic!()
    };
    assert_eq!(a.feature, b.feature);
    assert_eq!(a.input, b.input);
    assert_eq!(a.edges, b.edges);
    assert!(
        matches!(&b.parameters.size,Dimension::Expression{expression}if expression.text=="=$radius")
    );
    assert_eq!(b.parameters.resolved_size_mm(s.parameters()).unwrap(), 1.5);
    let first = command.prepare(&s, actor()).unwrap();
    let second = command.prepare(&s, actor()).unwrap();
    assert_eq!(first.prepared.document(), second.prepared.document());
    for value in ["0", "-1", "NaN", "=$radius * -1"] {
        p.size.text = value.into();
        assert!(p.request(s.document(), true).is_err());
    }
    assert_eq!(s.document(), &before);
}
#[test]
fn saved_edit_retains_frozen_input_feature_and_capture_ids() {
    let (mut s, scene, body, edges, _) = fixture();
    let p = EdgeTreatmentPanel::new(
        s.document(),
        s.revision(),
        &scene,
        body,
        edges[..1].to_vec(),
    )
    .unwrap();
    let EdgeTreatmentCommand::Create(request) = p.request(s.document(), true).unwrap() else {
        panic!()
    };
    s.commit(
        model::prepare_edge_treatment(&s, s.revision(), actor(), &request)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let mut saved = EdgeTreatmentPanel::existing(s.document(), request.feature).unwrap();
    assert!(!saved.picking());
    assert_eq!(saved.input, request.input);
    assert_eq!(saved.edges, request.edges);
    saved.profile = EdgeTreatmentProfile::Chamfer;
    saved.size.text = "1".into();
    let command = saved.request(s.document(), true).unwrap();
    assert_eq!(command.edit_id(), Some(request.feature));
    let prepared = command.prepare(&s, actor()).unwrap();
    let (definition, captures) =
        model::read_edge_treatment(prepared.prepared.document(), request.feature).unwrap();
    assert_eq!(definition.input, request.input);
    assert_eq!(captures, request.edges);
    assert_eq!(definition.parameters.profile, EdgeTreatmentProfile::Chamfer);
}
#[test]
fn controller_capture_updates_are_source_bound_and_toolbar_preview_stops_picking() {
    let (s, scene, body, edges, _) = fixture();
    let before = s.document().clone();
    let mut c = ModelToolController::default();
    c.set_backend(Box::new(native_fixture::NativeBackend::new()));
    c.begin_edge_treatment(&s, &Default::default(), &scene, body, vec![], Channel::Ui)
        .unwrap();
    assert!(c.edge_treatment_active() && c.edge_treatment_picking());
    assert!(c.preview(&s, &Default::default(), Channel::Ui).is_err());
    assert!(!c.edge_treatment_picking());
    let foreign = Session::new(s.document().clone()).unwrap();
    assert!(
        c.set_edge_treatment_edges(&foreign, &Default::default(), &scene, edges.clone())
            .is_err()
    );
    let old = c.stamp();
    c.set_edge_treatment_edges(&s, &Default::default(), &scene, edges.clone())
        .unwrap();
    assert_ne!(c.stamp(), old);
    assert_eq!(c.edge_treatment_edges(), Some(edges.as_slice()));
    assert_eq!(c.edge_treatment_input().unwrap().body, body);
    c.cancel();
    assert!(!c.edge_treatment_active());
    assert_eq!(s.document(), &before);
}
#[test]
fn renamed_parameter_retains_bound_size_and_missing_parameter_is_diagnostic() {
    let (s, scene, body, edges, _) = fixture();
    let mut p = EdgeTreatmentPanel::new(
        s.document(),
        s.revision(),
        &scene,
        body,
        edges[..1].to_vec(),
    )
    .unwrap();
    p.size.text = "=$radius".into();
    let dimension = p.parameters(s.document()).unwrap().size;
    p.size.remember(&dimension);
    let mut d = s.document().clone();
    let mut parameter = d.entities().find(|e| e.name == "radius").unwrap().clone();
    parameter.name = "renamed_radius".into();
    d.replace(parameter.clone()).unwrap();
    assert_eq!(p.parameters(&d).unwrap().size, dimension);
    d.remove(parameter.id).unwrap();
    assert!(p.parameters(&d).is_err());
    assert!(BoundExpression::bind(s.document(), "=$radius", "mm", None).is_ok());
}

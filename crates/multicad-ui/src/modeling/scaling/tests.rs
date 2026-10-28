use super::*;
use crate::{
    egui::{Context, Event, Modifiers, PointerButton, Pos2, Rect},
    modeling::native_fixture,
};
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::Entity,
    model::{PrimitiveParameters, PrimitiveRequest},
    parameters::BoundExpression,
};
use multicad_model::Compiler;
use std::sync::Arc;
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn fixture() -> (Session, Arc<Scene>, EntityId) {
    let mut d = Document::new("Scale controls").unwrap();
    d.insert(Entity::new(
        "factor",
        EntityData::Parameter {
            expression: "2".into(),
            unit: "ratio".into(),
            exposed: true,
            inputs: vec![],
        },
    ))
    .unwrap();
    let mut s = Session::new(d).unwrap();
    let p = PrimitiveRequest::new(
        "Stock",
        PrimitiveParameters::Box {
            size: [model::length(10.), model::length(8.), model::length(6.)],
            frame: Default::default(),
        },
    );
    s.commit(
        model::prepare_primitive(&s, s.revision(), actor(), &p)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let m = Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
    let scene = native_fixture::scene(s.document(), m.snapshot(), 0).unwrap();
    (s, scene, p.body)
}
#[test]
fn exact_bounds_pivot_ratio_formulas_invalid_controls_and_empty_targets() {
    let (s, scene, body) = fixture();
    let before = s.document().clone();
    let mut p = ScalePanel::new(
        s.document(),
        s.revision(),
        &scene,
        &BTreeSet::from([body]),
        false,
    )
    .unwrap();
    let parameters = p.parameters(s.document()).unwrap();
    assert_eq!(
        parameters.resolve(s.parameters()).unwrap().pivot_mm,
        [5., 4., 3.]
    );
    p.factors[0].text = "=$factor".into();
    let bound = p.request(s.document(), true).unwrap();
    let ScaleCommand::Batch(batch) = &bound else {
        panic!()
    };
    assert_eq!(
        batch.parameters.resolve(s.parameters()).unwrap().factors,
        [2., 1., 1.]
    );
    assert_eq!(
        bound.prepare(&s, actor()).unwrap().prepared.document(),
        bound.prepare(&s, actor()).unwrap().prepared.document()
    );
    for invalid in ["0", "-1", "NaN", "inf", "=$factor * -1"] {
        p.factors[1].text = invalid.into();
        assert!(p.request(s.document(), true).is_err(), "{invalid}");
    }
    p.factors[1].text = "1".into();
    p.retarget(s.document(), s.revision(), &scene, &BTreeSet::new())
        .unwrap();
    assert!(p.request(s.document(), true).is_err());
    p.retarget(s.document(), s.revision(), &scene, &BTreeSet::from([body]))
        .unwrap();
    assert_eq!(p.factors[0].text, "=$factor");
    assert_eq!(s.document(), &before);
}
fn frame(ctx: &Context, p: &mut ScalePanel, s: &Session, events: Vec<Event>) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            focused: true,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(350., 900.))),
            events,
            ..Default::default()
        },
        |ui| {
            let event = p.show(ui, s.document(), true, PreviewState::Idle);
            assert!(!matches!(event, Some(ModelEvent::Apply)));
        },
    )
}
#[test]
fn actual_checkbox_and_copy_modifier_keep_allocated_identities_and_original_source() {
    let (s, scene, body) = fixture();
    let original = s.document().clone();
    let mut p = ScalePanel::new(
        s.document(),
        s.revision(),
        &scene,
        &BTreeSet::from([body]),
        false,
    )
    .unwrap();
    p.factors[0].text = "2".into();
    let ctx = Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut p, &s, vec![]).drop_without_applying_deltas();
    let output = frame(&ctx, &mut p, &s, vec![]);
    let bounds = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Create copy"))
        .and_then(|(_, n)| n.bounds())
        .unwrap();
    let pos = Pos2::new(
        ((bounds.x0 + bounds.x1) * 0.5) as f32,
        ((bounds.y0 + bounds.y1) * 0.5) as f32,
    );
    output.drop_without_applying_deltas();
    for pressed in [true, false] {
        frame(
            &ctx,
            &mut p,
            &s,
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
    assert!(p.create_copy);
    let copy = p.request(s.document(), true).unwrap();
    let ids = copy.instances();
    assert_eq!(ids.len(), 1);
    p.create_copy = false;
    p.set_copy_modifier(true);
    let held = p.request(s.document(), true).unwrap();
    assert_eq!(held.instances(), ids);
    assert_eq!(
        copy.prepare(&s, actor()).unwrap().prepared.document(),
        held.prepare(&s, actor()).unwrap().prepared.document()
    );
    assert_eq!(s.document(), &original);
}
#[test]
fn saved_edit_retains_pivot_axes_bound_formulas_and_deleted_parameter_diagnostic() {
    let (mut s, scene, body) = fixture();
    let mut p = ScalePanel::new(
        s.document(),
        s.revision(),
        &scene,
        &BTreeSet::from([body]),
        false,
    )
    .unwrap();
    p.factors[0].text = "=$factor".into();
    p.pivot[1].text = "17".into();
    p.set_axes([[0., 1., 0.], [-1., 0., 0.], [0., 0., 1.]])
        .unwrap();
    let command = p.request(s.document(), true).unwrap();
    let prepared = command.prepare(&s, actor()).unwrap();
    let feature = prepared.outputs[0].feature;
    s.commit(prepared.prepared).unwrap();
    let saved = ScalePanel::existing(s.document(), feature).unwrap();
    assert_eq!(
        saved.parameters(s.document()).unwrap(),
        p.parameters(s.document()).unwrap()
    );
    assert!(saved.alternate.is_none());
    let mut parameter = s
        .document()
        .entities()
        .find(|e| e.name == "factor")
        .unwrap()
        .clone();
    parameter.name = "renamed_factor".into();
    let mut d = s.document().clone();
    d.replace(parameter.clone()).unwrap();
    let mut field = DimensionField::from_dimension(
        &Dimension::Expression {
            expression: BoundExpression::bind(s.document(), "=$factor", "ratio", None).unwrap(),
        },
        "ratio",
    )
    .unwrap();
    assert!(field.resolve(&d).is_ok());
    d.remove(parameter.id).unwrap();
    assert!(field.resolve(&d).is_err());
    field.text = "2".into();
    assert!(field.resolve(&d).is_ok());
}

//! Typed nut bindings select named choices; parameter names never select roles.
use super::fields::Overrides;
use crate::egui;
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, Entity, EntityData, EntityId},
    model::{
        self, MechanicalPartParameters, MechanicalPartRequest, ThreadParameters,
        ThreadRepresentation, length,
    },
    symbols::{self, SymbolAuthorRequest, SymbolDefinition, SymbolKind},
};
use std::collections::BTreeMap;

fn fixture() -> (SymbolDefinition, EntityId, EntityId) {
    let mut document = Document::new("Named nut representation").unwrap();
    // Deliberately unrelated name for the actual typed role.
    let mode = Entity::new(
        "unrelated_count",
        EntityData::Parameter {
            expression: "1".into(),
            unit: "count".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let binding = mode.id;
    // A misleading name and binary default must not make an enum control.
    let ordinary = Entity::new(
        "thread_representation",
        EntityData::Parameter {
            expression: "1".into(),
            unit: "count".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let ordinary_id = ordinary.id;
    document.insert(mode).unwrap();
    document.insert(ordinary).unwrap();
    let mut session = Session::new(document).unwrap();
    let actor = || Actor::local(Channel::Ui);
    let request = MechanicalPartRequest::new(
        "Dynamic M3 nut",
        MechanicalPartParameters::HexNut {
            across_flats: length(5.5),
            thread: Box::new(ThreadParameters {
                major_diameter: length(3.),
                pitch: length(0.5),
                length: length(2.4),
                representation: ThreadRepresentation::CosmeticEnvelope,
                representation_parameter: Some(binding),
                ..Default::default()
            }),
        },
    );
    session
        .commit(
            model::prepare_mechanical_part(&session, 0, actor(), &request)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let author = SymbolAuthorRequest::new(
        "Configurable nut",
        request.body,
        SymbolKind::Dynamic,
        vec![binding, ordinary_id],
        None,
    );
    session
        .commit(
            symbols::prepare_author(&session, session.revision(), actor(), &author)
                .unwrap()
                .prepared,
        )
        .unwrap();
    (
        symbols::read_definition(session.document(), author.definition).unwrap(),
        binding,
        ordinary_id,
    )
}
fn frame(
    ctx: &egui::Context,
    fields: &mut Overrides,
    definition: &SymbolDefinition,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(500., 900.),
            )),
            events,
            focused: true,
            ..Default::default()
        },
        |ui| {
            fields.show(ui, definition);
        },
    )
}
fn texts(output: &egui::FullOutput) -> Vec<(String, egui::Pos2)> {
    fn visit(shape: &egui::Shape, result: &mut Vec<(String, egui::Pos2)>) {
        match shape {
            egui::Shape::Text(text) => result.push((
                text.galley.text().to_owned(),
                text.pos + text.galley.size() / 2.,
            )),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, result);
                }
            }
            _ => {}
        }
    }
    let mut result = vec![];
    for shape in &output.shapes {
        visit(&shape.shape, &mut result);
    }
    result
}
fn click(ctx: &egui::Context, fields: &mut Overrides, definition: &SymbolDefinition, label: &str) {
    // A second frame lets newly opened ComboBox menus settle before selection.
    frame(ctx, fields, definition, vec![]).drop_without_applying_deltas();
    let output = frame(ctx, fields, definition, vec![]);
    let pos = texts(&output)
        .into_iter()
        .rev()
        .find_map(|(text, pos)| (text == label).then_some(pos))
        .unwrap_or_else(|| panic!("Missing painted control {label}: {:?}", texts(&output)));
    output.drop_without_applying_deltas();
    for pressed in [true, false] {
        frame(
            ctx,
            fields,
            definition,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        )
        .drop_without_applying_deltas();
    }
}

#[test]
fn dynamic_nut_override_uses_named_choices_resets_default_and_keeps_numeric_inputs_numeric() {
    let (definition, binding, ordinary) = fixture();
    let source = definition.clone();
    let pin = definition.pin().unwrap();
    let mut fields = Overrides::new(&definition, &BTreeMap::new()).unwrap();
    assert!(fields.resolve(&definition).unwrap().is_empty());
    let context = egui::Context::default();
    let output = frame(&context, &mut fields, &definition, vec![]);
    let labels = texts(&output);
    assert!(
        labels
            .iter()
            .any(|(text, _)| text == "Default: Modeled helix")
    );
    assert!(
        labels
            .iter()
            .any(|(text, _)| text == "Override Thread representation")
    );
    assert!(
        labels
            .iter()
            .any(|(text, _)| text == "Override thread_representation")
    );
    assert!(!labels.iter().any(|(text, _)| text == "0" || text == "1"));
    output.drop_without_applying_deltas();
    click(
        &context,
        &mut fields,
        &definition,
        "Override Thread representation",
    );
    click(&context, &mut fields, &definition, "Modeled helix");
    click(&context, &mut fields, &definition, "Cosmetic envelope");
    let cosmetic = fields.resolve(&definition).unwrap();
    assert_eq!(cosmetic.len(), 1);
    assert_eq!(cosmetic[&binding].value, 0.);
    assert_eq!(cosmetic[&binding].unit, "count");
    assert!(!cosmetic.contains_key(&ordinary));
    click(&context, &mut fields, &definition, "Cosmetic envelope");
    click(&context, &mut fields, &definition, "Modeled helix");
    let modeled = fields.resolve(&definition).unwrap();
    assert_eq!(modeled[&binding].value, 1.);
    assert_eq!(modeled[&binding].unit, "count");
    click(
        &context,
        &mut fields,
        &definition,
        "Override Thread representation",
    );
    assert!(!fields.0[&binding].enabled);
    assert_eq!(fields.0[&binding].text, "1");
    assert!(fields.resolve(&definition).unwrap().is_empty());
    click(
        &context,
        &mut fields,
        &definition,
        "Override thread_representation",
    );
    let output = frame(&context, &mut fields, &definition, vec![]);
    let labels = texts(&output);
    assert!(
        labels.iter().any(|(text, _)| text == "count"),
        "ordinary numeric editor keeps units"
    );
    assert!(
        !labels
            .iter()
            .any(|(text, _)| text == "Cosmetic envelope" || text == "Modeled helix"),
        "ordinary Count1 must not use a representation ComboBox"
    );
    output.drop_without_applying_deltas();
    fields.0.get_mut(&ordinary).unwrap().text = "7".into();
    let numeric = fields.resolve(&definition).unwrap();
    assert_eq!(numeric.len(), 1);
    assert_eq!(numeric[&ordinary].value, 7.);
    assert_eq!(numeric[&ordinary].unit, "count");
    assert_eq!(definition, source);
    assert_eq!(definition.pin().unwrap(), pin);
}

#[test]
fn typed_representation_override_rejects_invalid_binary_values_and_wrong_units() {
    let (definition, binding, _) = fixture();
    let before = definition.clone();
    let mut fields = Overrides::new(&definition, &BTreeMap::new()).unwrap();
    let field = fields.0.get_mut(&binding).unwrap();
    field.enabled = true;
    for invalid in ["2", "-1", "0.5", "1.0000000001", "NaN"] {
        fields.0.get_mut(&binding).unwrap().text = invalid.into();
        assert!(
            fields.resolve(&definition).is_err(),
            "invalid representation {invalid}"
        );
    }
    let field = fields.0.get_mut(&binding).unwrap();
    field.text = "1".into();
    field.unit = "mm".into();
    assert!(fields.resolve(&definition).is_err());
    fields.0.get_mut(&binding).unwrap().unit = "count".into();
    assert_eq!(fields.resolve(&definition).unwrap()[&binding].value, 1.);
    fields.0.get_mut(&binding).unwrap().enabled = false;
    assert!(fields.resolve(&definition).unwrap().is_empty());
    assert_eq!(definition, before);
}

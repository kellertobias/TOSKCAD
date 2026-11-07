//! Painted source controls and draft lifecycle. These tests do not substitute
//! for the native complete-connection controller and desktop workflow gates.
use super::*;
use multicad_core::{
    document::{Entity, EntityData, Mutation},
    parameters::BoundExpression,
};

fn frame(
    context: &egui::Context,
    panel: &mut ScrewConnectionPanel,
    document: &Document,
    events: Vec<egui::Event>,
    width: f32,
) -> egui::FullOutput {
    context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 700.),
            )),
            focused: true,
            events,
            ..Default::default()
        },
        |ui| {
            crate::inspector_actions::scope(
                ui,
                Some(crate::inspector_actions::Owner::Model),
                crate::registry::Mode::Model.color(),
                |ui| {
                    panel.show(ui, document, true, PreviewState::Idle);
                },
            );
        },
    )
}
fn click(
    context: &egui::Context,
    panel: &mut ScrewConnectionPanel,
    document: &Document,
    label: &str,
) {
    frame(context, panel, document, vec![], 320.).drop_without_applying_deltas();
    let output = frame(context, panel, document, vec![], 320.);
    let point = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                let center = text.pos + text.galley.size() * 0.5;
                shape.clip_rect.contains(center).then_some(center)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing visible connection control {label}"));
    output.drop_without_applying_deltas();
    for pressed in [true, false] {
        frame(
            context,
            panel,
            document,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
            320.,
        )
        .drop_without_applying_deltas();
    }
}

#[test]
fn painted_five_modes_have_explicit_missing_input_diagnostics_and_fixed_actions() {
    let document = Document::new("Connection mode choices").unwrap();
    let original = document.clone();
    let mut panel = ScrewConnectionPanel::new(&document).unwrap();
    let context = egui::Context::default();
    context.enable_accesskit();
    for (_, label) in modes() {
        let current = mode_name(panel.mode);
        click(&context, &mut panel, &document, current);
        click(&context, &mut panel, &document, label);
        assert_eq!(mode_name(panel.mode), label);
        let output = frame(&context, &mut panel, &document, vec![], 244.);
        let nodes = &output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes;
        for action in ["Preview", "Apply", "Cancel"] {
            let node = nodes
                .iter()
                .find(|(_, node)| node.label() == Some(action))
                .unwrap_or_else(|| panic!("Missing fixed {action}"));
            if action == "Apply" {
                assert!(node.1.is_disabled());
            }
        }
        assert!(panel.request(&document, true).is_err());
        output.drop_without_applying_deltas();
    }
    assert_eq!(document, original);
}

#[test]
fn returning_to_a_mode_preserves_role_ids_and_authored_dimension_bindings() {
    let parameter = Entity::new(
        "grip",
        EntityData::Parameter {
            expression: "4".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let document = Document::new("Independent connection drafts")
        .unwrap()
        .edited(&[Mutation::Insert(parameter)])
        .unwrap();
    let mut panel = ScrewConnectionPanel::new(&document).unwrap();
    panel.parameters.grip = Dimension::Expression {
        expression: BoundExpression::bind(&document, "=$grip", "mm", None).unwrap(),
    };
    let original_ids = panel
        .hardware
        .iter()
        .map(|hardware| (hardware.role_id, hardware.instance, hardware.driver))
        .collect::<Vec<_>>();
    let original_grip = panel.parameters.grip.clone();
    panel.select_mode(ScrewConnectionMode::DirectWood);
    panel.parameters.grip = length(30.);
    panel.select_mode(ScrewConnectionMode::ThroughNut);
    assert_eq!(panel.parameters.grip, original_grip);
    assert_eq!(
        panel
            .hardware
            .iter()
            .map(|hardware| (hardware.role_id, hardware.instance, hardware.driver))
            .collect::<Vec<_>>(),
        original_ids
    );
    let ScrewConnectionHardwareSource::Generated { part } = &panel.hardware[0].source else {
        panic!("custom screw")
    };
    let MechanicalPartParameters::MachineScrew { thread, .. } = &part.parameters else {
        panic!("machine screw")
    };
    assert_eq!(thread.fit, ThreadFit::Metric6gExternal);
    assert!(matches!(
        thread.metric_fit_engagement,
        Some(MetricFitEngagement::NormalGroupAssumed {})
    ));
    let nut = panel
        .hardware
        .iter()
        .find(|hardware| hardware.role == ScrewConnectionHardwareRole::Nut)
        .unwrap();
    let ScrewConnectionHardwareSource::Generated { part } = &nut.source else {
        panic!("custom nut")
    };
    let MechanicalPartParameters::HexNut { thread, .. } = &part.parameters else {
        panic!("hex nut")
    };
    assert_eq!(thread.fit, ThreadFit::Metric6HInternal);
}

#[test]
fn two_connection_drafts_allocate_independent_owned_hardware_and_receiver_sources() {
    let document = Document::new("Two connections").unwrap();
    let first = ScrewConnectionPanel::new(&document).unwrap();
    let second = ScrewConnectionPanel::new(&document).unwrap();
    assert_ne!(first.connection, second.connection);
    assert_ne!(first.axis_id, second.axis_id);
    for a in &first.hardware {
        for b in &second.hardware {
            assert_ne!(a.role_id, b.role_id);
            assert_ne!(a.instance, b.instance);
            assert_ne!(a.driver, b.driver);
        }
    }
    assert_eq!(document.entities().count(), 0);
}

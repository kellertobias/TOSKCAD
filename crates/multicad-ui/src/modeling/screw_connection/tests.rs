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

#[test]
fn mode_defaults_distinguish_headless_hangers_and_independent_receiving_cut_purposes() {
    let document = Document::new("Mode-specific connection defaults").unwrap();
    let mut panel = ScrewConnectionPanel::new(&document).unwrap();
    let washer = panel
        .hardware
        .iter()
        .find(|role| role.role == ScrewConnectionHardwareRole::HeadWasher)
        .unwrap()
        .role_id;
    panel.select_mode(ScrewConnectionMode::HangerBolt);
    assert!(
        !panel
            .hardware
            .iter()
            .any(|role| role.role == ScrewConnectionHardwareRole::HeadWasher)
    );
    let input = BodyInput {
        body: EntityId::new(),
        producer: EntityId::new(),
    };
    let occurrence = OccurrencePath {
        root: input.body,
        members: vec![],
    };
    panel.add_receiver(input.clone(), occurrence.clone());
    assert!(
        matches!(&panel.receivers[0].source.cut, ConnectionReceiverCut::Round { diameter } if *diameter == length(4.))
    );
    panel.select_mode(ScrewConnectionMode::WoodInsert);
    assert!(
        matches!(&panel.receivers[0].source.cut, ConnectionReceiverCut::Round { diameter } if *diameter == length(8.))
    );
    panel.select_mode(ScrewConnectionMode::TappedReceiver);
    let ConnectionReceiverCut::Tapped { thread } = &panel.receivers[0].source.cut else {
        panic!("modeled receiving thread")
    };
    assert_eq!(thread.fit, ThreadFit::Metric6HInternal);
    assert!(matches!(
        thread.metric_fit_engagement,
        Some(MetricFitEngagement::NormalGroupAssumed {})
    ));
    panel.select_mode(ScrewConnectionMode::ThroughNut);
    assert!(panel.hardware.iter().any(|role| role.role_id == washer));
    // A newly picked receiver uses the selected mode even before any mode switch
    // has a receiving cut to rewrite.
    panel.select_mode(ScrewConnectionMode::TappedReceiver);
    panel.receivers.clear();
    panel.add_receiver(input, occurrence);
    assert!(
        matches!(&panel.receivers[0].source.cut, ConnectionReceiverCut::Tapped { thread } if thread.fit == ThreadFit::Metric6HInternal)
    );
}

#[test]
fn choosing_a_unique_hardware_role_reuses_its_identity_without_adding_duplicates() {
    let document = Document::new("Unique hardware roles").unwrap();
    let mut panel = ScrewConnectionPanel::new(&document).unwrap();
    let before = panel.hardware.clone();
    panel
        .choose_hardware(ScrewConnectionHardwareRole::Screw)
        .unwrap();
    assert!(panel.library.active());
    assert!(panel.pending_roles.is_empty());
    assert_eq!(panel.hardware, before);
    panel.library.cancel();
    assert!(
        panel
            .choose_hardware(ScrewConnectionHardwareRole::HangerBolt)
            .is_err()
    );
    assert!(
        panel
            .choose_hardware(ScrewConnectionHardwareRole::Insert)
            .is_err()
    );
    panel.select_mode(ScrewConnectionMode::WoodInsert);
    let before = panel.hardware.clone();
    panel
        .choose_hardware(ScrewConnectionHardwareRole::Insert)
        .unwrap();
    assert!(panel.pending_roles.is_empty());
    assert_eq!(panel.hardware, before);
    assert!(!panel.role_available(ScrewConnectionHardwareRole::Nut));
    panel.select_mode(ScrewConnectionMode::HangerBolt);
    assert!(!panel.role_available(ScrewConnectionHardwareRole::Screw));
    assert!(!panel.role_available(ScrewConnectionHardwareRole::HeadWasher));
    assert!(panel.role_available(ScrewConnectionHardwareRole::Nut));
}

// Synthetic named planes below exercise editor source retention only. Native
// geometry and associative publication are separate NativeBackend controls.
fn editor_face(source: EntityId, z: f64) -> FaceInput {
    use multicad_core::references::*;
    let origin = Origin {
        source,
        role: format!("editor-plane-{z}"),
    };
    let frame = ReferenceFrame {
        origin_mm: [0., 0., z],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
    };
    let key = TopologyKey {
        kind: TopologyKind::Face,
        ordinal: 0,
    };
    let snapshot = TopologySnapshot {
        owner: source,
        revision: 0,
        elements: vec![TopologyElement {
            key,
            origins: std::collections::BTreeSet::from([origin.clone()]),
            boundary: Default::default(),
            frames: vec![AnchoredFrame { origin, frame }],
            geometry: Some(AnalyticGeometry::Plane { frame }),
        }],
    };
    capture_profile_face(&snapshot, key, None).unwrap()
}

#[test]
fn captured_end_and_receiver_entries_have_one_authority_and_restore_the_manual_binding() {
    let parameter = Entity::new(
        "grip",
        EntityData::Parameter {
            expression: "4.1".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let document = Document::new("Associative editor sources")
        .unwrap()
        .edited(&[Mutation::Insert(parameter)])
        .unwrap();
    let original = document.clone();
    let mut panel = ScrewConnectionPanel::new(&document).unwrap();
    let manual = Dimension::Expression {
        expression: BoundExpression::bind(&document, "=$grip", "mm", None).unwrap(),
    };
    panel.parameters.grip = manual.clone();
    let input = BodyInput {
        body: EntityId::new(),
        producer: EntityId::new(),
    };
    let occurrence = OccurrencePath {
        root: input.body,
        members: vec![],
    };
    let entry = editor_face(input.producer, 0.);
    panel.pick = Some(ScrewConnectionPick {
        receiver: input.clone(),
        occurrence: occurrence.clone(),
        kind: ScrewConnectionPickKind::EntryAxis,
    });
    panel.capture(entry.clone()).unwrap();
    assert_eq!(panel.receivers[0].source.entry_reference, Some(entry));
    assert_eq!(panel.receivers[0].source.station, length(0.));
    let end = editor_face(input.producer, 4.1);
    panel.pick = Some(ScrewConnectionPick {
        receiver: input.clone(),
        occurrence: occurrence.clone(),
        kind: ScrewConnectionPickKind::GripEnd,
    });
    panel.capture(end.clone()).unwrap();
    assert_eq!(panel.parameters.grip, length(0.));
    assert_eq!(panel.parameters.end_reference.as_ref().unwrap().face, end);
    let context = egui::Context::default();
    let output = frame(&context, &mut panel, &document, vec![], 320.);
    let texts = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(texts.contains(&"Use explicit grip"));
    assert!(
        !texts.contains(&"Grip"),
        "Dormant manual grip must not appear as a second driving value"
    );
    assert!(
        !texts.contains(&"Cut start along axis"),
        "Captured entry must not expose a dormant station control"
    );
    output.drop_without_applying_deltas();
    panel.select_mode(ScrewConnectionMode::DirectWood);
    panel.select_mode(ScrewConnectionMode::ThroughNut);
    panel.use_explicit_grip();
    assert_eq!(panel.parameters.grip, manual);
    assert!(panel.parameters.end_reference.is_none());
    assert_eq!(document, original);
}

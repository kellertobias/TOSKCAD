//! Synthetic scenes validate numeric form intent and events only. Exact native
//! placement geometry and occurrence publication have model integration tests.
use super::*;
use crate::viewport::{Geometry, Instance, SceneStamp, Transform};
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Entity, Mutation, Placement},
    model::{ModelEvaluationInfo, PrimitiveParameters, PrimitiveRequest},
    parameters,
};
use std::sync::Arc;
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn fixture() -> (Session, Arc<Scene>, BTreeSet<EntityId>) {
    let mut d = Document::new("Numeric placement").unwrap();
    d.insert(Entity::new(
        "offset",
        EntityData::Parameter {
            expression: "25".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    ))
    .unwrap();
    let mut s = Session::new(d).unwrap();
    let p = PrimitiveRequest::new(
        "Stock",
        PrimitiveParameters::Box {
            size: [model::length(10.), model::length(8.), model::length(3.)],
            frame: Default::default(),
        },
    );
    s.commit(
        model::prepare_primitive(&s, 0, actor(), &p)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let secondary = PrimitiveRequest::new(
        "Independent copy component",
        PrimitiveParameters::Box {
            size: [model::length(6.), model::length(4.), model::length(2.)],
            frame: Default::default(),
        },
    );
    s.commit(
        model::prepare_primitive(&s, s.revision(), actor(), &secondary)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let i = Entity::new(
        "Placed stock",
        EntityData::Instance {
            component: secondary.body,
            placement: Placement {
                translation_mm: [40., 0., 0.],
                ..Default::default()
            },
            placement_source: None,
        },
    );
    let instance = i.id;
    s.apply(multicad_core::commands::Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Source occurrence".into(),
        edits: vec![Mutation::Insert(i)],
    })
    .unwrap();
    let geometry = Arc::new(
        Geometry::new(
            p.feature,
            vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            vec![0, 1, 2],
            vec![0],
            vec![],
        )
        .unwrap(),
    );
    let scene = Scene::new(
        SceneStamp {
            document: s.document().id(),
            revision: s.revision(),
            generation: 1,
        },
        vec![
            geometry,
            Arc::new(
                Geometry::new(
                    secondary.feature,
                    vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
                    vec![0, 1, 2],
                    vec![0],
                    vec![],
                )
                .unwrap(),
            ),
        ],
        vec![
            Instance {
                id: p.body,
                geometry: p.feature,
                transform: Transform::default(),
                color: [0, 0, 0, 255],
            },
            Instance {
                id: instance,
                geometry: secondary.feature,
                transform: Transform::default(),
                color: [0, 0, 0, 255],
            },
        ],
    )
    .unwrap()
    .with_model_evaluation(Arc::new(ModelEvaluationInfo {
        screw_connection_checks: vec![],
        source_hash: ContentHash::document(s.document()).unwrap(),
        consumed_bodies: BTreeSet::new(),
        cut_targets: vec![],
    }))
    .unwrap();
    (s, Arc::new(scene), [p.body, instance].into())
}
fn batch(command: &PlacementCommand) -> &PlacementBatchRequest {
    let PlacementCommand::Batch(batch) = command else {
        panic!("Expected new placement batch")
    };
    batch
}
fn parameters(command: &PlacementCommand) -> &NumericTransform {
    match command {
        PlacementCommand::Batch(b) => b
            .transforms
            .first()
            .map(|r| &r.parameters)
            .or_else(|| b.instances.first().map(|r| &r.parameters))
            .or_else(|| b.copies.first().map(|r| &r.parameters))
            .unwrap(),
        PlacementCommand::EditPart { parameters, .. }
        | PlacementCommand::EditInstance { parameters, .. } => parameters,
    }
}
#[test]
fn selection_is_explicit_current_and_source_bound_without_mutating_scene_or_document() {
    let (s, scene, selected) = fixture();
    let before = s.document().clone();
    let panel = PlacementPanel::new(
        s.document(),
        s.revision(),
        &scene,
        &selected,
        PlacementMode::Move,
    )
    .unwrap();
    let request = panel.request(s.document(), true).unwrap();
    assert_eq!(batch(&request).transforms.len(), 1);
    assert_eq!(batch(&request).instances.len(), 1);
    assert!(batch(&request).copies.is_empty());
    assert!(
        PlacementPanel::new(
            s.document(),
            s.revision() + 1,
            &scene,
            &selected,
            PlacementMode::Move
        )
        .is_err()
    );
    let mut changed = before.clone();
    changed.rename("Different canonical source").unwrap();
    assert!(
        PlacementPanel::new(
            &changed,
            s.revision(),
            &scene,
            &selected,
            PlacementMode::Move
        )
        .is_err()
    );
    assert!(
        PlacementPanel::new(
            s.document(),
            s.revision(),
            &scene,
            &BTreeSet::new(),
            PlacementMode::Move
        )
        .is_err()
    );
    assert_eq!(s.document(), &before);
    assert_eq!(scene.reference_instances().len(), 2);
}
#[test]
fn preview_remembers_bound_dimensions_after_parameter_rename_without_baking_values() {
    let (s, scene, selected) = fixture();
    let mut d = s.document().clone();
    let mut p =
        PlacementPanel::new(&d, s.revision(), &scene, &selected, PlacementMode::Move).unwrap();
    p.translation[0].text = "=$offset + 5".into();
    p.pivot[1].text = "=$offset / 5".into();
    p.angle.text = "90".into();
    let before = d.clone();
    let first = p.preview_request(&d, true).unwrap();
    let bound = parameters(&first).clone();
    assert!(bound.translation[0].expression().is_some());
    let first_bytes = serde_json::to_value(batch(&first)).unwrap();
    let mut parameter = d.entities().find(|e| e.name == "offset").unwrap().clone();
    let parameter_id = parameter.id;
    parameter.name = "renamed_offset".into();
    d.replace(parameter).unwrap();
    let again = p.preview_request(&d, true).unwrap();
    assert_eq!(serde_json::to_value(batch(&again)).unwrap(), first_bytes);
    let pose = parameters(&again)
        .resolve(&parameters::evaluate(&d).unwrap())
        .unwrap();
    assert_eq!(pose.point([0., 0., 0.]), [35., 5., 0.]);
    assert_eq!(s.document(), &before);
    d.remove(parameter_id).unwrap();
    assert!(p.preview_request(&d, true).is_err());
}
#[test]
fn copy_output_ids_and_authored_units_remain_stable_across_repeated_previews() {
    let (s, scene, selected) = fixture();
    let mut p = PlacementPanel::new(
        s.document(),
        s.revision(),
        &scene,
        &selected,
        PlacementMode::Copy,
    )
    .unwrap();
    let first = p.preview_request(s.document(), true).unwrap();
    let ids: Vec<_> = batch(&first)
        .copies
        .iter()
        .map(|r| (r.instance, r.driver))
        .collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.iter().all(|(i, _)| !selected.contains(i)));
    p.translation[0].text = "12".into();
    let next = p.preview_request(s.document(), true).unwrap();
    assert_eq!(
        batch(&next)
            .copies
            .iter()
            .map(|r| (r.instance, r.driver))
            .collect::<Vec<_>>(),
        ids
    );
    let mut parameters = NumericTransform::default();
    parameters.translation[0] = Dimension::Literal {
        value: 2.,
        unit: "cm".into(),
    };
    parameters.pivot[2] = Dimension::Literal {
        value: 0.5,
        unit: "m".into(),
    };
    parameters.angle = Dimension::Literal {
        value: std::f64::consts::FRAC_PI_2,
        unit: "rad".into(),
    };
    let command = first.with_parameters(parameters.clone());
    let mut existing =
        PlacementPanel::from_command(command, parameters.clone(), vec!["Authored units".into()])
            .unwrap();
    assert_eq!(
        self::parameters(&existing.preview_request(s.document(), true).unwrap()),
        &parameters
    );
    assert_eq!(existing.translation[0].text, "20");
}
fn frame(
    panel: &mut PlacementPanel,
    d: &Document,
    ctx: &egui::Context,
    preview: PreviewState<'_>,
    enabled: bool,
    events: Vec<egui::Event>,
) -> (Option<ModelEvent>, egui::FullOutput) {
    let mut event = None;
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(360., 1200.),
            )),
            events,
            ..Default::default()
        },
        |ui| event = panel.show(ui, d, enabled, preview),
    );
    (event, output)
}
fn click(
    panel: &mut PlacementPanel,
    d: &Document,
    ctx: &egui::Context,
    preview: PreviewState<'_>,
    enabled: bool,
    label: &str,
) -> Option<ModelEvent> {
    frame(panel, d, ctx, preview, enabled, vec![])
        .1
        .drop_without_applying_deltas();
    let (_, out) = frame(panel, d, ctx, preview, enabled, vec![]);
    let pos = out
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos + t.galley.size() * 0.5),
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing {label}"));
    out.drop_without_applying_deltas();
    let mut result = None;
    for pressed in [true, false] {
        let (event, out) = frame(
            panel,
            d,
            ctx,
            preview,
            enabled,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
        out.drop_without_applying_deltas();
        if event.is_some() {
            result = event;
        }
    }
    result
}
#[test]
fn actual_preview_apply_cancel_buttons_emit_only_local_intent_and_one_explicit_commit_is_undoable()
{
    let (mut s, scene, selected) = fixture();
    let before = s.document().clone();
    let mut panel = PlacementPanel::new(
        s.document(),
        s.revision(),
        &scene,
        &selected,
        PlacementMode::Move,
    )
    .unwrap();
    panel.translation[0].text = "10".into();
    let ctx = egui::Context::default();
    assert!(
        click(
            &mut panel,
            s.document(),
            &ctx,
            PreviewState::Idle,
            true,
            "Apply"
        )
        .is_none()
    );
    let Some(ModelEvent::Preview {
        stamp,
        request: ModelRequest::Placement(request),
    }) = click(
        &mut panel,
        s.document(),
        &ctx,
        PreviewState::Idle,
        true,
        "Preview",
    )
    else {
        panic!("Expected actual Preview click")
    };
    let prepared = request.prepare(&s, actor()).unwrap();
    assert_eq!(s.document(), &before);
    let repeated = click(
        &mut panel,
        s.document(),
        &ctx,
        PreviewState::Pending { stamp },
        true,
        "Preview",
    );
    let Some(ModelEvent::Preview {
        request: ModelRequest::Placement(repeated),
        ..
    }) = repeated
    else {
        panic!()
    };
    assert_eq!(
        serde_json::to_value(batch(&request)).unwrap(),
        serde_json::to_value(batch(&repeated)).unwrap()
    );
    assert!(matches!(
        click(
            &mut panel,
            s.document(),
            &ctx,
            PreviewState::Ready { stamp },
            true,
            "Apply"
        ),
        Some(ModelEvent::Apply)
    ));
    assert_eq!(s.document(), &before);
    s.commit(prepared.prepared).unwrap();
    let applied = s.document().clone();
    assert_ne!(applied, before);
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
    assert!(matches!(
        click(
            &mut panel,
            s.document(),
            &ctx,
            PreviewState::Idle,
            true,
            "Cancel"
        ),
        Some(ModelEvent::Cancel)
    ));
    assert_eq!(s.document(), &before);
}
#[test]
fn invalid_controls_and_unavailable_provider_block_preview_but_keep_cancel_available() {
    let (s, scene, selected) = fixture();
    let mut p = PlacementPanel::new(
        s.document(),
        s.revision(),
        &scene,
        &selected,
        PlacementMode::Move,
    )
    .unwrap();
    let ctx = egui::Context::default();
    for axis in [[0.; 3], [f64::NAN, 0., 1.]] {
        p.axis = axis;
        assert!(p.request(s.document(), true).is_err());
        assert!(
            click(
                &mut p,
                s.document(),
                &ctx,
                PreviewState::Idle,
                true,
                "Preview"
            )
            .is_none()
        );
    }
    p.axis = [0., 0., 1.];
    p.translation[0].text = "=$offset +".into();
    assert!(p.request(s.document(), true).is_err());
    assert!(matches!(
        click(
            &mut p,
            s.document(),
            &ctx,
            PreviewState::Idle,
            false,
            "Cancel"
        ),
        Some(ModelEvent::Cancel)
    ));
}
#[test]
fn editing_visible_numeric_input_invalidates_ready_stamp_before_apply_and_binds_on_preview() {
    let (s, scene, selected) = fixture();
    let before = s.document().clone();
    let mut p = PlacementPanel::new(
        s.document(),
        s.revision(),
        &scene,
        &selected,
        PlacementMode::Move,
    )
    .unwrap();
    let ctx = egui::Context::default();
    let stamp = p.stamp();
    let ready = PreviewState::Ready { stamp };
    frame(&mut p, s.document(), &ctx, ready, true, vec![])
        .1
        .drop_without_applying_deltas();
    let (_, out) = frame(&mut p, s.document(), &ctx, ready, true, vec![]);
    let pos = out
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == "X (mm)" => {
                Some(t.pos + t.galley.size() * 0.5 + egui::vec2(0., 23.))
            }
            _ => None,
        })
        .unwrap();
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        frame(
            &mut p,
            s.document(),
            &ctx,
            ready,
            true,
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
        .1
        .drop_without_applying_deltas();
    }
    let (event, out) = frame(
        &mut p,
        s.document(),
        &ctx,
        ready,
        true,
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            },
            egui::Event::Text("=$offset + 1".into()),
        ],
    );
    out.drop_without_applying_deltas();
    assert_eq!(p.translation[0].text, "=$offset + 1");
    assert!(matches!(event, Some(ModelEvent::Changed)));
    assert_ne!(p.stamp(), stamp);
    assert!(click(&mut p, s.document(), &ctx, ready, true, "Apply").is_none());
    let Some(ModelEvent::Preview {
        request: ModelRequest::Placement(request),
        stamp: next,
    }) = click(
        &mut p,
        s.document(),
        &ctx,
        PreviewState::Idle,
        true,
        "Preview",
    )
    else {
        panic!("Edited numeric input must emit a new checked intent")
    };
    assert_eq!(next, p.stamp());
    assert!(parameters(&request).translation[0].expression().is_some());
    assert_eq!(s.document(), &before);
}
#[test]
fn moving_definition_and_its_own_instance_is_explicitly_rejected_but_copy_is_independent() {
    let (s, scene, selected) = fixture();
    let part = selected
        .iter()
        .find(|id| {
            matches!(
                s.document().get(**id).unwrap().data,
                EntityData::Part { .. }
            )
        })
        .copied()
        .unwrap();
    let instance = selected
        .iter()
        .find(|id| {
            matches!(
                s.document().get(**id).unwrap().data,
                EntityData::Instance { .. }
            )
        })
        .copied()
        .unwrap();
    let mut d = s.document().clone();
    let mut e = d.get(instance).unwrap().clone();
    let EntityData::Instance { component, .. } = &mut e.data else {
        panic!()
    };
    *component = part;
    d.replace(e).unwrap();
    let geometry = scene
        .reference_instances()
        .iter()
        .find(|i| i.id == part)
        .unwrap()
        .geometry;
    let instances = scene
        .reference_instances()
        .iter()
        .cloned()
        .map(|mut i| {
            if i.id == instance {
                i.geometry = geometry;
            }
            i
        })
        .collect();
    let scene = Scene::new(
        scene.stamp(),
        scene.geometries().values().cloned().collect(),
        instances,
    )
    .unwrap()
    .with_model_evaluation(Arc::new(ModelEvaluationInfo {
        screw_connection_checks: vec![],
        source_hash: ContentHash::document(&d).unwrap(),
        consumed_bodies: BTreeSet::new(),
        cut_targets: vec![],
    }))
    .unwrap();
    let panel =
        PlacementPanel::new(&d, s.revision(), &scene, &selected, PlacementMode::Move).unwrap();
    let error = panel.request(&d, true).err().unwrap();
    assert!(error.contains("Part definition or its linked Instance"));
    let p = PlacementPanel::new(&d, s.revision(), &scene, &selected, PlacementMode::Copy).unwrap();
    assert_eq!(batch(&p.request(&d, true).unwrap()).copies.len(), 2);
}

#[test]
fn actual_copy_checkbox_and_temporary_modifier_keep_controls_and_stable_distinct_commands() {
    let (session, scene, selected) = fixture();
    let mut panel = PlacementPanel::new(
        session.document(),
        session.revision(),
        &scene,
        &selected,
        PlacementMode::Move,
    )
    .unwrap();
    panel.translation[0].text = "=$offset + 5".into();
    panel.pivot[1].text = "7".into();
    panel.angle.text = "90".into();
    let original = session.document().clone();
    let moved = panel.preview_request(session.document(), true).unwrap();
    let values = parameters(&moved).clone();
    let ctx = egui::Context::default();
    assert!(matches!(
        click(
            &mut panel,
            session.document(),
            &ctx,
            PreviewState::Idle,
            true,
            "Create copy"
        ),
        Some(ModelEvent::Changed)
    ));
    let copied = panel.preview_request(session.document(), true).unwrap();
    assert_eq!(parameters(&copied), &values);
    assert_eq!(batch(&copied).copies.len(), selected.len());
    assert!(batch(&copied).transforms.is_empty() && batch(&copied).instances.is_empty());
    let copy_bytes = serde_json::to_value(batch(&copied)).unwrap();
    assert!(matches!(
        click(
            &mut panel,
            session.document(),
            &ctx,
            PreviewState::Idle,
            true,
            "Create copy"
        ),
        Some(ModelEvent::Changed)
    ));
    assert_eq!(
        serde_json::to_value(batch(&panel.request(session.document(), true).unwrap())).unwrap(),
        serde_json::to_value(batch(&moved)).unwrap()
    );
    assert!(panel.set_copy_modifier(true));
    assert_eq!(
        serde_json::to_value(batch(&panel.request(session.document(), true).unwrap())).unwrap(),
        copy_bytes
    );
    assert!(!panel.set_copy_modifier(true));
    assert!(panel.set_copy_modifier(false));
    assert_eq!(panel.selected(), &selected);
    assert_eq!(session.document(), &original);
}

#[test]
fn changing_targets_preserves_numeric_intent_and_empty_selection_cannot_apply() {
    let (session, scene, selected) = fixture();
    let first = BTreeSet::from([*selected.first().unwrap()]);
    let mut panel = PlacementPanel::new(
        session.document(),
        session.revision(),
        &scene,
        &first,
        PlacementMode::Move,
    )
    .unwrap();
    panel.translation[0].text = "=$offset + 5".into();
    panel.create_copy = true;
    let values = parameters(&panel.preview_request(session.document(), true).unwrap()).clone();
    panel
        .retarget(session.document(), session.revision(), &scene, &selected)
        .unwrap();
    let request = panel.request(session.document(), true).unwrap();
    assert_eq!(batch(&request).copies.len(), selected.len());
    assert_eq!(parameters(&request), &values);
    let stamp = panel.stamp();
    let bytes = serde_json::to_value(batch(&request)).unwrap();
    panel
        .retarget(session.document(), session.revision(), &scene, &selected)
        .unwrap();
    assert_eq!(panel.stamp(), stamp);
    assert_eq!(
        serde_json::to_value(batch(&panel.request(session.document(), true).unwrap())).unwrap(),
        bytes
    );
    panel
        .retarget(
            session.document(),
            session.revision(),
            &scene,
            &BTreeSet::new(),
        )
        .unwrap();
    assert!(panel.request(session.document(), true).is_err());
    panel
        .retarget(session.document(), session.revision(), &scene, &first)
        .unwrap();
    assert_eq!(
        parameters(&panel.request(session.document(), true).unwrap()),
        &values
    );
}
#[test]
fn numeric_placement_fields_stay_within_a_resizable_inspector_across_repeated_frames() {
    let (session, scene, selected) = fixture();
    let original = session.document().clone();
    for width in [180., 250.] {
        let ctx = egui::Context::default();
        let mut panel = PlacementPanel::new(
            session.document(),
            session.revision(),
            &scene,
            &selected,
            PlacementMode::Move,
        )
        .unwrap();
        for iteration in 0..120 {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200., 1200.),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let inspector = egui::Panel::right("numeric-placement-size")
                        .default_size(width)
                        .min_size(0.)
                        .resizable(true)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                let available = ui.available_width();
                                assert!(
                                    panel
                                        .show(ui, session.document(), true, PreviewState::Idle)
                                        .is_none()
                                );
                                assert!(ui.min_rect().width() <= available + 1.);
                            });
                        });
                    assert!(
                        inspector.response.rect.width() <= width + 1.,
                        "Frame {iteration}: Inspector grew to {}",
                        inspector.response.rect.width()
                    );
                    assert!(ui.available_width() >= 940.);
                },
            )
            .drop_without_applying_deltas();
        }
    }
    assert_eq!(session.document(), &original);
}

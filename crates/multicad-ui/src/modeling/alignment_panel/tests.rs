use super::*;
use crate::{
    camera::Camera,
    modeling::{TerminationViewport, native_fixture},
};
use multicad_core::{
    commands::{Actor, Channel, Session},
    model::{PrimitiveParameters, PrimitiveRequest, length},
    references::TopologyKind,
};
use multicad_model::Compiler;
use std::sync::Arc;
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn fixture() -> (Session, PrimitiveRequest, PrimitiveRequest, Arc<Scene>) {
    let mut session =
        Session::new(Document::new("Exact native alignment controls").unwrap()).unwrap();
    let a = PrimitiveRequest::new(
        "Moving",
        PrimitiveParameters::Box {
            size: [length(10.), length(10.), length(5.)],
            frame: Default::default(),
        },
    );
    let b = PrimitiveRequest::new(
        "Target",
        PrimitiveParameters::Box {
            size: [length(10.), length(10.), length(5.)],
            frame: model::PrimitiveFrame {
                origin: [length(20.), length(0.), length(0.)],
                ..Default::default()
            },
        },
    );
    for request in [&a, &b] {
        let prepared =
            model::prepare_primitive(&session, session.revision(), actor(), request).unwrap();
        session.commit(prepared.prepared).unwrap();
    }
    let native = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let scene = native_fixture::scene(session.document(), native.snapshot(), 0).unwrap();
    (session, a, b, scene)
}
fn pick(
    panel: &mut AlignmentPanel,
    session: &Session,
    scene: &Scene,
    side: Side,
    point: [f32; 3],
) -> bool {
    panel.picking = Some(side);
    let camera = Camera {
        yaw: 0.,
        pitch: std::f32::consts::FRAC_PI_2,
        scale: 10.,
        target: [15., 5., 0.],
        perspective: false,
    };
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500., 500.));
    let p = camera.project(point, rect);
    let ctx = egui::Context::default();
    let mut changed = false;
    ctx.run_ui(Default::default(), |ui| {
        changed = panel.viewport(
            session.document(),
            session.revision(),
            ContentHash::document(session.document()).unwrap(),
            TerminationViewport {
                scene: Some(scene),
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
fn actual_named_native_face_picks_create_one_frozen_source_and_dynamic_target_alignment_and_reopen()
{
    let (mut session, a, b, scene) = fixture();
    let original = session.document().clone();
    let mut panel = AlignmentPanel::new(
        session.document(),
        session.revision(),
        &scene,
        &BTreeSet::from([a.body]),
    )
    .unwrap();
    assert!(panel.request(session.document(), true).is_err());
    assert!(
        pick(&mut panel, &session, &scene, Side::Source, [5., 5., 5.]),
        "{:?}",
        panel.error
    );
    assert!(
        pick(&mut panel, &session, &scene, Side::Target, [25., 5., 5.]),
        "{:?}",
        panel.error
    );
    panel.offset[2].text = "2".into();
    let request = panel.request(session.document(), true).unwrap();
    let AlignmentCommand::Part(ref request) = request else {
        panic!()
    };
    assert_eq!(request.references[0].source, a.feature);
    assert_eq!(request.references[1].source, b.body);
    assert!(
        request
            .references
            .iter()
            .all(|r| r.intent.attachment.is_some())
    );
    let second = panel.request(session.document(), true).unwrap();
    let AlignmentCommand::Part(second) = second else {
        panic!()
    };
    assert_eq!(second.feature, request.feature);
    assert_eq!(second.references, request.references);
    let prepared =
        model::prepare_alignment(&session, session.revision(), actor(), request).unwrap();
    let native = Compiler::default()
        .compile(prepared.prepared.document(), session.revision())
        .unwrap();
    assert_eq!(
        native.snapshot().shapes[&request.feature].bounds.min,
        [20., 0., 2.]
    );
    assert_eq!(
        native.snapshot().shapes[&request.feature].bounds.max,
        [30., 10., 7.]
    );
    assert_eq!(session.document(), &original);
    let feature = request.feature;
    session.commit(prepared.prepared).unwrap();
    let existing = AlignmentPanel::existing(session.document(), feature).unwrap();
    let AlignmentCommand::EditPart {
        parameters,
        references,
        ..
    } = existing.request(session.document(), true).unwrap()
    else {
        panic!()
    };
    assert_eq!(parameters, second.parameters);
    assert_eq!(references, second.references);
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &original);
}
#[test]
fn overlapping_instance_source_pick_is_component_local_and_leaves_part_geometry_untouched() {
    let (mut session, a, b, _) = fixture();
    let copy = model::InstanceCopyRequest::new(
        model::InstanceCopySource::Part {
            source: BodyInput {
                body: a.body,
                producer: a.feature,
            },
        },
        Default::default(),
    );
    let prepared = model::prepare_instance_copies(
        &session,
        session.revision(),
        actor(),
        std::slice::from_ref(&copy),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let original = session.document().clone();
    let native = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let scene = native_fixture::scene(session.document(), native.snapshot(), 0).unwrap();
    let mut panel = AlignmentPanel::new(
        session.document(),
        session.revision(),
        &scene,
        &BTreeSet::from([copy.instance]),
    )
    .unwrap();
    assert!(
        pick(&mut panel, &session, &scene, Side::Source, [5., 5., 5.]),
        "{:?}",
        panel.error
    );
    assert!(
        pick(&mut panel, &session, &scene, Side::Target, [25., 5., 5.]),
        "{:?}",
        panel.error
    );
    let command = panel.request(session.document(), true).unwrap();
    let AlignmentCommand::Instance(request) = &command else {
        panic!()
    };
    assert_eq!(request.references[0].source, a.body);
    assert_eq!(request.references[1].source, b.body);
    assert_eq!(command.instances(), vec![copy.instance]);
    let prepared = command.prepare(&session, actor()).unwrap();
    let native = Compiler::default()
        .compile(prepared.prepared.document(), session.revision())
        .unwrap();
    assert_eq!(
        native.snapshot().occurrences[&copy.instance].bounds.min,
        [20., 0., 0.]
    );
    assert_eq!(
        native.snapshot().shapes[&a.feature].bounds.min,
        [0., 0., 0.]
    );
    assert_eq!(
        prepared.prepared.document().get(a.body),
        original.get(a.body)
    );
    assert_eq!(session.document(), &original);
}
#[test]
fn native_edge_and_vertex_filters_author_only_exact_descriptor_intent_and_stale_pick_is_neutral() {
    let (session, a, _b, scene) = fixture();
    let original = session.document().clone();
    let mut panel = AlignmentPanel::new(
        session.document(),
        session.revision(),
        &scene,
        &BTreeSet::from([a.body]),
    )
    .unwrap();
    for (mode, kind, source, target) in [
        (
            AlignmentMode::Axis,
            TopologyKind::Edge,
            [5., 0., 5.],
            [25., 0., 5.],
        ),
        (
            AlignmentMode::Position,
            TopologyKind::Vertex,
            [0., 0., 5.],
            [20., 0., 5.],
        ),
    ] {
        panel.mode = mode;
        panel.source = None;
        panel.target = None;
        assert!(
            pick(&mut panel, &session, &scene, Side::Source, source),
            "{:?}",
            panel.error
        );
        assert!(
            pick(&mut panel, &session, &scene, Side::Target, target),
            "{:?}",
            panel.error
        );
        let command = panel.request(session.document(), true).unwrap();
        let AlignmentCommand::Part(request) = &command else {
            panic!()
        };
        assert!(
            request
                .references
                .iter()
                .all(|r| r.intent.kind == kind && r.intent.attachment.is_none())
        );
        let prepared = command.prepare(&session, actor()).unwrap();
        Compiler::default()
            .compile(prepared.prepared.document(), session.revision())
            .unwrap();
    }
    let before = panel.source.as_ref().unwrap().input.clone();
    let wrong = Scene::new(
        crate::viewport::SceneStamp {
            revision: session.revision() + 1,
            ..scene.stamp()
        },
        scene.geometries().values().cloned().collect(),
        scene.reference_instances().to_vec(),
    )
    .unwrap();
    assert!(!pick(
        &mut panel,
        &session,
        &wrong,
        Side::Source,
        [0., 0., 5.]
    ));
    assert!(panel.error.as_ref().unwrap().contains("stale"));
    assert_eq!(panel.source.as_ref().unwrap().input, before);
    assert_eq!(session.document(), &original);
}
#[test]
fn multiple_semantic_face_frames_require_an_explicit_inspector_choice_instead_of_first_frame() {
    use crate::viewport::Geometry;
    use multicad_core::references::Origin;
    let (session, a, _b, scene) = fixture();
    let original = session.document().clone();
    let native = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let shape = &native.snapshot().shapes[&a.feature];
    let mut topology = shape.topology.elements.clone();
    let top = topology
        .iter_mut()
        .find(|e| {
            e.key.kind == TopologyKind::Face
                && e.frames
                    .iter()
                    .any(|f| f.frame.origin_mm[2] == 5. && f.frame.normal()[2].abs() == 1.)
        })
        .unwrap();
    let mut additional = top.frames[0].clone();
    additional.origin = Origin {
        source: a.feature,
        role: "explicit alternate frame".into(),
    };
    top.origins.insert(additional.origin.clone());
    top.frames.push(additional.clone());
    // A detached topology fixture adds a second legitimate semantic frame to
    // the same real native face, exercising only the UI ambiguity decision.
    let replacement = Arc::new(
        Geometry::new(
            a.feature,
            shape.mesh.vertices.as_chunks::<3>().0.to_vec(),
            shape.mesh.indices.clone(),
            shape.mesh.face_indices.clone(),
            topology,
        )
        .unwrap(),
    );
    let geometries = scene
        .geometries()
        .iter()
        .map(|(id, g)| {
            if *id == a.feature {
                replacement.clone()
            } else {
                g.clone()
            }
        })
        .collect();
    let scene = Scene::new(
        scene.stamp(),
        geometries,
        scene.reference_instances().to_vec(),
    )
    .unwrap()
    .with_reference_aliases(native.snapshot().body_sources.clone())
    .unwrap()
    .with_model_evaluation(scene.model_evaluation().unwrap().clone())
    .unwrap();
    let mut panel = AlignmentPanel::new(
        session.document(),
        session.revision(),
        &scene,
        &BTreeSet::from([a.body]),
    )
    .unwrap();
    assert!(pick(
        &mut panel,
        &session,
        &scene,
        Side::Source,
        [5., 5., 5.]
    ));
    assert!(panel.source.is_none());
    assert_eq!(panel.pending.as_ref().unwrap().anchors.len(), 2);
    assert!(
        panel
            .request(session.document(), true)
            .unwrap_err()
            .contains("explicit semantic")
    );
    let ctx = egui::Context::default();
    let raw = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(400., 1300.),
        )),
        ..Default::default()
    };
    ctx.run_ui(raw(), |ui| {
        panel.show(ui, session.document(), true, PreviewState::Idle);
    })
    .drop_without_applying_deltas();
    let out = ctx.run_ui(raw(), |ui| {
        panel.show(ui, session.document(), true, PreviewState::Idle);
    });
    let pointer = out
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text().contains("explicit alternate frame") => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .expect("explicit choice button");
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        let mut input = raw();
        input.events = vec![
            egui::Event::PointerMoved(pointer),
            egui::Event::PointerButton {
                pos: pointer,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ];
        ctx.run_ui(input, |ui| {
            panel.show(ui, session.document(), true, PreviewState::Idle);
        })
        .drop_without_applying_deltas();
    }
    assert!(panel.pending.is_none());
    assert_eq!(
        panel
            .source
            .as_ref()
            .unwrap()
            .input
            .as_ref()
            .unwrap()
            .intent
            .attachment
            .as_ref()
            .unwrap()
            .anchor,
        additional.origin
    );
    assert_eq!(session.document(), &original);
}
#[test]
fn saved_part_alignment_repick_uses_unique_frozen_input_provenance_from_the_displayed_head() {
    let (mut session, a, _b, scene) = fixture();
    let mut panel = AlignmentPanel::new(
        session.document(),
        session.revision(),
        &scene,
        &BTreeSet::from([a.body]),
    )
    .unwrap();
    assert!(pick(
        &mut panel,
        &session,
        &scene,
        Side::Source,
        [5., 5., 5.]
    ));
    assert!(pick(
        &mut panel,
        &session,
        &scene,
        Side::Target,
        [25., 5., 5.]
    ));
    let command = panel.request(session.document(), true).unwrap();
    let AlignmentCommand::Part(request) = &command else {
        panic!()
    };
    let feature = request.feature;
    let prepared = command.prepare(&session, actor()).unwrap();
    session.commit(prepared.prepared).unwrap();
    let original = session.document().clone();
    let native = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let scene = native_fixture::scene(session.document(), native.snapshot(), 0).unwrap();
    assert_eq!(native.snapshot().bodies[&a.body], feature);
    // Isolate the explicit moving Part because its target now overlaps it.
    let scene = scene.visible_subset(&BTreeSet::from([a.body])).unwrap();
    let mut reopened = AlignmentPanel::existing(session.document(), feature).unwrap();
    assert!(
        pick(&mut reopened, &session, &scene, Side::Source, [25., 5., 5.]),
        "{:?}",
        reopened.error
    );
    let AlignmentCommand::EditPart { references, .. } =
        reopened.request(session.document(), true).unwrap()
    else {
        panic!()
    };
    assert_eq!(references[0].source, a.feature);
    assert_eq!(references[0].intent, request.references[0].intent);
    assert_eq!(session.document(), &original);
}
#[test]
fn alignment_preview_keeps_new_offset_formula_bindings_across_parameter_rename() {
    use multicad_core::document::{Entity, Mutation};
    let (mut session, a, _b, _) = fixture();
    let mut parameter = Entity::new(
        "gap",
        EntityData::Parameter {
            expression: "5".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    session
        .apply(multicad_core::commands::Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Parameter".into(),
            edits: vec![Mutation::Insert(parameter.clone())],
        })
        .unwrap();
    let native = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let scene = native_fixture::scene(session.document(), native.snapshot(), 0).unwrap();
    let mut panel = AlignmentPanel::new(
        session.document(),
        session.revision(),
        &scene,
        &BTreeSet::from([a.body]),
    )
    .unwrap();
    let frame = AlignmentParameters::default().source;
    for side in [Side::Source, Side::Target] {
        panel.set_anchor(
            side,
            AnchorChoice {
                anchor: frame.clone(),
                input: None,
                label: "Explicit XY".into(),
            },
        );
    }
    panel.offset[2].text = "=$gap + 2".into();
    let request = panel.request(session.document(), true).unwrap();
    panel.remember(&request);
    parameter.name = "renamed_gap".into();
    let renamed = session
        .document()
        .edited(&[Mutation::Replace(parameter)])
        .unwrap();
    let AlignmentCommand::Part(rebound) = panel.request(&renamed, true).unwrap() else {
        panic!()
    };
    let AlignmentCommand::Part(original) = request else {
        panic!()
    };
    assert_eq!(rebound.parameters.offset, original.parameters.offset);
}
#[test]
fn alignment_controls_do_not_grow_a_resizable_inspector_across_idle_and_picking_frames() {
    let (session, a, _b, scene) = fixture();
    let original = session.document().clone();
    for width in [180., 250.] {
        let ctx = egui::Context::default();
        let mut panel = AlignmentPanel::new(
            session.document(),
            session.revision(),
            &scene,
            &BTreeSet::from([a.body]),
        )
        .unwrap();
        for frame in 0..120 {
            if frame == 40 {
                panel.picking = Some(Side::Source);
            }
            if frame == 80 {
                panel.picking = None;
                panel.mode = AlignmentMode::Axis;
            }
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200., 1200.),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let inspector = egui::Panel::right("alignment-size-regression")
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
                                assert!(
                                    ui.min_rect().width() <= available + 1.,
                                    "Frame {frame}: content grew from {available} to {}",
                                    ui.min_rect().width()
                                );
                            });
                        });
                    assert!(
                        inspector.response.rect.width() <= width + 1.,
                        "Frame {frame}: inspector grew from {width} to {}",
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

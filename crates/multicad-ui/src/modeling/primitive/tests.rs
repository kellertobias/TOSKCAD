use super::*;
use multicad_core::{
    document::{Entity, EntityData},
    model::ResolvedPrimitive,
    parameters::{self, Unit},
};
fn panel(kind: ModelingTool) -> PrimitivePanel {
    let parameters = match kind {
        ModelingTool::Box => PrimitiveParameters::Box {
            size: [model::length(100.), model::length(60.), model::length(20.)],
            frame: PrimitiveFrame::default(),
        },
        ModelingTool::Cylinder => PrimitiveParameters::Cylinder {
            radius: model::length(20.),
            height: model::length(50.),
            frame: PrimitiveFrame::default(),
        },
        ModelingTool::Sphere => PrimitiveParameters::Sphere {
            radius: model::length(20.),
            frame: PrimitiveFrame::default(),
        },
        _ => unreachable!(),
    };
    PrimitivePanel::new(PrimitiveRequest::new("Editable body", parameters)).unwrap()
}
#[test]
fn orientation_and_formula_origin_are_canonical_source_and_not_display_moves() {
    let mut document = Document::new("Primitive fields").unwrap();
    document
        .insert(Entity::new(
            "offset",
            EntityData::Parameter {
                expression: "25".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        ))
        .unwrap();
    let before = document.clone();
    for kind in [
        ModelingTool::Box,
        ModelingTool::Cylinder,
        ModelingTool::Sphere,
    ] {
        let mut panel = panel(kind);
        let original = panel.request(&document, true).unwrap();
        panel.origin[0].text = "=$offset + 5".into();
        panel.origin[1].text = "-12".into();
        panel.set_plane(Datum::Xz);
        let request = panel.request(&document, true).unwrap();
        assert_eq!(
            (request.feature, request.body),
            (original.feature, original.body)
        );
        let resolved = request
            .parameters
            .resolve(&parameters::evaluate(&document).unwrap())
            .unwrap();
        let frame = match resolved {
            ResolvedPrimitive::Box { frame, .. }
            | ResolvedPrimitive::Cylinder { frame, .. }
            | ResolvedPrimitive::Sphere { frame, .. } => frame,
        };
        assert_eq!(frame.origin, [30., -12., 0.]);
        assert_eq!(frame.u, [1., 0., 0.]);
        assert_eq!(frame.v, [0., 0., 1.]);
        assert_eq!(frame.n, [0., -1., 0.]);
        assert_eq!(document, before);
    }
}
#[test]
fn automatic_preview_retains_the_validated_parameter_identity() {
    use super::super::{ExtrudeCapabilities, form::ModelForm};

    let mut document = Document::new("Primitive binding").unwrap();
    let mut width = Entity::new(
        "width",
        EntityData::Parameter {
            expression: "25".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    document.insert(width.clone()).unwrap();
    let mut primitive = panel(ModelingTool::Box);
    primitive.size[0].text = "=$width".into();
    let mut form = ModelForm::Primitive(Box::new(primitive));
    let capabilities = ExtrudeCapabilities::distance_bodies();
    let request = form.request(&document, capabilities).unwrap();
    form.remember_preview(&request);

    width.name = "renamed_width".into();
    document.replace(width.clone()).unwrap();
    let ModelRequest::Primitive(original) = request else {
        panic!("Expected a primitive request");
    };
    let ModelRequest::Primitive(after_rename) = form.request(&document, capabilities).unwrap()
    else {
        panic!("Expected a primitive request");
    };
    assert_eq!(after_rename.parameters, original.parameters);

    document.remove(width.id).unwrap();
    assert!(form.request(&document, capabilities).is_err());
}
#[test]
fn primitive_controls_share_edit_preview_apply_stamps_and_allow_intentional_names() {
    let mut document = Document::new("Primitive events").unwrap();
    document
        .insert(Entity::new(
            "Intentionally duplicated label",
            EntityData::Group {},
        ))
        .unwrap();
    let mut panel = panel(ModelingTool::Box);
    let stamp = panel.stamp();
    panel.size[0].text = "135".into();
    let Some(ModelEvent::Preview {
        stamp: new,
        request: ModelRequest::Primitive(request),
    }) = panel.finish_frame(
        true,
        Some(FormAction::Preview),
        &document,
        true,
        PreviewState::Ready { stamp },
    )
    else {
        panic!("Preview must use this frame's new values")
    };
    assert_ne!(new, stamp);
    let PrimitiveParameters::Box { size, .. } = request.parameters else {
        panic!()
    };
    assert_eq!(size[0].value_in(Unit::Millimeter, None).unwrap(), 135.);
    assert!(
        panel
            .finish_frame(
                false,
                Some(FormAction::Apply),
                &document,
                true,
                PreviewState::Ready { stamp }
            )
            .is_none()
    );
    assert!(matches!(
        panel.finish_frame(
            false,
            Some(FormAction::Apply),
            &document,
            true,
            PreviewState::Ready { stamp: new }
        ),
        Some(ModelEvent::Apply)
    ));
    assert!(matches!(
        panel.finish_frame(
            true,
            Some(FormAction::Cancel),
            &document,
            true,
            PreviewState::Ready { stamp: new }
        ),
        Some(ModelEvent::Cancel)
    ));
    assert!(panel.request(&document, false).is_err());
    panel.name.clear();
    assert!(panel.request(&document, true).is_err());
    panel.name = "Intentionally duplicated label".into();
    assert_eq!(
        panel.request(&document, true).unwrap().name,
        "Intentionally duplicated label"
    );
}
#[test]
fn primitive_forms_stay_narrow_and_source_neutral_across_frames() {
    let document = Document::new("Narrow primitive Inspector").unwrap();
    let before = document.clone();
    for kind in [
        ModelingTool::Box,
        ModelingTool::Cylinder,
        ModelingTool::Sphere,
    ] {
        let context = egui::Context::default();
        let mut panel = panel(kind);
        let ids = panel.request(&document, true).unwrap();
        for _ in 0..90 {
            context
                .run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1200., 1000.),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        let inspector = egui::Panel::right("primitive-inspector")
                            .default_size(180.)
                            .min_size(0.)
                            .resizable(true)
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    assert!(
                                        panel
                                            .show(ui, &document, true, PreviewState::Idle)
                                            .is_none()
                                    );
                                });
                            });
                        assert!(inspector.response.rect.width() <= 181.);
                    },
                )
                .drop_without_applying_deltas();
            let request = panel.request(&document, true).unwrap();
            assert_eq!((request.body, request.feature), (ids.body, ids.feature));
        }
    }
    assert_eq!(document, before);
}

#[test]
fn saved_arbitrary_primitive_frame_and_formulas_survive_reopen_and_native_edit() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        constraint_sketch::Dimension,
        parameters::BoundExpression,
        project_file::{self, ProjectFile},
    };
    let mut document = Document::new("Stored formula controls").unwrap();
    document
        .insert(Entity::new(
            "width",
            EntityData::Parameter {
                expression: "75".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        ))
        .unwrap();
    let formula = |text| Dimension::Expression {
        expression: BoundExpression::bind(&document, text, "mm", None).unwrap(),
    };
    let width = formula("=$width + 5");
    let frame = PrimitiveFrame {
        origin: [
            formula("=$width - 50"),
            model::length(3.),
            model::length(4.),
        ],
        u: [0.6, 0.8, 0.],
        v: [0., 0., 1.],
    };
    let request = PrimitiveRequest::new(
        "Saved body name",
        PrimitiveParameters::Box {
            size: [width.clone(), model::length(30.), model::length(20.)],
            frame: frame.clone(),
        },
    );
    let mut session = Session::new(document).unwrap();
    let actor = Actor::local(Channel::Ui);
    session
        .commit(
            model::prepare_primitive(&session, 0, actor.clone(), &request)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let file = ProjectFile::new(session.document().clone()).unwrap();
    let mut bytes = Vec::new();
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document;
    let mut session = Session::new(reopened).unwrap();
    let stored = model::read_primitive(session.document(), request.feature).unwrap();
    let mut panel = PrimitivePanel::existing(PrimitiveRequest {
        body: stored.output.body,
        parameters: stored.parameters,
        ..request.clone()
    })
    .unwrap();
    assert!(panel.editing);
    assert_eq!(panel.request(session.document(), true).unwrap(), request);
    panel.height.text = "60".into(); // Irrelevant cylinder control cannot change a box.
    panel.size[2].text = "24".into();
    let Some(ModelEvent::Preview {
        request: ModelRequest::Primitive(changed),
        ..
    }) = panel.finish_frame(
        true,
        Some(FormAction::Preview),
        session.document(),
        true,
        PreviewState::Idle,
    )
    else {
        panic!("Reopened arbitrary frame must produce a valid edit");
    };
    assert_eq!(
        (changed.feature, changed.body),
        (request.feature, request.body)
    );
    assert_eq!(changed.name, request.name);
    let PrimitiveParameters::Box {
        size,
        frame: edited_frame,
    } = &changed.parameters
    else {
        panic!("The existing primitive must stay a box");
    };
    assert_eq!(edited_frame, &frame);
    assert_eq!(size[0], width);
    assert_eq!(size[2], model::length(24.));

    session
        .commit(
            model::prepare_primitive_edit(&session, 0, actor, changed.feature, &changed.parameters)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let compiled = multicad_model::Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let snapshot = compiled.snapshot();
    assert_eq!(snapshot.bodies[&request.body], request.feature);
    let shape = &snapshot.shapes[&request.feature];
    assert_eq!(shape.solid_count, 1);
    assert!((shape.volume_mm3 - 80. * 30. * 24.).abs() < 1e-6);
    for (actual, expected) in shape
        .bounds
        .min
        .into_iter()
        .chain(shape.bounds.max)
        .zip([25., -11.4, 4., 92.2, 67., 34.])
    {
        assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
    }
}

#[test]
fn viewport_corner_drag_updates_two_draft_dimensions_only_when_input_is_allowed() {
    let session =
        multicad_core::commands::Session::new(Document::new("Primitive corner resize").unwrap())
            .unwrap();
    let before = session.document().clone();
    let mut panel = panel(ModelingTool::Box);
    let context = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.));
    let camera = crate::camera::Camera {
        yaw: 0.,
        pitch: std::f32::consts::FRAC_PI_2,
        scale: 4.,
        target: [50., 30., 10.],
        perspective: false,
    };
    let project = |point| {
        let [x, y, _] = camera.project(point, rect);
        egui::pos2(x, y)
    };
    let frame = |panel: &mut PrimitivePanel, events, start_allowed| {
        let mut result = (false, false, false);
        context
            .run_ui(
                egui::RawInput {
                    screen_rect: Some(rect),
                    events,
                    ..Default::default()
                },
                |ui| {
                    result = panel.viewport(ui, &session, camera, rect, start_allowed, true);
                },
            )
            .drop_without_applying_deltas();
        result
    };
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: egui::Modifiers::NONE,
    };
    let corner = project([100., 60., 20.]);
    let target = project([125., 72., 20.]);
    let request = panel.request(session.document(), true).unwrap();
    for allowed in [false, true] {
        frame(&mut panel, vec![], allowed);
        let (owned, _, _) = frame(
            &mut panel,
            vec![egui::Event::PointerMoved(corner), button(corner, true)],
            allowed,
        );
        assert_eq!(owned, allowed);
        let (_, changed, _) = frame(&mut panel, vec![egui::Event::PointerMoved(target)], allowed);
        assert_eq!(changed, allowed);
        let (_, _, released) = frame(&mut panel, vec![button(target, false)], allowed);
        assert_eq!(released, allowed);
        if !allowed {
            assert_eq!(panel.request(session.document(), true).unwrap(), request);
        }
    }
    let ResolvedPrimitive::Box { size_mm, frame } = panel
        .request(session.document(), true)
        .unwrap()
        .parameters
        .resolve(session.parameters())
        .unwrap()
    else {
        panic!("Resize must preserve the shape type");
    };
    for (actual, expected) in size_mm.into_iter().zip([125., 72., 20.]) {
        assert!((actual - expected).abs() < 1e-5, "{size_mm:?}");
    }
    assert_eq!(frame.origin, [0.; 3]);
    assert_eq!(session.document(), &before);
}

use super::*;

fn canvas(
    ctx: &egui::Context,
    controller: &mut SymbolController,
    session: &Session,
    events: Vec<egui::Event>,
) -> bool {
    let mut owned = false;
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.));
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(rect),
            events,
            focused: true,
            ..Default::default()
        },
        |ui| {
            owned = controller
                .placement_viewport(
                    ui,
                    session,
                    &Default::default(),
                    crate::modeling::PlacementViewport {
                        scene: None,
                        rect,
                        camera: crate::camera::Camera {
                            yaw: 0.,
                            pitch: 0.8,
                            scale: 2.4,
                            target: [0.; 3],
                            perspective: false,
                        },
                        basis: Default::default(),
                        start_allowed: true,
                    },
                )
                .unwrap();
        },
    );
    output.textures_delta.clear();
    owned
}
fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}
#[test]
fn library_gizmo_moves_display_preserves_inputs_and_validates_without_committing() {
    let (mut source, part, width) = fixture();
    let definition_id = embed(&mut source, part.body, width);
    let content = symbols::read_definition(source.document(), definition_id).unwrap();
    let mut session = Session::new(Document::new("Graphical Library receiver").unwrap()).unwrap();
    let before = session.document().clone();
    let resources = ProjectResources::default();
    let mut controller = controller();
    controller.set_library_snapshot(vec![content]).unwrap();
    begin(
        &mut controller,
        &session,
        SymbolTool::Library,
        Default::default(),
    );
    controller
        .place_loaded_library(&session, &resources)
        .unwrap();
    wait(&mut controller, &session, &resources);
    // An exposed override and pre-existing orientation must survive translation.
    let form = controller.form.as_mut().unwrap();
    let Operation::Place { pose, values, .. } = &mut form.operation else {
        panic!()
    };
    pose.axis = ["1".into(), "0".into(), "0".into()];
    pose.angle = "30".into();
    let field = values.0.get_mut(&width).unwrap();
    field.enabled = true;
    field.text = "77".into();
    form.changed();
    controller
        .preview(&session, &resources, Channel::Ui)
        .unwrap();
    wait(&mut controller, &session, &resources);
    assert!(controller.can_apply(&session, &resources));
    let (request, old_bounds) = {
        let Operation::Place {
            request: Some(request),
            ..
        } = &controller.form.as_ref().unwrap().operation
        else {
            panic!()
        };
        (
            request.clone(),
            controller
                .display_scene(&session, &resources)
                .unwrap()
                .instance_bounds(request.instance)
                .unwrap(),
        )
    };
    let ctx = egui::Context::default();
    canvas(&ctx, &mut controller, &session, vec![]);
    let start = egui::pos2(475., 300.);
    let end = egui::pos2(523., 300.);
    assert!(canvas(
        &ctx,
        &mut controller,
        &session,
        vec![egui::Event::PointerMoved(start), pointer(start, true)]
    ));
    assert!(!controller.can_apply(&session, &resources));
    assert!(
        controller.display_scene(&session, &resources).is_some(),
        "pressing a handle retains the candidate display"
    );
    assert!(canvas(
        &ctx,
        &mut controller,
        &session,
        vec![egui::Event::PointerMoved(end)]
    ));
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), 0);
    let display = controller.display_scene(&session, &resources).unwrap();
    assert!(!display.is_full_model());
    assert!(display.model_evaluation().is_none());
    let bounds = display.world_bounds().unwrap();
    assert!((bounds[0][0] - old_bounds[0][0] - 20.).abs() < 1e-4);
    assert!(canvas(
        &ctx,
        &mut controller,
        &session,
        vec![pointer(end, false)]
    ));
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), 0);
    wait(&mut controller, &session, &resources);
    assert!(
        controller.can_apply(&session, &resources),
        "{:?}",
        controller.error
    );
    let Operation::Place {
        request: Some(current),
        ..
    } = &controller.form.as_ref().unwrap().operation
    else {
        panic!()
    };
    assert_eq!(current.instance, request.instance);
    assert_eq!(current.record, request.record);
    assert_eq!(current.ids, request.ids);
    assert_eq!(current.overrides, request.overrides);
    assert_eq!(
        current.placement.rotation_xyzw,
        request.placement.rotation_xyzw
    );
    assert!((current.placement.translation_mm[0] - 20.).abs() < 1e-4);
    controller.apply(&mut session, &resources).unwrap();
    assert_eq!(session.revision(), 1);
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(session.revision(), actor()).unwrap();
    assert!(session.document().get(request.instance).is_some());
}

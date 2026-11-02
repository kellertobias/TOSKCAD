//! Production native preview/commit lifecycle; no OS-window interaction claim.
use super::*;
use multicad_core::{
    commands::{Actor, Channel, Session},
    model::{self, LevelDefinition, length},
    project_file::ProjectResources,
};
use multicad_ui::{
    camera::Camera,
    egui,
    modeling::{ModelToolController, ModelingTool, TerminationViewport},
};
use std::time::{Duration, Instant};

fn form_frame(
    ctx: &egui::Context,
    controller: &mut ModelToolController,
    session: &mut Session,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900., 2400.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            controller
                .show(ui, session, &ProjectResources::default())
                .unwrap();
        },
    );
    output.textures_delta.clear();
    output
}
fn click_control(
    ctx: &egui::Context,
    controller: &mut ModelToolController,
    session: &mut Session,
    label: &str,
) {
    let output = form_frame(ctx, controller, session, vec![]);
    let bounds = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find_map(|(_, node)| {
            (node.label() == Some(label))
                .then(|| node.bounds())
                .flatten()
        })
        .unwrap_or_else(|| panic!("missing painted control: {label}"));
    let pos = egui::pos2(
        ((bounds.x0 + bounds.x1) * 0.5) as f32,
        ((bounds.y0 + bounds.y1) * 0.5) as f32,
    );
    for pressed in [true, false] {
        form_frame(
            ctx,
            controller,
            session,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}
fn plan_click(
    ctx: &egui::Context,
    controller: &mut ModelToolController,
    session: &Session,
    scene: Option<&viewport::Scene>,
    point: [f32; 3],
    opening: bool,
) {
    let mut output = ctx.run_ui(Default::default(), |ui| {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.));
        let camera = Camera {
            yaw: 0.,
            pitch: std::f32::consts::FRAC_PI_2,
            scale: 0.1,
            target: [3000., 0., 3000.],
            perspective: false,
        };
        let p = camera.project(point, rect);
        let view = TerminationViewport {
            scene,
            report: None,
            camera,
            rect,
            pointer: Some(egui::pos2(p[0], p[1])),
            clicked: true,
            painter: ui.painter(),
            section: None,
        };
        assert!(if opening {
            controller.opening_viewport(session, view)
        } else {
            controller.outline_viewport(session, view)
        });
    });
    output.textures_delta.clear();
}
fn preview(controller: &mut ModelToolController, session: &Session) -> Arc<viewport::Scene> {
    let resources = ProjectResources::default();
    let source = session.document().clone();
    controller
        .preview(session, &resources, Channel::Ui)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !controller.can_apply(session, &resources) && Instant::now() < deadline {
        controller.poll(session, &resources);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        controller.can_apply(session, &resources),
        "current native preview must permit Apply"
    );
    assert_eq!(session.document(), &source);
    controller.display_scene(session, &resources).unwrap()
}
fn canonical_scene(session: &Session) -> Arc<viewport::Scene> {
    let mut backend = Backend::new();
    backend
        .request(
            session.document(),
            viewport::SceneStamp {
                document: session.document().id(),
                revision: session.revision(),
                generation: 1,
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(reply) = backend.poll() {
            return reply.result.unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "native canonical scene timed out"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn graphical_upper_level_wall_and_openings_use_real_native_preview_cancel_apply_and_history() {
    let mut session =
        Session::new(Document::new("Graphical native Architecture").unwrap()).unwrap();
    let actor = Actor::local(Channel::Ui);
    let (level, prepared) = model::prepare_level(
        &session,
        session.revision(),
        actor.clone(),
        "Upper",
        &LevelDefinition {
            elevation: length(3000.),
            height: length(3000.),
            slab_thickness: length(200.),
        },
    )
    .unwrap();
    session.commit(prepared).unwrap();
    let initial = session.document().clone();
    let resources = ProjectResources::default();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    multicad_ui::theme::configure(&ctx);
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(Backend::new()));
    controller.set_architecture_level(Some(level));
    controller
        .begin_wall(&session, &resources, Channel::Ui)
        .unwrap();
    click_control(&ctx, &mut controller, &mut session, "Draw in viewport");
    for point in [[0., 0., 3000.], [6000., 0., 3000.]] {
        plan_click(&ctx, &mut controller, &session, None, point, false);
    }
    click_control(&ctx, &mut controller, &mut session, "Finish");
    let candidate = preview(&mut controller, &session);
    assert!(candidate.architecture_plan().unwrap().is_ok());
    controller.cancel();
    assert_eq!(session.document(), &initial);
    controller
        .begin_wall(&session, &resources, Channel::Ui)
        .unwrap();
    click_control(&ctx, &mut controller, &mut session, "Draw in viewport");
    for point in [[0., 0., 3000.], [6000., 0., 3000.]] {
        plan_click(&ctx, &mut controller, &session, None, point, false);
    }
    click_control(&ctx, &mut controller, &mut session, "Finish");
    preview(&mut controller, &session);
    let wall = controller.apply(&mut session, &resources).unwrap()[0].clone();
    let definition = model::read_wall(session.document(), wall.feature).unwrap();
    assert_eq!(definition.level, Some(level));
    assert_eq!(
        definition.path,
        vec![[length(0.), length(0.)], [length(6000.), length(0.)]]
    );
    for (tool, x) in [(ModelingTool::Door, 1500.), (ModelingTool::Window, 4000.)] {
        let native = canonical_scene(&session);
        let before = session.document().clone();
        controller
            .begin_opening(&session, &resources, tool, Some(wall.feature), Channel::Ui)
            .unwrap();
        plan_click(
            &ctx,
            &mut controller,
            &session,
            Some(&native),
            [x, 0., 3000.],
            true,
        );
        let checked = preview(&mut controller, &session);
        assert_eq!(
            checked
                .architecture_plan()
                .unwrap()
                .as_ref()
                .unwrap()
                .sources,
            std::collections::BTreeSet::from([wall.body])
        );
        assert_eq!(session.document(), &before);
        controller.apply(&mut session, &resources).unwrap();
        let after = session.document().clone();
        session.undo(session.revision(), actor.clone()).unwrap();
        assert_eq!(session.document(), &before);
        session.redo(session.revision(), actor.clone()).unwrap();
        assert_eq!(session.document(), &after);
    }
    let definition = model::read_wall(session.document(), wall.feature).unwrap();
    assert_eq!(definition.openings.len(), 2);
    assert_eq!(definition.openings[0].offset, length(1050.));
    assert_eq!(definition.openings[1].offset, length(3400.));
    let native = canonical_scene(&session);
    assert_eq!(native.reference_instances()[0].id, wall.body);
    assert!(
        native
            .architecture_plan()
            .unwrap()
            .as_ref()
            .unwrap()
            .vectors
            .iter()
            .any(|v| matches!(
                v.curve,
                multicad_core::drawings::VectorCurve::Ellipse { .. }
            ))
    );
    let before_move = session.document().clone();
    let opening_ids: Vec<_> = definition
        .openings
        .iter()
        .map(|opening| opening.id)
        .collect();
    controller
        .begin_existing(&session, &resources, wall.feature, Channel::Ui)
        .unwrap();
    click_control(&ctx, &mut controller, &mut session, "Draw in viewport");
    for point in [[0., 500., 3000.], [6000., 500., 3000.]] {
        plan_click(&ctx, &mut controller, &session, None, point, false);
    }
    click_control(&ctx, &mut controller, &mut session, "Finish");
    preview(&mut controller, &session);
    let edited = controller.apply(&mut session, &resources).unwrap();
    assert_eq!(edited[0].feature, wall.feature);
    assert_eq!(edited[0].body, wall.body);
    let after_move = session.document().clone();
    let moved = model::read_wall(session.document(), wall.feature).unwrap();
    assert_eq!(
        moved.path,
        vec![[length(0.), length(500.)], [length(6000.), length(500.)]]
    );
    assert_eq!(
        moved
            .openings
            .iter()
            .map(|opening| opening.id)
            .collect::<Vec<_>>(),
        opening_ids
    );
    session.undo(session.revision(), actor.clone()).unwrap();
    assert_eq!(session.document(), &before_move);
    session.redo(session.revision(), actor).unwrap();
    assert_eq!(session.document(), &after_move);
    let project = multicad_core::project_file::ProjectFile::new(after_move.clone()).unwrap();
    let mut bytes = Vec::new();
    multicad_core::project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let reopened = multicad_core::project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &multicad_core::project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert_eq!(reopened.document, after_move);
    let reopened = Session::new(reopened.document).unwrap();
    let native = canonical_scene(&reopened);
    let bounds = native.instance_bounds(wall.body).unwrap();
    assert_eq!(bounds[0][2], 3000.);
    assert!(bounds[0][1] > 0. && bounds[1][1] < 1000.);
    assert_eq!(
        native
            .architecture_plan()
            .unwrap()
            .as_ref()
            .unwrap()
            .source_hash,
        reopened.source_hash().unwrap()
    );
}

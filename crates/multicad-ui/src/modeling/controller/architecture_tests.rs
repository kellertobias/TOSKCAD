//! Form routing checks; a pending scripted lane is never treated as native proof.
use super::*;
use multicad_core::{
    document::{Document, EntityData},
    model,
};
use std::sync::Mutex;

#[derive(Default)]
struct Capture(Arc<Mutex<Option<Document>>>);
impl SceneBackend for Capture {
    fn request(&mut self, document: &Document, _: SceneStamp) -> Result<(), String> {
        *self.0.lock().unwrap() = Some(document.clone());
        Ok(())
    }
    fn poll(&mut self) -> Option<crate::viewport::SceneReply> {
        None
    }
}
fn controller() -> (ModelToolController, Arc<Mutex<Option<Document>>>) {
    let capture = Capture::default();
    let sent = capture.0.clone();
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(capture));
    (controller, sent)
}
#[test]
fn active_level_seeds_new_wall_stair_slab_and_equipment_but_never_reassigns_saved_features() {
    let (session, levels, _, features) = crate::architecture_view::tests::building();
    let resources = ProjectResources::default();
    let before = session.document().clone();
    let (mut controller, sent) = controller();
    controller.set_architecture_level(Some(levels[1]));
    controller
        .begin_wall(&session, &resources, Channel::Ui)
        .unwrap();
    controller
        .preview(&session, &resources, Channel::Ui)
        .unwrap();
    let wall_document = sent.lock().unwrap().clone().unwrap();
    let wall = model::walls(&wall_document).next().unwrap();
    assert_eq!(
        model::read_wall(&wall_document, wall.id).unwrap().level,
        Some(levels[1])
    );
    assert!(
        !controller.can_apply(&session, &resources),
        "a captured request is not a native receipt"
    );
    controller.cancel();
    controller
        .begin_stairs(&session, &resources, Channel::Ui)
        .unwrap();
    let ctx = crate::egui::Context::default();
    for point in [[1000., 500., 3000.], [2000., 500., 3000.]] {
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let camera = crate::camera::Camera {
                yaw: 0.,
                pitch: std::f32::consts::FRAC_PI_2,
                target: [0., 0., 3000.],
                scale: 0.1,
                perspective: false,
            };
            let rect = crate::egui::Rect::from_min_size(
                crate::egui::Pos2::ZERO,
                crate::egui::vec2(800., 600.),
            );
            let p = camera.project(point, rect);
            assert!(controller.outline_viewport(
                &session,
                super::super::TerminationViewport {
                    scene: None,
                    report: None,
                    camera,
                    rect,
                    pointer: Some(crate::egui::pos2(p[0], p[1])),
                    clicked: true,
                    painter: ui.painter(),
                    section: None,
                }
            ));
        });
        output.textures_delta.clear();
    }
    let Some(ModelForm::Architecture(panel)) = &controller.form else {
        panic!("stair form")
    };
    let super::super::architecture::ArchitectureCommand::Stair { stair, .. } =
        panel.request(session.document(), true).unwrap()
    else {
        panic!("stair request")
    };
    assert_eq!(stair.level, levels[1]);
    controller.cancel();
    controller
        .begin_architecture(&session, &resources, Channel::Ui)
        .unwrap();
    controller
        .preview(&session, &resources, Channel::Ui)
        .unwrap();
    let slab_document = sent.lock().unwrap().clone().unwrap();
    let slab = model::slabs(&slab_document)
        .find(|slab| session.document().get(slab.id).is_none())
        .unwrap();
    assert_eq!(
        model::read_slab(&slab_document, slab.id).unwrap().level,
        levels[1]
    );
    controller.cancel();
    assert!(ModelToolController::editable(
        session.document(),
        features[0]
    ));
    controller
        .begin_existing(&session, &resources, features[0], Channel::Ui)
        .unwrap();
    controller
        .preview(&session, &resources, Channel::Ui)
        .unwrap();
    assert_eq!(
        model::read_slab(sent.lock().unwrap().as_ref().unwrap(), features[0])
            .unwrap()
            .level,
        levels[0]
    );
    controller.cancel();
    controller
        .begin_equipment_preset(
            &session,
            &resources,
            Channel::Ui,
            multicad_core::equipment::Kind::Refrigerator,
        )
        .unwrap();
    // The existing Inspector must still require a graphical placement before Preview.
    assert!(
        controller
            .preview(&session, &resources, Channel::Ui)
            .is_err()
    );
    let ctx = crate::egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |ui| {
        let rect = crate::egui::Rect::from_min_size(
            crate::egui::Pos2::ZERO,
            crate::egui::vec2(800., 600.),
        );
        let camera = crate::camera::Camera {
            yaw: 0.,
            pitch: std::f32::consts::FRAC_PI_2,
            target: [0., 0., 3000.],
            scale: 0.1,
            perspective: false,
        };
        assert!(controller.outline_viewport(
            &session,
            super::super::TerminationViewport {
                scene: None,
                report: None,
                camera,
                rect,
                pointer: Some(rect.center()),
                clicked: true,
                painter: ui.painter(),
                section: None,
            }
        ));
    });
    output.textures_delta.clear();
    controller
        .preview(&session, &resources, Channel::Ui)
        .unwrap();
    let equipment_document = sent.lock().unwrap().clone().unwrap();
    let attachment = equipment_document.entities().find_map(|entity|
        matches!(&entity.data,EntityData::Extension{definition,..} if definition.type_id == multicad_core::equipment::ATTACHMENT_TYPE)
            .then(||multicad_core::equipment::read_attachment(&equipment_document,entity.id).unwrap())).unwrap();
    assert_eq!(attachment.floor, levels[1]);
    multicad_model::Compiler::default()
        .compile(&equipment_document, session.revision())
        .unwrap();
    assert_eq!(session.document(), &before);
}

#[test]
fn native_candidate_level_filter_uses_prepared_membership_and_keeps_canonical_context() {
    let (mut session, levels, bodies, _) = crate::architecture_view::tests::building();
    let source = session.document().clone();
    let resources = ProjectResources::default();
    let mut controller = ModelToolController::default();
    controller.set_backend(Box::new(super::super::native_fixture::NativeBackend::new()));
    controller.set_architecture_level(Some(levels[1]));
    controller
        .begin_wall(&session, &resources, Channel::Ui)
        .unwrap();
    controller
        .preview(&session, &resources, Channel::Ui)
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !controller.can_apply(&session, &resources) && std::time::Instant::now() < deadline {
        controller.poll(&session, &resources);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(controller.can_apply(&session, &resources));
    let native = controller.display_scene(&session, &resources).unwrap();
    let wall = controller.candidate.as_ref().unwrap().outputs[0].body;
    let prepared = controller
        .architecture_preview_levels(&session, &native)
        .unwrap();
    assert_eq!(prepared.level_for(wall), Some(Some(levels[1])));
    let mut context = crate::architecture_view::Context::default();
    context.sync(session.document(), session.revision());
    let mut camera = crate::camera::Camera::default();
    context.set_enabled(true, &mut camera);
    context.set_plan(true, &mut camera);
    let ground = context.preview_scene(&native, prepared).unwrap();
    assert_eq!(
        ground
            .instances()
            .iter()
            .map(|instance| instance.id)
            .collect::<Vec<_>>(),
        vec![bodies[0]]
    );
    assert!(Arc::ptr_eq(
        &ground,
        &context.preview_scene(&native, prepared).unwrap()
    ));
    context.recall.level = Some(levels[1]);
    let upper = context.preview_scene(&native, prepared).unwrap();
    let visible: std::collections::BTreeSet<_> = upper
        .instances()
        .iter()
        .map(|instance| instance.id)
        .collect();
    assert_eq!(visible, std::collections::BTreeSet::from([bodies[1], wall]));
    assert!(
        context.levels.as_ref().unwrap().level_for(wall).is_none(),
        "preview facts must not replace canonical level ownership"
    );
    for (id, geometry) in native.geometries() {
        assert!(Arc::ptr_eq(geometry, &upper.geometries()[id]));
    }
    assert_eq!(session.document(), &source);
    controller.apply(&mut session, &resources).unwrap();
    context.sync(session.document(), session.revision());
    let handoff = controller.display_scene(&session, &resources).unwrap();
    let levels = controller
        .architecture_preview_levels(&session, &handoff)
        .unwrap();
    let view = context.preview_scene(&handoff, levels).unwrap();
    assert_eq!(
        view.instances()
            .iter()
            .map(|instance| instance.id)
            .collect::<std::collections::BTreeSet<_>>(),
        visible
    );
    session
        .undo(session.revision(), Actor::local(Channel::Ui))
        .unwrap();
    assert_eq!(session.document(), &source);
}

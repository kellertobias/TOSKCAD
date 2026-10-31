//! Viewer edge picking for the Sheet part / flanges form.
use super::*;
impl ModelToolController {
    /// Whether viewer clicks choose sheet edges: the Sheet part form is
    /// open on a rectangle base.
    pub fn sheet_edge_picking(&self) -> bool {
        matches!(&self.form, Some(ModelForm::Sheet(panel)) if panel.edge_picking())
    }
    /// Hover, click and draw sheet edges and flange guides. A click that
    /// adds a flange or flips its direction edits the same rows as the
    /// Inspector and invalidates the preview.
    pub fn sheet_viewport(
        &mut self,
        session: &Session,
        view: super::super::TerminationViewport<'_>,
    ) -> bool {
        let changed = match self.form.as_mut() {
            Some(ModelForm::Sheet(panel)) => panel.viewport(session.document(), view),
            _ => false,
        };
        if changed {
            self.invalidate();
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modeling::native_fixture;
    use multicad_core::{
        document::{Document, Entity, EntityData},
        materials::{LibraryDefinition, Stock, StockKind, prepare_library},
    };
    use multicad_model::Compiler;
    use std::time::{Duration, Instant};

    fn session() -> Session {
        let mut s = Session::new(Document::new("Front panel").unwrap()).unwrap();
        let steel = Entity::new(
            "DC01",
            EntityData::Material {
                density_kg_m3: None,
                color_rgba: [180, 184, 190, 255],
                section_hatch: None,
            },
        );
        let stock = LibraryDefinition::Stock(Stock {
            material: steel.id,
            specification: StockKind::Sheet {
                thickness_mm: 1.,
                length_mm: 2000.,
                width_mm: 1000.,
                inside_bend_radius_mm: 1.,
                k_factor: 0.4,
            },
        })
        .entity(EntityId::new(), "DC01 1.0")
        .unwrap();
        s.commit(prepare_library(&s, 0, Actor::local(Channel::Ui), &[steel, stock]).unwrap())
            .unwrap();
        s
    }
    fn click(
        controller: &mut ModelToolController,
        session: &Session,
        point: [f64; 3],
        clicked: bool,
    ) -> bool {
        let camera = crate::camera::Camera {
            yaw: 0.5,
            pitch: 0.6,
            scale: 2.5,
            target: [100., 50., 0.],
            perspective: false,
        };
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.));
        let p = camera.project(point.map(|v| v as f32), rect);
        let ctx = egui::Context::default();
        let mut changed = false;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            changed = controller.sheet_viewport(
                session,
                super::super::super::TerminationViewport {
                    scene: None,
                    report: None,
                    camera,
                    rect,
                    pointer: Some(egui::pos2(p[0], p[1])),
                    clicked,
                    painter: ui.painter(),
                    section: None,
                },
            );
        });
        output.textures_delta.clear();
        changed
    }
    fn settle(controller: &mut ModelToolController, session: &Session, r: &ProjectResources) {
        let until = Instant::now() + Duration::from_secs(10);
        while controller.pending() {
            controller.poll(session, r);
            assert!(Instant::now() < until, "native sheet preview timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// The 200 × 100 × 1 mm front panel: edge 1 is chosen in the viewer,
    /// previewed natively, applied, and reopened with its flange selected.
    #[test]
    fn viewer_chosen_flange_previews_applies_and_reopens() {
        let mut session = session();
        let resources = ProjectResources::default();
        let mut controller = ModelToolController::default();
        controller.set_backend(Box::new(native_fixture::NativeBackend::new()));
        controller
            .begin_sheet(&session, &resources, Channel::Ui)
            .unwrap();
        assert!(controller.sheet_edge_picking());
        assert!(!click(&mut controller, &session, [100., 0., 0.], false));
        assert!(click(&mut controller, &session, [100., 0., 0.], true));
        controller
            .preview(&session, &resources, Channel::Ui)
            .unwrap();
        settle(&mut controller, &session, &resources);
        assert_eq!(controller.error(), None);
        assert!(controller.can_apply(&session, &resources));
        let created = controller.apply(&mut session, &resources).unwrap();
        assert_eq!(created.len(), 1);
        let records = Compiler::default()
            .compile(session.document(), session.revision())
            .unwrap()
            .sheet_metal(session.document())
            .unwrap();
        assert_eq!(records[0].bends.len(), 1);
        assert_eq!(records[0].bends[0].edge, 0);
        assert_eq!(records[0].bends[0].angle_deg, 90.);
        assert_eq!(records[0].bends[0].inside_radius_mm, 1.);
        let feature = records[0].feature;

        controller.cancel();
        controller
            .begin_existing(&session, &resources, feature, Channel::Ui)
            .unwrap();
        assert!(controller.sheet_edge_picking());
        let Some(ModelForm::Sheet(panel)) = &controller.form else {
            panic!("the sheet form reopens");
        };
        let review = panel.review(session.document()).unwrap();
        assert_eq!(review.guides.len(), 1);
        assert_eq!(review.guides[0].edge, 0);
        assert_eq!(review.guides[0].leg_mm, 20.);
        // A neighboring flange chosen in the viewer is diagnosed, and the
        // native preview refuses the same corner.
        assert!(click(&mut controller, &session, [200., 50., 0.], true));
        controller
            .preview(&session, &resources, Channel::Ui)
            .unwrap();
        settle(&mut controller, &session, &resources);
        let error = controller.error().unwrap_or_default().to_owned();
        assert!(error.contains("Flanges 1 and 2"), "{error}");
        assert!(!controller.can_apply(&session, &resources));
    }
}

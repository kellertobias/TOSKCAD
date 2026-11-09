//! Viewer edge and face picking for sheet-metal authoring.
use super::super::sheet::CutoutPanel;
use super::*;
use multicad_core::model::CutoutFace;
impl ModelToolController {
    pub fn sheet_flange_availability(document: &Document, target: EntityId) -> Result<(), String> {
        super::super::sheet::SheetPanel::flange_source(document, target).map(|_| ())
    }

    pub fn begin_sheet_flange(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        target: EntityId,
        channel: Channel,
    ) -> Result<(), String> {
        let (feature, panel) =
            super::super::sheet::SheetPanel::for_flange(session.document(), target)?;
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Sheet(Box::new(panel)),
        )?;
        self.scope.as_mut().expect("opened scope").execution = Execution::Edit { feature };
        Ok(())
    }

    pub fn sheet_cutout_availability(document: &Document, target: EntityId) -> Result<(), String> {
        CutoutPanel::source(document, target).map(|_| ())
    }

    pub fn begin_sheet_cutout(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        target: EntityId,
        channel: Channel,
    ) -> Result<(), String> {
        let (feature, panel) = CutoutPanel::new(session.document(), target)?;
        if multicad_core::symbols::owned_entities_in_session(session)
            .map_err(|error| error.to_string())?
            .contains(&feature)
        {
            return Err("Edit the Library definition to add a cutout to this sheet".into());
        }
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::SheetCutout(Box::new(panel)),
        )?;
        self.scope.as_mut().expect("opened scope").execution = Execution::Edit { feature };
        Ok(())
    }

    pub fn sheet_cutout_target(&self) -> Option<(EntityId, EntityId)> {
        match &self.form {
            Some(ModelForm::SheetCutout(panel)) => panel.picking_target(),
            _ => None,
        }
    }

    pub fn pick_sheet_cutout(&mut self, face: CutoutFace, uv: [f64; 2]) -> bool {
        if let Some(ModelForm::SheetCutout(panel)) = &mut self.form
            && panel.pick(face, uv)
        {
            self.invalidate();
            true
        } else {
            false
        }
    }

    /// Whether viewer clicks choose sheet edges: the Sheet part form is
    /// open on a rectangle base.
    pub fn sheet_edge_picking(&self) -> bool {
        self.stock_editor.is_none()
            && matches!(&self.form, Some(ModelForm::Sheet(panel)) if panel.edge_picking())
    }
    /// Hover, click and draw sheet edges and flange guides. A click that
    /// adds a flange or flips its direction edits the same rows as the
    /// Inspector and invalidates the preview.
    pub fn sheet_viewport(
        &mut self,
        session: &Session,
        view: super::super::TerminationViewport<'_>,
    ) -> bool {
        if self.stock_editor.is_some() {
            return false;
        }
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
        model::{CutoutShape, HardwarePreset, HardwareSide, MountingPattern, Thread},
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

    #[test]
    fn add_flange_previews_an_in_place_edit_and_preserves_attached_hardware() {
        let mut session = session();
        let resources = ProjectResources::default();
        let mut controller = ModelToolController::default();
        controller.set_backend(Box::new(native_fixture::NativeBackend::new()));
        controller
            .begin_sheet(&session, &resources, Channel::Ui)
            .unwrap();
        assert_eq!(controller.label(), "Create sheet part");
        assert!(click(&mut controller, &session, [100., 100., 0.], true));
        controller
            .preview(&session, &resources, Channel::Ui)
            .unwrap();
        settle(&mut controller, &session, &resources);
        let sheet = controller.apply(&mut session, &resources).unwrap()[0].clone();

        let fastener = MountingPattern {
            preset: HardwarePreset::Screw {
                thread: Thread::M3,
                length: length(10.),
                washers: false,
                nut: false,
            },
            face: CutoutFace::Flange { index: 0 },
            side: HardwareSide::Material,
            points: vec![[length(100.), length(10.)]],
        };
        let prepared = model::prepare_sheet_fasteners(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            sheet.feature,
            &[fastener],
        )
        .unwrap();
        session.commit(prepared.model.prepared).unwrap();
        let original = model::read_sheet(session.document(), sheet.feature).unwrap();
        let hardware = model::sheet_hardware(session.document(), sheet.feature).unwrap();

        controller
            .begin_sheet_flange(&session, &resources, sheet.body, Channel::Ui)
            .unwrap();
        assert_eq!(controller.label(), "Add flange");
        assert!(controller.preview_waits_for_input());
        assert!(!controller.can_apply(&session, &resources));
        assert!(!click(&mut controller, &session, [100., 100., 0.], true));
        assert!(click(&mut controller, &session, [100., 0., 0.], true));
        assert!(!controller.preview_waits_for_input());

        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            controller.tick_auto_preview(ui, &session, &resources, true);
        });
        output.textures_delta.clear();
        settle(&mut controller, &session, &resources);
        assert_eq!(controller.error(), None);
        assert!(controller.can_apply(&session, &resources));
        assert_eq!(
            controller.apply(&mut session, &resources).unwrap(),
            vec![sheet.clone()]
        );
        let after = model::read_sheet(session.document(), sheet.feature).unwrap();
        assert_eq!(
            after.flanges.iter().map(|f| f.edge).collect::<Vec<_>>(),
            [2, 0]
        );
        let mut expected = original;
        expected.flanges.push(after.flanges[1].clone());
        assert_eq!(after, expected);
        assert_eq!(
            model::sheet_hardware(session.document(), sheet.feature).unwrap(),
            hardware
        );
    }
    #[test]
    fn folded_cutout_picks_apply_to_base_and_flange_without_changing_existing_operations() {
        let mut session = session();
        let resources = ProjectResources::default();
        let mut controller = ModelToolController::default();
        controller.set_backend(Box::new(native_fixture::NativeBackend::new()));
        controller
            .begin_sheet(&session, &resources, Channel::Ui)
            .unwrap();
        assert!(click(&mut controller, &session, [100., 100., 0.], true));
        controller
            .preview(&session, &resources, Channel::Ui)
            .unwrap();
        settle(&mut controller, &session, &resources);
        let sheet = controller.apply(&mut session, &resources).unwrap()[0].clone();
        let pattern = MountingPattern {
            preset: HardwarePreset::Screw {
                thread: Thread::M3,
                length: length(10.),
                washers: false,
                nut: false,
            },
            face: CutoutFace::Flange { index: 0 },
            side: HardwareSide::Material,
            points: vec![[length(100.), length(10.)]],
        };
        let prepared = model::prepare_sheet_fasteners(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            sheet.feature,
            &[pattern],
        )
        .unwrap();
        session.commit(prepared.model.prepared).unwrap();
        let original = model::read_sheet(session.document(), sheet.feature).unwrap();
        let hardware = model::sheet_hardware(session.document(), sheet.feature).unwrap();

        let base_center = [55.123_456_789, 45.987_654_321];
        let flange_center = [60., 10.];
        for (face, center) in [
            (CutoutFace::Base, base_center),
            (CutoutFace::Flange { index: 0 }, flange_center),
        ] {
            controller
                .begin_sheet_cutout(&session, &resources, sheet.body, Channel::Ui)
                .unwrap();
            assert_eq!(
                controller.sheet_cutout_target(),
                Some((sheet.body, sheet.body))
            );
            assert!(!controller.can_apply(&session, &resources));
            assert!(!controller.pick_sheet_cutout(CutoutFace::Flange { index: 9 }, center));
            assert!(controller.pick_sheet_cutout(face, center));
            assert_eq!(controller.sheet_cutout_target(), None);
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(Default::default(), |ui| {
                controller.tick_auto_preview(ui, &session, &resources, true);
            });
            output.textures_delta.clear();
            settle(&mut controller, &session, &resources);
            assert_eq!(controller.error(), None);
            assert_eq!(
                controller.apply(&mut session, &resources).unwrap(),
                vec![sheet.clone()]
            );
        }

        let updated = model::read_sheet(session.document(), sheet.feature).unwrap();
        assert_eq!(updated.cutouts.len(), 2);
        for (cutout, (face, center)) in updated.cutouts.iter().zip([
            (CutoutFace::Base, base_center),
            (CutoutFace::Flange { index: 0 }, flange_center),
        ]) {
            assert_eq!(cutout.face, face);
            assert_eq!(
                cutout.shape,
                CutoutShape::Circle {
                    center: center.map(length),
                    diameter: length(6.)
                }
            );
        }
        let mut unchanged = updated;
        unchanged.cutouts.clear();
        assert_eq!(unchanged, original);
        assert_eq!(
            model::sheet_hardware(session.document(), sheet.feature).unwrap(),
            hardware
        );
        let native = Compiler::default()
            .compile(session.document(), session.revision())
            .unwrap();
        let developed = native.unfold(session.document(), sheet.body).unwrap();
        let drill = |id: &str| {
            developed
                .drills
                .iter()
                .find(|drill| drill.id == id)
                .unwrap()
                .center
        };
        let base = drill("cutout.0");
        assert!((base[0] - base_center[0]).abs() < 1e-6);
        assert!((base[1] - base_center[1]).abs() < 1e-6);
        let flange = drill("cutout.1");
        let allowance = developed.bends[0].record.allowance_mm;
        assert!((flange[0] - (200. - flange_center[0])).abs() < 1e-6);
        assert!((flange[1] - (100. + allowance + flange_center[1])).abs() < 1e-6);

        for _ in 0..2 {
            session
                .undo(session.revision(), Actor::local(Channel::Ui))
                .unwrap();
        }
        assert_eq!(
            model::read_sheet(session.document(), sheet.feature).unwrap(),
            original
        );
    }
}

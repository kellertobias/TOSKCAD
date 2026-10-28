use super::*;
use multicad_core::{model::BodyInput, references::GeometryIntent};
impl ModelToolController {
    pub fn begin_edge_treatment(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        body: EntityId,
        edges: Vec<GeometryIntent>,
        channel: Channel,
    ) -> Result<(), String> {
        let panel = super::super::edge_treatment::EdgeTreatmentPanel::new(
            session.document(),
            session.revision(),
            scene,
            body,
            edges,
        )?;
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::EdgeTreatment(Box::new(panel)),
        )?;
        let result = self.verify_context(session, resources, scene);
        self.context_result(scene.stamp(), &result);
        if let Err(error) = result {
            self.cancel();
            return Err(error);
        }
        Ok(())
    }
    pub fn edge_treatment_active(&self) -> bool {
        matches!(self.form, Some(ModelForm::EdgeTreatment(_)))
    }
    pub fn edge_treatment_input(&self) -> Option<&BodyInput> {
        match &self.form {
            Some(ModelForm::EdgeTreatment(panel)) => Some(panel.input()),
            _ => None,
        }
    }
    pub fn edge_treatment_edges(&self) -> Option<&[GeometryIntent]> {
        match &self.form {
            Some(ModelForm::EdgeTreatment(panel)) => Some(panel.edges()),
            _ => None,
        }
    }
    pub fn edge_treatment_picking(&self) -> bool {
        matches!(&self.form,Some(ModelForm::EdgeTreatment(panel))if panel.picking())
    }
    pub fn edge_treatment_pick_kind(&self) -> Option<multicad_core::references::TopologyKind> {
        match &self.form {
            Some(ModelForm::EdgeTreatment(p)) => p.pick_kind(),
            _ => None,
        }
    }
    pub fn edge_treatment_pick_captures(&self) -> Option<&[GeometryIntent]> {
        match &self.form {
            Some(ModelForm::EdgeTreatment(p)) => Some(p.pick_captures()),
            _ => None,
        }
    }
    pub fn set_edge_treatment_pick_captures(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        captures: Vec<GeometryIntent>,
    ) -> Result<(), String> {
        self.verify_context(session, resources, scene)?;
        if !scene.is_full_model() {
            return Err("Chamfer captures require the complete current canonical model".into());
        }
        let Some(ModelForm::EdgeTreatment(panel)) = &mut self.form else {
            return Err("Open Fillet / Chamfer before selecting native geometry".into());
        };
        if panel.set_pick_captures(scene, &captures)? {
            self.invalidate();
        }
        Ok(())
    }
    pub fn set_edge_treatment_edges(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        edges: Vec<GeometryIntent>,
    ) -> Result<(), String> {
        self.verify_context(session, resources, scene)?;
        if !scene.is_full_model() {
            return Err("Edge captures require the complete current canonical model".into());
        }
        let Some(ModelForm::EdgeTreatment(panel)) = &mut self.form else {
            return Err("Open Fillet / Chamfer before selecting edges".into());
        };
        if panel.set_edges(scene, &edges)? {
            self.invalidate();
        }
        Ok(())
    }
    pub(super) fn stop_edge_treatment_picking(&mut self) {
        if let Some(ModelForm::EdgeTreatment(panel)) = &mut self.form {
            panel.stop_picking();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{inspector_actions, modeling::native_fixture, registry::Mode};
    use multicad_core::{document::Document, model, references::TopologyKind};
    use multicad_model::Compiler;
    use std::time::{Duration, Instant};

    #[test]
    fn failed_native_fillet_receipt_is_visible_repairable_and_cannot_commit() {
        let actor = Actor::local(Channel::Ui);
        let mut seed = Session::new(Document::new("Tiny fillet stock").unwrap()).unwrap();
        let stock = model::PrimitiveRequest::new(
            "Tiny stock",
            model::PrimitiveParameters::Box {
                size: std::array::from_fn(|_| model::length(1.)),
                frame: Default::default(),
            },
        );
        seed.commit(
            model::prepare_primitive(&seed, seed.revision(), actor, &stock)
                .unwrap()
                .prepared,
        )
        .unwrap();
        // A reopened source has no existing Undo entries to hide a failed commit.
        let mut session = Session::new(seed.document().clone()).unwrap();
        session.mark_saved(0).unwrap();
        let original = session.document().clone();
        let native = Compiler::default().compile(&original, 0).unwrap();
        let scene = native_fixture::scene(&original, native.snapshot(), 0).unwrap();
        let topology = scene.geometry_snapshot(stock.feature).unwrap();
        let edge = topology
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Edge)
            .unwrap();
        let capture = GeometryIntent::capture(&topology, edge.key).unwrap();
        let resources = ProjectResources::default();
        let mut controller = ModelToolController::default();
        controller.set_backend(Box::new(native_fixture::NativeBackend::new()));
        controller
            .begin_edge_treatment(
                &session,
                &resources,
                &scene,
                stock.body,
                vec![capture.clone()],
                Channel::Ui,
            )
            .unwrap();
        // The default 2 mm radius is structurally valid but impossible on 1 mm stock.
        controller
            .preview(&session, &resources, Channel::Ui)
            .unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while controller.pending() {
            controller.poll(&session, &resources);
            assert!(Instant::now() < until, "native fillet receipt timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
        let native_error = controller
            .error()
            .expect("native geometry must reject this radius")
            .to_owned();
        assert!(
            native_error
                .to_lowercase()
                .contains("edge treatment failed"),
            "{native_error}"
        );
        assert!(!controller.can_apply(&session, &resources));
        assert!(controller.edge_treatment_active());
        assert_eq!(
            controller.edge_treatment_edges(),
            Some(std::slice::from_ref(&capture))
        );
        assert!(!controller.edge_treatment_picking());

        // Exercise the production Inspector, including its visible native error,
        // enabled repair controls, and disabled pinned Apply action.
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        for _ in 0..2 {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(360., 650.),
                    )),
                    ..Default::default()
                },
                |ui| {
                    inspector_actions::scope(
                        ui,
                        Some(inspector_actions::Owner::Model),
                        Mode::Model.color(),
                        |ui| {
                            assert!(
                                controller
                                    .show(ui, &mut session, &resources)
                                    .unwrap()
                                    .is_none()
                            );
                        },
                    );
                },
            );
            let nodes = &output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes;
            let find = |label: &str| {
                nodes
                    .iter()
                    .find(|(_, n)| n.label() == Some(label))
                    .map(|(_, n)| n)
                    .unwrap_or_else(|| panic!("Missing Inspector control: {label}"))
            };
            assert!(!find("Fillet").is_disabled());
            assert!(!find("Chamfer").is_disabled());
            assert!(!find("Select / reselect edges").is_disabled());
            assert!(find("Apply").is_disabled());
            let error_visible = output.shapes.iter().any(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().contains(&native_error) => shape
                    .clip_rect
                    .intersects(egui::Rect::from_min_size(text.pos, text.galley.size())),
                _ => false,
            });
            output.drop_without_applying_deltas();
            assert!(
                error_visible,
                "Native failure is not painted in the Inspector clip"
            );
        }
        assert!(controller.apply(&mut session, &resources).is_err());
        assert_eq!(session.document(), &original);
        assert_eq!(session.revision(), 0);
        assert!(!session.dirty());
        assert!(!session.can_undo());
        assert!(!session.can_redo());
        assert_eq!(session.activity().count(), 0);
        controller.cancel();
        assert!(!controller.active());
        assert_eq!(session.document(), &original);
    }
}

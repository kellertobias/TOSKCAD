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

//! Saved source controls remain editable when the canonical model cannot build.
//! Only verified native context enables new target choices and viewer picking.
use super::*;
impl ModelToolController {
    pub(super) fn verify_context(
        &self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
    ) -> Result<(), String> {
        self.current(session, resources, true)?;
        let source = self.scope.as_ref().expect("checked scope").source;
        if scene.stamp().document != session.document().id()
            || scene.stamp().revision != session.revision()
            || scene
                .model_evaluation()
                .is_none_or(|info| info.source_hash != source)
        {
            return Err("The native context does not match the current source".into());
        }
        Ok(())
    }
    pub(super) fn context_result(&mut self, stamp: SceneStamp, result: &Result<(), String>) {
        self.context_attempt = Some(stamp);
        self.context_valid = result.is_ok();
        self.context_notice=result.as_ref().err().map(|error|format!("Native context unavailable: {error}. Saved controls remain editable; new native targets and viewer picking are unavailable."));
        if let Some(panel) = self.form.as_mut().and_then(ModelForm::extrusion_mut) {
            panel.set_picking_available(self.context_valid);
        }
    }
    pub fn source_only_context(&mut self, reason: &str) {
        self.context_valid = false;
        self.context_notice = Some(format!(
            "{reason}. Edit the saved source controls and Preview the complete model before Apply. New native target choices and viewer picking are unavailable."
        ));
        if let Some(panel) = self.form.as_mut().and_then(ModelForm::extrusion_mut) {
            panel.set_picking_available(false);
        }
    }
    pub fn refresh_source_context(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
    ) {
        if !self.active() || self.context_valid || self.context_attempt == Some(scene.stamp()) {
            return;
        }
        // Adding unselected choices does not change the form's semantic inputs
        // or invalidate a separately checked candidate of the same source.
        if self.operations.is_some() {
            let _ = self.set_extrusion_context(session, resources, scene);
        } else {
            let _ = self.set_feature_context(session, resources, scene);
        }
    }
}

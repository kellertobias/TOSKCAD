//! Picker delegation uses the original source scope, never the candidate model.
use super::*;
impl ModelToolController {
    pub fn termination_picking(&self) -> bool {
        self.form
            .as_ref()
            .and_then(ModelForm::extrusion)
            .is_some_and(ExtrudePanel::termination_picking)
    }
    pub fn termination_highlight(&self) -> Option<crate::viewport::Selection> {
        if self.error.is_some() {
            return None;
        }
        self.form
            .as_ref()
            .and_then(ModelForm::extrusion)
            .and_then(ExtrudePanel::termination_highlight)
    }
    pub fn termination_viewport(
        &mut self,
        session: &Session,
        view: super::super::TerminationViewport<'_>,
    ) -> bool {
        let Some(scope) = &self.scope else {
            return false;
        };
        if session.validate_stamp(&scope.session).is_err() {
            return false;
        }
        let changed = match self.form.as_mut().and_then(ModelForm::extrusion_mut) {
            Some(panel) => panel.termination_viewport(
                session.document(),
                session.revision(),
                scope.source,
                view,
            ),
            None => false,
        };
        if changed {
            self.invalidate();
        }
        changed
    }
}

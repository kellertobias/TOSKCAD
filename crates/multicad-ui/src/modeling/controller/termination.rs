//! Picker delegation uses the original source scope, never the candidate model.
use super::*;
impl ModelToolController {
    pub fn termination_picking(&self) -> bool {
        matches!(&self.form,Some(ModelForm::Extrude(panel)) if panel.termination_picking())
    }
    pub fn termination_highlight(&self) -> Option<crate::viewport::Selection> {
        if self.error.is_some() {
            return None;
        }
        match &self.form {
            Some(ModelForm::Extrude(panel)) => panel.termination_highlight(),
            _ => None,
        }
    }
    pub fn termination_viewport(
        &mut self,
        session: &Session,
        view: super::super::TerminationViewport<'_>,
    ) -> bool {
        let Some(scope) = &self.scope else {
            return false;
        };
        if session.validate_preview(&scope.session).is_err() {
            return false;
        }
        let changed = match &mut self.form {
            Some(ModelForm::Extrude(panel)) => panel.termination_viewport(
                session.document(),
                session.revision(),
                scope.source,
                view,
            ),
            _ => false,
        };
        if changed {
            self.invalidate();
        }
        changed
    }
}

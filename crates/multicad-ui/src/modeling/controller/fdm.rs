//! Viewer placement for the FDM aid form: whether it is open or picking,
//! and the per-frame overlay and pick hand-off.
use super::{ModelForm, ModelToolController};
use multicad_core::commands::Session;

impl ModelToolController {
    pub fn fdm_aid_active(&self) -> bool {
        matches!(&self.form, Some(ModelForm::FdmAid(_)))
    }
    pub fn fdm_aid_picking(&self) -> bool {
        matches!(&self.form, Some(ModelForm::FdmAid(panel)) if panel.picking())
    }
    /// Paint the aid's contact marker, direction handle and review overlay,
    /// and apply a viewer pick. A pick only changes the form; it drops the
    /// stale preview. Returns the status message of a pick.
    pub fn fdm_aid_viewport(
        &mut self,
        session: &Session,
        view: super::super::TerminationViewport<'_>,
    ) -> Option<String> {
        let message = match self.form.as_mut() {
            Some(ModelForm::FdmAid(panel)) => {
                panel.viewport(session.document(), session.revision(), view)
            }
            _ => None,
        };
        if message.is_some() {
            self.invalidate();
        }
        message
    }
}

#[cfg(test)]
mod tests;

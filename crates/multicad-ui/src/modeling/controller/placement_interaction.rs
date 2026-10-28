use super::*;
use std::collections::BTreeSet;

impl ModelToolController {
    pub fn placement_active(&self) -> bool {
        matches!(self.form, Some(ModelForm::Placement(_)))
    }

    /// Explicit canonical targets, without expanding Part definitions to their
    /// dependent occurrences. The target-acquisition host owns selection UX.
    pub fn placement_targets(&self) -> Option<&BTreeSet<EntityId>> {
        match &self.form {
            Some(ModelForm::Placement(panel)) => Some(panel.selected()),
            _ => None,
        }
    }

    pub fn set_placement_copy_modifier(&mut self, active: bool) -> bool {
        let changed = match &mut self.form {
            Some(ModelForm::Placement(panel)) => panel.set_copy_modifier(active),
            _ => false,
        };
        if changed {
            self.invalidate();
        }
        changed
    }

    /// Retarget a new placement batch while retaining its numeric controls and
    /// copy choice. An empty set keeps those controls but cannot produce Apply.
    pub fn retarget_placement(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        selected: &BTreeSet<EntityId>,
    ) -> Result<(), String> {
        self.verify_context(session, resources, scene)?;
        if !scene.is_full_model() {
            return Err("Placement targets require the complete current native model".into());
        }
        let Some(ModelForm::Placement(panel)) = self.form.as_mut() else {
            return Err("Start Move / Copy before choosing its targets".into());
        };
        if panel.selected() == selected {
            return Ok(());
        }
        panel.retarget(session.document(), session.revision(), scene, selected)?;
        self.invalidate();
        Ok(())
    }
}

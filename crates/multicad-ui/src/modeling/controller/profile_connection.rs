use super::*;
use crate::modeling::ProfileConnectorPick;
use multicad_core::model::ProfileConnectorAnchor;
use std::collections::BTreeSet;

impl ModelToolController {
    pub fn begin_profile_connection(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        selected: &BTreeSet<EntityId>,
        channel: Channel,
    ) -> Result<(), String> {
        let panel = super::super::profile_connection::ProfileConnectionPanel::new(
            session.document(),
            selected,
        )?;
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::ProfileConnection(Box::new(panel)),
        )
    }

    pub(crate) fn set_profile_connection_snap_scene(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: Option<&Arc<Scene>>,
    ) {
        if !matches!(self.form, Some(ModelForm::ProfileConnection(_))) {
            return;
        }
        let scene = scene
            .filter(|scene| self.verify_context(session, resources, scene).is_ok())
            .cloned();
        if let Some(ModelForm::ProfileConnection(panel)) = &mut self.form {
            panel.set_snap_scene(scene);
        }
    }

    pub(crate) fn profile_connection_picker(&self) -> Option<ProfileConnectorPick> {
        match self.form.as_ref()? {
            ModelForm::ProfileConnection(panel) => panel.picker().cloned(),
            _ => None,
        }
    }

    pub(crate) fn set_profile_connection_capture(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        pick: &ProfileConnectorPick,
        anchor: ProfileConnectorAnchor,
        length_mm: f64,
    ) -> Result<(), String> {
        self.verify_context(session, resources, scene)?;
        let Some(ModelForm::ProfileConnection(panel)) = &mut self.form else {
            return Err("Open the profile connection before picking an attachment".into());
        };
        panel.capture(pick, anchor, length_mm)?;
        self.invalidate();
        Ok(())
    }
}

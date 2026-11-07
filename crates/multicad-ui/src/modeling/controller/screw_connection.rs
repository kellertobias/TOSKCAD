use super::*;
use crate::modeling::{HardwareLibraryRequest, ScrewConnectionPick};
use multicad_core::symbols::SymbolDefinition;

impl ModelToolController {
    pub fn begin_screw_connection(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let mut panel =
            super::super::screw_connection::ScrewConnectionPanel::new(session.document())?;
        panel.set_entries(self.connection_entries.clone());
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::ScrewConnection(Box::new(panel)),
        )
    }

    pub fn screw_connection_for(document: &Document, id: EntityId) -> Option<EntityId> {
        let mut matches = document.entities().filter_map(|entity| {
            if !matches!(&entity.data, EntityData::Extension { definition, .. } if definition.type_id == model::SCREW_CONNECTION_TYPE) { return None; }
            let source = model::read_screw_connection(document, entity.id).ok()?;
            let owned = entity.id == id || source.axis == id || source.receivers.iter().any(|feature| *feature == id || model::read_connection_receiver(document,*feature).is_ok_and(|receiver| receiver.output.body == id))
                || source.hardware.iter().any(|role| {
                    role.instance == id || role.driver == id || match role.source {
                        model::ScrewConnectionHardwareBinding::Generated { feature, body } => feature == id || body == id,
                        model::ScrewConnectionHardwareBinding::Library { record } => record == id || multicad_core::symbols::read_instance(document,record).is_ok_and(|instance| instance.wrapper == id || instance.member == id || instance.ids.values().any(|mapped|*mapped == id)),
                    }
                });
            owned.then_some(entity.id)
        });
        let result = matches.next()?;
        matches.next().is_none().then_some(result)
    }

    pub(crate) fn screw_connection_picker(&self) -> Option<ScrewConnectionPick> {
        match self.form.as_ref()? {
            ModelForm::ScrewConnection(panel) => panel.picker().cloned(),
            _ => None,
        }
    }
    pub(crate) fn set_connection_capture(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        face: model::FaceInput,
    ) -> Result<(), String> {
        self.verify_context(session, resources, scene)?;
        let Some(ModelForm::ScrewConnection(panel)) = &mut self.form else {
            return Err("Open the connection editor before choosing a receiving face".into());
        };
        panel.capture(face)?;
        self.invalidate();
        Ok(())
    }
    pub(crate) fn set_connection_library_entries(
        &mut self,
        entries: Arc<Vec<crate::library::LibraryEntry>>,
    ) {
        self.connection_entries = entries.clone();
        if let Some(ModelForm::ScrewConnection(panel)) = &mut self.form {
            panel.set_entries(entries);
        }
    }
    pub(crate) fn connection_library_request(&self) -> Option<HardwareLibraryRequest> {
        match self.form.as_ref()? {
            ModelForm::ScrewConnection(panel) => panel.library_request().cloned(),
            _ => None,
        }
    }
    pub(crate) fn accept_connection_library(
        &mut self,
        request: &HardwareLibraryRequest,
        definitions: Result<Vec<SymbolDefinition>, String>,
    ) -> Result<(), String> {
        let Some(ModelForm::ScrewConnection(panel)) = &mut self.form else {
            return Err("The connection hardware chooser is no longer open".into());
        };
        panel.accept_library(request, definitions)
    }
    pub(crate) fn connection_needs_library(&self) -> bool {
        matches!(self.form, Some(ModelForm::ScrewConnection(_)))
            && self.connection_entries.is_empty()
    }
    pub(crate) fn connection_view_options(&self) -> Option<(bool, bool)> {
        match self.form.as_ref()? {
            ModelForm::ScrewConnection(panel) => Some((panel.section, panel.explode)),
            _ => None,
        }
    }
    pub(crate) fn connection_axis(&self) -> Option<model::ScrewConnectionAxisRequest> {
        match self.form.as_ref()? {
            ModelForm::ScrewConnection(panel) => panel.axis.clone(),
            _ => None,
        }
    }
    pub(crate) fn connection_hardware_roles(
        &self,
    ) -> Vec<(EntityId, model::ScrewConnectionHardwareRole)> {
        match self.form.as_ref() {
            Some(ModelForm::ScrewConnection(panel)) => panel
                .hardware
                .iter()
                .map(|role| (role.instance, role.role))
                .collect(),
            _ => vec![],
        }
    }
}

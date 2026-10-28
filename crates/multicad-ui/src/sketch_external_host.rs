//! Narrow host routing keeps external-reference algorithms out of the editor.
use super::*;
use crate::{
    external_references::{ExternalTool, Proposal},
    viewport::SceneFeed,
};
impl SketchEditor {
    pub fn set_external_backend(&mut self, backend: Box<dyn crate::viewport::SceneBackend>) {
        self.external.set_backend(backend);
    }
    pub fn has_external_backend(&self) -> bool {
        self.external.has_backend()
    }
    pub fn set_external_query_backend(
        &mut self,
        backend: Box<dyn crate::external_references::ExternalQueryBackend>,
    ) {
        self.external.set_query_backend(backend);
    }
    pub fn has_external_query_backend(&self) -> bool {
        self.external.has_query_backend()
    }
    pub(super) fn poll_external(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
    ) -> bool {
        self.external.poll_native(session, resources);
        if let Some(result) = self.external.poll_query(session, resources) {
            let result = result.and_then(|proposal| {
                let sketch = self.selected.ok_or("Open the owning sketch")?;
                self.adopt_external(session, sketch, proposal, resources)
            });
            if let Err(error) = result {
                self.external.report_error(error.clone());
                self.message = error;
            }
        }
        if self.constraints.external_pending()
            && let Some(result) = self.external.candidate_validation().cloned()
        {
            self.constraints.complete_external_native(result);
        }
        self.external.native_pending() || self.external.query_pending()
    }

    pub(super) fn refresh_external(&mut self, session: &Session, feed: &SceneFeed) {
        // A canonical evaluation attempt may retain unresolved references. It still
        // contains source-current geometry; model-tool candidate scenes never enter.
        let source = feed
            .references()
            .filter(|evaluation| {
                evaluation.stamp.document == session.document().id()
                    && evaluation.stamp.revision == session.revision()
            })
            .map(|evaluation| evaluation.candidate.clone())
            .or_else(|| self.display_scene.clone());
        let source = source.and_then(|scene| {
            self.visibility
                .apply(&scene, session.document(), session.revision())
                .ok()
        });
        let pose = self
            .selected
            .and_then(|id| self.contexts.get(&id))
            .copied()
            .unwrap_or_default();
        self.external.refresh(
            session,
            self.selected.filter(|_| self.editing),
            source,
            pose,
        );
    }
    pub fn external_ready(&self, session: &Session) -> bool {
        self.editing
            && self.selected.is_some()
            && !self.picking
            && !self.moving
            && !self.has_pending_edits()
            && self.external.ready_for(session)
    }
    pub fn begin_external(&mut self, session: &Session, tool: ExternalTool) -> Result<(), String> {
        if !self.external_ready(session) {
            return Err("Open a sketch with current native definition geometry and finish pending edits before selecting external references".into());
        }
        self.external.begin(session, tool)?;
        self.tool = None;
        self.operations.cancel();
        self.cancel_move();
        self.moving = false;
        self.clear_drawing_progress();
        self.cancel_profile_clicks();
        self.profiles.clear();
        self.constraints.clear_geometry_selection();
        self.element = None;
        self.message = format!(
            "{} · select a source edge or vertex in the viewport",
            tool.label()
        );
        Ok(())
    }
    pub(super) fn external_inspector(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        sketch: EntityId,
        resources: &multicad_core::project_file::ProjectResources,
    ) {
        let enabled = self.preview.is_none()
            && !self.constraints.dirty()
            && !self.repair.has_pending()
            && !self.tree.tool_active()
            && !self.model.active()
            && !self.materials.pending()
            && !self.picking
            && !self.moving;
        let proposal = self.external.inspector(
            ui,
            session,
            sketch,
            self.evaluated.as_ref(),
            enabled,
            resources,
        );
        if let Some(proposal) = proposal
            && let Err(error) = self.adopt_external(session, sketch, proposal, resources)
        {
            self.message = error;
        }
    }
    fn adopt_external(
        &mut self,
        session: &Session,
        sketch: EntityId,
        proposal: Proposal,
        resources: &multicad_core::project_file::ProjectResources,
    ) -> Result<(), String> {
        self.constraints.accept_external_edit_with_report(
            session,
            sketch,
            proposal.reference,
            proposal.edit,
            proposal.label,
            Some(proposal.report),
        )?;
        self.external.cancel();
        self.constraints.require_external_native();
        let result = self.external.validate_candidate(
            session,
            self.constraints
                .external_prepared()
                .ok_or("Reference preview is missing")?,
            resources,
        );
        if let Err(error) = result {
            self.constraints.complete_external_native(Err(error));
        }
        self.tool = None;
        self.operations.cancel();
        self.clear_drawing_progress();
        self.cancel_profile_clicks();
        self.profiles.clear();
        self.element = None;
        self.message = "Checked external reference preview · Apply or discard in Inspector".into();
        Ok(())
    }
    pub fn external_active(&self) -> bool {
        self.external.active()
    }
    pub fn external_pending(&self) -> bool {
        self.constraints.external_pending()
    }
    pub fn external_can_preview(&self, session: &Session) -> bool {
        self.external.can_preview(session)
    }
    pub fn external_can_apply(&self, session: &Session) -> bool {
        self.selected
            .is_some_and(|id| self.constraints.external_can_apply(session, id))
    }
    pub fn preview_external(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let id = self.selected.ok_or("Open the owning sketch")?;
        if let Some(proposal) = self.external.request_preview(session, resources, channel)? {
            self.adopt_external(session, id, proposal, resources)
        } else {
            self.message = "Checking exact reference geometry and its consumer…".into();
            Ok(())
        }
    }
    pub fn apply_external(&mut self, session: &mut Session) -> Result<(), String> {
        if !self.external_pending() {
            return Err("Preview an external reference first".into());
        }
        self.constraints.apply(session)?;
        self.external.cancel();
        self.message = "External reference applied · one undoable source edit".into();
        Ok(())
    }
    pub fn discard_external(&mut self) {
        if self.external_pending() {
            self.constraints.discard();
        }
        self.external.cancel();
        self.message = "External reference cancelled · document unchanged".into();
    }
}

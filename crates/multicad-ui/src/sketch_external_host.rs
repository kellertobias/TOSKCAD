//! Narrow host routing keeps external-reference algorithms out of the editor.
use super::*;
use crate::{
    external_references::{ExternalTool, Proposal},
    viewport::SceneFeed,
};
impl SketchEditor {
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
    ) {
        let enabled = self.preview.is_none()
            && !self.constraints.dirty()
            && !self.repair.has_pending()
            && !self.tree.tool_active()
            && !self.model.active()
            && !self.materials.pending()
            && !self.picking
            && !self.moving;
        let proposal =
            self.external
                .inspector(ui, session, sketch, self.evaluated.as_ref(), enabled);
        if let Some(proposal) = proposal
            && let Err(error) = self.adopt_external(session, sketch, proposal)
        {
            self.message = error;
        }
    }
    fn adopt_external(
        &mut self,
        session: &Session,
        sketch: EntityId,
        proposal: Proposal,
    ) -> Result<(), String> {
        self.constraints.accept_external_edit(
            session,
            sketch,
            proposal.reference,
            proposal.edit,
            proposal.label,
        )?;
        self.external.cancel();
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
    pub fn preview_external(&mut self, session: &Session, channel: Channel) -> Result<(), String> {
        let id = self.selected.ok_or("Open the owning sketch")?;
        let proposal = self.external.prepare(session, channel)?;
        self.adopt_external(session, id, proposal)
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

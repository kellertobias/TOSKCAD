//! Hosts the STEP and KiCad import Inspector in the 3D workspace.
use super::*;
use crate::mechanical_import::{
    MechanicalImport, MechanicalImportBackendRef, Outcome, SourceKind, Target,
};

impl SketchEditor {
    pub fn mechanical_import_active(&self) -> bool {
        self.mechanical.active()
    }
    pub fn mechanical_import(&mut self) -> &mut MechanicalImport {
        &mut self.mechanical
    }
    /// The import to update: the one owning the selected Parts, or the only
    /// import in the document.
    pub fn mechanical_update_target(
        &self,
        document: &Document,
    ) -> Result<(SourceKind, EntityId), String> {
        let selection = self
            .tree_selection
            .iter()
            .copied()
            .chain(self.solid.selected_ids());
        crate::mechanical_import::update_target(document, selection)
    }
    /// Starts reading a picked STEP or KiCad file; nothing changes until Apply.
    pub fn begin_mechanical_import(
        &mut self,
        backend: MechanicalImportBackendRef,
        kind: SourceKind,
        path: std::path::PathBuf,
        target: Target,
    ) -> Result<(), String> {
        if self.has_pending_edits() {
            return Err("Apply or discard pending edits before importing".into());
        }
        self.tool = None;
        self.clear_drawing_progress();
        self.mechanical.begin(backend, kind, path, target)?;
        self.message = format!(
            "{} {} · review the preview in the Inspector, then Apply",
            if matches!(target, Target::Update(_)) {
                "Update"
            } else {
                "Import"
            },
            kind.label()
        );
        Ok(())
    }
    pub(super) fn mechanical_import_inspector(&mut self, ui: &mut egui::Ui, session: &mut Session) {
        match self.mechanical.show(ui, session) {
            Some(Outcome::Applied { label, summary }) => {
                self.message = format!("{label} · {summary} · Undo restores the previous document");
                self.fit_requested = true;
            }
            Some(Outcome::Cancelled) => {
                self.message = "Import cancelled · document unchanged".into();
            }
            None => {}
        }
    }
}

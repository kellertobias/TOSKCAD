//! Native, source-neutral catalog and selected-part metadata editor.
mod library;
mod part;
use eframe::egui;
pub use library::LibraryEditor;
use multicad_core::{commands::Session, document::EntityId};
pub use part::PartEditor;
#[derive(Default)]
pub struct MaterialsEditor {
    pub library: LibraryEditor,
    pub parts: PartEditor,
}
impl MaterialsEditor {
    pub fn cancel(&mut self) {
        *self = Self::default();
    }
    pub fn pending(&self) -> bool {
        self.library.dirty() || self.parts.dirty()
    }
    pub fn open(&mut self, session: &Session) {
        self.library.open(session);
    }
    pub fn show(&mut self, ctx: &egui::Context, session: &mut Session) {
        self.library.show(ctx, session);
    }
    pub fn inspector(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        selected: &[EntityId],
        enabled: bool,
    ) {
        self.parts
            .show(ui, session, selected, enabled && !self.library.dirty());
    }
}

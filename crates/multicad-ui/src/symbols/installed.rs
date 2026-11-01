//! Browse the same original-location Library used by Parameters.
use super::SymbolAction;
use crate::{
    egui,
    library::{LibraryEntry, PreviewTextures},
};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct Installed {
    pub entries: Arc<Vec<LibraryEntry>>,
    previews: PreviewTextures,
}
impl Installed {
    pub fn show(&mut self, ui: &mut egui::Ui, filter: &str) -> Option<SymbolAction> {
        ui.strong("Installation Library");
        ui.small("Original files listed with Add to Library. Load a file, then explicitly import and place its pinned definitions.");
        let mut action = None;
        let mut shown = 0;
        for entry in self.entries.iter().filter(|entry| matches(entry, filter)) {
            shown += 1;
            ui.push_id(entry.id, |ui| ui.group(|ui| {
                ui.strong(&entry.name);
                self.previews.show(ui, entry.preview.as_ref(), egui::vec2(140., 80.), &entry.name);
                if !entry.description.is_empty() { ui.small(&entry.description); }
                ui.small(entry.path.display().to_string());
                if let Some(problem) = &entry.problem {
                    ui.colored_label(egui::Color32::LIGHT_RED, problem);
                } else if entry.symbol_definitions == 0 {
                    ui.small("No reusable definitions. Open this file and use Create symbol before placing it here.");
                } else if ui.button("Load definitions").clicked() {
                    action = Some(SymbolAction::OpenInstalled(entry.path.clone()));
                }
            }));
        }
        if shown == 0 {
            ui.small("No matching listed files. Save a file with reusable symbols and choose Add to Library in Parameters.");
        }
        action
    }
}
fn matches(entry: &LibraryEntry, filter: &str) -> bool {
    let searchable = format!(
        "{} {} {}",
        entry.name,
        entry.description,
        entry.path.display()
    )
    .to_lowercase();
    filter
        .split_whitespace()
        .all(|term| searchable.contains(&term.to_lowercase()))
}

//! Detached hardware selection for one connection role. Selecting a card never
//! imports a definition or starts the independent Library placement tool.
use crate::{
    controls, egui,
    library::{
        LibraryEntry,
        facets::{
            CatalogFilters, LoadedHardware, SearchIndex, SearchQuery, definition_search_in_file,
            entry_search,
        },
    },
};
use multicad_core::{document::EntityId, symbols::SymbolDefinition};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HardwareLibraryRequest {
    pub generation: u64,
    pub intent: EntityId,
    pub role: EntityId,
    pub path: PathBuf,
}

#[derive(Default)]
pub(super) struct HardwareLibrary {
    entries: Arc<Vec<LibraryEntry>>,
    search: Vec<SearchIndex>,
    facets: CatalogFilters,
    role: Option<EntityId>,
    query: String,
    generation: u64,
    request: Option<HardwareLibraryRequest>,
    loaded: Vec<SymbolDefinition>,
    loaded_search: Vec<SearchIndex>,
    error: Option<String>,
}

impl HardwareLibrary {
    pub fn set_entries(&mut self, entries: Arc<Vec<LibraryEntry>>) {
        if !Arc::ptr_eq(&self.entries, &entries) {
            self.search = entries.iter().map(entry_search).collect();
            self.entries = entries;
        }
    }

    pub fn begin(&mut self, role: EntityId) {
        self.generation = self.generation.wrapping_add(1);
        self.role = Some(role);
        self.request = None;
        self.loaded.clear();
        self.loaded_search.clear();
        self.error = None;
    }

    pub fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.role = None;
        self.request = None;
        self.loaded.clear();
        self.loaded_search.clear();
        self.error = None;
    }

    pub fn active(&self) -> bool {
        self.role.is_some()
    }

    pub fn pending_request(&self) -> Option<&HardwareLibraryRequest> {
        self.request.as_ref()
    }

    pub fn accept(
        &mut self,
        request: &HardwareLibraryRequest,
        result: Result<LoadedHardware, String>,
    ) -> Result<(), String> {
        if self.request.as_ref() != Some(request) || self.role != Some(request.role) {
            return Err("This hardware choice belongs to an earlier connection draft".into());
        }
        self.request = None;
        match result {
            Ok(LoadedHardware {
                definitions,
                file_text,
            }) => {
                if definitions.is_empty() {
                    self.error = Some("This file contains no reusable hardware definitions".into());
                } else {
                    for definition in &definitions {
                        definition.validate().map_err(|error| error.to_string())?;
                    }
                    self.loaded_search = definitions
                        .iter()
                        .map(|definition| definition_search_in_file(definition, &file_text))
                        .collect();
                    self.loaded = definitions;
                }
            }
            Err(error) => self.error = Some(error),
        }
        Ok(())
    }

    /// Returns a detached selection for the active role. The canonical
    /// connection composer decides whether this definition is eligible.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<(EntityId, SymbolDefinition)> {
        let role = self.role?;
        ui.separator();
        ui.strong("Choose hardware for this stack role");
        ui.small("Selecting a definition changes this draft only. Preview and Apply place the complete connection together.");
        ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .id(ui.make_persistent_id(("connection-hardware-search", role)))
                .hint_text("Search included or registered hardware")
                .desired_width(controls::inspector_width(ui)),
        );
        self.facets.show(ui, &self.entries);
        let query = SearchQuery::new(&self.query);
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        let mut chosen = None;
        let mut shown = 0;
        for (definition, search) in self.loaded.iter().zip(&self.loaded_search) {
            // The original may have changed after its listing was read.
            if !search.matches(&query, &self.facets) {
                continue;
            }
            shown += 1;
            ui.push_id(definition.symbol, |ui| {
                ui.group(|ui| {
                    ui.label(&definition.name);
                    ui.small(format!("Pinned revision {}", definition.revision));
                    if ui.button("Use this definition").clicked() {
                        chosen = Some((role, definition.clone()));
                    }
                });
            });
        }
        if self.request.is_some() {
            ui.label("Reading hardware definition…");
        } else if self.loaded.is_empty() {
            egui::ScrollArea::vertical()
                .id_salt(("connection-hardware-cards", role))
                .max_height(240.)
                .show(ui, |ui| {
                    for (entry, search) in self.entries.iter().zip(&self.search) {
                        if !search.matches(&query, &self.facets) {
                            continue;
                        }
                        shown += 1;
                        ui.push_id(entry.id, |ui| {
                            ui.group(|ui| {
                                ui.label(&entry.name);
                                let unavailable = entry.problem.as_deref().or_else(|| {
                                    (entry.symbol_definitions == 0)
                                        .then_some("No reusable definition in this file")
                                });
                                if let Some(reason) = unavailable {
                                    ui.small(reason);
                                }
                                if ui
                                    .add_enabled(
                                        unavailable.is_none(),
                                        egui::Button::new("Choose hardware"),
                                    )
                                    .clicked()
                                {
                                    self.generation = self.generation.wrapping_add(1);
                                    self.request = Some(HardwareLibraryRequest {
                                        generation: self.generation,
                                        intent: EntityId::new(),
                                        role,
                                        path: entry.path.clone(),
                                    });
                                }
                            });
                        });
                    }
                });
        }
        if self.request.is_none() && shown == 0 {
            ui.small(
                "No matching hardware source. Clear or change the filters to see other choices.",
            );
        }
        if ui.button("Cancel hardware choice").clicked() {
            self.cancel();
        }
        if chosen.is_some() {
            self.cancel();
        }
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_or_retargeted_hardware_load_cannot_replace_another_role() {
        let mut library = HardwareLibrary::default();
        let role = EntityId::new();
        library.begin(role);
        let request = HardwareLibraryRequest {
            role,
            generation: library.generation,
            intent: EntityId::new(),
            path: "/tmp/hardware.tcd".into(),
        };
        library.request = Some(request.clone());
        library.cancel();
        let empty_file = || {
            Ok(LoadedHardware {
                definitions: vec![],
                file_text: String::new(),
            })
        };
        assert!(library.accept(&request, empty_file()).is_err());
        library.begin(EntityId::new());
        assert!(library.accept(&request, empty_file()).is_err());
        assert!(library.loaded.is_empty());
    }
}

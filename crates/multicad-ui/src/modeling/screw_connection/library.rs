//! Detached hardware selection for one connection role. Selecting a card never
//! imports a definition or starts the independent Library placement tool.
use crate::{controls, egui, library::LibraryEntry};
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
    role: Option<EntityId>,
    query: String,
    generation: u64,
    request: Option<HardwareLibraryRequest>,
    loaded: Vec<SymbolDefinition>,
    error: Option<String>,
}

impl HardwareLibrary {
    pub fn set_entries(&mut self, entries: Arc<Vec<LibraryEntry>>) {
        self.entries = entries;
    }

    pub fn begin(&mut self, role: EntityId) {
        self.generation = self.generation.wrapping_add(1);
        self.role = Some(role);
        self.request = None;
        self.loaded.clear();
        self.error = None;
    }

    pub fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.role = None;
        self.request = None;
        self.loaded.clear();
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
        result: Result<Vec<SymbolDefinition>, String>,
    ) -> Result<(), String> {
        if self.request.as_ref() != Some(request) || self.role != Some(request.role) {
            return Err("This hardware choice belongs to an earlier connection draft".into());
        }
        self.request = None;
        match result {
            Ok(definitions) => {
                if definitions.is_empty() {
                    self.error = Some("This file contains no reusable hardware definitions".into());
                } else {
                    for definition in &definitions {
                        definition.validate().map_err(|error| error.to_string())?;
                    }
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
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        let mut chosen = None;
        for definition in &self.loaded {
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
            let query = self.query.to_lowercase();
            egui::ScrollArea::vertical()
                .id_salt(("connection-hardware-cards", role))
                .max_height(240.)
                .show(ui, |ui| {
                    for entry in self.entries.iter().filter(|entry| {
                        query.is_empty()
                            || entry.name.to_lowercase().contains(&query)
                            || entry.description.to_lowercase().contains(&query)
                            || entry.mechanical_parts.iter().any(|part| {
                                part.family.to_lowercase().contains(&query)
                                    || part
                                        .designation
                                        .as_deref()
                                        .is_some_and(|name| name.to_lowercase().contains(&query))
                            })
                    }) {
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
        assert!(library.accept(&request, Ok(vec![])).is_err());
        library.begin(EntityId::new());
        assert!(library.accept(&request, Ok(vec![])).is_err());
        assert!(library.loaded.is_empty());
    }
}

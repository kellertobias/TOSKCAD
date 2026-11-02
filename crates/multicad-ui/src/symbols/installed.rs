//! Browse the same included/original-file Library used by Project.
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
    family: String,
}
impl Installed {
    pub fn show(&mut self, ui: &mut egui::Ui, filter: &str) -> Option<SymbolAction> {
        ui.strong("Installation Library");
        ui.small("Included definitions and files listed with Add to Library. Load a file, then explicitly import and place its pinned definitions.");
        let families = self
            .entries
            .iter()
            .flat_map(|e| &e.mechanical_parts)
            .map(|p| p.family.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        crate::controls::inspector_combo(ui, "installed-library-family")
            .selected_text(if self.family.is_empty() {
                "All part families"
            } else {
                &self.family
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.family, String::new(), "All part families");
                for family in families {
                    ui.selectable_value(&mut self.family, family.into(), family);
                }
            });
        let mut action = None;
        let mut shown = 0;
        for entry in self
            .entries
            .iter()
            .filter(|entry| matches(entry, filter, &self.family))
        {
            shown += 1;
            ui.push_id(entry.id, |ui| ui.group(|ui| {
                ui.strong(&entry.name);
                if entry.origin == crate::library::LibraryOrigin::Bundled { ui.small("Included with ToskCAD"); }
                self.previews.show(ui, entry.preview.as_ref(), egui::vec2(140., 80.), &entry.name);
                if !entry.description.is_empty() { ui.add(egui::Label::new(egui::RichText::new(&entry.description).small()).truncate()).on_hover_text(&entry.description); }
                ui.small(entry.path.display().to_string());
                for part in entry.mechanical_parts.iter().filter(|p| self.family.is_empty() || p.family == self.family) {
                    ui.strong(format!("{} · {}", part.name, part.family));
                    if let Some((_, revision)) = &part.symbol { ui.small(format!("Pinned template defaults · revision {revision}")); }
                    else { ui.small("Source recipe dimensions"); }
                    for (name, value) in &part.dimensions_mm { ui.small(format!("{name}: {value} mm")); }
                    ui.small(&part.representation);
                    if let Some(designation) = &part.designation {
                        ui.small(format!("Reference: {designation} · {}", if part.matches_reference_dimensions { "dimensions match retained reference" } else { "custom / derived dimensions" }));
                        ui.small("Reference metadata is not catalog or stock certification.");
                    } else { ui.small("Custom geometry · no catalog attribution"); }
                    if let Some(source) = &part.source_url { ui.hyperlink_to("Dimensional source", source); }
                    if let Some(problem) = &part.problem { ui.colored_label(egui::Color32::LIGHT_RED, problem); }
                }
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
            ui.small("No matching listed files. Save a file with reusable symbols and choose Add to Library in Project.");
        }
        action
    }
}
fn matches(entry: &LibraryEntry, filter: &str, family: &str) -> bool {
    let base = format!(
        "{} {} {}",
        entry.name,
        entry.description,
        entry.path.display()
    )
    .to_lowercase();
    let contains = |searchable: &str| {
        filter
            .split_whitespace()
            .all(|term| searchable.contains(&term.to_lowercase()))
    };
    if family.is_empty() && contains(&base) {
        return true;
    }
    entry
        .mechanical_parts
        .iter()
        .filter(|p| family.is_empty() || p.family == family)
        .any(|p| {
            let facts = format!(
                "{base} {} {} {} {} {:?}",
                p.name,
                p.family,
                p.designation.as_deref().unwrap_or(""),
                p.representation,
                p.dimensions_mm
            )
            .to_lowercase();
            contains(&facts)
        })
}

pub(super) fn contains_terms(searchable: &str, filter: &str) -> bool {
    filter
        .split_whitespace()
        .all(|term| searchable.contains(&term.to_lowercase()))
}
/// Construct once when a definition snapshot changes, never during painting.
pub(super) fn definition_search(definition: &multicad_core::symbols::SymbolDefinition) -> String {
    let mut text = format!("{} {}", definition.name, definition.revision);
    for card in multicad_core::model::mechanical_part_cards(&definition.template) {
        text.push_str(&format!(
            " {} {} {} {} {:?}",
            card.name,
            card.family,
            card.designation.as_deref().unwrap_or(""),
            card.representation,
            card.dimensions_mm
        ));
    }
    text.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{document::EntityId, model::MechanicalPartCard};
    #[test]
    fn hardware_search_keeps_family_and_dimensions_on_the_same_source_recipe() {
        let washer = MechanicalPartCard {
            feature: EntityId::new(),
            name: "Thin ring".into(),
            family: "Plain washer".into(),
            symbol: Some((EntityId::new(), "1".into())),
            dimensions_mm: [("Bore".into(), 3.2), ("Thickness".into(), 0.5)].into(),
            representation: "Plain annular solid".into(),
            designation: Some("Fixture W".into()),
            source_url: None,
            matches_reference_dimensions: true,
            problem: None,
        };
        let mut bearing = washer.clone();
        bearing.family = "Bearing mounting envelope".into();
        bearing.name = "Large bearing".into();
        bearing.designation = Some("Fixture B".into());
        bearing.dimensions_mm = [("Bore".into(), 10.), ("Width".into(), 9.)].into();
        let entry = LibraryEntry {
            origin: crate::library::LibraryOrigin::ListedOriginal,
            id: EntityId::new(),
            document: EntityId::new(),
            path: "/library/rings.tcd".into(),
            name: "Mounting parts".into(),
            description: "Steel".into(),
            preview: None,
            registered_unix_ms: 1,
            symbol_definitions: 2,
            mechanical_parts: vec![washer, bearing],
            problem: None,
        };
        assert!(matches(&entry, "STEEL fixture W 3.2", "Plain washer"));
        assert!(matches(&entry, "fixture B 9", "Bearing mounting envelope"));
        assert!(!matches(&entry, "fixture B 9", "Plain washer"));
        assert!(
            !matches(&entry, "thin 9", ""),
            "cannot mix dimensions from another recipe"
        );
        assert!(!matches(&entry, "", "Screw"));
    }
}

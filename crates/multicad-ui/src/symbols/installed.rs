//! Browse the same included/original-file Library used by Project.
use super::SymbolAction;
use crate::{
    egui,
    library::{LibraryEntry, PreviewTextures},
};
use multicad_core::model::MechanicalPartCard;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct Installed {
    entries: Arc<Vec<LibraryEntry>>,
    search: Vec<SearchIndex>,
    previews: PreviewTextures,
    family: String,
}
impl Installed {
    pub fn set_entries(&mut self, entries: Arc<Vec<LibraryEntry>>) {
        if Arc::ptr_eq(&self.entries, &entries) {
            return;
        }
        self.search = entries.iter().map(entry_search).collect();
        self.entries = entries;
    }
    pub fn show(&mut self, ui: &mut egui::Ui, query: &SearchQuery) -> Option<SymbolAction> {
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
            .zip(&self.search)
            .filter(|(_, search)| search.matches(query, &self.family))
        {
            let (entry, search) = entry;
            shown += 1;
            ui.push_id(entry.id, |ui| ui.group(|ui| {
                ui.strong(&entry.name);
                if entry.origin == crate::library::LibraryOrigin::Bundled { ui.small("Included with ToskCAD"); }
                self.previews.show(ui, entry.preview.as_ref(), egui::vec2(140., 80.), &entry.name);
                if !entry.description.is_empty() { ui.add(egui::Label::new(egui::RichText::new(&entry.description).small()).truncate()).on_hover_text(&entry.description); }
                ui.small(entry.path.display().to_string());
                search.show_cards(ui, query, &self.family);
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
pub(super) struct SearchQuery {
    terms: Vec<String>,
    nominal: Option<f64>,
    conflicting_nominals: bool,
}
impl SearchQuery {
    pub fn new(filter: &str) -> Self {
        let mut query = Self {
            terms: vec![],
            nominal: None,
            conflicting_nominals: false,
        };
        for term in filter.split_whitespace().map(str::to_lowercase) {
            let nominal = term
                .strip_prefix('m')
                .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit() || c == '.'))
                .and_then(|n| n.parse::<f64>().ok())
                .filter(|n| n.is_finite());
            if let Some(nominal) = nominal {
                query.conflicting_nominals |=
                    query.nominal.is_some_and(|previous| previous != nominal);
                query.nominal = Some(nominal);
            } else {
                query.terms.push(term);
            }
        }
        query
    }
    fn contains(&self, text: &str) -> bool {
        self.terms.iter().all(|term| text.contains(term))
    }
}
struct CardSearch {
    card: MechanicalPartCard,
    text: String,
}
pub(super) struct SearchIndex {
    base: String,
    parts: Vec<CardSearch>,
}
impl SearchIndex {
    fn new(base: String, cards: Vec<MechanicalPartCard>) -> Self {
        let base = base.to_lowercase();
        let parts = cards
            .into_iter()
            .map(|card| {
                let mut text = format!(
                    "{base} {} {} {} {} {:?}",
                    card.name,
                    card.family,
                    card.designation.as_deref().unwrap_or(""),
                    card.representation,
                    card.dimensions_mm
                );
                if let Some(facts) = &card.catalog_facts {
                    for value in [&facts.series, &facts.material, &facts.grade]
                        .into_iter()
                        .flatten()
                    {
                        text.push(' ');
                        text.push_str(value);
                    }
                    if let Some(nominal) = facts.nominal_fastener_diameter_mm {
                        text.push_str(&format!(" nominal fastener {nominal} mm"));
                    }
                }
                CardSearch {
                    card,
                    text: text.to_lowercase(),
                }
            })
            .collect();
        Self { base, parts }
    }
    fn card_matches(part: &CardSearch, query: &SearchQuery, family: &str) -> bool {
        !query.conflicting_nominals
            && (family.is_empty() || part.card.family == family)
            && query.nominal.is_none_or(|nominal| {
                part.card
                    .catalog_facts
                    .as_ref()
                    .and_then(|facts| facts.nominal_fastener_diameter_mm)
                    == Some(nominal)
            })
            && query.contains(&part.text)
    }
    pub fn matches(&self, query: &SearchQuery, family: &str) -> bool {
        if query.conflicting_nominals {
            return false;
        }
        (query.nominal.is_none() && family.is_empty() && query.contains(&self.base))
            || self
                .parts
                .iter()
                .any(|part| Self::card_matches(part, query, family))
    }
    pub fn show_cards(&self, ui: &mut egui::Ui, query: &SearchQuery, family: &str) {
        for part in self
            .parts
            .iter()
            .filter(|part| Self::card_matches(part, query, family))
        {
            show_card(ui, &part.card);
        }
    }
}
fn show_card(ui: &mut egui::Ui, part: &MechanicalPartCard) {
    ui.strong(format!("{} · {}", part.name, part.family));
    if let Some((_, revision)) = &part.symbol {
        ui.small(format!("Pinned template defaults · revision {revision}"));
    } else {
        ui.small("Source recipe dimensions");
    }
    for (name, value) in &part.dimensions_mm {
        ui.small(format!("{name}: {value} mm"));
    }
    let facts = part.catalog_facts.as_ref();
    if let Some(nominal) = facts.and_then(|f| f.nominal_fastener_diameter_mm) {
        ui.small(format!("Source nominal fastener diameter: {nominal} mm"));
    } else {
        ui.small("Source nominal fastener diameter: not specified");
    }
    for (label, value) in [
        ("Series", facts.and_then(|f| f.series.as_deref())),
        ("Material", facts.and_then(|f| f.material.as_deref())),
        ("Grade", facts.and_then(|f| f.grade.as_deref())),
    ] {
        ui.small(format!(
            "Source {}: {}",
            label.to_lowercase(),
            value.unwrap_or("not specified")
        ));
    }
    ui.small(&part.representation);
    if let Some(designation) = &part.designation {
        ui.small(format!(
            "Reference: {designation} · {}",
            if part.matches_reference_dimensions {
                "dimensions match retained reference"
            } else {
                "custom / derived dimensions"
            }
        ));
        ui.small("Reference metadata is not catalog or stock certification.");
    } else {
        ui.small("Custom geometry · no catalog attribution");
    }
    if let Some(source) = &part.source_url {
        ui.hyperlink_to("Dimensional source", source);
    }
    if let Some(problem) = &part.problem {
        ui.colored_label(egui::Color32::LIGHT_RED, problem);
    }
}
fn entry_search(entry: &LibraryEntry) -> SearchIndex {
    SearchIndex::new(
        format!(
            "{} {} {}",
            entry.name,
            entry.description,
            entry.path.display()
        ),
        entry.mechanical_parts.clone(),
    )
}
/// Construct once when a definition snapshot changes, never during painting.
pub(super) fn definition_search(
    definition: &multicad_core::symbols::SymbolDefinition,
) -> SearchIndex {
    let mut cards = multicad_core::model::mechanical_part_cards(&definition.template);
    for card in &mut cards {
        if card.symbol.is_none() {
            card.symbol = Some((definition.symbol, definition.revision.clone()));
        }
    }
    SearchIndex::new(
        format!("{} {}", definition.name, definition.revision),
        cards,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{document::EntityId, model::MechanicalPartCard};
    fn matches(entry: &LibraryEntry, filter: &str, family: &str) -> bool {
        entry_search(entry).matches(&SearchQuery::new(filter), family)
    }
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
            catalog_facts: None,
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
    #[test]
    fn nominal_queries_require_exact_sourced_facts_and_keep_other_terms_on_that_card() {
        use multicad_core::model::MechanicalPartCatalogFacts;
        let card =
            |name: &str, nominal: Option<f64>, bore: f64, material: &str| MechanicalPartCard {
                feature: EntityId::new(),
                name: name.into(),
                family: "Plain washer".into(),
                symbol: None,
                dimensions_mm: [("Bore".into(), bore)].into(),
                representation: "Annular solid".into(),
                designation: Some(name.into()),
                source_url: None,
                matches_reference_dimensions: true,
                problem: None,
                catalog_facts: nominal.map(|value| MechanicalPartCatalogFacts {
                    nominal_fastener_diameter_mm: Some(value),
                    series: Some("Fixture normal series".into()),
                    material: Some(material.into()),
                    grade: None,
                }),
            };
        let three = card("Small fixture", Some(3.), 3.2, "Fixture steel");
        let decimal = card("Decimal fixture", Some(3.5), 3.7, "Fixture steel");
        let thirty = card("Large fixture", Some(30.), 31., "Fixture brass");
        let unknown = card("Unattributed M3 fixture", None, 3.2, "Fixture steel");
        let index = SearchIndex::new(
            "File M3 M30".into(),
            vec![
                three.clone(),
                decimal.clone(),
                thirty.clone(),
                unknown.clone(),
            ],
        );
        for query in ["M3", "m3.0 steel normal", "M3 3.2"] {
            assert!(index.matches(&SearchQuery::new(query), ""), "{query}");
        }
        for query in ["M3 M30", "M3 brass", "M3 31", "M3 grade-8.8", "M4.5", "M9"] {
            assert!(!index.matches(&SearchQuery::new(query), ""), "{query}");
        }
        assert!(
            !SearchIndex::new(
                "M3 only in file name".into(),
                vec![decimal, thirty, unknown]
            )
            .matches(&SearchQuery::new("M3"), "")
        );
        let small = SearchIndex::new("Small source".into(), vec![three]);
        assert!(!small.matches(&SearchQuery::new("M3.5"), ""));
        assert!(!small.matches(&SearchQuery::new("M30"), ""));
        assert!(small.matches(&SearchQuery::new("washer 3.2"), "Plain washer"));
        assert!(!small.matches(&SearchQuery::new("M3"), "Hex nut"));
    }
}

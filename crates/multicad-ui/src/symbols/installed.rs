//! Browse the same included/original-file Library used by Project.
use super::SymbolAction;
pub(super) use crate::library::facets::{SearchIndex, definition_search};
use crate::{
    egui,
    library::{
        LibraryEntry, PreviewTextures,
        facets::{CatalogFilters, SearchQuery, entry_search},
    },
};
use multicad_core::model::MechanicalPartCard;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct Installed {
    entries: Arc<Vec<LibraryEntry>>,
    search: Vec<SearchIndex>,
    previews: PreviewTextures,
    facets: CatalogFilters,
}
impl Installed {
    pub fn set_entries(&mut self, entries: Arc<Vec<LibraryEntry>>) {
        if Arc::ptr_eq(&self.entries, &entries) {
            return;
        }
        self.search = entries.iter().map(entry_search).collect();
        self.entries = entries;
    }
    pub fn facets(&self) -> &CatalogFilters {
        &self.facets
    }
    pub fn show_filters(&mut self, ui: &mut egui::Ui) {
        self.facets.show(ui, &self.entries);
    }
    pub fn show(&mut self, ui: &mut egui::Ui, query: &SearchQuery) -> Option<SymbolAction> {
        ui.strong("Installation Library");
        ui.small("Included definitions and files listed with Add to Library. Place a pinned definition directly, or load a file to inspect and choose its definitions.");
        let exact = self.facets.active() || query.has_structured_nominal();
        let mut action = None;
        let mut shown = 0;
        for entry in self
            .entries
            .iter()
            .zip(&self.search)
            .filter(|(_, search)| search.matches(query, &self.facets))
        {
            let (entry, search) = entry;
            shown += 1;
            ui.push_id(entry.id, |ui| ui.group(|ui| {
                ui.strong(&entry.name);
                if entry.origin == crate::library::LibraryOrigin::Bundled { ui.small("Included with ToskCAD"); }
                let unavailable = entry.problem.as_deref().or_else(|| (entry.symbol_definitions == 0).then_some("No reusable definitions. Open this file and use Create symbol before placing it here."));
                if let Some(problem) = unavailable {
                    ui.colored_label(egui::Color32::LIGHT_RED, problem);
                }
                ui.horizontal_wrapped(|ui| {
                    let place = ui.add_enabled(unavailable.is_none(), egui::Button::new(if exact { "Choose matching definition" } else { "Place" }));
                    let place = if let Some(reason) = unavailable { place.on_disabled_hover_text(reason) } else { place.on_hover_text("Choose this file's pinned definition and placement; Preview and Apply embed and place it together") };
                    if place.clicked() {
                        // Re-read originals and let the exact filters inspect the
                        // loaded definition before it can start a placement.
                        action = Some(if exact { SymbolAction::OpenInstalled(entry.path.clone()) } else { SymbolAction::OpenInstalledForPlace(entry.path.clone()) });
                    }
                    if !exact && ui.add_enabled(unavailable.is_none(), egui::Button::new("Load definitions")).clicked() {
                        action = Some(SymbolAction::OpenInstalled(entry.path.clone()));
                    }
                });
                let preview_width = crate::controls::inspector_width(ui).min(140.);
                self.previews.show(ui, entry.preview.as_ref(), egui::vec2(preview_width, 80.), &entry.name);
                if !entry.description.is_empty() { ui.add(egui::Label::new(egui::RichText::new(&entry.description).small()).truncate()).on_hover_text(&entry.description); }
                let path = entry.path.display().to_string();
                ui.add(egui::Label::new(egui::RichText::new(&path).small()).truncate())
                    .on_hover_text(path);
                show_cards(ui, search, query, &self.facets);
            }));
        }
        if shown == 0 {
            if exact {
                ui.small("No source recipe matches the hardware filters. Clear or change a filter to browse other files.");
            } else {
                ui.small("No matching listed files. Save a file with reusable symbols and choose Add to Library in Project.");
            }
        }
        action
    }
}
pub(super) fn show_cards(
    ui: &mut egui::Ui,
    search: &SearchIndex,
    query: &SearchQuery,
    facets: &CatalogFilters,
) {
    for card in search.matching_cards(query, facets) {
        show_card(ui, card);
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
    for detail in &part.profile_details {
        if let Some(angle) = detail.strip_suffix(" degrees") {
            ui.small(format!("{angle}°"));
        } else {
            ui.small(detail);
        }
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
#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{document::EntityId, model::MechanicalPartCard};
    fn matches(entry: &LibraryEntry, filter: &str, family: &str) -> bool {
        entry_search(entry).matches(
            &SearchQuery::new(filter),
            &CatalogFilters::for_family(family),
        )
    }
    #[test]
    fn installed_place_actions_precede_long_metadata_and_dimensions_without_scrolling() {
        let card = MechanicalPartCard {
            feature: EntityId::new(),
            name: "Detailed hardware".into(),
            family: "Fixture".into(),
            symbol: Some((EntityId::new(), "1".into())),
            recipe_facets: Default::default(),
            profile_details: Vec::new(),
            dimensions_mm: (0..48)
                .map(|index| (format!("Dimension {index:02}"), index as f64))
                .collect(),
            representation: "Modeled fixture".into(),
            designation: None,
            source_url: None,
            catalog_facts: None,
            matches_reference_dimensions: true,
            problem: None,
        };
        let entry = LibraryEntry {
            origin: crate::library::LibraryOrigin::Bundled,
            id: EntityId::new(),
            document: EntityId::new(),
            path: "/library/detailed-fixture.tcd".into(),
            name: "Detailed fixture".into(),
            description: "Long descriptive metadata ".repeat(40),
            preview: None,
            registered_unix_ms: 1,
            symbol_definitions: 1,
            mechanical_parts: vec![card],
            problem: None,
        };
        let mut installed = Installed::default();
        installed.set_entries(Arc::new(vec![entry]));
        let ctx = egui::Context::default();
        let frame = |installed: &mut Installed| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(250., 700.),
                    )),
                    ..Default::default()
                },
                |ui| {
                    installed.show(ui, &SearchQuery::new(""));
                },
            )
        };
        frame(&mut installed).drop_without_applying_deltas();
        let output = frame(&mut installed);
        fn painted(shape: &egui::Shape, text: &str) -> Option<egui::Rect> {
            match shape {
                egui::Shape::Text(shape) if shape.galley.text() == text => {
                    Some(egui::Rect::from_min_size(shape.pos, shape.galley.size()))
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| painted(shape, text)),
                _ => None,
            }
        }
        let rect = |text| {
            output
                .shapes
                .iter()
                .find_map(|shape| painted(&shape.shape, text))
                .unwrap_or_else(|| panic!("Missing painted {text}"))
        };
        let place = rect("Place");
        let load = rect("Load definitions");
        let title = rect("Detailed fixture");
        assert!(place.top() > title.bottom());
        assert!(
            place.bottom() < 350. && load.bottom() < 350.,
            "card actions must be immediately reachable before details"
        );
        assert!(place.left() >= 0. && load.right() <= 250.);
        output.drop_without_applying_deltas();
    }
    #[test]
    fn hardware_search_keeps_family_and_dimensions_on_the_same_source_recipe() {
        let washer = MechanicalPartCard {
            feature: EntityId::new(),
            name: "Thin ring".into(),
            family: "Plain washer".into(),
            symbol: Some((EntityId::new(), "1".into())),
            recipe_facets: Default::default(),
            profile_details: Vec::new(),
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
                recipe_facets: Default::default(),
                profile_details: Vec::new(),
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
        let mut wood = card("DIN571-related wood screw d6", Some(6.), 6., "");
        wood.family = "Wood screw envelope".into();
        let wood = SearchIndex::new("Wood source".into(), vec![wood]);
        assert!(wood.matches(&SearchQuery::new("wood d6"), &CatalogFilters::default()));
        assert!(!wood.matches(&SearchQuery::new("M6"), &CatalogFilters::default()));
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
            assert!(
                index.matches(&SearchQuery::new(query), &CatalogFilters::default()),
                "{query}"
            );
        }
        for query in ["M3 M30", "M3 brass", "M3 31", "M3 grade-8.8", "M4.5", "M9"] {
            assert!(
                !index.matches(&SearchQuery::new(query), &CatalogFilters::default()),
                "{query}"
            );
        }
        assert!(
            !SearchIndex::new(
                "M3 only in file name".into(),
                vec![decimal, thirty, unknown]
            )
            .matches(&SearchQuery::new("M3"), &CatalogFilters::default())
        );
        let small = SearchIndex::new("Small source".into(), vec![three]);
        assert!(!small.matches(&SearchQuery::new("M3.5"), &CatalogFilters::default()));
        assert!(!small.matches(&SearchQuery::new("M30"), &CatalogFilters::default()));
        assert!(small.matches(
            &SearchQuery::new("washer 3.2"),
            &CatalogFilters::for_family("Plain washer")
        ));
        assert!(!small.matches(
            &SearchQuery::new("M3"),
            &CatalogFilters::for_family("Hex nut")
        ));
    }
    #[test]
    fn configured_profile_details_are_searchable_and_painted_with_angle_units() {
        let card = MechanicalPartCard {
            feature: EntityId::new(),
            name: "Configured pan".into(),
            family: "Machine screw".into(),
            symbol: None,
            dimensions_mm: [("Head diameter".into(), 5.6)].into(),
            recipe_facets: Default::default(),
            profile_details: vec![
                "Phillips H1".into(),
                "Outer corner included angle: 92 degrees".into(),
            ],
            representation: "Configured engineering profile".into(),
            designation: None,
            source_url: None,
            catalog_facts: None,
            matches_reference_dimensions: false,
            problem: None,
        };
        let index = SearchIndex::new("Configured source".into(), vec![card.clone()]);
        assert!(index.matches(
            &SearchQuery::new("Phillips H1 92 degrees"),
            &CatalogFilters::for_family("Machine screw")
        ));
        let ctx = egui::Context::default();
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(350., 1200.),
                )),
                ..Default::default()
            },
            |ui| show_card(ui, &card),
        );
        let text = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert!(text.contains("Head diameter: 5.6 mm"));
        assert!(text.contains("92°"));
        assert!(!text.contains("92 mm"));
        out.drop_without_applying_deltas();
    }
}

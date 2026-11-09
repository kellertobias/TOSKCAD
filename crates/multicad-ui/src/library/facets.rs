//! Exact source facts for the installation Library and connection hardware picker.
use super::LibraryEntry;
use crate::{controls, egui};
use multicad_core::{
    model::{
        MechanicalDriveForm, MechanicalHeadForm, MechanicalPartCard, MechanicalRecipeLength,
        MechanicalRecipePitch, mechanical_part_definition_cards,
    },
    symbols::SymbolDefinition,
};
use std::{borrow::Cow, cmp::Ordering};

pub(crate) struct LoadedHardware {
    pub definitions: Vec<SymbolDefinition>,
    pub file_text: String,
}

#[derive(Clone, Copy)]
enum Facet {
    Family,
    Designation,
    Nominal,
    Length,
    Pitch,
    Head,
    Drive,
    Material,
}
impl Facet {
    const ALL: [Self; 8] = [
        Self::Family,
        Self::Designation,
        Self::Nominal,
        Self::Length,
        Self::Pitch,
        Self::Head,
        Self::Drive,
        Self::Material,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Family => "Part family",
            Self::Designation => "Source designation",
            Self::Nominal => "Source nominal diameter",
            Self::Length => "Authored screw length",
            Self::Pitch => "Pitch",
            Self::Head => "Head form",
            Self::Drive => "Drive",
            Self::Material => "Source material",
        }
    }

    fn value(self, card: &MechanicalPartCard) -> Option<FacetValue<'_>> {
        if card.problem.is_some() {
            return None;
        }
        match self {
            Self::Family => Some(FacetValue::Text(card.family.as_str().into())),
            Self::Designation => card
                .designation
                .as_deref()
                .map(|v| FacetValue::Text(v.into())),
            Self::Nominal => card
                .catalog_facts
                .as_ref()?
                .nominal_fastener_diameter_mm
                .map(FacetValue::Nominal),
            Self::Length => card.recipe_facets.length.map(FacetValue::Length),
            Self::Pitch => card.recipe_facets.pitch.map(FacetValue::Pitch),
            Self::Head => card.recipe_facets.head.map(FacetValue::Head),
            Self::Drive => card.recipe_facets.drive.map(FacetValue::Drive),
            Self::Material => card
                .catalog_facts
                .as_ref()?
                .material
                .as_deref()
                .map(|v| FacetValue::Text(v.into())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, PartialOrd)]
enum FacetValue<'a> {
    Text(Cow<'a, str>),
    Nominal(f64),
    Length(MechanicalRecipeLength),
    Pitch(MechanicalRecipePitch),
    Head(MechanicalHeadForm),
    Drive(MechanicalDriveForm),
}
impl FacetValue<'_> {
    fn owned(self) -> FacetValue<'static> {
        match self {
            Self::Text(value) => FacetValue::Text(Cow::Owned(value.into_owned())),
            Self::Nominal(value) => FacetValue::Nominal(value),
            Self::Length(value) => FacetValue::Length(value),
            Self::Pitch(value) => FacetValue::Pitch(value),
            Self::Head(value) => FacetValue::Head(value),
            Self::Drive(value) => FacetValue::Drive(value),
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Text(value) => value.to_string(),
            Self::Nominal(mm) => format!("{mm} mm"),
            Self::Length(MechanicalRecipeLength::Underhead(mm)) => format!("{mm} mm underhead"),
            Self::Length(MechanicalRecipeLength::Overall(mm)) => format!("{mm} mm overall"),
            Self::Pitch(MechanicalRecipePitch::Thread(mm)) => format!("{mm} mm thread pitch"),
            Self::Pitch(MechanicalRecipePitch::WoodReference(mm)) => {
                format!("{mm} mm wood reference pitch")
            }
            Self::Head(head) => match head {
                MechanicalHeadForm::SocketCap => "Socket-cap head",
                MechanicalHeadForm::ExternalHex => "External-hex head",
                MechanicalHeadForm::Pan => "Pan head",
                MechanicalHeadForm::Button => "Button head",
                MechanicalHeadForm::Countersunk => "Countersunk head",
            }
            .into(),
            Self::Drive(drive) => match drive {
                MechanicalDriveForm::HexSocket => "Hex socket",
                MechanicalDriveForm::ExternalHex => "External hex flats",
                MechanicalDriveForm::Hexalobular => "Hexalobular recess",
                MechanicalDriveForm::PhillipsH => "Phillips H recess",
            }
            .into(),
        }
    }
}

#[derive(Clone, Default, PartialEq)]
pub(crate) struct CatalogFilters {
    selected: [Option<FacetValue<'static>>; Facet::ALL.len()],
}
impl CatalogFilters {
    #[cfg(test)]
    pub(crate) fn for_family(family: &str) -> Self {
        let mut result = Self::default();
        if !family.is_empty() {
            result.selected[Facet::Family as usize] =
                Some(FacetValue::Text(family.to_owned().into()));
        }
        result
    }

    pub fn active(&self) -> bool {
        self.selected.iter().any(Option::is_some)
    }

    fn matches_except(&self, card: &MechanicalPartCard, except: Option<usize>) -> bool {
        Facet::ALL.into_iter().enumerate().all(|(index, facet)| {
            except == Some(index)
                || self.selected[index]
                    .as_ref()
                    .is_none_or(|selected| facet.value(card).as_ref() == Some(selected))
        })
    }

    pub fn matches(&self, card: &MechanicalPartCard) -> bool {
        self.matches_except(card, None)
    }

    fn options(&self, index: usize, cards: &[&MechanicalPartCard]) -> Vec<FacetValue<'static>> {
        let mut options = cards
            .iter()
            .filter(|card| self.matches_except(card, Some(index)))
            .filter_map(|card| Facet::ALL[index].value(card).map(FacetValue::owned))
            .collect::<Vec<_>>();
        options.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        options.dedup();
        options
    }

    pub fn show(&mut self, ui: &mut egui::Ui, entries: &[LibraryEntry]) {
        let count = self.selected.iter().flatten().count();
        let title = if count == 0 {
            "Hardware filters".into()
        } else {
            format!("Hardware filters ({count})")
        };
        egui::CollapsingHeader::new(title).id_salt("exact-hardware-filters").show(ui, |ui| {
            ui.small("Filter source recipe facts. Values are available only when a single card has the selected combination.");
            if ui.add_enabled(count != 0, egui::Button::new("Clear hardware filters")).clicked() {
                *self = Self::default();
            }
            let cards = entries.iter().filter(|entry| entry.problem.is_none())
                .flat_map(|entry| &entry.mechanical_parts).collect::<Vec<_>>();
            for (index, facet) in Facet::ALL.into_iter().enumerate() {
                let options = self.options(index, &cards);
                let selection = &mut self.selected[index];
                let text = match selection {
                    Some(value) if options.contains(value) => value.label(),
                    Some(value) => format!("{} (unavailable)", value.label()),
                    None if options.is_empty() => "Unavailable for this selection".into(),
                    None => "Any".into(),
                };
                ui.label(facet.label());
                ui.add_enabled_ui(!options.is_empty() || selection.is_some(), |ui| {
                    controls::inspector_combo(ui, ("hardware-facet", index)).selected_text(text).show_ui(ui, |ui| {
                        ui.selectable_value(selection, None, "Any");
                        for option in options {
                            let label = option.label();
                            ui.selectable_value(selection, Some(option), label);
                        }
                    });
                });
            }
        });
    }
}

pub(crate) struct SearchQuery {
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
    pub fn has_structured_nominal(&self) -> bool {
        self.nominal.is_some()
    }

    fn contains(&self, text: &str) -> bool {
        self.terms.iter().all(|term| text.contains(term))
    }
}
struct CardSearch {
    card: MechanicalPartCard,
    text: String,
}
pub(crate) struct SearchIndex {
    base: String,
    parts: Vec<CardSearch>,
}
impl SearchIndex {
    pub(crate) fn new(base: String, cards: Vec<MechanicalPartCard>) -> Self {
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
                for detail in &card.profile_details {
                    text.push(' ');
                    text.push_str(detail);
                }
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
                        if card.family == "Wood screw envelope" {
                            text.push_str(&format!(" d{nominal}"));
                        }
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
    fn card_matches(part: &CardSearch, query: &SearchQuery, facets: &CatalogFilters) -> bool {
        !query.conflicting_nominals
            && facets.matches(&part.card)
            && query.nominal.is_none_or(|nominal| {
                part.card.family != "Wood screw envelope"
                    && part
                        .card
                        .catalog_facts
                        .as_ref()
                        .and_then(|facts| facts.nominal_fastener_diameter_mm)
                        == Some(nominal)
            })
            && query.contains(&part.text)
    }
    pub fn matches(&self, query: &SearchQuery, facets: &CatalogFilters) -> bool {
        if query.conflicting_nominals {
            return false;
        }
        (query.nominal.is_none() && !facets.active() && query.contains(&self.base))
            || self
                .parts
                .iter()
                .any(|part| Self::card_matches(part, query, facets))
    }
    pub fn matching_cards<'a>(
        &'a self,
        query: &'a SearchQuery,
        facets: &'a CatalogFilters,
    ) -> impl Iterator<Item = &'a MechanicalPartCard> + 'a {
        self.parts
            .iter()
            .filter(move |part| Self::card_matches(part, query, facets))
            .map(|part| &part.card)
    }
}

pub(crate) fn entry_search(entry: &LibraryEntry) -> SearchIndex {
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
pub(crate) fn definition_search(definition: &SymbolDefinition) -> SearchIndex {
    definition_search_in_file(definition, "")
}

pub(crate) fn definition_search_in_file(
    definition: &SymbolDefinition,
    file_text: &str,
) -> SearchIndex {
    SearchIndex::new(
        format!("{} {} {file_text}", definition.name, definition.revision),
        mechanical_part_definition_cards(definition),
    )
}

pub(crate) fn file_search_text(
    document: &multicad_core::document::Document,
    path: &std::path::Path,
) -> Result<String, String> {
    let info = multicad_core::project_file::info::read(document)?;
    Ok(format!(
        "{} {} {}",
        info.name,
        info.description,
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        commands::{Actor, Channel, Session},
        document::{Document, EntityId},
        model::{self, MechanicalPartParameters as Part, MechanicalPartRequest, ScrewLengthInput},
        project_file::{self, ProjectFile},
        symbols::{self, SymbolAuthorRequest, SymbolKind},
    };

    fn add(
        session: &mut Session,
        name: &str,
        parameters: Part,
        facts: Option<(f64, &str)>,
    ) -> SymbolDefinition {
        let mut request = MechanicalPartRequest::new(name, parameters.clone());
        if let Some((nominal, material)) = facts {
            request.provenance = Some(model::MechanicalPartProvenance {
                catalog_id: "catalog-facet-test".into(),
                revision: "1".into(),
                designation: name.into(),
                source_url: "https://example.invalid/catalog".into(),
                reference_geometry: parameters,
                catalog_facts: Some(model::MechanicalPartCatalogFacts {
                    nominal_fastener_diameter_mm: Some(nominal),
                    material: Some(material.into()),
                    ..Default::default()
                }),
            });
        }
        let actor = Actor::local(Channel::Ui);
        let prepared =
            model::prepare_mechanical_part(session, session.revision(), actor.clone(), &request)
                .unwrap();
        session.commit(prepared.prepared).unwrap();
        let author = SymbolAuthorRequest::new(name, request.body, SymbolKind::Static, vec![], None);
        let prepared =
            symbols::prepare_author(session, session.revision(), actor, &author).unwrap();
        session.commit(prepared.prepared).unwrap();
        symbols::read_definition(session.document(), author.definition).unwrap()
    }

    fn original_entry(session: &Session) -> LibraryEntry {
        let mut bytes = vec![];
        let project = ProjectFile::new(session.document().clone()).unwrap();
        project_file::write(&project, &mut bytes, Default::default()).unwrap();
        let card = project_file::card(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap();
        LibraryEntry {
            origin: super::super::LibraryOrigin::ListedOriginal,
            id: EntityId::new(),
            document: card.document,
            path: "/originals/M3 stainless steel and bronze.tcd".into(),
            name: card.info.name,
            description: "Stainless steel and bronze fixtures".into(),
            preview: None,
            registered_unix_ms: 1,
            symbol_definitions: card.symbol_definitions,
            mechanical_parts: card.mechanical_parts,
            problem: None,
        }
    }

    fn select(filters: &mut CatalogFilters, facet: Facet, value: FacetValue<'_>) {
        filters.selected[facet as usize] = Some(value.owned());
    }

    #[test]
    fn exact_filters_use_one_template_recipe_and_recheck_loaded_definitions() {
        let mut session = Session::new(Document::new("Mixed source recipes").unwrap()).unwrap();
        let short = add(
            &mut session,
            "M3 socket cap short",
            model::default_machine_screw(),
            Some((3., "Steel")),
        );
        let mut long = model::default_machine_screw();
        let Part::MachineScrew {
            underhead_length,
            length_input,
            ..
        } = &mut long
        else {
            unreachable!()
        };
        *underhead_length = None;
        *length_input = Some(ScrewLengthInput::Overall {
            value: model::length(100.),
        });
        let long = add(
            &mut session,
            "M3 socket cap long",
            long,
            Some((3., "Brass")),
        );
        let washer = add(
            &mut session,
            "M3-looking washer",
            Part::Washer {
                bore_diameter: model::length(3.2),
                outside_diameter: model::length(7.),
                thickness: model::length(0.5),
                frame: Default::default(),
            },
            None,
        );
        let wood = add(
            &mut session,
            "Wood source",
            model::default_wood_screw(),
            Some((6., "Steel")),
        );
        let entry = original_entry(&session);
        let index = entry_search(&entry);
        let query = SearchQuery::new("");
        let mut filters = CatalogFilters::default();
        select(
            &mut filters,
            Facet::Family,
            FacetValue::Text("Machine screw".into()),
        );
        select(
            &mut filters,
            Facet::Designation,
            FacetValue::Text("M3 socket cap short".into()),
        );
        select(&mut filters, Facet::Nominal, FacetValue::Nominal(3.));
        select(
            &mut filters,
            Facet::Length,
            FacetValue::Length(MechanicalRecipeLength::Underhead(10.)),
        );
        select(
            &mut filters,
            Facet::Pitch,
            FacetValue::Pitch(MechanicalRecipePitch::Thread(0.5)),
        );
        select(
            &mut filters,
            Facet::Head,
            FacetValue::Head(MechanicalHeadForm::SocketCap),
        );
        select(
            &mut filters,
            Facet::Drive,
            FacetValue::Drive(MechanicalDriveForm::HexSocket),
        );
        select(
            &mut filters,
            Facet::Material,
            FacetValue::Text("Steel".into()),
        );
        assert!(index.matches(&query, &filters));
        assert_eq!(index.matching_cards(&query, &filters).count(), 1);
        assert!(definition_search(&short).matches(&query, &filters));
        assert!(!definition_search(&long).matches(&query, &filters));
        let cards = entry.mechanical_parts.iter().collect::<Vec<_>>();
        assert_eq!(
            filters.options(Facet::Material as usize, &cards),
            vec![FacetValue::Text("Steel".into())]
        );
        select(
            &mut filters,
            Facet::Material,
            FacetValue::Text("Brass".into()),
        );
        assert!(
            !index.matches(&query, &filters),
            "Other cards cannot supply material for the matching short screw"
        );
        select(
            &mut filters,
            Facet::Material,
            FacetValue::Text("bronze".into()),
        );
        assert!(
            !index.matches(&query, &filters),
            "File prose does not supply material"
        );
        filters = CatalogFilters::default();
        select(
            &mut filters,
            Facet::Length,
            FacetValue::Length(MechanicalRecipeLength::Overall(10.)),
        );
        assert!(
            !index.matches(&query, &filters),
            "Length is exact in both value and convention"
        );
        select(
            &mut filters,
            Facet::Length,
            FacetValue::Length(MechanicalRecipeLength::Overall(100.)),
        );
        assert!(definition_search(&long).matches(&query, &filters));
        filters = CatalogFilters::default();
        select(&mut filters, Facet::Nominal, FacetValue::Nominal(3.));
        assert!(!definition_search(&washer).matches(&query, &filters));
        select(&mut filters, Facet::Nominal, FacetValue::Nominal(3.2));
        assert!(
            !definition_search(&washer).matches(&query, &filters),
            "Bore is not a sourced nominal"
        );
        filters = CatalogFilters::default();
        select(
            &mut filters,
            Facet::Pitch,
            FacetValue::Pitch(MechanicalRecipePitch::Thread(2.6)),
        );
        assert!(!definition_search(&wood).matches(&query, &filters));
        select(
            &mut filters,
            Facet::Pitch,
            FacetValue::Pitch(MechanicalRecipePitch::WoodReference(2.6)),
        );
        assert!(definition_search(&wood).matches(&query, &filters));
    }

    #[test]
    fn nominal_shorthand_rechecks_fresh_definition_after_file_text_match() {
        let mut session = Session::new(Document::new("Listed original").unwrap()).unwrap();
        let short = add(
            &mut session,
            "M3 screw",
            model::default_machine_screw(),
            Some((3., "Steel")),
        );
        let replacement = add(
            &mut session,
            "M4 washer",
            Part::Washer {
                bore_diameter: model::length(4.3),
                outside_diameter: model::length(9.),
                thickness: model::length(0.8),
                frame: Default::default(),
            },
            Some((4., "Steel")),
        );
        let query = SearchQuery::new("M3 bronze");
        let facets = CatalogFilters::default();
        assert!(query.has_structured_nominal());
        assert!(!SearchQuery::new("bronze").has_structured_nominal());
        assert!(entry_search(&original_entry(&session)).matches(&query, &facets));
        assert!(!definition_search(&short).matches(&query, &facets));
        let loaded = definition_search_in_file(&short, "bronze in the original file name");
        assert!(loaded.matches(&query, &facets));
        assert!(!loaded.matches(&SearchQuery::new("M3 brass"), &facets));
        let replaced = definition_search_in_file(&replacement, "bronze in the original file name");
        assert!(!replaced.matches(&query, &facets));
        assert!(!loaded.matches(&SearchQuery::new("M3 M4 bronze"), &facets));
    }

    #[test]
    fn painted_material_choices_only_offer_explicit_source_values() {
        let mut session =
            Session::new(Document::new("Stainless steel in file name").unwrap()).unwrap();
        add(
            &mut session,
            "Brass source",
            model::default_machine_screw(),
            Some((3., "Brass")),
        );
        let entries = [original_entry(&session)];
        let context = egui::Context::default();
        context.style_mut_of(egui::Theme::Dark, |style| style.animation_time = 0.);
        context.style_mut_of(egui::Theme::Light, |style| style.animation_time = 0.);
        let mut filters = CatalogFilters::default();
        let paint = |filters: &mut CatalogFilters, events| {
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(260., 800.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| filters.show(ui, &entries),
            );
            let text = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some((
                        text.galley.job.text.clone(),
                        text.pos + text.galley.size() / 2.,
                    )),
                    _ => None,
                })
                .collect::<Vec<_>>();
            output.drop_without_applying_deltas();
            text
        };
        let click = |filters: &mut CatalogFilters, point: egui::Pos2| {
            for pressed in [true, false] {
                paint(
                    filters,
                    vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
        };
        let position = |text: &[(String, egui::Pos2)], label: &str| {
            text.iter()
                .find(|(s, _)| s == label)
                .unwrap_or_else(|| panic!("Missing {label}"))
                .1
        };
        let initial = paint(&mut filters, vec![]);
        click(&mut filters, position(&initial, "Hardware filters"));
        // Let the disclosure settle before locating the material control.
        for _ in 0..12 {
            paint(&mut filters, vec![]);
        }
        let open = paint(&mut filters, vec![]);
        let material_y = position(&open, "Source material").y;
        let material_combo = open
            .iter()
            .filter(|(s, p)| s == "Any" && p.y > material_y)
            .min_by(|(_, a), (_, b)| a.y.total_cmp(&b.y))
            .expect("Material selector")
            .1;
        click(&mut filters, material_combo);
        let popup = paint(&mut filters, vec![]);
        assert!(
            !popup
                .iter()
                .any(|(text, _)| text == "Stainless steel" || text == "bronze")
        );
        click(&mut filters, position(&popup, "Brass"));
        assert!(filters.active());
        assert_eq!(
            filters.selected[Facet::Material as usize],
            Some(FacetValue::Text("Brass".into()))
        );
    }
}

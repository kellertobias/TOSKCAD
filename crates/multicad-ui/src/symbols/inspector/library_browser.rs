//! Scrollable grid for bundled, listed, loaded, and embedded definitions.
use super::{
    Catalogue, Form, Next, SymbolAction, definition_input_summary, focus_button, summary_details,
    transition,
};
use crate::{
    egui,
    library::facets::{CatalogFilters, SearchQuery},
};
use multicad_core::{commands::Session, document::EntityId, symbols};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum LibraryItem {
    Loaded(usize),
    Embedded(usize),
    Installed(usize),
}

#[derive(Default)]
pub(in crate::symbols) struct LibraryBrowser {
    selected: Option<LibraryItem>,
    focused_search: bool,
    filter: String,
    facets: Option<CatalogFilters>,
    results: Vec<LibraryItem>,
    scroll_to_top: bool,
}

impl LibraryBrowser {
    pub(in crate::symbols) fn reset_catalogue(&mut self) {
        self.selected = None;
        self.facets = None;
        self.scroll_to_top = true;
    }

    fn update_results(&mut self, catalogue: &Catalogue<'_>, query: &SearchQuery) {
        let facets = catalogue.installed.facets();
        if self.facets.as_ref() == Some(facets) {
            return;
        }
        self.results.clear();
        self.results
            .extend(
                catalogue
                    .library_search
                    .iter()
                    .enumerate()
                    .filter_map(|(index, search)| {
                        search
                            .matches(query, facets)
                            .then_some(LibraryItem::Loaded(index))
                    }),
            );
        self.results
            .extend(
                catalogue
                    .embedded
                    .iter()
                    .enumerate()
                    .filter_map(|(index, (id, _))| {
                        catalogue.embedded_search[id]
                            .matches(query, facets)
                            .then_some(LibraryItem::Embedded(index))
                    }),
            );
        self.results.extend(
            catalogue
                .installed
                .matching(query)
                .map(LibraryItem::Installed),
        );
        self.facets = Some(facets.clone());
        self.selected = self
            .results
            .iter()
            .copied()
            .find(|item| catalogue.tile(*item, query).unavailable.is_none())
            .or_else(|| self.results.first().copied());
        self.scroll_to_top = true;
    }
}

enum LibraryAction {
    PlaceLoaded(usize),
    Import(usize),
    PlaceEmbedded(EntityId),
    Update(EntityId),
    Request(SymbolAction),
}

impl LibraryAction {
    fn choose(
        self,
        catalogue: &Catalogue<'_>,
        action: &mut Option<SymbolAction>,
        next: &mut Option<Next>,
    ) {
        *next = match self {
            Self::PlaceLoaded(index) => Some(Next::PlaceLibrary(Box::new(
                catalogue.library[index].clone(),
            ))),
            Self::Import(index) => Some(Next::Import(Box::new(catalogue.library[index].clone()))),
            Self::PlaceEmbedded(id) => Some(Next::Place(id)),
            Self::Update(id) => Some(Next::Update(id)),
            Self::Request(request) => {
                *action = Some(request);
                None
            }
        };
    }
}

struct LibraryTile<'a> {
    name: &'a str,
    source: &'static str,
    unavailable: Option<&'a str>,
    primary: (&'static str, LibraryAction),
    secondary: Option<(&'static str, LibraryAction)>,
}

impl Catalogue<'_> {
    fn tile_id(&self, item: LibraryItem) -> egui::Id {
        match item {
            LibraryItem::Loaded(index) => egui::Id::new(("loaded", self.library_pins[index])),
            LibraryItem::Embedded(index) => egui::Id::new(("embedded", self.embedded[index].0)),
            LibraryItem::Installed(index) => {
                egui::Id::new(("installed", self.installed.entry(index).id))
            }
        }
    }

    fn tile(&self, item: LibraryItem, query: &SearchQuery) -> LibraryTile<'_> {
        match item {
            LibraryItem::Loaded(index) => {
                let definition = &self.library[index];
                let pin = self.library_pins[index];
                let mut has_revision = false;
                let embedded = self.embedded.iter().any(|(id, existing)| {
                    if existing.symbol != definition.symbol {
                        return false;
                    }
                    has_revision = true;
                    self.embedded_pins.get(id) == Some(&pin)
                });
                let unavailable = if crate::symbols::architecture::role(definition).is_some() {
                    Some(
                        "Architectural construction needs a path and level. Import explicitly, then Draw wall or Draw fence.",
                    )
                } else if has_revision && !embedded {
                    Some(
                        "Another revision is embedded. Compare and update its pin before placing this revision.",
                    )
                } else {
                    None
                };
                LibraryTile {
                    name: &definition.name,
                    source: "Loaded definition",
                    unavailable,
                    primary: ("Place", LibraryAction::PlaceLoaded(index)),
                    secondary: (!embedded)
                        .then_some(("Import definition", LibraryAction::Import(index))),
                }
            }
            LibraryItem::Embedded(index) => {
                let (id, definition) = &self.embedded[index];
                let primary = match crate::symbols::architecture::role(definition) {
                    Some(role) => (
                        match role {
                            symbols::architecture::ArchitectureRole::Wall => "Draw wall",
                            symbols::architecture::ArchitectureRole::Fence => "Draw fence",
                        },
                        LibraryAction::Request(SymbolAction::DrawConstruction {
                            definition: *id,
                            role,
                        }),
                    ),
                    None => ("Place", LibraryAction::PlaceEmbedded(*id)),
                };
                LibraryTile {
                    name: &definition.name,
                    source: "In this project",
                    unavailable: None,
                    primary,
                    secondary: Some(("Compare update", LibraryAction::Update(*id))),
                }
            }
            LibraryItem::Installed(index) => {
                let entry = self.installed.entry(index);
                let exact = self.installed.facets().active() || query.has_structured_nominal();
                let place = if exact {
                    (
                        "Choose matching definition",
                        SymbolAction::OpenInstalled(entry.path.clone()),
                    )
                } else {
                    (
                        "Place",
                        SymbolAction::OpenInstalledForPlace(entry.path.clone()),
                    )
                };
                LibraryTile {
                    name: &entry.name,
                    source: match entry.origin {
                        crate::library::LibraryOrigin::Bundled => "Included with ToskCAD",
                        crate::library::LibraryOrigin::ListedOriginal => "Listed project",
                    },
                    unavailable: entry.problem.as_deref().or_else(|| (entry.symbol_definitions == 0)
                        .then_some("No reusable definitions. Open this file and use Create symbol before placing it here.")),
                    primary: (place.0, LibraryAction::Request(place.1)),
                    secondary: (!exact).then(|| (
                        "Load definitions",
                        LibraryAction::Request(SymbolAction::OpenInstalled(entry.path.clone())),
                    )),
                }
            }
        }
    }

    fn show_details(
        &self,
        ui: &mut egui::Ui,
        item: LibraryItem,
        query: &SearchQuery,
    ) -> Option<SymbolAction> {
        let tile = self.tile(item, query);
        ui.strong(tile.name);
        if let Some(problem) = tile.unavailable {
            ui.colored_label(egui::Color32::LIGHT_RED, problem);
        }
        match item {
            LibraryItem::Installed(index) => {
                let entry = self.installed.entry(index);
                if !entry.description.is_empty() {
                    ui.label(&entry.description);
                }
                let path = entry.path.to_string_lossy();
                ui.add(egui::Label::new(egui::RichText::new(path.as_ref()).weak()).truncate())
                    .on_hover_text(path);
                self.installed.show_details(ui, index, query);
            }
            LibraryItem::Loaded(index) => {
                summary_details(ui, &self.library[index]);
                crate::symbols::installed::show_cards(
                    ui,
                    &self.library_search[index],
                    query,
                    self.installed.facets(),
                );
            }
            LibraryItem::Embedded(index) => {
                let (id, definition) = &self.embedded[index];
                if focus_button(ui.small_button("Save definition…")) {
                    return Some(SymbolAction::SaveDefinition(*id));
                }
                summary_details(ui, definition);
                crate::symbols::installed::show_cards(
                    ui,
                    &self.embedded_search[id],
                    query,
                    self.installed.facets(),
                );
            }
        }
        None
    }
}

fn library_tile(
    ui: &mut egui::Ui,
    catalogue: &mut Catalogue<'_>,
    item: LibraryItem,
    query: &SearchQuery,
    width: f32,
    selected: &mut Option<LibraryItem>,
) -> Option<LibraryAction> {
    use crate::library::{project_card_frame, project_card_preview};
    let preview = project_card_preview(width);
    let mut choice = None;
    let card = project_card_frame().show(ui, |ui| {
        ui.set_width(preview.x);
        ui.spacing_mut().item_spacing.y = 3.;
        match item {
            LibraryItem::Installed(index) => catalogue.installed.show_preview(ui, index, preview),
            _ => crate::library::paint_preview_placeholder(ui, preview, "Symbol definition"),
        }
        let tile = catalogue.tile(item, query);
        let title = ui.add(
            egui::Label::new(egui::RichText::new(tile.name).strong())
                .truncate()
                .sense(egui::Sense::click()),
        );
        let title = if let Some(reason) = tile.unavailable {
            title.on_hover_text(reason)
        } else {
            title
        };
        if title.clicked() || title.has_focus() {
            *selected = Some(item);
        }
        ui.add(egui::Label::new(egui::RichText::new(tile.source).small().weak()).truncate());
        let summary = match item {
            LibraryItem::Loaded(index) => definition_input_summary(&catalogue.library[index]),
            LibraryItem::Embedded(index) => definition_input_summary(&catalogue.embedded[index].1),
            LibraryItem::Installed(index) => {
                let entry = catalogue.installed.entry(index);
                if tile.unavailable.is_some() {
                    "Unavailable".into()
                } else if entry.symbol_definitions == 1 {
                    "1 reusable definition".into()
                } else {
                    format!("{} reusable definitions", entry.symbol_definitions)
                }
            }
        };
        ui.add(egui::Label::new(egui::RichText::new(summary).small()).truncate());
        ui.horizontal(|ui| {
            let place = ui.add_enabled(
                tile.unavailable.is_none(),
                egui::Button::new(tile.primary.0),
            );
            if focus_button(place) {
                choice = Some(tile.primary.1);
            }
            if let Some((label, action)) = tile.secondary
                && focus_button(ui.add_enabled(
                    tile.unavailable.is_none() || matches!(item, LibraryItem::Loaded(_)),
                    egui::Button::new(label),
                ))
            {
                choice = Some(action);
            }
        });
    });
    if *selected == Some(item) {
        ui.painter().rect_stroke(
            card.response.rect,
            5,
            egui::Stroke::new(1., egui::Color32::from_rgb(103, 150, 207)),
            egui::StrokeKind::Inside,
        );
    }
    choice
}

pub(in crate::symbols) fn show_library(
    form: &mut Form,
    browser: &mut LibraryBrowser,
    ui: &mut egui::Ui,
    session: &Session,
    mut catalogue: Catalogue<'_>,
    enabled: bool,
    failure: Option<&str>,
) -> (Option<SymbolAction>, Option<String>, bool) {
    use crate::library::{
        PROJECT_CARD_GAP, PROJECT_CARD_PADDING, project_card_columns, project_card_preview,
    };
    let close = ui
        .horizontal(|ui| {
            ui.heading("Library");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.button("Close")
                    .on_hover_text("Close Library (Escape)")
                    .clicked()
            })
            .inner
        })
        .inner;
    if close {
        return (None, None, true);
    }
    let mut action = None;
    let mut next = None;
    ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| {
            let search = ui.add(
                egui::TextEdit::singleline(&mut form.filter)
                    .id_salt("library-search")
                    .hint_text("Search by name, size, or M3")
                    .desired_width((ui.available_width() - 170.).max(70.)),
            );
            if !browser.focused_search {
                search.request_focus();
                browser.focused_search = true;
            }
            if ui.button("Open library file…").clicked() {
                action = Some(SymbolAction::ImportLibrary);
            }
        });
    });
    if browser.filter != form.filter {
        browser.filter.clone_from(&form.filter);
        browser.reset_catalogue();
    }
    let query = SearchQuery::new(&form.filter);
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt("library-catalogue")
        .max_height((ui.available_height() - 48.).max(35.))
        .auto_shrink([false, false]);
    if std::mem::take(&mut browser.scroll_to_top) {
        scroll = scroll.vertical_scroll_offset(0.);
    }
    scroll.show(ui, |ui| {
        if let Some(failure) = failure {
            ui.colored_label(egui::Color32::LIGHT_RED, failure);
        }
        ui.add_enabled_ui(enabled, |ui| {
            catalogue.installed.show_filters(ui);
            browser.update_results(&catalogue, &query);
            if browser.results.is_empty() {
                ui.weak(
                    "No Library items match. Try another search or clear the hardware filters.",
                );
                return;
            }

            let (columns, width) = project_card_columns(ui.available_width());
            let row_height = project_card_preview(width).y + 2. * PROJECT_CARD_PADDING + 84.;
            let pitch = row_height + PROJECT_CARD_GAP;
            let rows = browser.results.len().div_ceil(columns);
            let grid_top = ui.cursor().top();
            let visible = ui.clip_rect();
            let first = (((visible.min.y - grid_top) / pitch).floor() as usize).min(rows);
            let end = (((visible.max.y - grid_top) / pitch).ceil() as usize).min(rows);
            let grid_size =
                egui::vec2(ui.available_width(), rows as f32 * pitch - PROJECT_CARD_GAP);
            let (_, grid) = ui.allocate_space(grid_size);
            if first < end {
                {
                    let ui = &mut ui.new_child(egui::UiBuilder::new().max_rect(grid));
                    ui.spacing_mut().item_spacing = egui::Vec2::splat(PROJECT_CARD_GAP);
                    ui.add_space(first as f32 * pitch);
                    for row in first..end {
                        let start = row * columns;
                        let items =
                            &browser.results[start..(start + columns).min(browser.results.len())];
                        ui.allocate_ui_with_layout(
                            egui::vec2(grid.width(), row_height),
                            egui::Layout::left_to_right(egui::Align::Min),
                            |ui| {
                                for &item in items {
                                    ui.scope_builder(
                                        egui::UiBuilder::new()
                                            .id_salt(catalogue.tile_id(item))
                                            .layout(egui::Layout::top_down(egui::Align::Min)),
                                        |ui| {
                                            if let Some(choice) = library_tile(
                                                ui,
                                                &mut catalogue,
                                                item,
                                                &query,
                                                width,
                                                &mut browser.selected,
                                            ) {
                                                choice.choose(&catalogue, &mut action, &mut next);
                                            }
                                        },
                                    );
                                }
                            },
                        );
                    }
                }
            }

            if let Some(item) = browser.selected
                && browser.results.len() <= columns
                && ui.cursor().top() <= ui.clip_rect().bottom()
            {
                ui.separator();
                action = catalogue.show_details(ui, item, &query).or(action.take());
            }
        });
    });

    ui.separator();
    ui.horizontal(|ui| {
        let tile = browser.selected.map(|item| catalogue.tile(item, &query));
        let label = match tile.as_ref().map(|tile| tile.primary.0) {
            Some("Choose matching definition") => "Choose matching definition",
            Some("Draw wall" | "Draw fence") => "Draw selected",
            _ => "Place selected",
        };
        let place = ui.add_enabled(
            enabled && tile.as_ref().is_some_and(|tile| tile.unavailable.is_none()),
            egui::Button::new(label)
                .fill(ui.visuals().selection.bg_fill)
                .min_size(egui::vec2(115., 30.)),
        );
        if let (Some(tile), Some(item)) = (tile, browser.selected) {
            let details = ui.button("Details");
            egui::Popup::from_toggle_button_response(&details)
                .align(egui::RectAlign::TOP_START)
                .show(|ui| {
                    ui.set_width(320.);
                    egui::ScrollArea::vertical()
                        .max_height(260.)
                        .show(ui, |ui| {
                            action = catalogue.show_details(ui, item, &query).or(action.take());
                        });
                });
            ui.add(egui::Label::new(tile.name).truncate());
            if place.clicked() {
                tile.primary.1.choose(&catalogue, &mut action, &mut next);
            }
        }
    });
    let error = next.and_then(|next| transition(form, session.document(), next).err());
    (action, error, false)
}

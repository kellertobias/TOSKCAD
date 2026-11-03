//! Cached project-wide drawing navigation; selection never edits source.
use super::*;
use multicad_core::commands::SessionStamp;
#[cfg(test)]
mod tests;

struct SheetEntry {
    id: EntityId,
    number: String,
    title: String,
}
struct BookEntry {
    id: EntityId,
    name: String,
    pages: Vec<SheetEntry>,
    error: Option<String>,
}
#[derive(Default)]
pub(super) struct State {
    catalog: Option<(SessionStamp, Vec<BookEntry>)>,
}
pub(super) enum Action {
    Select {
        book: EntityId,
        page: Option<EntityId>,
    },
    New,
}
pub(super) fn new_book_name(session: &Session) -> String {
    let names: std::collections::BTreeSet<_> = drawings::books(session.document())
        .filter_map(|id| session.document().get(id).map(|e| e.name.as_str()))
        .collect();
    let mut number = 1;
    loop {
        let name = format!("Drawing {number}");
        if !names.contains(name.as_str()) {
            return name;
        }
        number += 1;
    }
}
impl DrawingEditor {
    pub(super) fn navigation_ready(&self) -> bool {
        !self.dirty()
            && self.placement.is_none()
            && self.section_pick.is_none()
            && !self.schedules.chooser
            && !self.schedules.placing()
    }
    pub fn selected_page(&self) -> Option<multicad_core::exchange::PageRef> {
        Some(multicad_core::exchange::PageRef {
            book: self.book?,
            page: self.selected?,
        })
    }
    pub fn pdf_source_ready(&self) -> bool {
        self.navigation_ready() && self.selected_page().is_some()
    }
    pub fn take_pdf_request(&mut self) -> Option<multicad_core::exchange::PageRef> {
        std::mem::take(&mut self.pdf_requested)
            .then(|| self.selected_page())
            .flatten()
    }
    pub(super) fn select_sheet(
        &mut self,
        session: &Session,
        book: EntityId,
        page: Option<EntityId>,
    ) -> Result<(), String> {
        if self.book == Some(book) && self.selected == page {
            return Ok(());
        }
        if !self.navigation_ready() {
            return Err("Apply/discard page edits or finish/cancel the current sheet placement before switching sheets".into());
        }
        let source = drawings::read(session.document(), book).map_err(|e| e.to_string())?;
        if let Some(page) = page
            && !source.pages.iter().any(|p| p.id == page)
        {
            return Err("That sheet no longer belongs to the selected drawing".into());
        }
        self.feed.clear(); // Retains completed page cache; drops obsolete replies.
        self.book = Some(book);
        self.selected = page;
        self.revision = None;
        self.draft = None;
        self.original = None;
        self.preview_input = None;
        self.selected_view = None;
        self.selected_annotation = None;
        self.section_pick = None;
        self.last_paper = None;
        self.error = None;
        self.schedules = Default::default();
        self.sync(session)?;
        Ok(())
    }
    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui, session: &Session) -> Option<Action> {
        if self
            .sidebar
            .catalog
            .as_ref()
            .is_none_or(|(stamp, _)| session.validate_stamp(stamp).is_err())
        {
            let mut books: Vec<_> = drawings::books(session.document())
                .map(|id| {
                    let name = session
                        .document()
                        .get(id)
                        .map_or("Drawing", |e| e.name.as_str())
                        .to_owned();
                    match drawings::read(session.document(), id) {
                        Ok(book) => BookEntry {
                            id,
                            name,
                            pages: book
                                .pages
                                .into_iter()
                                .map(|p| SheetEntry {
                                    id: p.id,
                                    number: p.number,
                                    title: p.info.title,
                                })
                                .collect(),
                            error: None,
                        },
                        Err(error) => BookEntry {
                            id,
                            name,
                            pages: vec![],
                            error: Some(error.to_string()),
                        },
                    }
                })
                .collect();
            books.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
            self.sidebar.catalog = Some((session.stamp(), books));
        }
        let mut action = None;
        let ready = self.navigation_ready();
        let has_selection = self.selected_page().is_some();
        let mut pdf_requested = false;
        let books = &self.sidebar.catalog.as_ref().expect("current catalog").1;
        egui::Panel::left("drawing-sidebar")
            .default_size(224.).min_size(190.).max_size(340.).resizable(true)
            .frame(egui::Frame::new().fill(crate::theme::PANEL).inner_margin(12))
            .show(ui, |ui| {
                ui.strong("Drawings");
                ui.small(format!("{} drawings · {} sheets", books.len(), books.iter().map(|b| b.pages.len()).sum::<usize>()));
                ui.add_space(6.);
                if ui.add_enabled_ui(has_selection, |ui| crate::icons::labeled_button_sized(
                    ui, Icon::Export, "Export as PDF", egui::vec2(ui.available_width(), 34.)
                )).inner.on_disabled_hover_text("Add or select a drawing sheet first").clicked() {
                    pdf_requested = true;
                }
                if ui.add_enabled_ui(ready, |ui| crate::icons::labeled_button_sized(
                    ui, Icon::New, "New drawing", egui::vec2(ui.available_width(), 34.)
                )).inner.on_disabled_hover_text("Apply/discard page edits or finish/cancel the current placement first").clicked() {
                    action = Some(Action::New);
                }
                ui.separator();
                if !ready { ui.small("Apply/discard edits or finish/cancel the active placement before switching sheets."); }
                egui::ScrollArea::vertical().id_salt("drawing-sheet-list").auto_shrink([false, false]).show(ui, |ui| {
                    if books.is_empty() { ui.label("No drawings yet."); ui.small("Create a drawing to add its first sheet."); }
                    for book in books {
                        let current_book = self.book == Some(book.id);
                        if ui.add_enabled(ready || current_book, egui::Button::selectable(current_book, &book.name).wrap()
                            .min_size(egui::vec2(ui.available_width(), 28.))).clicked() && book.error.is_none() {
                            action = Some(Action::Select { book:book.id, page:if current_book {self.selected} else {book.pages.first().map(|p| p.id)} });
                        }
                        if let Some(error) = &book.error { ui.colored_label(egui::Color32::LIGHT_YELLOW, error); }
                        if book.pages.is_empty() && book.error.is_none() { ui.small("No sheets · select this drawing, then Add page."); }
                        for page in &book.pages {
                            let current = current_book && self.selected == Some(page.id);
                            let draft = current.then_some(self.draft.as_ref()).flatten();
                            let number = draft.map_or(page.number.as_str(), |p| p.number.as_str());
                            let title = draft.map_or(page.title.as_str(), |p| p.info.title.as_str());
                            let title = if title.trim().is_empty() { "Untitled sheet" } else { title };
                            let label = format!("{number} · {title}{}", if current && !ready { " •" } else { "" });
                            if ui.add_enabled(ready || current, egui::Button::selectable(current, &label).wrap()
                                .min_size(egui::vec2(ui.available_width(), 32.))).on_hover_text(&label).clicked() {
                                action = Some(Action::Select { book:book.id, page:Some(page.id) });
                            }
                        }
                        ui.add_space(8.);
                    }
                });
            });
        self.pdf_requested |= pdf_requested;
        action
    }
}

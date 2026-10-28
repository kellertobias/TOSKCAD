//! Page editing owns authored drawing source; it never invents model views.
mod canvas;
mod evaluation;
mod inspector;
mod views;
pub use evaluation::{DrawingBackend, DrawingBackendFactory, DrawingReply};
#[cfg(test)]
mod tests;
use crate::{egui, icons::Icon};
use multicad_core::{
    commands::{Actor, Channel, Session},
    document::EntityId,
    drawings::{self, DrawingBook, Page, PageAction, TextNote},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawingTool {
    NewDocument,
    AddPage,
    Duplicate,
    Remove,
    MoveEarlier,
    MoveLater,
    Settings,
    Apply,
    Discard,
    Text,
    Projection,
    Section,
    Detail,
}
impl DrawingTool {
    pub const ALL: [Self; 13] = [
        Self::NewDocument,
        Self::AddPage,
        Self::Duplicate,
        Self::Remove,
        Self::MoveEarlier,
        Self::MoveLater,
        Self::Settings,
        Self::Apply,
        Self::Discard,
        Self::Text,
        Self::Projection,
        Self::Section,
        Self::Detail,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Self::NewDocument => "drawing-new",
            Self::AddPage => "sheet-page",
            Self::Duplicate => "page-duplicate",
            Self::Remove => "page-remove",
            Self::MoveEarlier => "page-earlier",
            Self::MoveLater => "page-later",
            Self::Settings => "page-setup",
            Self::Apply => "page-apply",
            Self::Discard => "page-discard",
            Self::Text => "drawing-note",
            Self::Projection => "drawing-projection",
            Self::Section => "drawing-section",
            Self::Detail => "drawing-detail",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::NewDocument => "New drawing",
            Self::AddPage => "Add page",
            Self::Duplicate => "Duplicate page",
            Self::Remove => "Remove page",
            Self::MoveEarlier => "Move earlier",
            Self::MoveLater => "Move later",
            Self::Settings => "Page setup",
            Self::Apply => "Apply page",
            Self::Discard => "Discard edits",
            Self::Text => "Text note",
            Self::Projection => "Model view",
            Self::Section => "Section view",
            Self::Detail => "Detail view",
        }
    }
    pub fn icon(self) -> Icon {
        match self {
            Self::NewDocument | Self::AddPage => Icon::New,
            Self::Remove => Icon::Trash,
            Self::MoveEarlier => Icon::ArrowUp,
            Self::MoveLater => Icon::ArrowDown,
            Self::Settings => Icon::Settings,
            Self::Apply => Icon::Save,
            Self::Discard => Icon::Undo,
            Self::Text => Icon::Sketch,
            Self::Duplicate => Icon::Page,
            Self::Projection => Icon::Orthographic,
            Self::Section => Icon::Section,
            Self::Detail => Icon::Fit,
        }
    }
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::NewDocument => &["newdrawing"],
            Self::AddPage => &["newpage"],
            Self::Duplicate => &["duplicatepage"],
            Self::Remove => &["removepage"],
            Self::MoveEarlier => &["pageup"],
            Self::MoveLater => &["pagedown"],
            Self::Settings => &["pagesetup"],
            Self::Apply => &["applypage"],
            Self::Discard => &["discardpage"],
            Self::Text => &["pagenote"],
            Self::Projection => &["drawingview"],
            Self::Section => &["drawingsection"],
            Self::Detail => &["drawingdetail"],
        }
    }
}
#[derive(Default)]
pub struct DrawingEditor {
    book: Option<EntityId>,
    selected: Option<EntityId>,
    draft: Option<Page>,
    original: Option<Page>,
    revision: Option<u64>,
    pub error: Option<String>,
    feed: evaluation::DrawingFeed,
    preview_input: Option<(EntityId, u64, Option<EntityId>, Option<Page>)>,
    selected_view: Option<EntityId>,
}
impl DrawingEditor {
    pub fn dirty(&self) -> bool {
        self.draft != self.original
    }
    pub fn sync(&mut self, session: &Session) -> Result<DrawingBook, String> {
        if self.dirty()
            && self
                .book
                .is_some_and(|id| session.document().get(id).is_none())
        {
            return Err(
                "The drawing source was removed. Discard the retained page draft to continue"
                    .into(),
            );
        }
        if self
            .book
            .is_none_or(|id| session.document().get(id).is_none())
        {
            self.book = drawings::books(session.document()).next();
            self.selected = None;
            self.draft = None;
            self.original = None;
            self.revision = None;
        }
        let book = match self.book {
            Some(id) => drawings::read(session.document(), id).map_err(|e| e.to_string())?,
            None => DrawingBook::default(),
        };
        if !self.dirty() && (self.revision != Some(session.revision()) || self.draft.is_none()) {
            if self
                .selected
                .is_none_or(|id| book.pages.iter().all(|page| page.id != id))
            {
                self.selected = book.pages.first().map(|page| page.id);
            }
            self.draft = book
                .pages
                .iter()
                .find(|page| Some(page.id) == self.selected)
                .cloned();
            self.original = self.draft.clone();
            self.revision = Some(session.revision());
        }
        Ok(book)
    }
    fn validate_draft(&self) -> Result<(), String> {
        DrawingBook {
            pages: vec![self.draft.clone().ok_or("Select a page first")?],
        }
        .validate()
        .map_err(|e| e.to_string())
    }
    pub fn enabled(&self, tool: DrawingTool, session: &Session) -> bool {
        match tool {
            DrawingTool::Apply => {
                self.dirty()
                    && self.revision == Some(session.revision())
                    && self
                        .book
                        .is_some_and(|id| session.document().get(id).is_some())
                    && self.validate_draft().is_ok()
                    && self.changed_views_ready(session)
            }
            DrawingTool::Discard => self.dirty(),
            DrawingTool::Settings => self.selected.is_some(),
            DrawingTool::Text => self.selected.is_some(),
            DrawingTool::Projection | DrawingTool::Section | DrawingTool::Detail => {
                self.selected.is_some() && self.has_backend_factory()
            }
            DrawingTool::AddPage | DrawingTool::NewDocument => !self.dirty(),
            _ => self.selected.is_some() && !self.dirty(),
        }
    }
    pub fn invoke(
        &mut self,
        session: &mut Session,
        tool: DrawingTool,
        channel: Channel,
    ) -> Result<(), String> {
        if tool == DrawingTool::Discard {
            self.draft = self.original.clone();
            if self
                .book
                .is_some_and(|id| session.document().get(id).is_none())
            {
                self.draft = None;
                self.original = None;
            }
            self.revision = None;
            self.error = None;
            self.sync(session)?;
            return Ok(());
        }
        let book = self.sync(session)?;
        let actor = Actor::local(channel);
        if tool == DrawingTool::Settings {
            self.selected_view = None;
            return Ok(());
        }
        let view_kind = match tool {
            DrawingTool::Projection => Some(drawings::ViewKind::Projection),
            DrawingTool::Section => Some(drawings::ViewKind::Section { offset_mm: 10. }),
            DrawingTool::Detail => Some(drawings::ViewKind::Detail {
                center_mm: [0.; 2],
                radius_mm: 25.,
            }),
            _ => None,
        };
        if let Some(kind) = view_kind {
            if !self.has_backend_factory() {
                return Err("Native drawing provider is unavailable".into());
            }
            return self.add_view(kind);
        }
        if tool == DrawingTool::Text {
            let page = self.draft.as_mut().ok_or("Add a page first")?;
            let margin = page.settings.margin_mm;
            page.notes.push(TextNote {
                id: EntityId::new(),
                position_mm: [margin + 5., margin + 5.],
                text: String::new(),
            });
            return Ok(());
        }
        if tool == DrawingTool::Apply {
            if !self.changed_views_ready(session) {
                return Err("Generate a successful current native preview for the changed drawing views before Apply".into());
            }
            if !self.dirty() {
                return Err("There are no unapplied page edits".into());
            }
            let page = self.draft.clone().ok_or("Select a page first")?;
            let id = self.book.ok_or("Create a drawing first")?;
            let prepared = drawings::prepare(
                session,
                self.revision.ok_or("Reload this page")?,
                actor,
                id,
                PageAction::Replace(page),
            )
            .map_err(|e| e.to_string())?;
            session.commit(prepared).map_err(|e| e.to_string())?;
            self.original = self.draft.clone();
            self.revision = None;
            self.sync(session)?;
            self.error = None;
            return Ok(());
        }
        if self.dirty() {
            return Err("Apply or discard page edits first".into());
        }
        if tool == DrawingTool::NewDocument || (tool == DrawingTool::AddPage && self.book.is_none())
        {
            let page = Page::new("1");
            self.selected = Some(page.id);
            let (id, prepared) = drawings::prepare_create(
                session,
                session.revision(),
                actor,
                "Drawing document",
                Some(page),
            )
            .map_err(|e| e.to_string())?;
            session.commit(prepared).map_err(|e| e.to_string())?;
            self.book = Some(id);
        } else {
            let selected = self.selected;
            let action = match tool {
                DrawingTool::AddPage => {
                    let page = Page::new(drawings::next_number(&book));
                    self.selected = Some(page.id);
                    PageAction::Add(page)
                }
                DrawingTool::Duplicate => {
                    let id = EntityId::new();
                    self.selected = Some(id);
                    PageAction::Duplicate {
                        page: selected.ok_or("Select a page")?,
                        new_page: id,
                    }
                }
                DrawingTool::Remove => PageAction::Remove(selected.ok_or("Select a page")?),
                DrawingTool::MoveEarlier | DrawingTool::MoveLater => {
                    let selected = selected.ok_or("Select a page")?;
                    let index = book
                        .pages
                        .iter()
                        .position(|page| page.id == selected)
                        .ok_or("Page is missing")?;
                    let before = if tool == DrawingTool::MoveEarlier {
                        if index == 0 {
                            return Ok(());
                        }
                        Some(book.pages[index - 1].id)
                    } else {
                        if index + 1 == book.pages.len() {
                            return Ok(());
                        }
                        book.pages.get(index + 2).map(|page| page.id)
                    };
                    PageAction::Move {
                        page: selected,
                        before,
                    }
                }
                _ => return Ok(()),
            };
            let prepared = drawings::prepare(
                session,
                session.revision(),
                actor,
                self.book.ok_or("Create a drawing first")?,
                action,
            )
            .map_err(|e| e.to_string())?;
            session.commit(prepared).map_err(|e| e.to_string())?;
        }
        self.revision = None;
        self.sync(session)?;
        self.error = None;
        Ok(())
    }
    fn select(&mut self, session: &Session, id: EntityId) -> Result<(), String> {
        if self.dirty() {
            return Err("Apply or discard page edits before switching pages".into());
        }
        self.selected = Some(id);
        self.revision = None;
        self.sync(session)?;
        Ok(())
    }
    pub fn show(&mut self, ui: &mut egui::Ui, session: &mut Session) {
        let book = match self.sync(session) {
            Ok(book) => book,
            Err(error) => {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
                return;
            }
        };
        let mut event = None;
        egui::Panel::bottom("drawing-page-strip")
            .default_size(38.)
            .show(ui, |ui| {
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (index, page) in book.pages.iter().enumerate() {
                            if ui
                                .add_enabled(
                                    !self.dirty(),
                                    egui::Button::new(format!("{} · {}", index + 1, page.number))
                                        .selected(self.selected == Some(page.id)),
                                )
                                .on_hover_text(if page.info.title.is_empty() {
                                    "Untitled page"
                                } else {
                                    &page.info.title
                                })
                                .clicked()
                            {
                                event = Some(page.id);
                            }
                        }
                    });
                });
            });
        if let Some(id) = event
            && let Err(error) = self.select(session, id)
        {
            self.error = Some(error);
        }
        egui::Panel::right("drawing-inspector")
            .default_size(270.)
            .min_size(190.)
            .resizable(true)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.inspector(ui, session);
                });
            });
        self.tick_views(ui.ctx(), session);
        let mut view_selected = None;
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(page) = &self.draft {
                view_selected = canvas::show(
                    ui,
                    page,
                    book.pages
                        .iter()
                        .position(|p| Some(p.id) == self.selected)
                        .unwrap_or(0)
                        + 1,
                    book.pages.len(),
                    if self.preview_matches(session) {
                        self.feed.result.as_deref()
                    } else {
                        None
                    },
                    self.selected_view,
                );
            } else {
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.heading("No drawing pages");
                        ui.label(
                            "Add paper to configure a drawing. Model views are added separately.",
                        );
                        if ui.button("Add page").clicked()
                            && let Err(error) =
                                self.invoke(session, DrawingTool::AddPage, Channel::Ui)
                        {
                            self.error = Some(error);
                        }
                    });
                });
            }
        });
        if let Some(id) = view_selected {
            self.selected_view = Some(id);
        }
    }
}

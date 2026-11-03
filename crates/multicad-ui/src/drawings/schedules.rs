//! Schedule tables on drawing pages. Placing and refreshing commit the whole
//! page as one undoable edit; a placed table keeps a copy of the cells and is
//! checked against a fresh native schedule of the current source.
use super::*;
use crate::schedules::{ScheduleReply, ScheduleTable};
use multicad_core::{
    drawings::{Annotation, AnnotationKind, DrawingStamp},
    project_file::ContentHash,
    schedules::{ScheduleKind, ScheduleSpec},
};
use std::collections::{BTreeMap, BTreeSet};

/// How a placed schedule table compares with the current source.
#[derive(Clone, Debug, PartialEq)]
pub enum ScheduleStatus {
    /// Not a table placed from a schedule.
    Unlinked,
    Checking,
    Current,
    Stale(String),
    Failed(String),
}

type Tables = BTreeMap<ScheduleSpec, Result<ScheduleTable, String>>;
pub(super) struct ScheduleState {
    /// The schedule the chooser places.
    pub choice: ScheduleSpec,
    /// The chooser is open, so its schedule is kept current.
    pub chooser: bool,
    requested: Option<(EntityId, u64, u64)>,
    expected: Option<DrawingStamp>,
    current: Option<(DrawingStamp, Tables)>,
    /// A placement waiting for its schedule: page and schedule.
    place: Option<(EntityId, ScheduleSpec)>,
    pub error: Option<String>,
}
impl Default for ScheduleState {
    fn default() -> Self {
        Self {
            choice: ScheduleSpec::new(ScheduleKind::Bom),
            chooser: false,
            requested: None,
            expected: None,
            current: None,
            place: None,
            error: None,
        }
    }
}
impl ScheduleState {
    pub(super) fn placing(&self) -> bool {
        self.place.is_some()
    }
}

impl DrawingEditor {
    /// Schedules the current page and the chooser need.
    fn needed_schedules(&self) -> BTreeSet<ScheduleSpec> {
        let mut specs: BTreeSet<_> = self
            .draft
            .iter()
            .flat_map(|p| &p.annotations)
            .filter_map(|a| match &a.kind {
                AnnotationKind::Table {
                    schedule: Some(spec),
                    ..
                } => Some(spec.normalized()),
                _ => None,
            })
            .collect();
        if self.schedules.chooser {
            specs.insert(self.schedules.choice);
        }
        if let Some((_, spec)) = self.schedules.place {
            specs.insert(spec);
        }
        specs
    }
    /// The fresh schedule for the session's current revision, if built.
    fn fresh_schedule(
        &self,
        session: &Session,
        spec: ScheduleSpec,
    ) -> Option<&Result<ScheduleTable, String>> {
        let (stamp, tables) = self.schedules.current.as_ref()?;
        (stamp.document == session.document().id() && stamp.revision == session.revision())
            .then(|| tables.get(&spec.normalized()))
            .flatten()
    }
    /// Request schedules for a new revision or set of tables, accept replies
    /// for the current request only, and finish a waiting placement.
    pub fn tick_schedules(&mut self, session: &mut Session) {
        let specs = self.needed_schedules();
        let key = (
            session.document().id(),
            session.revision(),
            ScheduleSpec::mask(specs.iter().copied()),
        );
        let mut replies = vec![];
        if let Some(backend) = self.feed.backend() {
            while let Some(reply) = backend.poll_schedules() {
                replies.push(reply);
            }
        }
        for reply in replies {
            self.accept_schedules(reply);
        }
        if !specs.is_empty() && self.schedules.requested != Some(key) {
            self.schedules.requested = Some(key);
            let result = ContentHash::document(session.document()).and_then(|source_hash| {
                let stamp = DrawingStamp {
                    document: session.document().id(),
                    revision: session.revision(),
                    source_hash,
                };
                let specs: Vec<_> = specs.iter().copied().collect();
                let backend = self
                    .feed
                    .backend()
                    .ok_or_else(|| "Native schedules are unavailable".to_string())?;
                backend.request_schedules(session.document(), stamp, &specs)?;
                Ok(stamp)
            });
            match result {
                Ok(stamp) => self.schedules.expected = Some(stamp),
                Err(error) => {
                    self.schedules.expected = None;
                    self.schedules.error = Some(error);
                }
            }
        }
        if let Some((page, spec)) = self.schedules.place
            && self.fresh_schedule(session, spec).is_some()
        {
            self.schedules.place = None;
            let result = if self.selected == Some(page) {
                self.place_schedule(session, spec, Channel::Ui).map(|_| ())
            } else {
                Err("The page changed before the schedule was built; place it again".into())
            };
            if let Err(error) = result {
                self.schedules.error = Some(error);
            }
        }
    }
    fn accept_schedules(&mut self, reply: ScheduleReply) {
        if Some(reply.stamp) != self.schedules.expected {
            return;
        }
        let tables = reply
            .tables
            .into_iter()
            .map(|(spec, result)| {
                let result = result.and_then(|t| {
                    if t.spec == spec && t.revision == reply.stamp.revision {
                        Ok(t)
                    } else {
                        Err("Native schedule does not match its request".into())
                    }
                });
                (spec, result)
            })
            .collect();
        self.schedules.current = Some((reply.stamp, tables));
        self.schedules.error = None;
    }
    /// Whether a placed table still shows its schedule's current cells.
    pub fn schedule_status(&self, session: &Session, annotation: EntityId) -> ScheduleStatus {
        let Some(kind) = self
            .draft
            .as_ref()
            .and_then(|p| p.annotations.iter().find(|a| a.id == annotation))
            .map(|a| &a.kind)
        else {
            return ScheduleStatus::Unlinked;
        };
        let AnnotationKind::Table {
            schedule: Some(spec),
            title,
            ..
        } = kind
        else {
            return ScheduleStatus::Unlinked;
        };
        match self.fresh_schedule(session, *spec) {
            None => ScheduleStatus::Checking,
            Some(Err(error)) => ScheduleStatus::Failed(error.clone()),
            Some(Ok(fresh)) if fresh.matches(kind) => ScheduleStatus::Current,
            Some(Ok(fresh)) => ScheduleStatus::Stale(format!(
                "The table \"{title}\" is stale: the {} at revision {} differs",
                spec.label(),
                fresh.revision
            )),
        }
    }
    fn commit_page(
        &mut self,
        session: &mut Session,
        channel: Channel,
        edit: impl FnOnce(&mut Page) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.dirty() {
            return Err("Apply or discard page edits first".into());
        }
        let book = self.book.ok_or("Create a drawing first")?;
        let selected = self.selected.ok_or("Select a page first")?;
        let mut page = drawings::read(session.document(), book)
            .map_err(|e| e.to_string())?
            .pages
            .into_iter()
            .find(|p| p.id == selected)
            .ok_or("The page is missing")?;
        edit(&mut page)?;
        DrawingBook {
            pages: vec![page.clone()],
        }
        .validate()
        .map_err(|e| e.to_string())?;
        let prepared = drawings::prepare(
            session,
            session.revision(),
            Actor::local(channel),
            book,
            PageAction::Replace(page),
        )
        .map_err(|e| e.to_string())?;
        session.commit(prepared).map_err(|e| e.to_string())?;
        self.revision = None;
        self.sync(session)?;
        self.error = None;
        Ok(())
    }
    /// Place a schedule as a linked table near the page's top-left margin,
    /// as one undoable page edit. Without a current schedule yet, the
    /// placement waits for it.
    pub fn place_schedule(
        &mut self,
        session: &mut Session,
        spec: ScheduleSpec,
        channel: Channel,
    ) -> Result<String, String> {
        let spec = spec.normalized();
        if self.dirty() {
            return Err("Apply or discard page edits first".into());
        }
        let page = self.selected.ok_or("Select a page first")?;
        if !self.has_backend_factory() {
            return Err("Native schedules are unavailable".into());
        }
        let fresh = match self.fresh_schedule(session, spec) {
            Some(Ok(fresh)) => fresh.clone(),
            Some(Err(error)) => {
                return Err(format!("The {} cannot be built: {error}", spec.label()));
            }
            None => {
                self.schedules.place = Some((page, spec));
                return Ok(format!("Building the {}…", spec.label()));
            }
        };
        let id = EntityId::new();
        self.commit_page(session, channel, |page| {
            let margin = page.settings.margin_mm;
            let mut kind = fresh.table.clone();
            if let AnnotationKind::Table { position_mm, .. } = &mut kind {
                *position_mm = [margin + 5., margin + 5.];
            }
            page.annotations.push(Annotation { id, kind });
            Ok(())
        })
        .map_err(|e| format!("The {} cannot be placed: {e}", spec.label()))?;
        self.selected_annotation = Some(id);
        self.selected_view = None;
        Ok(format!(
            "{} placed · revision {} · one Undo step",
            spec.label(),
            fresh.revision
        ))
    }
    /// Replace a stale table's cells with the current schedule, keeping its
    /// identity and position, as one undoable page edit.
    pub fn refresh_schedule_table(
        &mut self,
        session: &mut Session,
        annotation: EntityId,
        channel: Channel,
    ) -> Result<String, String> {
        let placed = self
            .draft
            .as_ref()
            .and_then(|p| p.annotations.iter().find(|a| a.id == annotation))
            .map(|a| a.kind.clone())
            .ok_or("The table is missing")?;
        let AnnotationKind::Table {
            schedule: Some(spec),
            ..
        } = placed
        else {
            return Err("This table was not placed from a schedule".into());
        };
        let fresh = match self.fresh_schedule(session, spec) {
            Some(Ok(fresh)) => fresh.clone(),
            Some(Err(error)) => return Err(error.clone()),
            None => return Err("The current schedule is still being built".into()),
        };
        if fresh.matches(&placed) {
            return Ok("The table is current".into());
        }
        self.commit_page(session, channel, |page| {
            let target = page
                .annotations
                .iter_mut()
                .find(|a| a.id == annotation)
                .ok_or("The table is missing")?;
            target.kind = fresh.placed_at(&target.kind);
            Ok(())
        })?;
        Ok(format!(
            "Table refreshed to revision {} · one Undo step",
            fresh.revision
        ))
    }
    /// The chooser, the selected table's status with Refresh, and a page
    /// summary of stale tables.
    pub(super) fn schedule_inspector(&mut self, ui: &mut egui::Ui, session: &mut Session) {
        if self.selected.is_none() {
            return;
        }
        let linked: Vec<EntityId> = self
            .draft
            .iter()
            .flat_map(|p| &p.annotations)
            .filter(|a| {
                matches!(
                    a.kind,
                    AnnotationKind::Table {
                        schedule: Some(_),
                        ..
                    }
                )
            })
            .map(|a| a.id)
            .collect();
        if !self.schedules.chooser && linked.is_empty() {
            return;
        }
        ui.separator();
        ui.strong("Schedule tables");
        let mut action: Option<Result<String, String>> = None;
        if self.schedules.chooser {
            let mut choice = self.schedules.choice;
            crate::schedules::spec_controls(ui, "drawing-schedule-kind", &mut choice);
            self.schedules.choice = choice;
            let status = match self.fresh_schedule(session, choice) {
                Some(Ok(fresh)) => match &fresh.table {
                    AnnotationKind::Table { rows, .. } => {
                        format!("{} rows at revision {}", rows.len(), fresh.revision)
                    }
                    _ => String::new(),
                },
                Some(Err(error)) => error.clone(),
                None => "Building the current schedule…".into(),
            };
            ui.small(status);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !self.dirty() && self.schedules.place.is_none(),
                        egui::Button::new("Place on page"),
                    )
                    .on_hover_text("Place the schedule as a table on this page; one Undo step")
                    .clicked()
                {
                    action = Some(self.place_schedule(session, choice, Channel::Ui));
                }
                if ui.button("Close").clicked() {
                    self.schedules.chooser = false;
                }
            });
        }
        let stale: Vec<_> = linked
            .iter()
            .filter(|id| {
                matches!(
                    self.schedule_status(session, **id),
                    ScheduleStatus::Stale(_)
                )
            })
            .copied()
            .collect();
        if !stale.is_empty() {
            ui.colored_label(
                egui::Color32::LIGHT_YELLOW,
                format!(
                    "{} schedule table(s) on this page are stale; PDF and DXF export is blocked until they are refreshed",
                    stale.len()
                ),
            );
        }
        if let Some(id) = self.selected_annotation.filter(|id| linked.contains(id)) {
            match self.schedule_status(session, id) {
                ScheduleStatus::Current => {
                    ui.small("The selected table shows the current schedule");
                }
                ScheduleStatus::Checking => {
                    ui.small("Checking the selected table…");
                }
                ScheduleStatus::Stale(message) => {
                    ui.colored_label(egui::Color32::LIGHT_YELLOW, message);
                }
                ScheduleStatus::Failed(message) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, message);
                }
                ScheduleStatus::Unlinked => {}
            }
            if ui
                .add_enabled(
                    !self.dirty()
                        && matches!(self.schedule_status(session, id), ScheduleStatus::Stale(_)),
                    egui::Button::new("Refresh table"),
                )
                .on_hover_text("Replace the cells with the current schedule; one Undo step")
                .clicked()
            {
                action = Some(self.refresh_schedule_table(session, id, Channel::Ui));
            }
        }
        if let Some(error) = &self.schedules.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        match action {
            Some(Ok(_)) => self.schedules.error = None,
            Some(Err(error)) => self.schedules.error = Some(error),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::commands::Request;
    use multicad_core::document::{Document, Entity, EntityData, Mutation};
    use std::sync::{Arc, Mutex};

    type Requested = (DrawingStamp, Vec<ScheduleSpec>);
    /// A controlled schedule service: rows come from a shared cell, requests
    /// are recorded and each is answered once on the next poll.
    #[derive(Clone, Default)]
    struct Service {
        rows: Arc<Mutex<Vec<Vec<String>>>>,
        requests: Arc<Mutex<Vec<Requested>>>,
        answered: Arc<Mutex<usize>>,
    }
    impl DrawingBackend for Service {
        fn request(&mut self, _: &Document, _: DrawingStamp) -> Result<(), String> {
            Ok(())
        }
        fn poll(&mut self) -> Option<super::super::DrawingReply> {
            None
        }
        fn request_schedules(
            &mut self,
            document: &Document,
            stamp: DrawingStamp,
            specs: &[ScheduleSpec],
        ) -> Result<(), String> {
            stamp.check(document, stamp.revision)?;
            self.requests.lock().unwrap().push((stamp, specs.to_vec()));
            Ok(())
        }
        fn poll_schedules(&mut self) -> Option<ScheduleReply> {
            let requests = self.requests.lock().unwrap();
            let mut answered = self.answered.lock().unwrap();
            let (stamp, specs) = requests.get(*answered)?.clone();
            *answered += 1;
            let rows = self.rows.lock().unwrap().clone();
            Some(ScheduleReply {
                stamp,
                tables: specs
                    .into_iter()
                    .map(|spec| {
                        let table = AnnotationKind::Table {
                            position_mm: [0.; 2],
                            title: format!("{} (rev {})", spec.kind.label(), stamp.revision),
                            columns: vec!["Part".into(), "Qty".into()],
                            rows: rows.clone(),
                            widths_mm: vec![20., 12.],
                            row_height_mm: 5.,
                            schedule: Some(spec),
                        };
                        (
                            spec,
                            Ok(ScheduleTable {
                                spec,
                                revision: stamp.revision,
                                table,
                            }),
                        )
                    })
                    .collect(),
            })
        }
    }
    fn rows(names: &[&str]) -> Vec<Vec<String>> {
        names
            .iter()
            .map(|n| vec![(*n).to_owned(), "1".into()])
            .collect()
    }
    fn tables(editor: &DrawingEditor) -> Vec<Annotation> {
        editor
            .draft
            .as_ref()
            .unwrap()
            .annotations
            .iter()
            .filter(|a| matches!(a.kind, AnnotationKind::Table { .. }))
            .cloned()
            .collect()
    }
    fn unrelated_edit(session: &mut Session) {
        session
            .apply(Request {
                expected_revision: session.revision(),
                actor: Actor::local(Channel::Mcp),
                label: "Model change".into(),
                edits: vec![Mutation::Insert(Entity::new(
                    "depth",
                    EntityData::Parameter {
                        expression: "5".into(),
                        unit: "mm".into(),
                        exposed: true,
                        inputs: vec![],
                    },
                ))],
            })
            .unwrap();
    }

    #[test]
    fn placing_waits_for_the_schedule_and_commits_one_undoable_page_edit() {
        let mut session = Session::new(Document::new("Schedules").unwrap()).unwrap();
        let mut editor = DrawingEditor::default();
        editor
            .invoke(&mut session, DrawingTool::AddPage, Channel::Ui)
            .unwrap();
        assert!(!editor.enabled(DrawingTool::ScheduleTable, &session));
        assert!(
            editor
                .invoke(&mut session, DrawingTool::ScheduleTable, Channel::Ui)
                .is_err()
        );
        let service = Service::default();
        *service.rows.lock().unwrap() = rows(&["Leg", "Top"]);
        let factory = service.clone();
        editor.set_backend_factory(Arc::new(move || Box::new(factory.clone())));
        assert!(editor.enabled(DrawingTool::ScheduleTable, &session));
        editor
            .invoke(&mut session, DrawingTool::ScheduleTable, Channel::Ui)
            .unwrap();
        assert!(editor.schedules.chooser);
        let before = session.document().clone();
        let revision = session.revision();

        // Without a current schedule the placement waits; nothing changes.
        let spec = ScheduleSpec::new(ScheduleKind::Bom);
        let message = editor
            .place_schedule(&mut session, spec, Channel::Ui)
            .unwrap();
        assert!(message.starts_with("Building"), "{message}");
        assert_eq!(session.revision(), revision);
        editor.tick_schedules(&mut session);
        assert_eq!(service.requests.lock().unwrap()[0].1, vec![spec]);
        assert_eq!(session.revision(), revision, "no reply yet");
        editor.tick_schedules(&mut session);
        assert_eq!(session.revision(), revision + 1, "one page edit");
        let placed = tables(&editor);
        assert_eq!(placed.len(), 1);
        let AnnotationKind::Table {
            rows: cells,
            schedule,
            position_mm,
            ..
        } = &placed[0].kind
        else {
            panic!()
        };
        assert_eq!(*cells, rows(&["Leg", "Top"]));
        assert_eq!(*schedule, Some(spec));
        let margin = editor.draft.as_ref().unwrap().settings.margin_mm;
        assert_eq!(*position_mm, [margin + 5., margin + 5.]);
        assert_eq!(editor.selected_annotation, Some(placed[0].id));

        // The placement changed the revision: the table is checked again
        // against the schedule of the new source.
        assert_eq!(
            editor.schedule_status(&session, placed[0].id),
            ScheduleStatus::Checking
        );
        editor.tick_schedules(&mut session);
        editor.tick_schedules(&mut session);
        assert_eq!(
            editor.schedule_status(&session, placed[0].id),
            ScheduleStatus::Current
        );
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(session.document(), &before);
        editor.sync(&session).unwrap();
        assert!(tables(&editor).is_empty());
    }

    #[test]
    fn a_changed_model_reports_the_table_stale_and_refresh_replaces_only_its_cells() {
        let mut session = Session::new(Document::new("Stale schedules").unwrap()).unwrap();
        let mut editor = DrawingEditor::default();
        let service = Service::default();
        *service.rows.lock().unwrap() = rows(&["Leg"]);
        let factory = service.clone();
        editor.set_backend_factory(Arc::new(move || Box::new(factory.clone())));
        editor
            .invoke(&mut session, DrawingTool::AddPage, Channel::Ui)
            .unwrap();
        let spec = ScheduleSpec::new(ScheduleKind::PanelCutList);
        editor.schedules.choice = spec;
        editor
            .invoke(&mut session, DrawingTool::ScheduleTable, Channel::Ui)
            .unwrap();
        editor.tick_schedules(&mut session);
        editor.tick_schedules(&mut session);
        editor
            .place_schedule(&mut session, spec, Channel::Ui)
            .unwrap();
        let id = tables(&editor)[0].id;
        // Move the table; a refresh keeps its position and identity.
        let mut page = drawings::read(session.document(), editor.book.unwrap())
            .unwrap()
            .pages
            .remove(0);
        let AnnotationKind::Table { position_mm, .. } = &mut page.annotations[0].kind else {
            panic!()
        };
        *position_mm = [60., 70.];
        session
            .commit(
                drawings::prepare(
                    &session,
                    session.revision(),
                    Actor::local(Channel::Ui),
                    editor.book.unwrap(),
                    PageAction::Replace(page),
                )
                .unwrap(),
            )
            .unwrap();
        editor.sync(&session).unwrap();

        // The model gains a panel.
        *service.rows.lock().unwrap() = rows(&["Leg", "Shelf"]);
        unrelated_edit(&mut session);
        editor.sync(&session).unwrap();
        editor.tick_schedules(&mut session);
        editor.tick_schedules(&mut session);
        let ScheduleStatus::Stale(message) = editor.schedule_status(&session, id) else {
            panic!("{:?}", editor.schedule_status(&session, id))
        };
        assert!(message.contains("Panel cut list"), "{message}");

        // Unapplied page edits block the refresh commit.
        editor.draft.as_mut().unwrap().info.title = "Pending".into();
        assert!(
            editor
                .refresh_schedule_table(&mut session, id, Channel::Ui)
                .is_err()
        );
        editor
            .invoke(&mut session, DrawingTool::Discard, Channel::Ui)
            .unwrap();
        let revision = session.revision();
        editor
            .refresh_schedule_table(&mut session, id, Channel::Ui)
            .unwrap();
        assert_eq!(session.revision(), revision + 1);
        let refreshed = tables(&editor);
        assert_eq!(refreshed.len(), 1);
        assert_eq!(refreshed[0].id, id);
        let AnnotationKind::Table {
            rows: cells,
            position_mm,
            ..
        } = &refreshed[0].kind
        else {
            panic!()
        };
        assert_eq!(*cells, rows(&["Leg", "Shelf"]));
        assert_eq!(*position_mm, [60., 70.]);
        editor.tick_schedules(&mut session);
        editor.tick_schedules(&mut session);
        assert_eq!(
            editor.schedule_status(&session, id),
            ScheduleStatus::Current
        );
    }
}

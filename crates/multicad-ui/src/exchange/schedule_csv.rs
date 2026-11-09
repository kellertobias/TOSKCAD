//! Export → Common → Schedule CSV: preview one schedule of the current
//! source, then write exactly that CSV to a file chosen in the native save
//! dialog. The output service builds and writes off the UI thread; a write
//! names the previewed CSV hash, so a changed source writes nothing.
use super::*;
use multicad_core::schedules::{ScheduleKind, ScheduleSpec};

pub(super) struct Job {
    document: EntityId,
    revision: u64,
    spec: ScheduleSpec,
    actor: Actor,
    write: Option<String>,
}
pub(super) struct ScheduleCsv {
    pub spec: ScheduleSpec,
    /// The panel is open, so its schedule preview is kept current.
    pub open: bool,
    /// Expand the panel on the next frame.
    reveal: bool,
    pub jobs: BTreeMap<EntityId, Job>,
    preview: Option<ScheduleReceipt>,
    requested: Option<(EntityId, u64, ScheduleSpec)>,
    save: bool,
    pub error: Option<String>,
    written: Option<String>,
}
impl Default for ScheduleCsv {
    fn default() -> Self {
        Self {
            spec: ScheduleSpec::new(ScheduleKind::Bom),
            open: false,
            reveal: false,
            jobs: BTreeMap::new(),
            preview: None,
            requested: None,
            save: false,
            error: None,
            written: None,
        }
    }
}

impl ExportWorkspace {
    /// The schedule preview for the current document, revision and choice.
    pub fn schedule_preview(&self, s: &Session) -> Option<&ScheduleReceipt> {
        self.schedule.preview.as_ref().filter(|p| {
            p.source.document == s.document().id()
                && p.source.revision == s.revision()
                && p.spec == self.schedule.spec
        })
    }
    pub fn schedule_spec(&self) -> ScheduleSpec {
        self.schedule.spec
    }
    pub fn set_schedule_spec(&mut self, spec: ScheduleSpec) {
        self.schedule.spec = spec.normalized();
    }
    /// A schedule CSV write is accepted and not yet acknowledged.
    pub fn writing_schedule(&self) -> bool {
        self.schedule.jobs.values().any(|j| j.write.is_some())
    }
    /// The panel asked for the native save dialog.
    pub fn take_schedule_save(&mut self) -> bool {
        std::mem::take(&mut self.schedule.save)
    }
    /// A file name for the save dialog: project and schedule kind.
    pub fn schedule_file_name(&self, s: &Session) -> String {
        format!(
            "{}_{}.csv",
            filename_token(s.document().name()),
            self.schedule.spec.kind.token()
        )
    }
    fn request_schedule_preview(&mut self, s: &Session, channel: Channel) -> Result<(), String> {
        let actor = Actor::local(channel);
        let spec = self.schedule.spec;
        let id = self
            .backend
            .as_mut()
            .ok_or("Native output service is not installed")?
            .schedule(s.document(), s.revision(), spec, None, actor.clone())
            .map_err(|e| e.to_string())?;
        self.schedule.requested = Some((s.document().id(), s.revision(), spec));
        self.schedule.jobs.insert(
            id,
            Job {
                document: s.document().id(),
                revision: s.revision(),
                spec,
                actor,
                write: None,
            },
        );
        Ok(())
    }
    /// Keep the open panel's preview current, one job at a time.
    pub(super) fn tick_schedule(&mut self, s: &Session, allow_io: bool) {
        let key = (s.document().id(), s.revision(), self.schedule.spec);
        if allow_io
            && self.schedule.open
            && self.backend.is_some()
            && self.schedule.jobs.is_empty()
            && self.schedule.requested != Some(key)
            && let Err(error) = self.request_schedule_preview(s, Channel::Ui)
        {
            self.schedule.requested = Some(key);
            self.schedule.error = Some(error);
        }
    }
    /// The ribbon command: open the panel and, with a current preview,
    /// write to `path` or ask for the save dialog.
    pub(super) fn schedule_csv(
        &mut self,
        s: &Session,
        path: Option<&str>,
        channel: Channel,
    ) -> Result<String, String> {
        self.schedule.open = true;
        self.schedule.reveal = true;
        if self.backend.is_none() {
            return Err("Native output service is not installed".into());
        }
        if let Some(path) = path {
            return self.write_schedule(s, Path::new(path), channel);
        }
        if self.schedule_preview(s).is_some() {
            self.schedule.save = true;
            return Ok(format!(
                "Choose where to save the {}",
                self.schedule.spec.label()
            ));
        }
        if self.schedule.jobs.is_empty() {
            self.request_schedule_preview(s, channel)?;
        }
        Ok(format!(
            "Building the {} for revision {}…",
            self.schedule.spec.label(),
            s.revision()
        ))
    }
    /// Write the previewed CSV of the current revision to `path`.
    pub fn write_schedule(
        &mut self,
        s: &Session,
        path: &Path,
        channel: Channel,
    ) -> Result<String, String> {
        let preview = self.schedule_preview(s).ok_or(
            "Preview the schedule of the current source before writing it; nothing was written",
        )?;
        preview
            .source
            .check(s.document(), s.revision())
            .map_err(|e| format!("{e}; nothing was written"))?;
        if !self.schedule.jobs.is_empty() {
            return Err("Wait for the schedule job in progress".into());
        }
        let (spec, hash) = (preview.spec, preview.csv_hash);
        let actor = Actor::local(channel);
        let id = self
            .backend
            .as_mut()
            .ok_or("Native output service is not installed")?
            .schedule(
                s.document(),
                s.revision(),
                spec,
                Some((path, hash)),
                actor.clone(),
            )
            .map_err(|e| e.to_string())?;
        self.schedule.jobs.insert(
            id,
            Job {
                document: s.document().id(),
                revision: s.revision(),
                spec,
                actor,
                write: Some(path.display().to_string()),
            },
        );
        Ok(format!(
            "Writing the {} for revision {}…",
            spec.label(),
            s.revision()
        ))
    }
    /// Accept a schedule reply for its own accepted job only.
    pub(super) fn accept_schedule(&mut self, job: Job, reply: ExportReply, s: &Session) {
        if reply.document != job.document
            || reply.revision != job.revision
            || reply.actor != job.actor
        {
            self.schedule.error = Some("Schedule result differs from its accepted request".into());
            return;
        }
        let ExportResult::Schedule(result) = reply.result else {
            self.schedule.error = Some("Output reply has the wrong operation kind".into());
            return;
        };
        let result = result.and_then(|receipt| {
            if receipt.spec != job.spec
                || receipt.source.document != job.document
                || receipt.source.revision != job.revision
                || receipt.written.is_some() != job.write.is_some()
            {
                Err(ExportError::new(
                    ErrorKind::Invalid,
                    None,
                    "Schedule receipt differs from its accepted request",
                ))
            } else {
                Ok(receipt)
            }
        });
        match (result, job.write) {
            (Ok(receipt), None) => {
                self.schedule.error = None;
                if receipt.source.check(s.document(), s.revision()).is_ok() {
                    self.schedule.preview = Some(*receipt);
                }
            }
            (Ok(receipt), Some(_)) => {
                let path = receipt.written.clone().unwrap_or_default();
                let text = format!(
                    "{} CSV written · {} rows · revision {} · {}",
                    receipt.spec.label(),
                    receipt.rows.len(),
                    receipt.source.revision,
                    path
                );
                self.schedule.error = None;
                self.schedule.written = Some(text.clone());
                self.notice = Some((text.clone(), false));
                self.remember(reply.id, text, None);
            }
            (Err(e), write) => {
                if e.kind == ErrorKind::Cancelled {
                    return;
                }
                let text = if write.is_some() {
                    format!("Schedule CSV not written · {e}")
                } else {
                    e.to_string()
                };
                self.schedule.error = Some(text.clone());
                if write.is_some() {
                    self.notice = Some((text.clone(), true));
                    self.remember(reply.id, text, None);
                }
            }
        }
    }
    /// The Schedule CSV panel.
    pub(super) fn schedule_section(&mut self, ui: &mut egui::Ui, s: &Session, allow_io: bool) {
        ui.separator();
        let id = ui.make_persistent_id("schedule-csv");
        let mut state =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
        if std::mem::take(&mut self.schedule.reveal) {
            state.set_open(true);
        }
        if super::output_row(
            ui,
            "schedule-row",
            "Schedule CSV",
            crate::icons::Icon::Grid,
            state.is_open(),
            true,
        )
        .clicked()
        {
            state.toggle(ui);
        }
        self.schedule.open = state.is_open();
        state.show_body_unindented(ui, |ui| {
            let mut spec = self.schedule.spec;
            crate::schedules::spec_controls(ui, "export-schedule-kind", &mut spec);
            self.schedule.spec = spec;
            match self.schedule_preview(s) {
                Some(preview) => {
                    ui.small(format!(
                        "{} rows × {} columns · revision {}",
                        preview.rows.len(),
                        preview.columns.len(),
                        preview.source.revision
                    ));
                }
                None if !self.schedule.jobs.is_empty() => {
                    ui.small("Building the current schedule…");
                }
                None => {
                    ui.small("No current preview");
                }
            }
            let ready =
                allow_io && self.schedule_preview(s).is_some() && self.schedule.jobs.is_empty();
            if ui
                .add_enabled(ready, egui::Button::new("Export CSV…"))
                .on_hover_text("Write exactly the previewed schedule as CSV to a file you choose")
                .clicked()
            {
                self.schedule.save = true;
            }
            if let Some(error) = &self.schedule.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            } else if let Some(written) = &self.schedule.written {
                ui.small(written);
            }
        });
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use multicad_core::{
        commands::Request,
        document::{Document, Entity, EntityData, Mutation},
        project_file::ContentHash,
    };
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
    };
    /// One accepted schedule job: id, snapshot, revision, spec, target, actor.
    pub type Accepted = (
        EntityId,
        Document,
        u64,
        ScheduleSpec,
        Option<(PathBuf, ContentHash)>,
        Actor,
    );
    /// A controlled output service that records schedule jobs and replies
    /// only when the test says so.
    #[derive(Clone, Default)]
    pub struct Service {
        pub jobs: Arc<Mutex<Vec<Accepted>>>,
        pub replies: Arc<Mutex<VecDeque<ExportReply>>>,
    }
    impl ExportBackend for Service {
        fn preview(
            &mut self,
            _: &Document,
            _: u64,
            _: EntityId,
            _: Actor,
        ) -> Result<EntityId, ExportError> {
            Err(ExportError::new(ErrorKind::Unsupported, None, "unused"))
        }
        fn run(
            &mut self,
            _: &Document,
            _: u64,
            _: &PreviewKey,
            _: &Path,
            _: Actor,
        ) -> Result<EntityId, ExportError> {
            Err(ExportError::new(ErrorKind::Unsupported, None, "unused"))
        }
        fn schedule(
            &mut self,
            document: &Document,
            revision: u64,
            spec: ScheduleSpec,
            target: Option<(&Path, ContentHash)>,
            actor: Actor,
        ) -> Result<EntityId, ExportError> {
            let id = EntityId::new();
            self.jobs.lock().unwrap().push((
                id,
                document.clone(),
                revision,
                spec,
                target.map(|(p, h)| (p.to_owned(), h)),
                actor,
            ));
            Ok(id)
        }
        fn cancel(&mut self, _: EntityId) -> bool {
            true
        }
        fn poll(&mut self) -> Option<ExportReply> {
            self.replies.lock().unwrap().pop_front()
        }
        fn pending(&self) -> bool {
            !self.replies.lock().unwrap().is_empty()
        }
    }
    impl Service {
        /// Answer job `index` as the native service would: CSV text for a
        /// preview, the written path for a write.
        pub fn answer(&self, index: usize) {
            let (id, document, revision, spec, target, actor) =
                self.jobs.lock().unwrap()[index].clone();
            let csv = format!(
                "# MultiCAD {} schedule\n# revision {revision}\nPart,Qty\nLeg,2\n",
                spec.kind.token()
            );
            let receipt = ScheduleReceipt {
                spec,
                source: ExportStamp::capture(&document, revision).unwrap(),
                title: spec.kind.label().into(),
                notes: vec![],
                columns: vec!["Part".into(), "Qty".into()],
                rows: vec![vec!["Leg".into(), "2".into()]],
                csv_hash: ContentHash::of(csv.as_bytes()),
                csv,
                written: target.map(|(p, _)| p.display().to_string()),
            };
            self.reply(id, &document, revision, actor, Ok(receipt));
        }
        pub fn reply(
            &self,
            id: EntityId,
            document: &Document,
            revision: u64,
            actor: Actor,
            result: Result<ScheduleReceipt, ExportError>,
        ) {
            self.replies.lock().unwrap().push_back(ExportReply {
                id,
                document: document.id(),
                revision,
                actor,
                result: ExportResult::Schedule(result.map(Box::new)),
            });
        }
    }
    fn session() -> Session {
        Session::new(Document::new("Shop").unwrap()).unwrap()
    }
    fn edit(s: &mut Session) {
        s.apply(Request {
            expected_revision: s.revision(),
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
    fn schedule_csv_previews_then_writes_exactly_the_previewed_hash() {
        let mut s = session();
        let service = Service::default();
        let mut w = ExportWorkspace::default();
        assert!(!w.enabled(ExportTool::ScheduleCsv, &s));
        w.set_backend(Box::new(service.clone())).unwrap();
        assert!(w.enabled(ExportTool::ScheduleCsv, &s));
        w.set_schedule_spec(ScheduleSpec::new(ScheduleKind::WeldSchedule));

        // Without a current preview the command builds one; no dialog yet.
        let message = w
            .execute(&mut s, ExportTool::ScheduleCsv, None, Channel::Ui)
            .unwrap();
        assert!(
            message.starts_with("Building the Weld schedule"),
            "{message}"
        );
        assert!(!w.take_schedule_save());
        assert!(
            w.write_schedule(&s, Path::new("/tmp/x.csv"), Channel::Ui)
                .is_err()
        );
        {
            let jobs = service.jobs.lock().unwrap();
            assert_eq!(jobs.len(), 1);
            assert_eq!(jobs[0].3.kind, ScheduleKind::WeldSchedule);
            assert!(jobs[0].4.is_none(), "a preview writes nothing");
        }
        service.answer(0);
        w.poll(&s);
        let preview = w.schedule_preview(&s).unwrap().clone();
        assert_eq!(preview.rows.len(), 1);
        assert_eq!(
            w.schedule_file_name(&s),
            "Shop_weld_schedule.csv",
            "the dialog suggests project and kind"
        );

        // With a current preview the command asks for the save dialog.
        w.execute(&mut s, ExportTool::ScheduleCsv, None, Channel::Ui)
            .unwrap();
        assert!(w.take_schedule_save());
        let path = Path::new("/private/tmp/shop_welds.csv");
        w.write_schedule(&s, path, Channel::Ui).unwrap();
        assert!(w.writing(), "an accepted write guards window close");
        {
            let jobs = service.jobs.lock().unwrap();
            let (_, document, revision, _, target, _) = &jobs[1];
            assert_eq!(
                target.as_ref().unwrap(),
                &(path.to_owned(), preview.csv_hash)
            );
            assert_eq!((document, *revision), (s.document(), s.revision()));
        }
        service.answer(1);
        w.poll(&s);
        assert!(!w.writing());
        let (notice, error) = w.take_notice().unwrap();
        assert!(!error && notice.contains("CSV written"), "{notice}");
        assert!(notice.contains("/private/tmp/shop_welds.csv"));
    }

    #[test]
    fn a_changed_source_or_foreign_reply_writes_nothing_and_says_why() {
        let mut s = session();
        let service = Service::default();
        let mut w = ExportWorkspace::default();
        w.set_backend(Box::new(service.clone())).unwrap();
        w.execute(&mut s, ExportTool::ScheduleCsv, None, Channel::Ui)
            .unwrap();
        // A reply attributed to another actor is refused.
        let (id, document, revision, ..) = service.jobs.lock().unwrap()[0].clone();
        let mut foreign = Actor::local(Channel::Mcp);
        foreign.id = "someone else".into();
        service.reply(
            id,
            &document,
            revision,
            foreign,
            Err(ExportError::new(ErrorKind::Empty, None, "x")),
        );
        w.poll(&s);
        assert!(w.schedule_preview(&s).is_none());
        assert!(w.schedule.error.as_deref().unwrap().contains("differs"));

        w.execute(&mut s, ExportTool::ScheduleCsv, None, Channel::Ui)
            .unwrap();
        service.answer(1);
        w.poll(&s);
        assert!(w.schedule_preview(&s).is_some());
        // An edit after the preview: the old CSV is never written.
        edit(&mut s);
        assert!(w.schedule_preview(&s).is_none());
        let error = w
            .write_schedule(&s, Path::new("/private/tmp/stale.csv"), Channel::Ui)
            .unwrap_err();
        assert!(error.contains("nothing was written"), "{error}");
        assert_eq!(service.jobs.lock().unwrap().len(), 2);

        // A write the service refuses as stale is reported, not written.
        w.execute(&mut s, ExportTool::ScheduleCsv, None, Channel::Ui)
            .unwrap();
        service.answer(2);
        w.poll(&s);
        w.write_schedule(&s, Path::new("/private/tmp/late.csv"), Channel::Ui)
            .unwrap();
        let (id, document, revision, _, _, actor) = service.jobs.lock().unwrap()[3].clone();
        service.reply(
            id,
            &document,
            revision,
            actor,
            Err(ExportError::new(
                ErrorKind::Stale,
                None,
                "The Bill of materials changed since it was previewed; nothing was written",
            )),
        );
        w.poll(&s);
        let (notice, error) = w.take_notice().unwrap();
        assert!(error && notice.contains("not written"), "{notice}");
        assert!(!w.writing());
    }
}

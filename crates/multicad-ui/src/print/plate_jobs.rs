//! Native preflight and write receipts for one project's print plates.
use super::*;
use multicad_core::commands::SessionStamp;

#[derive(Clone, PartialEq)]
struct Source {
    session: SessionStamp,
    document: EntityId,
    revision: u64,
    plate: EntityId,
    definition: PrintPlateDefinition,
}

struct Job {
    id: EntityId,
    source: Source,
    actor: Actor,
    expected: Option<PrintKey>,
}

#[derive(Default)]
pub(crate) struct PlateJobs {
    backend: Option<Box<dyn PrintBackend>>,
    source: Option<Source>,
    job: Option<Job>,
    due: Option<f64>,
    preview: Option<PrintPreview>,
    error: Option<String>,
    notice: Option<(String, bool)>,
}

impl PlateJobs {
    pub(crate) fn set_backend(&mut self, backend: Option<Box<dyn PrintBackend>>) {
        self.backend = backend;
        self.source = None;
        self.preview = None;
        self.error = None;
    }

    pub(crate) fn has_backend(&self) -> bool {
        self.backend.is_some()
    }

    pub(crate) fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub(crate) fn waiting(&self) -> bool {
        self.busy() || self.due.is_some()
    }

    pub(crate) fn writing(&self) -> bool {
        self.job.as_ref().is_some_and(|job| job.expected.is_some())
    }

    pub(crate) fn preview(&self) -> Option<&PrintPreview> {
        self.preview.as_ref()
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn take_notice(&mut self) -> Option<(String, bool)> {
        self.notice.take()
    }

    pub(crate) fn cancel(&mut self) {
        self.due = None;
        if let Some(job) = &self.job
            && let Some(backend) = &mut self.backend
        {
            backend.cancel(job.id);
        }
    }

    pub(crate) fn retry(&mut self, time: f64) {
        self.error = None;
        self.due = Some(time);
    }

    pub(crate) fn update(
        &mut self,
        s: &Session,
        plate: EntityId,
        definition: &PrintPlateDefinition,
        time: f64,
        allow_io: bool,
    ) {
        if self.source.as_ref().is_none_or(|source| {
            source.session != s.stamp() || source.plate != plate || source.definition != *definition
        }) {
            if !self.writing() {
                self.cancel();
            }
            self.source = Some(Source {
                session: s.stamp(),
                document: s.document().id(),
                revision: s.revision(),
                plate,
                definition: definition.clone(),
            });
            self.preview = None;
            self.error = None;
            self.due = (!definition.placements.is_empty()).then_some(time + 0.15);
        }
        if allow_io && !self.busy() && self.due.is_some_and(|due| due <= time) {
            self.due = None;
            if let Err(error) = self.request(s) {
                self.error = Some(error);
            }
        }
    }

    fn request(&mut self, s: &Session) -> Result<(), String> {
        let source = self.source.as_ref().ok_or("Choose a print plate")?;
        let actor = Actor::local(Channel::Ui);
        let id = self
            .backend
            .as_mut()
            .ok_or("Native print plate service is unavailable")?
            .preview_plate(
                s.document(),
                s.revision(),
                &source.definition,
                actor.clone(),
            )
            .map_err(|error| error.to_string())?;
        self.job = Some(Job {
            id,
            source: source.clone(),
            actor,
            expected: None,
        });
        Ok(())
    }

    pub(crate) fn can_write(
        &self,
        s: &Session,
        plate: EntityId,
        definition: &PrintPlateDefinition,
    ) -> bool {
        !self.busy()
            && self.source.as_ref().is_some_and(|source| {
                source.session == s.stamp()
                    && source.plate == plate
                    && source.definition == *definition
            })
            && self.preview.as_ref().is_some_and(PrintPreview::can_write)
    }

    pub(crate) fn write(
        &mut self,
        s: &Session,
        plate: EntityId,
        definition: &PrintPlateDefinition,
        path: &Path,
    ) -> Result<(), String> {
        if !self.can_write(s, plate, definition) {
            return Err("Wait for the current plate to pass native preflight".into());
        }
        let path = super::three_mf_path(path)?;
        let expected = self
            .preview
            .as_ref()
            .expect("ready plate preview")
            .key
            .clone();
        expected
            .source
            .check(s.document(), s.revision())
            .map_err(|error| error.to_string())?;
        let actor = Actor::local(Channel::Ui);
        let id = self
            .backend
            .as_mut()
            .ok_or("Native print plate service is unavailable")?
            .write_plate(
                s.document(),
                s.revision(),
                definition,
                &expected,
                &path,
                actor.clone(),
            )
            .map_err(|error| error.to_string())?;
        self.job = Some(Job {
            id,
            source: self.source.clone().expect("ready plate source"),
            actor,
            expected: Some(expected),
        });
        self.error = None;
        Ok(())
    }

    pub(crate) fn poll(&mut self, s: &Session) {
        while let Some(reply) = self.backend.as_mut().and_then(|backend| backend.poll()) {
            if self.job.as_ref().is_none_or(|job| job.id != reply.id) {
                continue;
            }
            let job = self.job.take().expect("matching job");
            let current =
                self.source.as_ref() == Some(&job.source) && job.source.session == s.stamp();
            let write = job.expected.is_some();
            if !current && !write {
                continue;
            }
            if reply.document != job.source.document
                || reply.revision != job.source.revision
                || reply.actor != job.actor
            {
                self.failed(
                    "Native print plate reply did not match its request".into(),
                    write,
                );
                continue;
            }
            match (reply.result, job.expected) {
                (PrintResult::Preview(Ok(preview)), None)
                    if preview.requested_by == job.actor
                        && preview.request
                            == job.source.definition.summary_request(s.document())
                        && preview.selection.stamp == preview.key.source
                        && preview.key.source.check(s.document(), s.revision()).is_ok() =>
                {
                    self.preview = Some(*preview);
                    self.error = None;
                }
                (PrintResult::Preview(Err(error)), None)
                | (PrintResult::Write(Err(error)), Some(_)) => {
                    if error.kind == ErrorKind::Cancelled && write {
                        self.notice = Some(("Print plate write canceled".into(), false));
                    } else {
                        self.failed(error.to_string(), write);
                    }
                }
                (PrintResult::Write(Ok(receipt)), Some(expected))
                    if receipt.actor == job.actor && receipt.key == expected =>
                {
                    let path = receipt
                        .files
                        .first()
                        .map_or("the selected folder", |file| file.path.as_str());
                    self.notice = Some((
                        format!("Wrote {} print plate file(s) · {path}", receipt.files.len()),
                        false,
                    ));
                }
                _ => self.failed(
                    "Native print plate receipt did not match its request".into(),
                    write,
                ),
            }
        }
    }

    fn failed(&mut self, error: String, write: bool) {
        if write {
            self.notice = Some((error, true));
        } else {
            self.error = Some(error);
        }
    }
}

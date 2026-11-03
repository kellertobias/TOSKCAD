//! Direct drawing PDF jobs use the existing export backend and publisher.
//! A prepared output-set overlay never becomes canonical drawing source.
use super::*;
use multicad_core::{
    commands::{Prepared, SessionStamp},
    drawings,
};
use std::path::PathBuf;
mod dialog;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawingPdfScope {
    #[default]
    Sheet,
    Drawing,
    Project,
}
struct Job {
    id: EntityId,
    prepared: Prepared,
    revision: u64,
    actor: Actor,
    directory: PathBuf,
    pages: Vec<PageRef>,
    expected: Option<PreviewKey>,
}
#[derive(Default)]
pub(super) struct State {
    open: bool,
    selected: Option<PageRef>,
    scope: DrawingPdfScope,
    set: EntityId,
    recipe: EntityId,
    catalog: Vec<form::PageChoice>,
    catalog_stamp: Option<SessionStamp>,
    job: Option<Job>,
    cancelled: bool,
    error: Option<String>,
    status: Option<String>,
    output: Option<String>,
}
impl State {
    pub(super) fn pending(&self) -> bool {
        self.job.is_some()
    }
    pub(super) fn writing(&self) -> bool {
        self.job.as_ref().is_some_and(|j| j.expected.is_some())
    }
    pub(super) fn owns(&self, id: EntityId) -> bool {
        self.job.as_ref().is_some_and(|j| j.id == id)
    }
    fn refresh_catalog(&mut self, s: &Session) {
        if self
            .catalog_stamp
            .as_ref()
            .is_none_or(|stamp| s.validate_stamp(stamp).is_err())
        {
            self.catalog = form::Catalog::new(s).pages;
            self.catalog.sort_by(|a, b| {
                a.book_name
                    .cmp(&b.book_name)
                    .then(a.reference.book.cmp(&b.reference.book))
            });
            self.catalog_stamp = Some(s.stamp());
        }
    }
    fn pages(&self) -> Vec<PageRef> {
        self.catalog
            .iter()
            .filter(|page| match self.scope {
                DrawingPdfScope::Sheet => self.selected.as_ref() == Some(&page.reference),
                DrawingPdfScope::Drawing => self
                    .selected
                    .as_ref()
                    .is_some_and(|s| s.book == page.reference.book),
                DrawingPdfScope::Project => true,
            })
            .map(|p| p.reference.clone())
            .collect()
    }
}
impl ExportWorkspace {
    pub fn open_drawing_pdf(&mut self, s: &Session, selected: PageRef) -> Result<(), String> {
        if self.drawing_pdf.pending() {
            return Err("Wait for the current PDF export to finish or cancel".into());
        }
        let book = drawings::read(s.document(), selected.book).map_err(|e| e.to_string())?;
        if !book.pages.iter().any(|p| p.id == selected.page) {
            return Err("The selected drawing sheet no longer exists".into());
        }
        let state = &mut self.drawing_pdf;
        if state.set.is_nil() {
            state.set = EntityId::new();
        }
        if state.recipe.is_nil() {
            state.recipe = EntityId::new();
        }
        state.open = true;
        state.selected = Some(selected);
        state.scope = DrawingPdfScope::Sheet;
        state.cancelled = false;
        state.error = None;
        state.status = None;
        state.output = None;
        state.refresh_catalog(s);
        Ok(())
    }
    pub fn drawing_pdf_open(&self) -> bool {
        self.drawing_pdf.open
    }
    pub fn cancel_drawing_pdf(&mut self) {
        self.drawing_pdf.cancelled = true;
        if let Some(job) = &self.drawing_pdf.job
            && let Some(backend) = &mut self.backend
        {
            backend.cancel(job.id);
        }
        self.drawing_pdf.open = false;
    }
    pub fn set_drawing_pdf_scope(&mut self, scope: DrawingPdfScope) -> Result<(), String> {
        if !self.drawing_pdf.open || self.drawing_pdf.pending() {
            return Err("Open an idle drawing PDF dialog first".into());
        }
        self.drawing_pdf.scope = scope;
        self.drawing_pdf.error = None;
        self.drawing_pdf.status = None;
        self.drawing_pdf.output = None;
        Ok(())
    }
    pub fn start_drawing_pdf(&mut self, s: &Session, channel: Channel) -> Result<String, String> {
        if !self.drawing_pdf.open || self.drawing_pdf.pending() {
            return Err("Open an idle drawing PDF dialog first".into());
        }
        if let Some(problem) = destination::diagnose(&self.directory) {
            return Err(problem);
        }
        let state = &mut self.drawing_pdf;
        state.refresh_catalog(s);
        let pages = state.pages();
        if pages.is_empty() {
            return Err(
                "The chosen scope has no drawing sheets; select a current sheet or drawing".into(),
            );
        }
        let definition = SetDefinition {
            pages: Pages::Selected {
                pages: pages.clone(),
            },
            recipes: vec![Recipe {
                id: state.recipe,
                name: "Drawing PDF".into(),
                output: RecipeOutput::Pdf,
            }],
            ..Default::default()
        };
        let actor = Actor::local(channel);
        let prepared = prepare_set(
            s,
            s.revision(),
            actor.clone(),
            state.set,
            "Drawing PDF",
            &definition,
        )
        .map_err(|e| e.to_string())?;
        let backend = self
            .backend
            .as_mut()
            .ok_or("Native PDF export is unavailable in this host")?;
        let id = backend
            .preview(prepared.document(), s.revision(), state.set, actor.clone())
            .map_err(|e| e.to_string())?;
        state.cancelled = false;
        state.error = None;
        state.output = None;
        state.status = Some(format!("Checking {} sheet(s) for PDF export…", pages.len()));
        state.job = Some(Job {
            id,
            prepared,
            revision: s.revision(),
            actor,
            directory: PathBuf::from(self.directory.trim()),
            pages,
            expected: None,
        });
        Ok(state.status.clone().expect("started PDF status"))
    }
    pub(super) fn receive_drawing_pdf(&mut self, reply: ExportReply, s: &Session) {
        let mut state = std::mem::take(&mut self.drawing_pdf);
        let mut job = state.job.take().expect("owned drawing PDF reply");
        let id = reply.id;
        let scope_matches = reply.document == job.prepared.document().id()
            && reply.revision == job.revision
            && reply.actor == job.actor;
        if !scope_matches {
            state.error = Some("PDF result could not be attributed to its accepted request".into());
        } else if job.expected.is_none() {
            match reply.result {
                ExportResult::Preview(Ok(preview)) if !state.cancelled => {
                    let result: Result<EntityId, String> = (|| {
                        s.validate_preview(&job.prepared)
                            .map_err(|e| e.to_string())?;
                        state.refresh_catalog(s);
                        if state.pages() != job.pages
                            || Path::new(self.directory.trim()) != job.directory.as_path()
                        {
                            return Err(
                                "PDF scope or destination changed; start the export again".into()
                            );
                        }
                        preview
                            .key
                            .source
                            .check(job.prepared.document(), job.revision)
                            .map_err(|e| e.to_string())?;
                        let resolved: Vec<_> = preview
                            .selection
                            .pages
                            .iter()
                            .map(|p| p.reference.clone())
                            .collect();
                        if preview.requested_by != job.actor
                            || preview.key.set != state.set
                            || preview.selection.set != state.set
                            || preview.selection.stamp != preview.key.source
                            || resolved != job.pages
                            || preview.selection.recipes.len() != 1
                            || preview.selection.recipes[0].id != state.recipe
                            || preview.selection.recipes[0].output != RecipeOutput::Pdf
                            || !preview.can_run()
                        {
                            return Err("Native PDF preview differs from the selected sheets or cannot be exported".into());
                        }
                        if let Some(problem) = destination::diagnose(
                            job.directory
                                .to_str()
                                .ok_or("Choose a UTF-8 output folder")?,
                        ) {
                            return Err(problem);
                        }
                        self.backend
                            .as_mut()
                            .ok_or("Native output service is unavailable")?
                            .run(
                                job.prepared.document(),
                                job.revision,
                                &preview.key,
                                &job.directory,
                                job.actor.clone(),
                            )
                            .map_err(|e| e.to_string())
                    })();
                    match result {
                        Ok(id) => {
                            job.id = id;
                            job.expected = Some(preview.key.clone());
                            state.status =
                                Some(format!("Writing {} sheet(s) to PDF…", job.pages.len()));
                            self.notice = Some((state.status.clone().unwrap(), false));
                            state.job = Some(job);
                        }
                        Err(error) => state.error = Some(error),
                    }
                }
                ExportResult::Preview(Ok(_)) => {
                    state.status = Some("PDF export cancelled · drawing edits retained".into())
                }
                ExportResult::Preview(Err(error)) if error.kind == ErrorKind::Cancelled => {
                    state.status = Some("PDF export cancelled · drawing edits retained".into())
                }
                ExportResult::Preview(Err(error)) => state.error = Some(error.to_string()),
                _ => {
                    state.error =
                        Some("Native output service returned the wrong PDF preview result".into())
                }
            }
        } else {
            match reply.result {
                ExportResult::Run(Ok(receipt)) => {
                    let valid = job.expected.as_ref() == Some(&receipt.key)
                        && receipt.actor == job.actor
                        && receipt.files.len() == 1
                        && receipt.files[0].artifact.output.recipe == state.recipe
                        && receipt.files[0].artifact.output.filename.ends_with(".pdf")
                        && receipt.files[0]
                            .artifact
                            .drawing
                            .as_ref()
                            .is_some_and(|d| d.pages == job.pages);
                    if valid {
                        let output = PathBuf::from(&receipt.directory)
                            .join(&receipt.files[0].artifact.output.filename)
                            .display()
                            .to_string();
                        state.status = Some(format!(
                            "{} {}-sheet PDF from revision {}",
                            if state.cancelled {
                                "Export completed before cancellation:"
                            } else if receipt.reused {
                                "Reused"
                            } else {
                                "Exported"
                            },
                            job.pages.len(),
                            job.revision
                        ));
                        state.output = Some(output);
                    } else {
                        state.error = Some("Published PDF receipt differs from the accepted sheet scope; inspect its manifest".into());
                    }
                }
                ExportResult::Run(Err(error)) if error.kind == ErrorKind::Cancelled => {
                    state.status = Some("PDF export cancelled · drawing edits retained".into())
                }
                ExportResult::Run(Err(error)) => state.error = Some(error.to_string()),
                _ => {
                    state.error = Some(
                        "Native output service returned the wrong PDF publication result".into(),
                    )
                }
            }
        }
        if !state.pending() {
            let error = state.error.is_some();
            let text = state
                .error
                .clone()
                .or_else(|| state.status.clone())
                .unwrap_or_else(|| "PDF export finished".into());
            self.notice = Some((
                state
                    .output
                    .as_ref()
                    .map_or_else(|| text.clone(), |path| format!("{text} · {path}")),
                error,
            ));
            self.remember(
                id,
                text,
                state
                    .output
                    .as_ref()
                    .and_then(|p| Path::new(p).parent().map(|p| p.display().to_string())),
            );
        }
        self.drawing_pdf = state;
    }
}

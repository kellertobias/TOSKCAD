//! Source/resource-bound async phases. No detached reply is trusted by token alone.
use super::{backend::*, controller::Proposal};
use multicad_core::{
    commands::{Actor, Prepared, Request, Session},
    document::EntityId,
    project_file::{ContentHash, ProjectResources},
    sketch_external::{ExternalCatalog, ExternalQuery, OccurrenceCaptureRequest},
};
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Task {
    Catalog(ExternalQuery),
    Prepare {
        input: OccurrenceCaptureRequest,
        repair: bool,
    },
    Remove {
        sketch: EntityId,
        reference: EntityId,
    },
}
struct Pending {
    request: u64,
    task: Task,
    actor: Actor,
    ticket: Prepared,
    hash: ContentHash,
    resources: ProjectResources,
}
pub(super) enum Ready {
    Catalog(Box<ExternalCatalog>),
    Proposal(Box<Proposal>),
}
#[derive(Default)]
pub(super) struct Queries {
    pub backend: Option<Box<dyn ExternalQueryBackend>>,
    pending: Option<Pending>,
}
impl Queries {
    pub fn set_backend(&mut self, backend: Box<dyn ExternalQueryBackend>) {
        self.cancel();
        self.backend = Some(backend);
    }
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
    pub fn cancel(&mut self) {
        if let Some(backend) = &mut self.backend {
            backend.cancel();
        }
        self.pending = None;
    }
    pub fn begin(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        actor: Actor,
        task: Task,
    ) -> Result<(), String> {
        let ticket = session
            .prepare(Request {
                expected_revision: session.revision(),
                actor: actor.clone(),
                label: "Guard occurrence reference query".into(),
                edits: vec![],
            })
            .map_err(|e| e.to_string())?;
        let hash = ContentHash::document(session.document())?;
        self.cancel();
        let backend = self
            .backend
            .as_mut()
            .ok_or("Occurrence reference query provider is unavailable")?;
        let snapshot = session.preparation_snapshot();
        let request = match &task {
            Task::Catalog(query) => {
                query.validate().map_err(|e| e.to_string())?;
                backend.request_catalog(snapshot, resources.clone(), query.clone())
            }
            Task::Prepare { input, repair } => backend.request_prepare(
                snapshot,
                resources.clone(),
                actor.clone(),
                input.clone(),
                *repair,
            ),
            Task::Remove { sketch, reference } => backend.request_remove(
                snapshot,
                resources.clone(),
                actor.clone(),
                *sketch,
                *reference,
            ),
        }?;
        self.pending = Some(Pending {
            request,
            task,
            actor,
            ticket,
            hash,
            resources: resources.clone(),
        });
        Ok(())
    }
    pub fn task(&self) -> Option<&Task> {
        self.pending.as_ref().map(|p| &p.task)
    }
    pub fn poll(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
    ) -> Option<Result<Ready, String>> {
        if self.pending.as_ref().is_some_and(|p| {
            session.validate_preview(&p.ticket).is_err() || p.resources != *resources
        }) {
            self.cancel();
            return Some(Err(
                "Source or project resources changed. Select current geometry and Preview again"
                    .into(),
            ));
        }
        let reply = self.backend.as_mut()?.poll()?;
        let token = match &reply {
            ExternalQueryReply::Catalog { request, .. }
            | ExternalQueryReply::Prepared { request, .. }
            | ExternalQueryReply::Removed { request, .. } => *request,
        };
        if self.pending.as_ref().is_none_or(|p| p.request != token) {
            return None;
        }
        let pending = self.pending.take().expect("matched pending query");
        Some((|| match (reply, &pending.task) {
            (ExternalQueryReply::Catalog { query, result, .. }, Task::Catalog(expected))
                if query == *expected =>
            {
                let catalog = *result?;
                catalog.validate().map_err(|e| e.to_string())?;
                if catalog.query != query
                    || catalog.document != session.document().id()
                    || catalog.revision != session.revision()
                    || catalog.source_hash != pending.hash
                {
                    return Err("Native result catalogue is foreign or stale".into());
                }
                Ok(Ready::Catalog(Box::new(catalog)))
            }
            (
                ExternalQueryReply::Prepared {
                    source_hash,
                    input,
                    repair,
                    result,
                    ..
                },
                Task::Prepare {
                    input: expected,
                    repair: expected_repair,
                },
            ) if input == *expected
                && repair == *expected_repair
                && source_hash == pending.hash =>
            {
                let value = *result?;
                checked(session, &pending, &value, input.query.reference)?;
                let preview = value.edit.preview.as_ref().expect("checked preview");
                let canonical = multicad_core::sketch_external::read_occurrence(
                    preview.document(),
                    input.query.reference,
                )
                .map_err(|e| e.to_string())?;
                if canonical != input {
                    return Err("Native preparation changed its canonical query, selector, behavior or output identities".into());
                }
                Ok(Ready::Proposal(Box::new(Proposal {
                    reference: value.reference,
                    edit: value.edit,
                    report: value.report,
                    label: if repair {
                        "Repair occurrence reference"
                    } else {
                        "Capture occurrence reference"
                    },
                })))
            }
            (
                ExternalQueryReply::Removed {
                    source_hash,
                    sketch,
                    reference,
                    result,
                    ..
                },
                Task::Remove {
                    sketch: expected_sketch,
                    reference: expected_reference,
                },
            ) if sketch == *expected_sketch
                && reference == *expected_reference
                && source_hash == pending.hash =>
            {
                let value = *result?;
                checked(session, &pending, &value, reference)?;
                if value
                    .edit
                    .preview
                    .as_ref()
                    .is_some_and(|p| p.document().get(reference).is_some())
                {
                    return Err("Native removal retained the reference being removed".into());
                }
                Ok(Ready::Proposal(Box::new(Proposal {
                    reference: value.reference,
                    edit: value.edit,
                    report: value.report,
                    label: "Remove occurrence reference",
                })))
            }
            _ => Err("Native reference reply does not match its exact pending request".into()),
        })())
    }
}
fn checked(
    session: &Session,
    pending: &Pending,
    value: &ExternalPrepared,
    reference: EntityId,
) -> Result<(), String> {
    if value.reference != reference || value.report.revision != session.revision() {
        return Err("Native reference preparation changed its owner or report revision".into());
    }
    let preview = value
        .edit
        .preview
        .as_ref()
        .ok_or("Native reference edit has no checked consumer solution")?;
    session
        .validate_preview(preview)
        .map_err(|e| e.to_string())?;
    if preview.actor() != &pending.actor {
        return Err("Native reference preparation changed its actor".into());
    }
    Ok(())
}

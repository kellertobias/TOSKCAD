//! Dedicated source inspection and full-candidate validation. Neither result is
//! ever published into the canonical SceneFeed.
use super::controller::{Context, Controller, Purpose};
use crate::viewport::{Scene, SceneBackend, SceneScope, SceneStamp};
use multicad_core::{
    commands::{Prepared, Session},
    document::{EntityData, EntityId},
    project_file::{ContentHash, ProjectResources},
    sketch_external,
};
use std::sync::Arc;
#[derive(Default)]
pub(super) struct Native {
    pub backend: Option<Box<dyn SceneBackend>>,
    generation: u64,
    pending: Option<Job>,
    pub scene: Option<Arc<Scene>>,
    pub validation: Option<Result<(), String>>,
}
struct Job {
    stamp: SceneStamp,
    hash: ContentHash,
    ticket: Prepared,
    kind: Kind,
}
enum Kind {
    Sources(Vec<EntityId>),
    Candidate,
}
impl Native {
    pub fn clear(&mut self) {
        if self.pending.is_some()
            && let Some(backend) = self.backend.as_mut()
        {
            backend.cancel();
        }
        self.pending = None;
        self.scene = None;
        self.validation = None;
    }
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
    fn stamp(&mut self, session: &Session) -> Result<SceneStamp, String> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("Reference request identity exhausted")?;
        Ok(SceneStamp {
            document: session.document().id(),
            revision: session.revision(),
            generation: self.generation,
        })
    }
    pub fn validate_candidate(
        &mut self,
        session: &Session,
        prepared: &Prepared,
        resources: &ProjectResources,
    ) -> Result<(), String> {
        self.validation = None;
        self.pending = None;
        session
            .validate_preview(prepared)
            .map_err(|e| e.to_string())?;
        let stamp = self.stamp(session)?;
        let hash = ContentHash::document(prepared.document())?;
        self.backend
            .as_mut()
            .ok_or("Native reference validation provider is unavailable")?
            .request_project(prepared.document(), resources, stamp)?;
        self.pending = Some(Job {
            stamp,
            hash,
            ticket: prepared.clone(),
            kind: Kind::Candidate,
        });
        Ok(())
    }
    pub fn poll(&mut self, session: &Session) -> Option<Result<(), String>> {
        let reply = self.backend.as_mut()?.poll()?;
        let expected = self.pending.as_ref()?;
        if reply.stamp != expected.stamp {
            return None;
        }
        let job = self.pending.take()?;
        let result = (|| {
            session
                .validate_preview(&job.ticket)
                .map_err(|e| e.to_string())?;
            let scene = reply.result?;
            if scene.stamp() != job.stamp
                || scene
                    .model_evaluation()
                    .is_none_or(|info| info.source_hash != job.hash)
            {
                return Err("Native reference result is foreign or stale".into());
            }
            match &job.kind {
                Kind::Sources(roots) => {
                    if scene.scope() != &SceneScope::Sources(roots.clone()) {
                        return Err("Native source inspection returned the wrong root scope".into());
                    }
                }
                Kind::Candidate => {
                    if !scene.is_full_model() {
                        return Err(
                            "A partial source scene cannot authorize reference Apply".into()
                        );
                    }
                }
            }
            Ok(scene)
        })();
        match job.kind {
            Kind::Sources(_) => match result {
                Ok(scene) => {
                    self.scene = Some(scene);
                    None
                }
                Err(error) => {
                    self.scene = None;
                    Some(Err(error))
                }
            },
            Kind::Candidate => {
                let result = result.map(|_| ());
                self.validation = Some(result.clone());
                Some(result)
            }
        }
    }
}
impl Controller {
    pub fn set_backend(&mut self, backend: Box<dyn SceneBackend>) {
        self.native = Native {
            backend: Some(backend),
            ..Default::default()
        };
    }
    pub fn has_backend(&self) -> bool {
        self.native.backend.is_some()
    }
    pub fn native_pending(&self) -> bool {
        self.native.pending()
    }
    pub fn poll_native(&mut self, session: &Session) -> Option<Result<(), String>> {
        let result = self.native.poll(session);
        if let Some(Err(error)) = &result {
            self.error = Some(error.clone());
        }
        result
    }
    pub(super) fn can_start_repair(&self, session: &Session) -> bool {
        self.context.as_ref().is_some_and(|context| {
            context.pose == Default::default()
                && session.validate_preview(&context.ticket).is_ok()
                && (context.scene.is_some() || self.has_backend())
        })
    }
    pub(super) fn request_source(
        &mut self,
        session: &Session,
        replacement: EntityId,
        resources: &ProjectResources,
    ) -> Result<(), String> {
        let context = self.context.as_ref().ok_or("Open the owning sketch")?;
        session
            .validate_preview(&context.ticket)
            .map_err(|e| e.to_string())?;
        let active = self.active.as_ref().ok_or("Start Repair reference")?;
        session
            .validate_preview(&active.ticket)
            .map_err(|e| e.to_string())?;
        let Purpose::Repair(reference) = active.purpose else {
            return Err(
                "Source recovery is only available while repairing an existing reference".into(),
            );
        };
        let EntityData::Sketch { plane, .. } = &session
            .document()
            .get(context.sketch)
            .ok_or("Sketch missing")?
            .data
        else {
            return Err("Not a sketch".into());
        };
        if !matches!(
            session.document().get(replacement).map(|e| &e.data),
            Some(EntityData::Feature { .. })
        ) {
            return Err("Select a canonical replacement Feature".into());
        }
        let mut roots = std::collections::BTreeSet::from([*plane, replacement]);
        for binding in sketch_external::bindings(session.document(), context.sketch)
            .map_err(|e| e.to_string())?
        {
            if binding.reference != reference {
                roots.insert(binding.source);
            }
        }
        if roots.len() > 128 {
            return Err("Repair source dependency root budget exceeded".into());
        }
        let roots: Vec<_> = roots.into_iter().collect();
        let ticket = context.ticket.clone();
        let hash = context.hash;
        let stamp = self.native.stamp(session)?;
        self.native.scene = None;
        self.native.validation = None;
        self.native.pending = None;
        let context = self.context.as_mut().expect("checked context");
        context.scene = None;
        context.snapshots.clear();
        let active = self.active.as_mut().expect("checked active");
        active.selected = None;
        active.request = None;
        active.inspected_source = Some(replacement);
        self.native
            .backend
            .as_mut()
            .ok_or("Source inspection provider is unavailable")?
            .request_sources(session.document(), resources, stamp, &roots)?;
        self.native.pending = Some(Job {
            stamp,
            hash,
            ticket,
            kind: Kind::Sources(roots),
        });
        self.error = None;
        Ok(())
    }
    pub(crate) fn validate_candidate(
        &mut self,
        session: &Session,
        prepared: &Prepared,
        resources: &ProjectResources,
    ) -> Result<(), String> {
        self.native.validate_candidate(session, prepared, resources)
    }
    pub(crate) fn candidate_validation(&self) -> Option<&Result<(), String>> {
        self.native.validation.as_ref()
    }
}
pub(super) fn current_source(
    native: &Native,
    context: &Context,
    session: &Session,
) -> Option<Arc<Scene>> {
    native
        .scene
        .as_ref()
        .filter(|scene| {
            scene.stamp().document == session.document().id()
                && scene.stamp().revision == session.revision()
                && scene
                    .model_evaluation()
                    .is_some_and(|info| info.source_hash == context.hash)
        })
        .cloned()
}

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;

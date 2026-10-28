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
    scene_resources: Option<ProjectResources>,
    validation_resources: Option<ProjectResources>,
}
struct Job {
    stamp: SceneStamp,
    hash: ContentHash,
    ticket: Prepared,
    kind: Kind,
    resources: ProjectResources,
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
        self.scene_resources = None;
        self.validation_resources = None;
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
        self.validation_resources = None;
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
            resources: resources.clone(),
        });
        Ok(())
    }
    fn resources_current(&self, resources: &ProjectResources) -> bool {
        self.pending
            .as_ref()
            .is_none_or(|job| &job.resources == resources)
            && self
                .scene_resources
                .as_ref()
                .is_none_or(|old| old == resources)
            && self
                .validation_resources
                .as_ref()
                .is_none_or(|old| old == resources)
    }
    pub fn poll(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
    ) -> Option<Result<(), String>> {
        if !self.resources_current(resources) {
            let candidate = self.validation.is_some()
                || self
                    .pending
                    .as_ref()
                    .is_some_and(|job| matches!(job.kind, Kind::Candidate));
            let error =
                "Project resources changed. Load current sources and Preview again".to_string();
            self.clear();
            if candidate {
                self.validation = Some(Err(error.clone()));
                self.validation_resources = Some(resources.clone());
            }
            return Some(Err(error));
        }
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
                    let report = scene
                        .native_reference_report(job.hash)
                        .cloned()
                        .unwrap_or_else(|| {
                            multicad_core::references::inspect(
                                job.ticket.document(),
                                session.revision(),
                                &scene.reference_snapshots(),
                            )
                        });
                    if !report.for_model(job.ticket.document())?.all_resolved() {
                        return Err("Complete native reference validation has unresolved driving references".into());
                    }
                }
            }
            Ok(scene)
        })();
        match job.kind {
            Kind::Sources(_) => match result {
                Ok(scene) => {
                    self.scene = Some(scene);
                    self.scene_resources = Some(job.resources);
                    None
                }
                Err(error) => {
                    self.scene = None;
                    self.scene_resources = None;
                    Some(Err(error))
                }
            },
            Kind::Candidate => {
                let result = result.map(|_| ());
                self.validation = Some(result.clone());
                self.validation_resources = Some(job.resources);
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
        self.native.pending() || self.query_pending()
    }
    pub fn poll_native(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
    ) -> Option<Result<(), String>> {
        let changed = !self.native.resources_current(resources);
        let result = self.native.poll(session, resources);
        if changed {
            if let Some(context) = &mut self.context {
                context.scene = None;
                context.snapshots.clear();
            }
            if let Some(active) = &mut self.active {
                active.selected = None;
                active.request = None;
            }
            self.hover = None;
        }
        if let Some(Err(error)) = &result {
            self.error = Some(error.clone());
        }
        result
    }
    pub(super) fn can_start_repair(&self, session: &Session) -> bool {
        self.context.as_ref().is_some_and(|context| {
            (self.has_query_backend() || context.pose == Default::default())
                && session.validate_preview(&context.ticket).is_ok()
                && (context.scene.is_some() || self.has_backend() || self.has_query_backend())
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
        let reference =
            match active.purpose {
                Purpose::Repair(reference) => Some(reference),
                Purpose::Capture if active.form.is_some() => None,
                _ => return Err(
                    "Source recovery is only available for a current reference capture or repair"
                        .into(),
                ),
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
        ) && !(active.form.is_some()
            && matches!(
                session.document().get(replacement).map(|e| &e.data),
                Some(EntityData::Part { .. })
            ))
        {
            return Err("Select a canonical source Part or retained Feature".into());
        }
        let mut roots = std::collections::BTreeSet::from([*plane, replacement]);
        for binding in sketch_external::bindings(session.document(), context.sketch)
            .map_err(|e| e.to_string())?
        {
            if Some(binding.reference) != reference {
                roots.insert(binding.source);
            }
        }
        for binding in sketch_external::occurrence_bindings(session.document(), context.sketch)
            .map_err(|e| e.to_string())?
        {
            if Some(binding.query.reference) != reference {
                roots.insert(binding.query.source.source);
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
            resources: resources.clone(),
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

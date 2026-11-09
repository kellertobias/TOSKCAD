//! A private native evaluation lane for uncommitted shared definitions.
use super::*;
use crate::viewport::{Scene, SceneBackend, SceneStamp};
use multicad_core::project_file::{ContentHash, ProjectResources};
use std::sync::Arc;

struct Candidate {
    prepared: Prepared,
    draft: Draft,
    resources: ProjectResources,
    content: ContentHash,
    stamp: SceneStamp,
    channel: Channel,
    scene: Option<Arc<Scene>>,
}
#[derive(Default)]
pub(super) struct Lane {
    backend: Option<Box<dyn SceneBackend>>,
    candidate: Option<Candidate>,
    generation: u64,
}
impl Lane {
    pub(super) fn clear(&mut self) {
        self.candidate = None;
        if let Some(backend) = &mut self.backend {
            backend.cancel();
        }
    }
}
impl LibraryEditor {
    pub fn set_backend(&mut self, backend: Box<dyn SceneBackend>) {
        self.preview.clear();
        self.confirm_on_ready = None;
        self.preview.backend = Some(backend);
    }
    pub fn has_backend(&self) -> bool {
        self.preview.backend.is_some()
    }
    fn candidate_current(&self, s: &Session, resources: &ProjectResources) -> bool {
        self.preview.candidate.as_ref().is_some_and(|candidate| {
            self.draft.as_ref() == Some(&candidate.draft)
                && &candidate.resources == resources
                && s.validate_preview(&candidate.prepared).is_ok()
        })
    }
    pub fn can_apply(&self, s: &Session, resources: &ProjectResources) -> bool {
        self.dirty()
            && self.candidate_current(s, resources)
            && self
                .preview
                .candidate
                .as_ref()
                .is_some_and(|c| c.scene.is_some())
    }
    pub fn can_preview(&self, s: &Session) -> bool {
        self.can_confirm(s) && self.preview.backend.is_some()
    }
    pub(crate) fn can_confirm(&self, s: &Session) -> bool {
        self.dirty()
            && self.valid_panel_draft()
            && self.confirm_on_ready.is_none()
            && self
                .guard
                .as_ref()
                .is_some_and(|g| s.validate_stamp(g).is_ok())
    }
    pub fn pending(&self) -> bool {
        self.preview
            .candidate
            .as_ref()
            .is_some_and(|c| c.scene.is_none())
    }
    pub fn preview_scene(&self, s: &Session, resources: &ProjectResources) -> Option<&Arc<Scene>> {
        self.candidate_current(s, resources)
            .then(|| self.preview.candidate.as_ref()?.scene.as_ref())
            .flatten()
    }
    pub fn refresh_preview(
        &mut self,
        s: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        self.preview.clear();
        self.confirm_on_ready = None;
        self.error = None;
        let result = self
            .prepare_checked(s, channel)
            .and_then(|prepared| self.request_native(s, resources, prepared, channel));
        if let Err(error) = &result {
            self.error = Some(error.clone());
        }
        result
    }

    fn prepare_checked(&self, s: &Session, channel: Channel) -> Result<Prepared, String> {
        let prepared = self.prepare(s, channel)?;
        let previous = diagnostics(s.document()).map_err(|e| e.to_string())?;
        let issues = diagnostics(prepared.document()).map_err(|e| e.to_string())?;
        if let Some(issue) = issues.iter().find(|issue| {
            matches!(
                issue.kind,
                DiagnosticKind::Missing
                    | DiagnosticKind::WrongKind
                    | DiagnosticKind::MaterialMismatch
            ) && !previous.contains(issue)
        }) {
            let owner = prepared
                .document()
                .get(issue.owner)
                .map_or("Dependent definition", |e| e.name.as_str());
            return Err(format!("{owner}: {}", issue.message));
        }
        Ok(prepared)
    }

    fn request_native(
        &mut self,
        s: &Session,
        resources: &ProjectResources,
        prepared: Prepared,
        channel: Channel,
    ) -> Result<(), String> {
        let draft = self.draft.clone().ok_or("Choose a definition first")?;
        let content = ContentHash::document(prepared.document())?;
        self.preview.generation = self
            .preview
            .generation
            .checked_add(1)
            .ok_or("Material preview generation limit reached")?;
        let stamp = SceneStamp {
            document: s.document().id(),
            revision: s.revision(),
            generation: self.preview.generation,
        };
        self.preview
            .backend
            .as_mut()
            .ok_or("Native validation is unavailable for a definition used by the model")?
            .request_evaluation(prepared.document(), resources, stamp, &Default::default())?;
        self.preview.candidate = Some(Candidate {
            prepared,
            draft,
            resources: resources.clone(),
            content,
            stamp,
            channel,
            scene: None,
        });
        Ok(())
    }

    pub(crate) fn confirm(
        &mut self,
        s: &mut Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        if self.candidate_current(s, resources)
            && self
                .preview
                .candidate
                .as_ref()
                .is_some_and(|c| c.channel == channel)
        {
            if self.pending() {
                self.confirm_on_ready = Some(channel);
                return Ok(());
            }
            return self.apply(s, resources, channel);
        }
        self.preview.clear();
        self.error = None;
        let prepared = self.prepare_checked(s, channel)?;
        let draft = self.draft.as_ref().ok_or("Choose a definition first")?;
        // Only stock dimensions and associations feed native geometry. New
        // stock and renames do not change any existing dependent geometry.
        let needs_native = self.original.as_ref().is_some_and(|original| {
            original.entity.data != draft.entity.data
                && matches!(read(&draft.entity), Ok(Some(LibraryDefinition::Stock(_))))
                && s.document().entities().any(|entity| {
                    entity
                        .references()
                        .iter()
                        .any(|link| link.target == draft.entity.id)
                })
        });
        if needs_native {
            self.request_native(s, resources, prepared, channel)?;
            self.confirm_on_ready = Some(channel);
        } else {
            self.commit(s, prepared, draft.entity.clone())?;
        }
        Ok(())
    }
    pub fn poll(&mut self, s: &Session, resources: &ProjectResources) {
        if self.preview.candidate.is_some() && !self.candidate_current(s, resources) {
            self.preview.clear();
            self.error =
                Some("Definition or project changed. Apply the current draft again.".into());
        }
        for _ in 0..16 {
            let Some(reply) = self.preview.backend.as_mut().and_then(|b| b.poll()) else {
                break;
            };
            let Some(candidate) = self.preview.candidate.as_ref() else {
                continue;
            };
            if candidate.stamp != reply.stamp || candidate.scene.is_some() {
                continue;
            }
            let result = (|| {
                let scene = reply.result?;
                if scene.stamp() != candidate.stamp || !scene.is_full_model() {
                    return Err(
                        "Native material preview returned a foreign or partial scene".into(),
                    );
                }
                let info = scene
                    .model_evaluation()
                    .ok_or("Material edits require source-bound native evaluation metadata")?;
                if info.source_hash != candidate.content {
                    return Err(
                        "Native material preview belongs to different source content".into(),
                    );
                }
                Ok(scene)
            })();
            match result {
                Ok(scene) => {
                    self.preview
                        .candidate
                        .as_mut()
                        .expect("matching candidate")
                        .scene = Some(scene);
                    self.error = None;
                }
                Err(error) => {
                    self.preview.clear();
                    self.error = Some(error);
                }
            }
        }
    }
    pub fn apply(
        &mut self,
        s: &mut Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.can_apply(s, resources) {
            return Err(
                "Preview the current material or stock draft successfully before Apply".into(),
            );
        }
        if self
            .preview
            .candidate
            .as_ref()
            .is_none_or(|c| c.channel != channel)
        {
            return Err("Preview and Apply must use the same control channel".into());
        }
        let candidate = self.preview.candidate.take().expect("checked candidate");
        self.commit(s, candidate.prepared, candidate.draft.entity)
    }
    fn commit(
        &mut self,
        s: &mut Session,
        prepared: Prepared,
        entity: Entity,
    ) -> Result<(), String> {
        s.commit(prepared).map_err(|e| e.to_string())?;
        self.select(s, entity);
        Ok(())
    }
    pub fn discard(&mut self, s: &Session) {
        self.preview.clear();
        self.confirm_on_ready = None;
        let selected = self
            .original
            .as_ref()
            .and_then(|d| s.document().get(d.entity.id))
            .cloned();
        self.draft = None;
        self.original = None;
        self.error = None;
        self.document = Some(s.document().id());
        self.revision = s.revision();
        self.guard = Some(s.stamp());
        if let Some(entity) = selected {
            self.select(s, entity);
        }
    }
}

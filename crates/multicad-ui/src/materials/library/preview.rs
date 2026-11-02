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
    pub(super) show_model: bool,
    camera: crate::camera::Camera,
    solid: crate::viewport::Interaction,
    fit: bool,
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
        self.dirty()
            && self.preview.backend.is_some()
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
        self.error = None;
        let result: Result<(), String> = (|| {
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
                .ok_or("The native material preview provider is unavailable")?
                .request_evaluation(prepared.document(), resources, stamp, &Default::default())?;
            self.preview.candidate = Some(Candidate {
                prepared,
                draft: self.draft.clone().ok_or("Choose a definition first")?,
                resources: resources.clone(),
                content,
                stamp,
                channel,
                scene: None,
            });
            self.preview.show_model = true;
            Ok(())
        })();
        if let Err(error) = &result {
            self.error = Some(error.clone());
        }
        result
    }
    pub fn poll(&mut self, s: &Session, resources: &ProjectResources) {
        if self.preview.candidate.is_some() && !self.candidate_current(s, resources) {
            self.preview.clear();
            self.error =
                Some("Definition or source changed. Preview the current draft again.".into());
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
                    self.preview.fit = true;
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
        s.commit(candidate.prepared).map_err(|e| e.to_string())?;
        self.select(s, candidate.draft.entity);
        Ok(())
    }
    pub fn discard(&mut self, s: &Session) {
        self.preview.clear();
        self.preview.show_model = false;
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
    pub(super) fn preview_status(&self, ui: &mut egui::Ui) {
        if self.pending() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Checking affected geometry with the native kernel…");
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        } else if self
            .preview
            .candidate
            .as_ref()
            .is_some_and(|c| c.scene.is_some())
        {
            ui.colored_label(
                Color32::LIGHT_GREEN,
                "Native preview valid · Apply commits one undoable edit",
            );
        } else if self.dirty() && self.error.is_none() {
            ui.label("Preview this draft to check affected geometry before Apply.");
        }
        if self.preview.backend.is_none() {
            ui.colored_label(
                Color32::LIGHT_YELLOW,
                "Native preview is unavailable; definition edits cannot be applied.",
            );
        }
    }
    pub(super) fn show_model(&mut self, ui: &mut egui::Ui) {
        let Some(scene) = self
            .preview
            .candidate
            .as_ref()
            .and_then(|c| c.scene.clone())
        else {
            ui.label("Use Preview to inspect the affected model. The committed document stays unchanged.");
            return;
        };
        ui.small("Read-only native preview · drag to orbit · scroll to zoom");
        if let Some(bounds) = scene.world_bounds() {
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), ui.available_height().max(120.)),
                egui::Sense::drag(),
            );
            if self.preview.fit {
                let corners: Vec<_> = (0..8)
                    .map(|bits| std::array::from_fn(|axis| bounds[(bits >> axis) & 1][axis] as f32))
                    .collect();
                self.preview.camera.fit(&corners, rect);
                self.preview.fit = false;
            }
            if response.dragged() {
                self.preview.camera.orbit(response.drag_delta());
            }
            if response.hovered() {
                self.preview
                    .camera
                    .zoom(ui.input(|i| i.smooth_scroll_delta.y));
            }
            ui.painter().rect_filled(rect, 0., crate::theme::SURFACE);
            self.preview.solid.show(
                ui,
                &response,
                scene,
                self.preview.camera,
                crate::navigation::DisplayStyle::VisibleEdges,
                false,
            );
        } else {
            ui.label(
                "No model Parts yet. The material and stock definitions passed native validation.",
            );
        }
    }
}

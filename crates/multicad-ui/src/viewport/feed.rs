//! Per-document, nonblocking display publication. Source compilation is supplied
//! by the native composition root; the UI never owns geometry kernel handles.
use super::{DisplayQuality, Refinement, Scene, SceneStamp, SceneStore};
use multicad_core::{
    document::{Document, EntityId},
    project_file::ProjectResources,
    references::{ReferenceGate, ReferenceReport, TopologySnapshot, inspect},
};
use std::{collections::BTreeMap, sync::Arc};
pub struct ReferenceEvaluation {
    pub stamp: SceneStamp,
    pub report: ReferenceReport,
    pub snapshots: BTreeMap<EntityId, TopologySnapshot>,
    pub candidate: Arc<Scene>,
}
pub struct SceneReply {
    pub stamp: SceneStamp,
    pub result: Result<Arc<Scene>, String>,
}
pub trait SceneBackend {
    /// Accept a source snapshot for evaluation outside the UI event loop.
    fn request(&mut self, document: &Document, stamp: SceneStamp) -> Result<(), String>;
    /// Resource-aware providers receive immutable critical payloads and cache guards.
    /// Legacy source-only providers retain their explicit implementation.
    fn request_project(
        &mut self,
        document: &Document,
        _resources: &ProjectResources,
        stamp: SceneStamp,
    ) -> Result<(), String> {
        self.request(document, stamp)
    }
    /// Evaluate source and optional typed native queries on the same worker.
    /// Existing providers remain usable for requests without query probes.
    fn request_evaluation(
        &mut self,
        document: &Document,
        resources: &ProjectResources,
        stamp: SceneStamp,
        options: &multicad_core::model::EvaluationOptions,
    ) -> Result<(), String> {
        options.validate().map_err(|error| error.to_string())?;
        if options.sheet_development.is_some() {
            return Err(
                "This display provider does not support native sheet development checks".into(),
            );
        }
        if !options.cut_targets.is_empty() {
            return Err("This display provider does not support native cut-target queries".into());
        }
        self.request_project(document, resources, stamp)
    }
    /// Separate current-source inspection. The returned Scene must be marked
    /// Sources and must never enter the canonical display/Apply publication lane.
    fn request_sources(
        &mut self,
        _document: &Document,
        _resources: &ProjectResources,
        _stamp: SceneStamp,
        _roots: &[EntityId],
    ) -> Result<(), String> {
        Err("This native provider does not support source-only repair inspection".into())
    }
    /// Drop queued work and obsolete replies without waiting for a native call.
    fn cancel(&mut self) {}
    /// Never waits. Replies retain their request stamp, including failures.
    fn poll(&mut self) -> Option<SceneReply>;
    /// Optional display-only refinement of the current retained exact geometry.
    fn refine(
        &mut self,
        _stamp: SceneStamp,
        _quality: DisplayQuality,
    ) -> Result<Refinement, String> {
        Ok(Refinement::Unsupported)
    }
}
pub type BackendFactory = Arc<dyn Fn() -> Box<dyn SceneBackend>>;
#[derive(Default)]
pub struct SceneFeed {
    backend: Option<Box<dyn SceneBackend>>,
    store: SceneStore,
    last_source: Option<(EntityId, u64)>,
    generation: u64,
    pending: bool,
    error: Option<String>,
    expected: Option<SceneStamp>,
    source_valid: bool,
    refining: bool,
    quality_error: Option<String>,
    quality_unsupported: bool,
    demand: Option<(DisplayQuality, f64)>,
    requested_quality: Option<DisplayQuality>,
    references: Option<ReferenceEvaluation>,
    reference_gate: ReferenceGate<Arc<Scene>>,
}
impl SceneFeed {
    pub fn set_backend(&mut self, backend: Box<dyn SceneBackend>) {
        self.backend = Some(backend);
        self.last_source = None;
        self.pending = false;
        self.error = None;
        self.source_valid = false;
        self.quality_unsupported = false;
        self.requested_quality = None;
        self.references = None;
        self.reference_gate = ReferenceGate::default();
    }
    pub fn has_backend(&self) -> bool {
        self.backend.is_some()
    }
    pub fn visible(&self) -> Option<&Arc<Scene>> {
        self.store.visible()
    }
    pub fn references(&self) -> Option<&ReferenceEvaluation> {
        self.references
            .as_ref()
            .filter(|e| self.last_source == Some((e.stamp.document, e.stamp.revision)))
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn pending(&self) -> bool {
        self.pending
    }
    pub fn refining(&self) -> bool {
        self.pending && self.refining
    }
    pub fn quality_error(&self) -> Option<&str> {
        self.quality_error.as_deref()
    }
    pub fn note_quality(&mut self, quality: DisplayQuality, now: f64) {
        if !now.is_finite() {
            return;
        }
        if self.demand.is_none_or(|(old, _)| old != quality) {
            let delay = if self.requested_quality.is_some_and(|old| old > quality) {
                0.6
            } else {
                0.15
            };
            self.demand = Some((quality, now + delay));
        }
    }
    pub fn retry_quality(&mut self) {
        self.requested_quality = None;
        self.quality_error = None;
    }
    pub fn quality_wakeup(&self, now: f64) -> Option<std::time::Duration> {
        if !now.is_finite() {
            return None;
        }
        let (quality, due) = self.demand?;
        (self.backend.is_some()
            && self.source_valid
            && !self.pending
            && !self.quality_unsupported
            && self.requested_quality != Some(quality))
        .then(|| std::time::Duration::from_secs_f64((due - now).clamp(0., 0.6)))
    }
    pub fn is_current(&self, document: EntityId, revision: u64) -> bool {
        self.source_valid
            && self.error.is_none()
            && self
                .visible()
                .is_some_and(|s| s.stamp().document == document && s.stamp().revision == revision)
    }
    pub fn retry(&mut self) {
        self.last_source = None;
    }
    pub fn tick(&mut self, document: &Document, revision: u64) {
        self.tick_at(document, revision, 0.);
    }
    pub fn tick_at(&mut self, document: &Document, revision: u64, now: f64) {
        self.tick_project(document, &ProjectResources::default(), revision, now);
    }
    pub fn tick_project(
        &mut self,
        document: &Document,
        resources: &ProjectResources,
        revision: u64,
        now: f64,
    ) {
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        let source = (document.id(), revision);
        if self.last_source != Some(source) {
            let Some(generation) = self.generation.checked_add(1) else {
                self.error = Some("Display generation limit reached".into());
                return;
            };
            self.generation = generation;
            let stamp = SceneStamp {
                document: document.id(),
                revision,
                generation,
            };
            self.last_source = Some(source);
            self.expected = Some(stamp);
            self.error = None;
            self.source_valid = false;
            self.refining = false;
            self.quality_error = None;
            self.requested_quality = None;
            self.quality_unsupported = false;
            if let Err(error) = self
                .store
                .begin(stamp)
                .and_then(|()| backend.request_project(document, resources, stamp))
            {
                self.error = Some(error);
                self.pending = false;
                return;
            }
            self.pending = true;
        }
        if !self.pending
            && self.source_valid
            && !self.quality_unsupported
            && let Some((quality, due)) = self.demand
            && now >= due
            && self.requested_quality != Some(quality)
        {
            self.requested_quality = Some(quality);
            let Some(generation) = self.generation.checked_add(1) else {
                self.quality_error = Some("Display generation limit reached".into());
                return;
            };
            self.generation = generation;
            let stamp = SceneStamp {
                document: document.id(),
                revision,
                generation,
            };
            match backend.refine(stamp, quality) {
                Ok(Refinement::Started) => {
                    if let Err(error) = self.store.begin(stamp) {
                        self.quality_error = Some(error);
                        return;
                    }
                    self.expected = Some(stamp);
                    self.pending = true;
                    self.refining = true;
                    self.quality_error = None;
                }
                Ok(Refinement::Unchanged) => self.quality_error = None,
                Ok(Refinement::Unsupported) => self.quality_unsupported = true,
                Err(error) => self.quality_error = Some(error),
            }
        }
        if !self.pending {
            return;
        }
        if let Some(reply) = backend.poll() {
            if self.expected != Some(reply.stamp) {
                return;
            }
            self.pending = false;
            match reply.result {
                Ok(scene) if scene.stamp() == reply.stamp && scene.is_full_model() => {
                    let source = match multicad_core::project_file::ContentHash::document(document)
                    {
                        Ok(source) => source,
                        Err(error) => {
                            self.error = Some(error);
                            return;
                        }
                    };
                    if scene
                        .model_evaluation()
                        .is_some_and(|info| info.source_hash != source)
                    {
                        self.error =
                            Some("Native scene belongs to a different source snapshot".into());
                        return;
                    }
                    let has_references = document
                        .entities()
                        .any(|e| e.references().iter().any(|l| l.geometry.is_some()));
                    let snapshots = if has_references {
                        scene.reference_snapshots()
                    } else {
                        BTreeMap::new()
                    };
                    let report = scene
                        .native_reference_report(source)
                        .cloned()
                        .unwrap_or_else(|| inspect(document, revision, &snapshots));
                    let driving = report.for_model(document);
                    let driving_resolved =
                        driving.as_ref().is_ok_and(ReferenceReport::all_resolved);
                    let approved = driving.and_then(|driving| {
                        self.reference_gate
                            .consider(revision, scene.clone(), driving)
                    });
                    if !self.refining || driving_resolved {
                        self.references = Some(ReferenceEvaluation {
                            stamp: reply.stamp,
                            report,
                            snapshots,
                            candidate: scene.clone(),
                        });
                    }
                    let publication=approved.and_then(|valid|if valid {self.store.publish(scene)} else {
                        Err("Geometry references need repair. The previous valid model is retained.".into())
                    });
                    if let Err(error) = publication {
                        if self.refining {
                            self.quality_error = Some(error)
                        } else {
                            self.error = Some(error)
                        }
                    } else {
                        self.source_valid = true;
                    }
                }
                Ok(_) => {
                    let error = "Display backend returned a mismatched stamp or an incomplete source-inspection scene".into();
                    if self.refining {
                        self.quality_error = Some(error)
                    } else {
                        self.error = Some(error)
                    }
                }
                Err(error) => {
                    if self.refining {
                        self.quality_error = Some(error)
                    } else {
                        self.error = Some(error)
                    }
                }
            }
            self.refining = false;
        }
    }
}

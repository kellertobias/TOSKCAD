//! Per-document, nonblocking display publication. Source compilation is supplied
//! by the native composition root; the UI never owns geometry kernel handles.
use super::{DisplayQuality, Refinement, Scene, SceneStamp, SceneStore};
use multicad_core::{
    document::{Document, EntityId},
    project_file::ProjectResources,
    references::{ReferenceGate, ReferenceReport, TopologySnapshot, inspect},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};
pub struct ReferenceEvaluation {
    pub stamp: SceneStamp,
    pub report: ReferenceReport,
    pub(crate) snapshots: OnceLock<BTreeMap<EntityId, TopologySnapshot>>,
    pub candidate: Arc<Scene>,
}
impl ReferenceEvaluation {
    pub fn snapshots(&self) -> &BTreeMap<EntityId, TopologySnapshot> {
        self.snapshots
            .get_or_init(|| self.candidate.reference_snapshots())
    }
}
pub struct SceneReply {
    pub stamp: SceneStamp,
    pub result: Result<Arc<Scene>, String>,
}
pub trait SceneBackend {
    /// Accept a source snapshot for evaluation outside the UI event loop.
    fn request(&mut self, document: &Document, stamp: SceneStamp) -> Result<(), String>;
    /// Whether hardware edits can use the short interactive preview debounce.
    fn supports_fast_hardware_preview(&self) -> bool {
        false
    }
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
    requested_source: Option<(Document, ProjectResources)>,
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
        self.requested_source = None;
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
    /// Publish a complete native preview whose evaluated source was committed.
    /// The controller verifies the transaction and resources. The feed checks
    /// the source and reference receipts again before enabling canonical picks.
    pub(crate) fn publish_committed_preview(
        &mut self,
        document: &Document,
        resources: &ProjectResources,
        revision: u64,
        preview: &Scene,
    ) -> Option<()> {
        let identity = (document.id(), revision);
        let previous = preview.stamp();
        if self.last_source == Some(identity)
            || previous.document != identity.0
            || previous.revision.checked_add(1) != Some(revision)
        {
            return None;
        }
        let backend = self.backend.as_mut()?;
        let source = multicad_core::project_file::ContentHash::document(document).ok()?;
        if preview.model_evaluation()?.source_hash != source {
            return None;
        }
        let generation = self.generation.checked_add(1)?;
        let stamp = SceneStamp {
            document: identity.0,
            revision,
            generation,
        };
        let scene = Arc::new(preview.rebind_source(stamp, source)?);
        let report = scene.native_reference_report(source)?.clone();
        let driving = report.for_model(document).ok()?;
        if !driving.all_resolved() {
            return None;
        }
        self.store.begin(stamp).ok()?;
        if !self
            .reference_gate
            .consider(revision, scene.clone(), driving)
            .ok()?
        {
            return None;
        }
        self.store.publish(scene.clone()).ok()?;
        backend.cancel();
        self.generation = generation;
        self.last_source = Some(identity);
        self.requested_source = Some((document.clone(), resources.clone()));
        self.expected = Some(stamp);
        self.pending = false;
        self.error = None;
        self.source_valid = true;
        self.refining = false;
        self.quality_error = None;
        self.requested_quality = None;
        self.quality_unsupported = true;
        self.references = Some(ReferenceEvaluation {
            stamp,
            report,
            snapshots: OnceLock::new(),
            candidate: scene,
        });
        Some(())
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
            let reused = self
                .source_valid
                .then(|| self.requested_source.as_ref().zip(self.store.visible()))
                .flatten()
                .filter(|((before, prior_resources), _)| {
                    prior_resources == resources && before.differs_only_in_presentation(document)
                })
                .and_then(|(_, scene)| {
                    let source =
                        multicad_core::project_file::ContentHash::document(document).ok()?;
                    scene.rebind_source(stamp, source)
                });
            self.last_source = Some(source);
            self.expected = Some(stamp);
            self.requested_source = Some((document.clone(), resources.clone()));
            self.error = None;
            self.source_valid = false;
            self.refining = false;
            self.quality_error = None;
            self.requested_quality = None;
            self.quality_unsupported = false;
            if let Some(scene) = reused {
                backend.cancel();
                let scene = Arc::new(scene);
                let accepted = self
                    .store
                    .begin(stamp)
                    .and_then(|()| self.store.publish(scene.clone()));
                if accepted.is_ok() {
                    if let Some(evaluation) = &mut self.references {
                        evaluation.stamp = stamp;
                        evaluation.report.revision = revision;
                        if let Some(info) = scene.model_evaluation()
                            && let Some(report) = scene.native_reference_report(info.source_hash)
                        {
                            evaluation.report = report.clone();
                        }
                        evaluation.snapshots = OnceLock::new();
                        evaluation.candidate = scene;
                    }
                    self.pending = false;
                    self.source_valid = true;
                    self.quality_unsupported = true;
                    return;
                }
            }
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
                    let snapshots = OnceLock::new();
                    let report = scene
                        .native_reference_report(source)
                        .cloned()
                        .unwrap_or_else(|| {
                            // Source-only/test providers need topology to resolve
                            // authored intents. Native providers send that report
                            // already; build the replacement catalogue only when
                            // the operator opens reference repair.
                            let snapshots = snapshots.get_or_init(|| {
                                if document
                                    .entities()
                                    .any(|e| e.references().iter().any(|l| l.geometry.is_some()))
                                {
                                    scene.reference_snapshots()
                                } else {
                                    BTreeMap::new()
                                }
                            });
                            inspect(document, revision, snapshots)
                        });
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

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{
        document::{Entity, EntityData, Mutation},
        model::ModelEvaluationInfo,
        project_file::ContentHash,
    };
    use std::{cell::Cell, rc::Rc};

    struct Backend {
        requests: Rc<Cell<usize>>,
        reply: Option<SceneReply>,
    }

    impl SceneBackend for Backend {
        fn request(&mut self, document: &Document, stamp: SceneStamp) -> Result<(), String> {
            self.requests.set(self.requests.get() + 1);
            let hash = ContentHash::document(document)?;
            let scene = Scene::new(stamp, vec![], vec![])?
                .with_model_evaluation(Arc::new(ModelEvaluationInfo {
                    source_hash: hash,
                    consumed_bodies: Default::default(),
                    cut_targets: vec![],
                    screw_connection_checks: vec![],
                }))?
                .with_native_reference_report(
                    hash,
                    ReferenceReport {
                        revision: stamp.revision,
                        entries: vec![],
                        external_results: BTreeMap::new(),
                        external_errors: BTreeMap::new(),
                        sketch_supports: BTreeMap::new(),
                    },
                )?;
            self.reply = Some(SceneReply {
                stamp,
                result: Ok(Arc::new(scene)),
            });
            Ok(())
        }

        fn poll(&mut self) -> Option<SceneReply> {
            self.reply.take()
        }
    }

    #[test]
    fn committed_native_preview_publishes_without_rebuilding_and_rejects_stale_sources() {
        let original = Document::new("Stock").unwrap();
        let candidate = original
            .edited(&[Mutation::Insert(Entity::new(
                "New part",
                EntityData::Group {},
            ))])
            .unwrap();
        let resources = ProjectResources::default();
        let requests = Rc::new(Cell::new(0));
        let mut native = Backend {
            requests: requests.clone(),
            reply: None,
        };
        native
            .request(
                &candidate,
                SceneStamp {
                    document: candidate.id(),
                    revision: 0,
                    generation: 1,
                },
            )
            .unwrap();
        let preview = native.poll().unwrap().result.unwrap();
        let mut feed = SceneFeed::default();
        feed.set_backend(Box::new(native));
        feed.tick(&original, 0);
        assert_eq!(requests.get(), 2);
        assert!(
            feed.publish_committed_preview(&original, &resources, 1, &preview)
                .is_none()
        );
        assert!(
            feed.publish_committed_preview(&candidate, &resources, 2, &preview)
                .is_none()
        );
        assert!(feed.is_current(original.id(), 0));

        assert!(
            feed.publish_committed_preview(&candidate, &resources, 1, &preview)
                .is_some()
        );
        feed.tick_project(&candidate, &resources, 1, 0.);
        assert_eq!(
            requests.get(),
            2,
            "Apply must reuse the validated native preview"
        );
        assert!(feed.is_current(candidate.id(), 1));
        assert!(!feed.pending());
        let references = feed.references().unwrap();
        assert_eq!(references.stamp.revision, 1);
        assert_eq!(references.report.revision, 1);
        assert!(references.snapshots.get().is_none());

        feed.tick(&original, 2);
        assert_eq!(requests.get(), 3, "Undo is evaluated on the canonical lane");
        assert!(feed.is_current(original.id(), 2));
    }

    #[test]
    fn authored_visibility_and_file_info_reuse_current_geometry_but_model_changes_do_not() {
        let group = Entity::new("Drawers and slides", EntityData::Group {});
        let mut document = Document::new("Cabinet")
            .unwrap()
            .edited(&[Mutation::Insert(group.clone())])
            .unwrap();
        let requests = Rc::new(Cell::new(0));
        let mut feed = SceneFeed::default();
        feed.set_backend(Box::new(Backend {
            requests: requests.clone(),
            reply: None,
        }));
        feed.tick(&document, 0);
        assert_eq!(requests.get(), 1);
        assert!(feed.references().unwrap().snapshots.get().is_none());

        let mut hierarchy = document.hierarchy().clone();
        hierarchy.hidden.insert(group.id);
        document = document
            .edited(&[Mutation::SetHierarchy(hierarchy)])
            .unwrap();
        feed.tick(&document, 1);
        assert_eq!(
            requests.get(),
            1,
            "visibility must not call the native builder"
        );
        let info = multicad_core::project_file::info::FileInfo {
            name: "Shop cabinet".into(),
            description: "Fitted drawers".into(),
            ..Default::default()
        };
        document = document
            .edited(&multicad_core::project_file::info::edits(&document, &info).unwrap())
            .unwrap();
        feed.tick(&document, 2);
        assert_eq!(
            requests.get(),
            1,
            "metadata must not call the native builder"
        );
        assert!(feed.is_current(document.id(), 2));
        let hash = ContentHash::document(&document).unwrap();
        let scene = feed.visible().unwrap();
        assert_eq!(scene.model_evaluation().unwrap().source_hash, hash);
        assert_eq!(scene.native_reference_report(hash).unwrap().revision, 2);

        let mut edited = group;
        edited.name = "Different model entry".into();
        document = document.edited(&[Mutation::Replace(edited)]).unwrap();
        feed.tick(&document, 3);
        assert_eq!(requests.get(), 2);
        assert!(feed.is_current(document.id(), 3));
    }
}

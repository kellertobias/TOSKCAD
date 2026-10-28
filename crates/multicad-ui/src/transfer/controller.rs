use super::{
    TransferCaptureKind, TransferLane, TransferOutcome, TransferPick, TransferSelection,
    form::{self, Form, SourceMode},
};
use crate::{
    egui,
    inspector_actions::{self, Button, Owner},
    viewport::{Scene, SceneBackend, SceneStamp, VisibilityFilter},
};
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    document::{Document, EntityId},
    model::{self, CreatedBody, OccurrencePath},
    project_file::{ContentHash, ProjectResources},
    references::{self, AnalyticGeometry, GeometryIntent, Resolution, TopologyKind},
};
use std::sync::Arc;
struct Scope {
    guard: Prepared,
    source: ContentHash,
    resources: ProjectResources,
    channel: Channel,
}
struct Inspection {
    pick: TransferPick,
    stamp: SceneStamp,
    pending: bool,
    scene: Option<Arc<Scene>>,
}
struct Candidate {
    prepared: Prepared,
    source: ContentHash,
    stamp: SceneStamp,
    version: u64,
    outputs: Vec<CreatedBody>,
    pending: bool,
    scene: Option<Arc<Scene>>,
}
struct Handoff {
    document: EntityId,
    revision: u64,
    resources: ProjectResources,
    scene: Arc<Scene>,
}
#[derive(Default)]
pub struct TransferController {
    form: Option<Form>,
    scope: Option<Scope>,
    backend: Option<Box<dyn SceneBackend>>,
    candidate: Option<Candidate>,
    inspection: Option<Inspection>,
    handoff: Option<Handoff>,
    generation: u64,
    error: Option<String>,
}
impl TransferController {
    pub fn set_backend(&mut self, backend: Box<dyn SceneBackend>) {
        self.cancel();
        self.backend = Some(backend);
    }
    pub fn has_backend(&self) -> bool {
        self.backend.is_some()
    }
    pub fn active(&self) -> bool {
        self.form.is_some()
    }
    pub fn dirty(&self) -> bool {
        self.active()
    }
    pub fn label(&self) -> &'static str {
        "Transfer features"
    }
    pub fn pending(&self) -> bool {
        self.candidate.as_ref().is_some_and(|c| c.pending)
            || self.inspection.as_ref().is_some_and(|i| i.pending)
    }
    pub fn editable(document: &Document, feature: EntityId) -> bool {
        model::TransferRequest::existing(document, feature).is_ok()
    }
    pub fn begin(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: Option<&Arc<Scene>>,
        selection: TransferSelection,
        channel: Channel,
    ) -> Result<(), String> {
        if let Some(feature) = selection.feature {
            return self.begin_existing(session, resources, scene, feature, channel);
        }
        let scene = scene.filter(|s| current_scene(session, s));
        let form = Form::new(selection, session.document(), scene)?;
        self.open(session, resources, form, channel)
    }
    pub fn begin_existing(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: Option<&Arc<Scene>>,
        feature: EntityId,
        channel: Channel,
    ) -> Result<(), String> {
        let request = model::TransferRequest::existing(session.document(), feature)
            .map_err(|e| e.to_string())?;
        let scene = scene.filter(|s| current_scene(session, s));
        let form = Form::from_request(request, session.document(), scene)?;
        self.open(session, resources, form, channel)
    }
    fn open(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        form: Form,
        channel: Channel,
    ) -> Result<(), String> {
        if self.active() {
            return Err("Apply or cancel the active transfer first".into());
        }
        if !self.has_backend() {
            return Err("The native transfer preview provider is unavailable".into());
        }
        let guard = session
            .prepare(Request {
                expected_revision: session.revision(),
                actor: Actor::local(channel),
                label: "Open transfer controls".into(),
                edits: vec![],
            })
            .map_err(|e| e.to_string())?;
        self.scope = Some(Scope {
            guard,
            source: ContentHash::document(session.document())?,
            resources: resources.clone(),
            channel,
        });
        self.form = Some(form);
        self.candidate = None;
        self.handoff = None;
        self.error = None;
        Ok(())
    }
    fn current(&self, session: &Session, resources: &ProjectResources) -> Result<(), String> {
        let scope = self.scope.as_ref().ok_or("No transfer tool is active")?;
        session
            .validate_preview(&scope.guard)
            .map_err(|_| "Source session changed; cancel and reopen the transfer".to_string())?;
        if &scope.resources != resources
            || ContentHash::document(session.document())? != scope.source
        {
            return Err(
                "Source or native resources changed; cancel and reopen the transfer".into(),
            );
        }
        Ok(())
    }
    pub fn refresh_scene(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: Option<&Arc<Scene>>,
    ) -> Result<(), String> {
        self.current(session, resources)?;
        let scene = scene.filter(|s| current_scene(session, s));
        let choices = form::catalogue(session.document(), scene)?;
        self.form.as_mut().expect("current form").choices = choices;
        Ok(())
    }
    pub fn picker(&self) -> Option<TransferPick> {
        self.form.as_ref()?.picker()
    }
    pub fn set_occurrence(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        lane: TransferLane,
        path: OccurrencePath,
    ) -> Result<(), String> {
        self.current(session, resources)?;
        let _ = scene;
        let form = self.form.as_mut().expect("current form");
        form.choices = form::catalogue_scene(session.document(), None)?;
        form.set_occurrence(lane, path)?;
        self.candidate = None;
        self.error = None;
        Ok(())
    }
    pub fn set_capture(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        lane: TransferLane,
        mut intent: GeometryIntent,
    ) -> Result<(), String> {
        self.current(session, resources)?;
        let pick = self
            .picker()
            .filter(|p| p.lane == lane)
            .ok_or("Activate the intended transfer capture lane first")?;
        let inspection = self
            .inspection
            .as_ref()
            .filter(|i| i.pick == pick && !i.pending)
            .ok_or("Wait for the current native source inspection before capturing geometry")?;
        if !inspection
            .scene
            .as_ref()
            .is_some_and(|current| std::ptr::eq(current.as_ref(), scene))
            || scene.stamp() != inspection.stamp
            || scene.scope() != &crate::viewport::SceneScope::Sources(vec![pick.producer])
            || scene
                .model_evaluation()
                .is_none_or(|i| Some(i.source_hash) != self.scope.as_ref().map(|s| s.source))
        {
            return Err(
                "Only the exact current guarded source inspection can provide transfer captures"
                    .into(),
            );
        }
        if intent.attachment.is_some() {
            return Err("Select exact geometry without an attachment offset for transfer".into());
        }
        let snapshot = scene
            .geometry_snapshot(pick.producer)
            .ok_or("The captured native producer is unavailable")?;
        let Resolution::Resolved { key, .. } =
            references::resolve(&intent, &snapshot, session.revision())
        else {
            return Err(
                "The transfer reference is missing or ambiguous; select it explicitly again".into(),
            );
        };
        let element = snapshot
            .elements
            .iter()
            .find(|e| e.key == key)
            .ok_or("Native reference element is missing")?;
        let eligible = match pick.kind {
            TransferCaptureKind::PlanarFace => {
                matches!(element.geometry, Some(AnalyticGeometry::Plane { .. }))
            }
            TransferCaptureKind::CircularFeature => match element.geometry {
                Some(AnalyticGeometry::CircularArc { sweep_rad, .. }) => {
                    (sweep_rad.abs() - std::f64::consts::TAU).abs() <= 1e-9
                }
                _ => {
                    key.kind == TopologyKind::Face
                        && !matches!(element.geometry, Some(AnalyticGeometry::Plane { .. }))
                }
            },
        };
        if !eligible {
            return Err(match pick.kind{TransferCaptureKind::PlanarFace=>"Select an actual planar face",TransferCaptureKind::CircularFeature=>"Select a full circular edge or a cylindrical face; native Preview checks its exact geometry"}.into());
        }
        let form = self.form.as_mut().expect("current form");
        match lane {
            TransferLane::Source => {
                if let Some(capture) = pick.replace_capture {
                    let old = form
                        .captures
                        .iter_mut()
                        .find(|r| r.id == capture)
                        .ok_or("The source reference being repaired no longer exists")?;
                    intent.id = capture;
                    *old = intent;
                } else if form.mode != SourceMode::Circles {
                    if let Some(previous) = form.captures.first() {
                        intent.id = previous.id;
                    }
                    form.captures = vec![intent];
                } else {
                    if form.captures.len() >= model::MAX_TRANSFER_SOURCES {
                        return Err(
                            "A transfer supports at most 256 explicit circular references".into(),
                        );
                    }
                    if form.captures.iter().any(|existing|matches!(references::resolve(existing,&snapshot,session.revision()),Resolution::Resolved{key:old,..}if old==key)){return Err("This circular reference is already selected".into());}
                    form.captures.push(intent);
                }
                form.replace_capture = None;
                if form.mode != SourceMode::Circles || pick.replace_capture.is_some() {
                    form.pick = None;
                }
            }
            TransferLane::Target { target } => {
                let target = form
                    .targets
                    .iter_mut()
                    .find(|t| t.id == target)
                    .ok_or("The receiving row no longer exists")?;
                if let Some(previous) = &target.face {
                    intent.id = previous.id;
                }
                target.face = Some(intent);
                form.pick = None;
            }
        }
        form.changed();
        self.candidate = None;
        self.error = None;
        Ok(())
    }
    pub fn cancel(&mut self) {
        if let Some(backend) = &mut self.backend {
            backend.cancel();
        }
        self.form = None;
        self.inspection = None;
        self.scope = None;
        self.candidate = None;
        self.handoff = None;
        self.error = None;
    }
    pub fn preview(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let result = self.request_preview(session, resources, channel);
        if let Err(error) = &result {
            self.candidate = None;
            self.error = Some(error.clone());
        }
        result
    }
    fn request_preview(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        self.current(session, resources)?;
        let form = self.form.as_mut().expect("current form");
        let request = form.request()?;
        let prepared = if form.editing {
            model::prepare_transfer_edit(
                session,
                session.revision(),
                Actor::local(channel),
                &request,
            )
        } else {
            model::prepare_transfer(session, session.revision(), Actor::local(channel), &request)
        }
        .map_err(|e| e.to_string())?;
        let source = ContentHash::document(prepared.prepared.document())?;
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("Transfer preview generation limit reached")?;
        let stamp = SceneStamp {
            document: session.document().id(),
            revision: session.revision(),
            generation: self.generation,
        };
        self.inspection = None;
        self.backend
            .as_mut()
            .ok_or("The native transfer preview provider is unavailable")?
            .request_evaluation(
                prepared.prepared.document(),
                resources,
                stamp,
                &Default::default(),
            )?;
        form.pick = None;
        form.replace_capture = None;
        self.candidate = Some(Candidate {
            prepared: prepared.prepared,
            source,
            stamp,
            version: form.version,
            outputs: prepared.outputs,
            pending: true,
            scene: None,
        });
        self.error = None;
        Ok(())
    }
    pub fn poll(&mut self, session: &Session, resources: &ProjectResources) {
        if self.active()
            && let Err(error) = self.current(session, resources)
        {
            self.candidate = None;
            self.inspection = None;
            if let Some(form) = &mut self.form {
                form.choices.clear();
            }
            self.error = Some(error);
        }
        if self.active()
            && self.current(session, resources).is_ok()
            && let Err(error) = self.ensure_inspection(session, resources)
        {
            self.error = Some(error);
        }
        for _ in 0..16 {
            let Some(reply) = self.backend.as_mut().and_then(|b| b.poll()) else {
                break;
            };
            if self
                .inspection
                .as_ref()
                .is_some_and(|i| i.pending && i.stamp == reply.stamp)
            {
                let result = (|| {
                    self.current(session, resources)?;
                    let inspection = self.inspection.as_ref().expect("matching inspection");
                    if self.picker().as_ref() != Some(&inspection.pick) {
                        return Err("The transfer capture lane changed".into());
                    }
                    let scene = reply.result?;
                    if scene.stamp() != inspection.stamp
                        || scene.scope()
                            != &crate::viewport::SceneScope::Sources(vec![inspection.pick.producer])
                        || scene.model_evaluation().is_none_or(|i| {
                            Some(i.source_hash) != self.scope.as_ref().map(|s| s.source)
                        })
                        || scene
                            .geometry_snapshot(inspection.pick.producer)
                            .is_none_or(|s| s.elements.is_empty())
                    {
                        return Err("Native transfer source inspection returned a different source, scope or producer".into());
                    }
                    Ok(scene)
                })();
                let inspection = self.inspection.as_mut().expect("matching inspection");
                inspection.pending = false;
                match result {
                    Ok(scene) => {
                        inspection.scene = Some(scene);
                        self.error = None;
                    }
                    Err(error) => {
                        inspection.scene = None;
                        self.error = Some(error);
                    }
                }
                continue;
            }
            let Some(candidate) = &self.candidate else {
                continue;
            };
            if !candidate.pending || candidate.stamp != reply.stamp {
                continue;
            }
            let result = (|| {
                self.current(session, resources)?;
                let candidate = self.candidate.as_ref().expect("matching candidate");
                session
                    .validate_preview(&candidate.prepared)
                    .map_err(|e| e.to_string())?;
                if self.form.as_ref().map(|f| f.version) != Some(candidate.version) {
                    return Err("Transfer inputs changed; Preview them again".into());
                }
                let scene = reply.result?;
                if scene.stamp() != candidate.stamp {
                    return Err("Native transfer preview returned a different scene scope".into());
                }
                super::proof::validate(candidate.prepared.document(), &scene, &candidate.outputs)?;
                VisibilityFilter::default().apply(
                    &scene,
                    candidate.prepared.document(),
                    candidate.stamp.revision,
                )
            })();
            let candidate = self.candidate.as_mut().expect("matching candidate");
            candidate.pending = false;
            match result {
                Ok(scene) => {
                    candidate.scene = Some(scene);
                    self.error = None;
                }
                Err(error) => {
                    candidate.scene = None;
                    self.error = Some(error);
                }
            }
        }
    }
    fn ensure_inspection(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
    ) -> Result<(), String> {
        let Some(pick) = self.picker() else {
            self.inspection = None;
            return Ok(());
        };
        if self.inspection.as_ref().is_some_and(|i| i.pick == pick) {
            return Ok(());
        }
        self.candidate = None;
        self.inspection = None;
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("Transfer inspection generation limit reached")?;
        let stamp = SceneStamp {
            document: session.document().id(),
            revision: session.revision(),
            generation: self.generation,
        };
        self.inspection = Some(Inspection {
            pick: pick.clone(),
            stamp,
            pending: true,
            scene: None,
        });
        if let Err(error) = self
            .backend
            .as_mut()
            .ok_or("The native source inspection provider is unavailable")?
            .request_sources(session.document(), resources, stamp, &[pick.producer])
        {
            self.inspection
                .as_mut()
                .expect("recorded inspection")
                .pending = false;
            return Err(error);
        }
        self.error = None;
        Ok(())
    }
    pub fn capture_scene(
        &self,
        session: &Session,
        resources: &ProjectResources,
    ) -> Option<Arc<Scene>> {
        self.current(session, resources).ok()?;
        let pick = self.picker()?;
        self.inspection
            .as_ref()
            .filter(|i| !i.pending && i.pick == pick)
            .and_then(|i| i.scene.clone())
    }
    pub fn can_apply(&self, session: &Session, resources: &ProjectResources) -> bool {
        self.current(session, resources).is_ok()
            && self.error.is_none()
            && self.form.as_ref().is_some_and(|f| f.pick.is_none())
            && self.candidate.as_ref().is_some_and(|c| {
                !c.pending
                    && c.scene.is_some()
                    && self.form.as_ref().is_some_and(|f| f.version == c.version)
                    && session.validate_preview(&c.prepared).is_ok()
            })
    }
    pub fn apply(
        &mut self,
        session: &mut Session,
        resources: &ProjectResources,
    ) -> Result<TransferOutcome, String> {
        self.current(session, resources)?;
        if !self.can_apply(session, resources) {
            return Err(
                "Wait for a successful native Preview of the current transfer inputs".into(),
            );
        }
        let candidate = self.candidate.take().expect("checked native candidate");
        if ContentHash::document(candidate.prepared.document())? != candidate.source {
            return Err("Transfer candidate content changed; Preview again".into());
        }
        let scene = candidate.scene.expect("checked native scene");
        let selected = candidate.outputs.iter().map(|o| o.body).collect();
        session
            .commit(candidate.prepared)
            .map_err(|e| e.to_string())?;
        self.cancel();
        self.handoff = Some(Handoff {
            document: session.document().id(),
            revision: session.revision(),
            resources: resources.clone(),
            scene,
        });
        Ok(TransferOutcome::Applied {
            selected,
            label: self.label(),
        })
    }
    pub fn display_scene(
        &self,
        session: &Session,
        resources: &ProjectResources,
    ) -> Option<Arc<Scene>> {
        if self.can_apply(session, resources) {
            return self.candidate.as_ref().and_then(|c| c.scene.clone());
        }
        self.handoff
            .as_ref()
            .filter(|h| {
                h.document == session.document().id()
                    && h.revision == session.revision()
                    && &h.resources == resources
            })
            .map(|h| h.scene.clone())
    }
    pub fn canonical_ready(&mut self, session: &Session) {
        if self.handoff.as_ref().is_some_and(|h| {
            h.document == session.document().id() && h.revision <= session.revision()
        }) {
            self.handoff = None;
        }
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        resources: &ProjectResources,
    ) -> Result<Option<TransferOutcome>, String> {
        let Some(form) = &mut self.form else {
            return Ok(None);
        };
        crate::controls::inspector_layout(ui);
        ui.heading("Transfer features");
        if form.choices.is_empty() {
            ui.small("Waiting for current native Part occurrences. Source and receiving references stay editable in this form.");
        }
        if form.show(ui, session.document()) {
            self.candidate = None;
            self.error = None;
        }
        if self
            .inspection
            .as_ref()
            .is_some_and(|i| !i.pending && i.scene.is_none())
            && ui.button("Retry source inspection").clicked()
        {
            self.inspection = None;
            self.error = None;
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        } else if self.pending() {
            ui.spinner();
            ui.label("Checking associative native cuts…");
        } else if self.can_apply(session, resources) {
            ui.label("Native preview ready");
        }
        let actions = inspector_actions::row(
            ui,
            Owner::Transfer,
            Some(Button::new(
                "Preview",
                !self.pending(),
                "Evaluate every receiving cut using the current native source geometry",
            )),
            Button::new(
                "Apply",
                self.can_apply(session, resources),
                "Commit all checked receiving features in one Undo step",
            ),
            Button::new(
                "Cancel",
                true,
                "Discard the transfer draft without changing source geometry",
            ),
        );
        if actions.cancel {
            self.cancel();
            return Ok(Some(TransferOutcome::Cancelled));
        }
        if actions.preview {
            let channel = self.scope.as_ref().map_or(Channel::Ui, |s| s.channel);
            self.preview(session, resources, channel)?;
        }
        if actions.apply {
            return self.apply(session, resources).map(Some);
        }
        Ok(None)
    }
}
fn current_scene(session: &Session, scene: &Scene) -> bool {
    scene.is_full_model()
        && scene.stamp().document == session.document().id()
        && scene.stamp().revision == session.revision()
        && scene.model_evaluation().is_some_and(|info| {
            Some(info.source_hash) == ContentHash::document(session.document()).ok()
        })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

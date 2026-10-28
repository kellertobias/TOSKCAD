//! A candidate evaluation lane, separate from canonical display publication.
use super::{
    ExtrudeCapabilities, ExtrudePanel, FormStamp, ModelRequest, ModelingTool, PreviewState,
    boolean::BooleanPanel,
    form::{ModelEvent, ModelForm, ToolProof},
    names::GeneratedNames,
    primitive::PrimitivePanel,
    revolve::RevolvePanel,
};
use crate::{
    egui,
    viewport::{Scene, SceneBackend, SceneStamp, VisibilityFilter},
};
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    document::{EntityData, EntityId},
    model::{
        CreatedBody, ExtrudeRequest, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest,
        RevolveRequest, length,
    },
    project_file::{ContentHash, ProjectResources},
    sketch_regions::RegionIntent,
};
use std::sync::Arc;

struct Scope {
    /// A source-neutral transaction also binds a newly opened form to the
    /// originating Session before there is any extrusion transaction.
    session: Prepared,
    source: ContentHash,
    resources: ProjectResources,
}
struct Candidate {
    prepared: Prepared,
    content: ContentHash,
    form: FormStamp,
    stamp: SceneStamp,
    outputs: Vec<CreatedBody>,
    tools: Vec<ToolProof>,
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
pub struct ModelToolController {
    form: Option<ModelForm>,
    backend: Option<Box<dyn SceneBackend>>,
    scope: Option<Scope>,
    candidate: Option<Candidate>,
    handoff: Option<Handoff>,
    generation: u64,
    error: Option<String>,
}
pub enum ModelToolOutcome {
    Applied(Vec<CreatedBody>),
    Cancelled,
}
impl ModelToolController {
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
    pub fn label(&self) -> &'static str {
        self.form.as_ref().map_or("Modeling", ModelForm::label)
    }
    pub fn pending(&self) -> bool {
        self.candidate
            .as_ref()
            .is_some_and(|candidate| candidate.pending)
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn stamp(&self) -> Option<FormStamp> {
        self.form.as_ref().map(ModelForm::stamp)
    }
    pub fn capabilities(&self) -> ExtrudeCapabilities {
        if self.has_backend() {
            ExtrudeCapabilities::distance_bodies()
        } else {
            ExtrudeCapabilities::default()
        }
    }
    pub fn begin(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        sketch: EntityId,
        profiles: Vec<RegionIntent>,
        channel: Channel,
    ) -> Result<(), String> {
        let mut request = ExtrudeRequest::new(sketch, profiles, Default::default());
        let mut names = GeneratedNames::new(session.document());
        for output in &mut request.outputs {
            output.name = names.numbered("Extruded part", " extrusion");
        }
        let mut panel = ExtrudePanel::default();
        panel.begin_request(request)?;
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Extrude(Box::new(panel)),
        )
    }
    pub fn begin_primitive(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        kind: ModelingTool,
        channel: Channel,
    ) -> Result<(), String> {
        let parameters = match kind {
            ModelingTool::Box => PrimitiveParameters::Box {
                size: [length(100.), length(60.), length(20.)],
                frame: PrimitiveFrame::default(),
            },
            ModelingTool::Cylinder => PrimitiveParameters::Cylinder {
                radius: length(20.),
                height: length(50.),
                frame: PrimitiveFrame::default(),
            },
            _ => return Err("Choose a box or cylinder primitive".into()),
        };
        let name = GeneratedNames::new(session.document()).numbered(kind.label(), " feature");
        let request = PrimitiveRequest::new(name, parameters);
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Primitive(Box::new(PrimitivePanel::new(request)?)),
        )
    }
    pub fn begin_revolve(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        sketch: EntityId,
        profiles: Vec<RegionIntent>,
        channel: Channel,
    ) -> Result<(), String> {
        let mut request = RevolveRequest::new(sketch, profiles, Default::default());
        let mut names = GeneratedNames::new(session.document());
        for output in &mut request.outputs {
            output.name = names.numbered("Revolved part", " feature");
        }
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Revolve(Box::new(RevolvePanel::new(request)?)),
        )
    }
    fn begin_form(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
        form: ModelForm,
    ) -> Result<(), String> {
        if !self.has_backend() {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.active() {
            return Err("Apply or cancel the active modeling tool first".into());
        }
        let scope = session
            .prepare(Request {
                expected_revision: session.revision(),
                actor: Actor::local(channel),
                label: "Open modeling controls".into(),
                edits: vec![],
            })
            .map_err(|error| error.to_string())?;
        let source = ContentHash::document(session.document())?;
        self.form = Some(form);
        self.scope = Some(Scope {
            session: scope,
            source,
            resources: resources.clone(),
        });
        self.candidate = None;
        self.handoff = None;
        self.error = None;
        Ok(())
    }
    pub fn boolean_available(
        document: &multicad_core::document::Document,
        revision: u64,
        scene: &Scene,
    ) -> bool {
        BooleanPanel::available(document, revision, scene).is_ok_and(|count| count >= 2)
    }
    pub fn begin_boolean(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        selected: &std::collections::BTreeSet<EntityId>,
        channel: Channel,
    ) -> Result<(), String> {
        let panel = BooleanPanel::new(session.document(), session.revision(), scene, selected)?;
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Boolean(Box::new(panel)),
        )
    }
    pub fn cancel(&mut self) {
        self.form = None;
        self.scope = None;
        self.candidate = None;
        self.handoff = None;
        self.error = None;
        // Generation does not reset: a delayed reply from a cancelled tool can
        // never match a subsequent request using this same worker.
    }
    fn current(
        &self,
        session: &Session,
        resources: &ProjectResources,
        full: bool,
    ) -> Result<(), String> {
        let scope = self.scope.as_ref().ok_or("Start a modeling tool first")?;
        session.validate_preview(&scope.session).map_err(|_| {
            "The source session changed; cancel the modeling tool and start again".to_string()
        })?;
        if &scope.resources != resources {
            return Err(
                "Project resources changed; cancel the modeling tool and start again".into(),
            );
        }
        if full && ContentHash::document(session.document())? != scope.source {
            return Err(
                "The source content changed; cancel the modeling tool and start again".into(),
            );
        }
        Ok(())
    }
    fn invalidate(&mut self) {
        self.candidate = None;
        self.error = None;
    }
    pub fn preview(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        self.invalidate();
        let result: Result<(), String> = (|| {
            self.current(session, resources, true)?;
            let stamp = self.stamp().ok_or("Start a modeling tool first")?;
            let request = self
                .form
                .as_ref()
                .ok_or("Start a modeling tool first")?
                .request(session.document(), self.capabilities())?;
            self.submit(session, resources, channel, stamp, &request)
        })();
        if let Err(error) = &result {
            self.error = Some(error.clone());
        }
        result
    }
    fn submit(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
        form: FormStamp,
        request: &ModelRequest,
    ) -> Result<(), String> {
        self.current(session, resources, true)?;
        if self.stamp() != Some(form) {
            return Err("Modeling inputs changed; preview the current values".into());
        }
        let prepared = request.prepare(session, Actor::local(channel))?;
        session
            .validate_preview(&prepared.prepared)
            .map_err(|error| error.to_string())?;
        let content = ContentHash::document(prepared.prepared.document())?;
        let tools = request.tool_proof(prepared.prepared.document())?;
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("Model preview generation limit reached")?;
        let stamp = SceneStamp {
            document: session.document().id(),
            revision: session.revision(),
            generation: self.generation,
        };
        self.backend
            .as_mut()
            .ok_or("The native modeling preview provider is unavailable")?
            .request_project(prepared.prepared.document(), resources, stamp)?;
        self.candidate = Some(Candidate {
            prepared: prepared.prepared,
            content,
            form,
            stamp,
            outputs: prepared.outputs,
            tools,
            pending: true,
            scene: None,
        });
        self.error = None;
        Ok(())
    }
    /// Polling is bounded and never waits. Only a matching reply can become a
    /// candidate preview; source validation is independent of SceneFeed.
    pub fn poll(&mut self, session: &Session, resources: &ProjectResources) {
        if self.active()
            && let Err(error) = self.current(session, resources, false)
        {
            self.candidate = None;
            self.error = Some(error);
        }
        for _ in 0..16 {
            let Some(reply) = self.backend.as_mut().and_then(|backend| backend.poll()) else {
                break;
            };
            let Some(candidate) = &self.candidate else {
                continue;
            };
            if !candidate.pending || reply.stamp != candidate.stamp {
                continue;
            }
            let result = (|| {
                self.current(session, resources, true)?;
                let candidate = self.candidate.as_ref().expect("matching candidate");
                session
                    .validate_preview(&candidate.prepared)
                    .map_err(|error| error.to_string())?;
                if self.stamp() != Some(candidate.form)
                    || ContentHash::document(candidate.prepared.document())? != candidate.content
                {
                    return Err("Modeling candidate changed; preview it again".into());
                }
                let scene = reply.result?;
                if scene.stamp() != candidate.stamp {
                    return Err("Native preview returned the wrong scene scope".into());
                }
                for output in &candidate.outputs {
                    if !matches!(candidate.prepared.document().get(output.body).map(|entity|&entity.data),Some(EntityData::Part{features,..}) if features.contains(&output.feature))
                        || !scene.reference_instances().iter().any(|instance| {
                            instance.id == output.body && instance.geometry == output.feature
                        })
                        || !scene
                            .geometries()
                            .get(&output.feature)
                            .is_some_and(|geometry| {
                                !geometry.vertices().is_empty() && !geometry.indices().is_empty()
                            })
                    {
                        return Err(
                            "Native preview did not produce every requested body and producer"
                                .into(),
                        );
                    }
                }
                for proof in &candidate.tools {
                    let tool = &proof.input;
                    let present = scene.reference_instances().iter().any(|instance| {
                        instance.id == tool.body && instance.geometry == tool.producer
                    });
                    if (proof.consumed
                        && scene
                            .reference_instances()
                            .iter()
                            .any(|instance| instance.id == tool.body))
                        || (!proof.consumed && !present)
                    {
                        return Err("Native preview tool membership disagrees with the requested producer-version consumption".into());
                    }
                }
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
    pub fn can_apply(&self, session: &Session, resources: &ProjectResources) -> bool {
        self.current(session, resources, false).is_ok()
            && self.error.is_none()
            && self.candidate.as_ref().is_some_and(|candidate| {
                !candidate.pending
                    && candidate.scene.is_some()
                    && self.stamp() == Some(candidate.form)
                    && session.validate_preview(&candidate.prepared).is_ok()
            })
    }
    pub fn apply(
        &mut self,
        session: &mut Session,
        resources: &ProjectResources,
    ) -> Result<Vec<CreatedBody>, String> {
        let result: Result<Vec<CreatedBody>, String> = (|| {
            self.current(session, resources, true)?;
            if !self.can_apply(session, resources) {
                return Err(
                    "Wait for a successful native preview of the current modeling inputs".into(),
                );
            }
            let candidate = self.candidate.as_ref().expect("checked preview");
            if ContentHash::document(candidate.prepared.document())? != candidate.content {
                return Err("Modeling candidate content changed; preview it again".into());
            }
            session
                .validate_preview(&candidate.prepared)
                .map_err(|error| error.to_string())?;
            let candidate = self.candidate.take().expect("checked preview");
            let scene = candidate.scene.expect("checked preview");
            session
                .commit(candidate.prepared)
                .map_err(|error| error.to_string())?;
            self.cancel();
            // Keep the same checked geometry visible while the canonical lane
            // rebuilds the new revision. It remains noninteractive/display-only.
            self.handoff = Some(Handoff {
                document: session.document().id(),
                revision: session.revision(),
                resources: resources.clone(),
                scene,
            });
            Ok(candidate.outputs)
        })();
        if let Err(error) = &result {
            self.error = Some(error.clone());
        }
        result
    }
    pub fn display_scene(
        &self,
        session: &Session,
        resources: &ProjectResources,
    ) -> Option<Arc<Scene>> {
        if self.can_apply(session, resources) {
            return self
                .candidate
                .as_ref()
                .and_then(|candidate| candidate.scene.clone());
        }
        self.handoff
            .as_ref()
            .filter(|handoff| {
                handoff.document == session.document().id()
                    && handoff.revision == session.revision()
                    && &handoff.resources == resources
            })
            .map(|handoff| handoff.scene.clone())
    }
    pub fn canonical_ready(&mut self, session: &Session) {
        if self.handoff.as_ref().is_some_and(|handoff| {
            handoff.document == session.document().id() && handoff.revision <= session.revision()
        }) {
            self.handoff = None;
        }
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        resources: &ProjectResources,
    ) -> Result<Option<ModelToolOutcome>, String> {
        let stamp = self.stamp();
        let preview = match (&self.error, &self.candidate, stamp) {
            (Some(error), _, Some(stamp)) => PreviewState::Failed {
                stamp,
                message: error,
            },
            (_, Some(candidate), _) if candidate.pending => PreviewState::Pending {
                stamp: candidate.form,
            },
            (_, Some(candidate), _) if candidate.scene.is_some() => PreviewState::Ready {
                stamp: candidate.form,
            },
            _ => PreviewState::Idle,
        };
        let capabilities = if self.backend.is_some() {
            ExtrudeCapabilities::distance_bodies()
        } else {
            ExtrudeCapabilities::default()
        };
        let event = self
            .form
            .as_mut()
            .ok_or("Start a modeling tool first")?
            .show(ui, session.document(), capabilities, preview);
        let result = match event {
            Some(ModelEvent::Changed) => {
                self.invalidate();
                Ok(None)
            }
            Some(ModelEvent::Preview { stamp, request }) => {
                self.invalidate();
                self.submit(session, resources, Channel::Ui, stamp, &request)
                    .map(|()| None)
            }
            Some(ModelEvent::Apply) => self
                .apply(session, resources)
                .map(|outputs| Some(ModelToolOutcome::Applied(outputs))),
            Some(ModelEvent::Cancel) => {
                self.cancel();
                Ok(Some(ModelToolOutcome::Cancelled))
            }
            None => Ok(None),
        };
        if let Err(error) = &result {
            self.error = Some(error.clone());
        }
        result
    }
}

#[cfg(test)]
mod tests;

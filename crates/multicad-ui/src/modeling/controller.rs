//! A candidate evaluation lane, separate from canonical display publication.
use super::{
    ExtrudeCapabilities, ExtrudePanel, FormStamp, ModelRequest, ModelingTool, PreviewState,
    boolean::BooleanPanel,
    boolean_edit::BooleanEditPanel,
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
    execution: Execution,
}
#[derive(Clone, Copy)]
enum Execution {
    Create,
    Edit { feature: EntityId },
}
struct Candidate {
    prepared: Prepared,
    content: ContentHash,
    form: FormStamp,
    stamp: SceneStamp,
    outputs: Vec<CreatedBody>,
    tools: Vec<ToolProof>,
    execution: Execution,
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
        if self.editing() {
            "Feature edit"
        } else {
            self.form.as_ref().map_or("Modeling", ModelForm::label)
        }
    }
    pub fn editing(&self) -> bool {
        self.scope
            .as_ref()
            .is_some_and(|scope| matches!(scope.execution, Execution::Edit { .. }))
    }
    pub fn editable(document: &multicad_core::document::Document, feature: EntityId) -> bool {
        matches!(document.get(feature).map(|entity|&entity.data),Some(EntityData::Feature{definition,suppressed:false,..}) if matches!(definition.type_id.as_str(),multicad_core::model::EXTRUDE_TYPE|multicad_core::model::PRIMITIVE_TYPE|multicad_core::model::REVOLVE_TYPE|multicad_core::model::BOOLEAN_TYPE))
    }
    pub fn begin_existing(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        feature: EntityId,
        channel: Channel,
    ) -> Result<(), String> {
        use multicad_core::model;
        let document = session.document();
        if !Self::editable(document, feature) {
            return Err("Choose a supported, unsuppressed applied model feature".into());
        }
        let entity = document.get(feature).expect("checked feature");
        let EntityData::Feature { definition, .. } = &entity.data else {
            unreachable!()
        };
        let body_name = |body| {
            document
                .get(body)
                .map(|entity| entity.name.clone())
                .ok_or_else(|| {
                    "The feature's source body is missing; repair it before editing".to_string()
                })
        };
        let form = match definition.type_id.as_str() {
            model::EXTRUDE_TYPE => {
                let source =
                    model::read_extrude(document, feature).map_err(|error| error.to_string())?;
                let mut panel = ExtrudePanel::default();
                panel.begin_request(ExtrudeRequest {
                    sketch: source.profile.sketch,
                    outputs: vec![model::ExtrudeOutput {
                        feature,
                        body: source.output.body,
                        name: body_name(source.output.body)?,
                        region: source.profile.region,
                    }],
                    parameters: source.parameters,
                    operation: source.operation,
                    targets: source.targets,
                    keep_tools: source.keep_tools,
                    faces: vec![],
                })?;
                // Validate the current form up front. In particular associative
                // limits must not silently become distance-only controls.
                panel.request(document, self.capabilities())?;
                ModelForm::Extrude(Box::new(panel))
            }
            model::PRIMITIVE_TYPE => {
                let source =
                    model::read_primitive(document, feature).map_err(|error| error.to_string())?;
                ModelForm::Primitive(Box::new(PrimitivePanel::existing(PrimitiveRequest {
                    feature,
                    body: source.output.body,
                    name: body_name(source.output.body)?,
                    parameters: source.parameters,
                })?))
            }
            model::REVOLVE_TYPE => {
                let source =
                    model::read_revolve(document, feature).map_err(|error| error.to_string())?;
                ModelForm::Revolve(Box::new(RevolvePanel::new(RevolveRequest {
                    sketch: source.profile.sketch,
                    outputs: vec![model::RevolveOutput {
                        feature,
                        body: source.output.body,
                        name: body_name(source.output.body)?,
                        region: source.profile.region,
                    }],
                    parameters: source.parameters,
                })?))
            }
            model::BOOLEAN_TYPE => {
                ModelForm::BooleanEdit(Box::new(BooleanEditPanel::new(document, feature)?))
            }
            _ => unreachable!(),
        };
        self.begin_form(session, resources, channel, form)?;
        self.scope.as_mut().expect("opened scope").execution = Execution::Edit { feature };
        Ok(())
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
            execution: Execution::Create,
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
        let execution = self.scope.as_ref().expect("current scope").execution;
        let prepared = match execution {
            Execution::Create => request.prepare(session, Actor::local(channel))?,
            Execution::Edit { feature } => {
                request.prepare_edit(session, Actor::local(channel), feature)?
            }
        };
        session
            .validate_preview(&prepared.prepared)
            .map_err(|error| error.to_string())?;
        let content = ContentHash::document(prepared.prepared.document())?;
        let tools = match execution {
            Execution::Create => request.tool_proof(prepared.prepared.document())?,
            Execution::Edit { .. } => vec![],
        };
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
            .request_evaluation(
                prepared.prepared.document(),
                resources,
                stamp,
                &Default::default(),
            )?;
        self.candidate = Some(Candidate {
            prepared: prepared.prepared,
            content,
            form,
            stamp,
            outputs: prepared.outputs,
            tools,
            execution,
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
                if let Some(info) = scene.model_evaluation() {
                    if info.source_hash != candidate.content {
                        return Err("Native preview returned evaluation metadata for different source content".into());
                    }
                } else if matches!(candidate.execution, Execution::Edit { .. }) {
                    return Err("Editing applied features requires a native provider with source-bound evaluation metadata".into());
                }
                for output in &candidate.outputs {
                    let instance_proven = match candidate.execution {
                        Execution::Create => scene.reference_instances().iter().any(|instance| {
                            instance.id == output.body && instance.geometry == output.feature
                        }),
                        Execution::Edit { feature } => {
                            if feature != output.feature {
                                return Err("Feature edit changed its producer identity".into());
                            }
                            let info = scene.model_evaluation().expect("checked edit metadata");
                            let retained = scene
                                .geometry_snapshot(feature)
                                .is_some_and(|snapshot| !snapshot.elements.is_empty());
                            let membership = if info.consumed_bodies.contains(&output.body) {
                                !scene
                                    .reference_instances()
                                    .iter()
                                    .any(|instance| instance.id == output.body)
                            } else {
                                let head = candidate.prepared.document().get(output.body).and_then(
                                    |entity| match &entity.data {
                                        EntityData::Part { features, .. } => {
                                            features.iter().rev().copied().find(|id| {
                                                matches!(
                                                    candidate
                                                        .prepared
                                                        .document()
                                                        .get(*id)
                                                        .map(|entity| &entity.data),
                                                    Some(EntityData::Feature {
                                                        suppressed: false,
                                                        ..
                                                    })
                                                )
                                            })
                                        }
                                        _ => None,
                                    },
                                );
                                head.is_some_and(|head| {
                                    scene.reference_instances().iter().any(|instance| {
                                        instance.id == output.body && instance.geometry == head
                                    }) && scene.geometries().get(&head).is_some_and(|geometry| {
                                        !geometry.vertices().is_empty()
                                            && !geometry.indices().is_empty()
                                    })
                                })
                            };
                            retained && membership
                        }
                    };
                    if !matches!(candidate.prepared.document().get(output.body).map(|entity|&entity.data),Some(EntityData::Part{features,..}) if features.contains(&output.feature))
                        || !instance_proven
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
    /// The viewport may still show checked geometry after Apply while its
    /// canonical revision rebuilds. Both views use solid bounds for camera Fit.
    pub(crate) fn owns_display_scene(&self, scene: &Arc<Scene>) -> bool {
        self.candidate
            .as_ref()
            .and_then(|candidate| candidate.scene.as_ref())
            .is_some_and(|preview| Arc::ptr_eq(preview, scene))
            || self
                .handoff
                .as_ref()
                .is_some_and(|handoff| Arc::ptr_eq(&handoff.scene, scene))
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
        if self.editing() {
            ui.strong("Edit applied feature");
            if let Some(Scope {
                execution: Execution::Edit { feature },
                ..
            }) = &self.scope
            {
                ui.label(
                    session
                        .document()
                        .get(*feature)
                        .map_or("Missing feature", |entity| entity.name.as_str()),
                );
            }
            ui.small("Preview rebuilds this producer and its downstream model. Apply retains all body, feature and profile identities.");
            ui.separator();
        }
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

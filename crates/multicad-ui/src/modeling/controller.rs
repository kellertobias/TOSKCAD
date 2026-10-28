//! A candidate evaluation lane, separate from canonical display publication.
use super::{ExtrudeCapabilities, ExtrudeEvent, ExtrudePanel, FormStamp, PreviewState};
use crate::{
    egui,
    viewport::{Scene, SceneBackend, SceneStamp, VisibilityFilter},
};
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    document::{EntityData, EntityId},
    model::{self, CreatedBody, ExtrudeRequest},
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
    panel: ExtrudePanel,
    backend: Option<Box<dyn SceneBackend>>,
    scope: Option<Scope>,
    candidate: Option<Candidate>,
    handoff: Option<Handoff>,
    generation: u64,
    error: Option<String>,
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
        self.panel.active()
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
        self.panel.stamp()
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
                label: "Open extrusion controls".into(),
                edits: vec![],
            })
            .map_err(|error| error.to_string())?;
        let source = ContentHash::document(session.document())?;
        self.panel.begin(sketch, profiles)?;
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
    pub fn cancel(&mut self) {
        self.panel.cancel();
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
        let scope = self.scope.as_ref().ok_or("Start Extrude first")?;
        session.validate_preview(&scope.session).map_err(|_| {
            "The source session changed; cancel Extrude and start again".to_string()
        })?;
        if &scope.resources != resources {
            return Err("Project resources changed; cancel Extrude and start again".into());
        }
        if full && ContentHash::document(session.document())? != scope.source {
            return Err("The source content changed; cancel Extrude and start again".into());
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
            let stamp = self.panel.stamp().ok_or("Start Extrude first")?;
            let request = self
                .panel
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
        request: &ExtrudeRequest,
    ) -> Result<(), String> {
        self.current(session, resources, true)?;
        if self.panel.stamp() != Some(form) {
            return Err("Extrusion inputs changed; preview the current values".into());
        }
        let prepared =
            model::prepare_extrude(session, session.revision(), Actor::local(channel), request)
                .map_err(|error| error.to_string())?;
        session
            .validate_preview(&prepared.prepared)
            .map_err(|error| error.to_string())?;
        let content = ContentHash::document(prepared.prepared.document())?;
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
                if self.panel.stamp() != Some(candidate.form)
                    || ContentHash::document(candidate.prepared.document())? != candidate.content
                {
                    return Err("Extrusion candidate changed; preview it again".into());
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
                    && self.panel.stamp() == Some(candidate.form)
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
                    "Wait for a successful native preview of the current extrusion inputs".into(),
                );
            }
            let candidate = self.candidate.as_ref().expect("checked preview");
            if ContentHash::document(candidate.prepared.document())? != candidate.content {
                return Err("Extrusion candidate content changed; preview it again".into());
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
    ) -> Result<Option<Vec<CreatedBody>>, String> {
        let stamp = self.panel.stamp();
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
            .panel
            .show(ui, session.document(), capabilities, preview);
        let result = match event {
            Some(ExtrudeEvent::Changed(_)) => {
                self.invalidate();
                Ok(None)
            }
            Some(ExtrudeEvent::Preview { stamp, request }) => {
                self.invalidate();
                self.submit(session, resources, Channel::Ui, stamp, &request)
                    .map(|()| None)
            }
            Some(ExtrudeEvent::Apply(_)) => self.apply(session, resources).map(Some),
            Some(ExtrudeEvent::Cancel) => {
                self.cancel();
                Ok(None)
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

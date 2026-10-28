//! Native occurrence phases are independent of the legacy analytic adapter.
use super::{
    backend::ExternalQueryBackend,
    controller::{Context, Controller, Proposal, Purpose, scope},
    occurrence_form::{Consumer, Form},
    queries::{Ready, Task},
};
use crate::viewport::{Hit, Scene, SceneScope, Transform};
use multicad_core::{
    commands::{Actor, Channel, Prepared, Session},
    document::EntityId,
    project_file::ProjectResources,
    references::GeometryIntent,
    sketch_external::{self, OccurrenceContext},
};
use std::sync::Arc;
pub(super) struct View {
    pub sketch: EntityId,
    pub ticket: Prepared,
    pub consumer: Consumer,
    pub pose: Option<Transform>,
    pub resources: Option<ProjectResources>,
    pub rejected_scene: Option<crate::viewport::SceneStamp>,
}
impl Controller {
    pub fn set_query_backend(&mut self, backend: Box<dyn ExternalQueryBackend>) {
        self.queries.set_backend(backend);
        if let Some(form) = self.active.as_mut().and_then(|a| a.form.as_mut()) {
            form.clear_results();
        }
    }
    pub fn has_query_backend(&self) -> bool {
        self.queries.backend.is_some()
    }
    pub fn query_pending(&self) -> bool {
        self.queries.pending()
    }
    pub(crate) fn report_error(&mut self, error: String) {
        self.error = Some(error);
    }
    pub fn chosen_sketch_context(&self) -> Option<OccurrenceContext> {
        self.occurrence_view
            .as_ref()?
            .consumer
            .context()
            .ok()
            .flatten()
    }
    pub fn editing_pose(&self) -> Option<Transform> {
        self.occurrence_view.as_ref()?.pose
    }
    pub fn waiting_for_editing_pose(&self) -> bool {
        if let Some(form) = self.active.as_ref().and_then(|a| a.form.as_ref()) {
            return match form.consumer {
                Consumer::Unset => true,
                Consumer::World => false,
                Consumer::Occurrence(_) => self.editing_pose().is_none(),
            };
        }
        self.occurrence_view
            .as_ref()
            .is_some_and(|view| match view.consumer {
                Consumer::Unset => true,
                Consumer::World => false,
                Consumer::Occurrence(_) => view.pose.is_none(),
            })
    }
    pub(super) fn refresh_occurrence_view(&mut self, session: &Session) {
        let Some(context) = &self.context else {
            self.occurrence_view = None;
            return;
        };
        if self.occurrence_view.as_ref().is_some_and(|v| {
            v.sketch == context.sketch && session.validate_preview(&v.ticket).is_ok()
        }) {
            if let Some(view) = &mut self.occurrence_view {
                let rejected = context
                    .scene
                    .as_ref()
                    .is_some_and(|scene| view.rejected_scene == Some(scene.stamp()));
                if (view.consumer == Consumer::World || !rejected)
                    && let Some(pose) = consumer_pose(context, &view.consumer)
                {
                    view.pose = Some(pose);
                }
            }
            return;
        }
        self.occurrence_view = None;
        let Ok(bindings) = sketch_external::occurrence_bindings(session.document(), context.sketch)
        else {
            return;
        };
        let mut consumers = Vec::new();
        for binding in bindings {
            let consumer = binding
                .query
                .sketch_context
                .map_or(Consumer::World, Consumer::Occurrence);
            if !consumers.contains(&consumer) {
                consumers.push(consumer);
            }
        }
        if let [consumer] = consumers.as_slice() {
            self.occurrence_view = Some(View {
                sketch: context.sketch,
                ticket: context.ticket.clone(),
                consumer: consumer.clone(),
                pose: consumer_pose(context, consumer),
                resources: None,
                rejected_scene: None,
            });
        }
    }
    pub(super) fn update_occurrence_view(&mut self, session: &Session) {
        let (Some(context), Some(active)) = (&self.context, &self.active) else {
            return;
        };
        let Some(form) = &active.form else {
            return;
        };
        if session.validate_preview(&active.ticket).is_err() {
            return;
        }
        let rejected_scene = self
            .occurrence_view
            .as_ref()
            .and_then(|view| view.rejected_scene);
        let rejected = context
            .scene
            .as_ref()
            .is_some_and(|scene| rejected_scene == Some(scene.stamp()));
        let pose = if form.consumer == Consumer::World {
            Some(Transform::default())
        } else {
            form.catalog
                .as_ref()
                .and_then(|catalog| Transform::from_rigid_pose(catalog.sketch_world_pose).ok())
                .or_else(|| {
                    (!rejected)
                        .then(|| consumer_pose(context, &form.consumer))
                        .flatten()
                })
        };
        self.occurrence_view = Some(View {
            sketch: context.sketch,
            ticket: active.ticket.clone(),
            consumer: form.consumer.clone(),
            pose,
            resources: form.catalog_resources.clone(),
            rejected_scene: if form.catalog.is_some() {
                None
            } else {
                rejected_scene
            },
        });
    }
    pub(crate) fn request_preview(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<Option<Proposal>, String> {
        let Some(active) = &self.active else {
            return Err("Start a reference tool".into());
        };
        session
            .validate_preview(&active.ticket)
            .map_err(|e| e.to_string())?;
        if self.query_pending() {
            return Err("Wait for the current native reference query".into());
        }
        let Some(form) = &active.form else {
            return self.prepare(session, channel).map(Some);
        };
        let task = if form.catalog.is_none() {
            Task::Catalog(form.query(session.document())?)
        } else {
            Task::Prepare {
                input: form.request(session.document(), active.behavior)?,
                repair: form.previous.is_some(),
            }
        };
        self.queries
            .begin(session, resources, Actor::local(channel), task)?;
        self.error = None;
        Ok(None)
    }
    pub(crate) fn poll_query(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
    ) -> Option<Result<Proposal, String>> {
        if let Some(view) = &mut self.occurrence_view
            && view.resources.as_ref().is_some_and(|old| old != resources)
        {
            if view.consumer != Consumer::World {
                view.pose = None;
            }
            view.resources = None;
            view.rejected_scene = self
                .context
                .as_ref()
                .and_then(|context| context.scene.as_ref())
                .map(|scene| scene.stamp());
        }
        if self
            .active
            .as_ref()
            .and_then(|a| a.form.as_ref())
            .and_then(|f| f.catalog_resources.as_ref())
            .is_some_and(|old| old != resources)
        {
            if let Some(form) = self.active.as_mut().and_then(|a| a.form.as_mut()) {
                form.clear_results();
            }
            if let Some(view) = &mut self.occurrence_view {
                if view.consumer != Consumer::World {
                    view.pose = None;
                }
                view.resources = None;
            }
            self.queries.cancel();
            self.error =
                Some("Project resources changed. Find current native results again".into());
        }
        let expected = self.queries.task().cloned();
        let reply = self.queries.poll(session, resources)?;
        let result: Result<Option<Proposal>, String> = (|| {
            let reply = reply?;
            if let Some(active) = &self.active {
                session
                    .validate_preview(&active.ticket)
                    .map_err(|e| e.to_string())?;
                if let Some(form) = &active.form {
                    let current = match expected.as_ref() {
                        Some(Task::Catalog(_)) => Task::Catalog(form.query(session.document())?),
                        Some(Task::Prepare { .. }) => Task::Prepare {
                            input: form.request(session.document(), active.behavior)?,
                            repair: form.previous.is_some(),
                        },
                        _ => {
                            return Err(
                                "Native reference phase changed while it was pending".into()
                            );
                        }
                    };
                    if Some(current) != expected {
                        return Err(
                            "Reference controls changed. Preview current choices again".into()
                        );
                    }
                }
            }
            match reply {
                Ready::Catalog(catalog) => {
                    let form = self
                        .active
                        .as_mut()
                        .and_then(|a| a.form.as_mut())
                        .ok_or("Reference tool was closed")?;
                    form.accept_catalog(session.document(), resources, *catalog)?;
                    self.update_occurrence_view(session);
                    self.error = None;
                    Ok(None)
                }
                Ready::Proposal(proposal) => Ok(Some(*proposal)),
            }
        })();
        match result {
            Ok(Some(proposal)) => Some(Ok(proposal)),
            Ok(None) => None,
            Err(error) => {
                self.report_error(error.clone());
                Some(Err(error))
            }
        }
    }
    pub(super) fn select_occurrence(&mut self, session: &Session, hit: Hit) -> Result<(), String> {
        let context = self.context.as_ref().ok_or("Open a sketch")?;
        let active = self.active.as_ref().ok_or("Start a reference tool")?;
        session
            .validate_preview(&context.ticket)
            .map_err(|e| e.to_string())?;
        session
            .validate_preview(&active.ticket)
            .map_err(|e| e.to_string())?;
        let scene = context
            .scene
            .as_ref()
            .ok_or("Load current source geometry before picking")?;
        if hit.stamp != scene.stamp() {
            return Err("Selected native source geometry is stale".into());
        }
        let form = active
            .form
            .as_ref()
            .ok_or("Start an occurrence reference")?;
        let source = if let Some(source) = &form.source {
            source.clone()
        } else {
            let receipt = scene
                .assembly_evaluation()
                .ok_or("Choose the source occurrence first")?;
            let candidates: Vec<_> = receipt
                .leaves
                .iter()
                .filter(|leaf| {
                    leaf.path.root == hit.instance
                        && leaf.path.members.is_empty()
                        && leaf.producer == hit.geometry
                })
                .collect();
            let [leaf] = candidates.as_slice() else {
                return Err("Choose the exact nested source occurrence in Inspector first".into());
            };
            form.sources
                .iter()
                .find(|source| source.path == leaf.path && source.component == leaf.component)
                .cloned()
                .ok_or("The selected occurrence is unavailable")?
        };
        let producer = if source.source == source.component {
            scene.source_producer(source.component)
        } else {
            Some(source.source)
        }
        .ok_or("The source Part has no current native producer")?;
        if hit.geometry != producer {
            return Err("Pick geometry belonging to the chosen source occurrence/history".into());
        }
        let definition_view = match scene.scope() {
            SceneScope::Sources(roots) => {
                roots.contains(&source.source) || roots.contains(&producer)
            }
            _ => false,
        };
        let physical = scene.assembly_evaluation().is_some_and(|info| {
            info.source_hash == context.hash
                && info.leaves.iter().any(|leaf| {
                    leaf.path == source.path
                        && leaf.component == source.component
                        && leaf.producer == producer
                        && hit.instance == source.path.root
                })
        });
        if !definition_view && !physical {
            return Err(
                "The selected source has no current occurrence or source-inspection receipt".into(),
            );
        }
        let snapshot = scene
            .geometry_snapshot(producer)
            .ok_or("Native source topology is unavailable")?;
        let intent = GeometryIntent::capture(&snapshot, hit.key)?;
        self.queries.cancel();
        let active = self.active.as_mut().expect("checked active");
        let form = active.form.as_mut().expect("checked form");
        form.source = Some(source);
        form.clear_results();
        form.intent = Some(intent);
        active.selected = Some(hit);
        active.request = None;
        self.error = None;
        Ok(())
    }
    pub(super) fn occurrence_scene(&self) -> Option<Arc<Scene>> {
        let active = self.active.as_ref()?;
        let form = active.form.as_ref()?;
        let source = form.source.as_ref()?;
        let scene = self.context.as_ref()?.scene.as_ref()?;
        if scene.is_full_model() && source.source == source.component {
            scene.isolated_occurrence_source(&source.path).ok()
        } else {
            scene.isolated_source(source.source).ok()
        }
    }
    pub(super) fn begin_occurrence_row(
        &mut self,
        session: &Session,
        request: sketch_external::OccurrenceCaptureRequest,
    ) -> Result<(), String> {
        let sketch = request.query.sketch;
        let context = self.context.as_ref().ok_or("Open the owning sketch")?;
        if context.sketch != sketch {
            return Err("Open the owning sketch before editing its reference".into());
        }
        let (ticket, _) = scope(session, sketch)?;
        let form = Form::new(
            session.document(),
            sketch,
            super::ExternalTool::Project,
            Some(request.clone()),
        )?;
        self.queries.cancel();
        self.active = Some(super::controller::Active {
            ticket,
            purpose: Purpose::Repair(request.query.reference),
            method: if request.query.method == sketch_external::QueryMethod::Intersect {
                sketch_external::Method::Intersect
            } else {
                sketch_external::Method::Coplanar
            },
            behavior: request.behavior,
            filter: if request.query.intent.kind == multicad_core::references::TopologyKind::Face {
                crate::viewport::PickFilter::Faces
            } else if request.query.intent.kind == multicad_core::references::TopologyKind::Vertex {
                crate::viewport::PickFilter::Vertices
            } else {
                crate::viewport::PickFilter::Edges
            },
            selected: None,
            request: None,
            include_hidden: false,
            inspected_source: None,
            form: Some(Box::new(form)),
        });
        self.error = None;
        self.update_occurrence_view(session);
        Ok(())
    }
    pub(super) fn remove_occurrence(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        reference: EntityId,
    ) -> Result<(), String> {
        let sketch = self
            .context
            .as_ref()
            .ok_or("Open the owning sketch")?
            .sketch;
        self.queries.begin(
            session,
            resources,
            Actor::local(Channel::Ui),
            Task::Remove { sketch, reference },
        )
    }
}
fn consumer_pose(context: &Context, consumer: &Consumer) -> Option<Transform> {
    match consumer {
        Consumer::World => Some(Transform::default()),
        Consumer::Unset => None,
        Consumer::Occurrence(chosen) => {
            let scene = context.scene.as_ref()?;
            let receipt = scene.assembly_evaluation()?;
            if receipt.source_hash != context.hash {
                return None;
            }
            let leaf = receipt
                .leaves
                .iter()
                .find(|leaf| leaf.path == chosen.path && leaf.component == chosen.component)?;
            Transform::from_rigid_pose(leaf.pose).ok()
        }
    }
}

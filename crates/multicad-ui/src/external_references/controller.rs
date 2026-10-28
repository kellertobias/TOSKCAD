use super::ExternalTool;
use crate::{
    camera::Camera,
    egui,
    viewport::{Hit, PickFilter, PickOptions, Scene, SectionPlane, Selection, Transform},
};
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    constraint_sketch::source::Source,
    document::{EntityData, EntityId},
    project_file::ContentHash,
    references::{GeometryIntent, TopologyKind, TopologySnapshot},
    sketch_external::{self, Behavior, CaptureInput, CaptureRequest, Method},
};
use multicad_solver::SketchEdit;
use std::{collections::BTreeMap, sync::Arc};
pub(crate) struct Proposal {
    pub reference: EntityId,
    pub edit: SketchEdit,
    pub label: &'static str,
    pub report: multicad_core::references::ReferenceReport,
}
pub(super) struct Context {
    pub(super) ticket: Prepared,
    pub(super) hash: ContentHash,
    pub(super) sketch: EntityId,
    pub(super) scene: Option<Arc<Scene>>,
    pub(super) snapshots: BTreeMap<EntityId, TopologySnapshot>,
    pub(super) pose: Transform,
}
#[derive(Clone, Copy)]
pub(super) enum Purpose {
    Capture,
    Repair(EntityId),
}
pub(super) struct Active {
    pub(super) ticket: Prepared,
    pub purpose: Purpose,
    pub method: Method,
    pub behavior: Behavior,
    pub filter: PickFilter,
    pub selected: Option<Hit>,
    pub request: Option<CaptureRequest>,
    pub include_hidden: bool,
    pub inspected_source: Option<EntityId>,
}
#[derive(Default)]
pub struct Controller {
    pub(super) native: super::native::Native,
    pub(super) context: Option<Context>,
    pub(super) active: Option<Active>,
    hover: Option<Hit>,
    pub(super) error: Option<String>,
}
pub struct Pointer<'a> {
    pub camera: Camera,
    pub rect: egui::Rect,
    pub position: Option<egui::Pos2>,
    pub clicked: bool,
    pub section: Option<SectionPlane>,
    pub session: &'a Session,
}
impl Controller {
    pub fn active(&self) -> bool {
        self.active.is_some()
    }
    pub fn cancel(&mut self) {
        self.active = None;
        self.hover = None;
        self.error = None;
        self.native.clear();
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn refresh(
        &mut self,
        session: &Session,
        sketch: Option<EntityId>,
        scene: Option<Arc<Scene>>,
        pose: Transform,
    ) {
        let Some(sketch) = sketch else {
            self.cancel();
            self.context = None;
            return;
        };
        if self.context.as_ref().is_none_or(|context| {
            context.sketch != sketch || session.validate_preview(&context.ticket).is_err()
        }) {
            match scope(session, sketch) {
                Ok((ticket, hash)) => {
                    self.native.clear();
                    self.context = Some(Context {
                        ticket,
                        hash,
                        sketch,
                        scene: None,
                        snapshots: BTreeMap::new(),
                        pose,
                    })
                }
                Err(error) => {
                    self.error = Some(error);
                    self.context = None;
                    return;
                }
            }
        }
        let context = self.context.as_mut().expect("checked context");
        context.pose = pose;
        let canonical = scene.filter(|scene| {
            scene.is_full_model()
                && scene.stamp().document == session.document().id()
                && scene.stamp().revision == session.revision()
                && scene
                    .model_evaluation()
                    .is_some_and(|info| info.source_hash == context.hash)
        });
        let current = super::native::current_source(&self.native, context, session).or(canonical);
        if context.scene.as_ref().map(|scene| scene.stamp())
            != current.as_ref().map(|scene| scene.stamp())
        {
            context.snapshots = current
                .as_ref()
                .map(|scene| scene.reference_snapshots())
                .unwrap_or_default();
        }
        context.scene = current;
        if let Some(active) = &mut self.active {
            if session.validate_preview(&active.ticket).is_err() {
                active.selected = None;
                active.request = None;
                self.hover = None;
                self.error =
                    Some("The source changed. Cancel or start the reference tool again".into());
            } else if active.selected.as_ref().is_some_and(|hit| {
                context
                    .scene
                    .as_ref()
                    .is_none_or(|scene| scene.stamp() != hit.stamp)
            }) {
                active.selected = None;
                active.request = None;
                self.hover = None;
                self.error = Some("Native geometry changed. Select the source again".into());
            }
        }
    }
    pub fn ready(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(|context| context.scene.is_some() && context.pose == Transform::default())
    }
    pub fn ready_for(&self, session: &Session) -> bool {
        self.ready()
            && self
                .context
                .as_ref()
                .is_some_and(|context| session.validate_preview(&context.ticket).is_ok())
    }
    pub fn can_preview(&self, session: &Session) -> bool {
        self.ready_for(session)
            && self.active.as_ref().is_some_and(|active| {
                session.validate_preview(&active.ticket).is_ok()
                    && active.selected.as_ref().is_some_and(|hit| {
                        self.context
                            .as_ref()
                            .and_then(|context| context.scene.as_ref())
                            .is_some_and(|scene| scene.stamp() == hit.stamp)
                    })
            })
    }
    pub fn begin(&mut self, session: &Session, tool: ExternalTool) -> Result<(), String> {
        let context = self.context.as_ref().ok_or("Open a sketch first")?;
        session
            .validate_preview(&context.ticket)
            .map_err(|e| e.to_string())?;
        if !self.ready() {
            return Err(self.unavailable());
        }
        let (ticket, _) = scope(session, context.sketch)?;
        self.active = Some(Active {
            ticket,
            purpose: Purpose::Capture,
            method: tool.method(),
            behavior: Behavior::ReferenceOnly,
            filter: PickFilter::Edges,
            selected: None,
            request: None,
            include_hidden: false,
            inspected_source: None,
        });
        self.hover = None;
        self.error = None;
        Ok(())
    }
    pub(super) fn begin_repair(
        &mut self,
        session: &Session,
        reference: EntityId,
    ) -> Result<(), String> {
        let context = self.context.as_ref().ok_or("Open the owning sketch")?;
        session
            .validate_preview(&context.ticket)
            .map_err(|error| error.to_string())?;
        if !self.can_start_repair(session) {
            return Err(self.unavailable());
        }
        let binding = sketch_external::bindings(session.document(), context.sketch)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|binding| binding.reference == reference)
            .ok_or("Reference is not owned by the active sketch")?;
        let (ticket, _) = scope(session, context.sketch)?;
        self.active = Some(Active {
            ticket,
            purpose: Purpose::Repair(reference),
            method: binding.definition.method,
            behavior: binding.definition.behavior,
            filter: if binding.intent.kind == TopologyKind::Vertex {
                PickFilter::Vertices
            } else {
                PickFilter::Edges
            },
            selected: None,
            request: None,
            include_hidden: false,
            inspected_source: None,
        });
        self.error = None;
        self.hover = None;
        Ok(())
    }
    pub fn scene(&self) -> Option<Arc<Scene>> {
        let active = self.active.as_ref()?;
        let scene = self.context.as_ref()?.scene.clone()?;
        if let Some(source) = active.inspected_source {
            return scene.isolated_source(source).ok();
        }
        Some(scene)
    }
    pub fn highlight(&self) -> Option<Selection> {
        let hit = self
            .hover
            .as_ref()
            .or_else(|| self.active.as_ref()?.selected.as_ref())?;
        Some(match hit.key.kind {
            TopologyKind::Face => Selection::Face {
                instance: hit.instance,
                ordinal: hit.key.ordinal,
            },
            TopologyKind::Edge => Selection::Edge {
                instance: hit.instance,
                ordinal: hit.key.ordinal,
            },
            TopologyKind::Vertex => Selection::Vertex {
                instance: hit.instance,
                ordinal: hit.key.ordinal,
            },
        })
    }
    pub fn pointer(&mut self, input: Pointer<'_>) -> bool {
        let Some(active) = &self.active else {
            return false;
        };
        let Some(_context) = &self.context else {
            return false;
        };
        if input.session.validate_preview(&active.ticket).is_err() || !self.ready() {
            self.hover = None;
            return false;
        }
        let filter = if active.method == Method::Intersect {
            PickFilter::Edges
        } else {
            active.filter
        };
        self.hover = input.position.and_then(|position| {
            self.scene()?.pick_screen_with(
                input.camera,
                input.rect,
                position,
                PickOptions {
                    filter,
                    include_hidden: active.include_hidden,
                    section: input.section,
                },
            )
        });
        if input.clicked {
            let result = self
                .hover
                .clone()
                .ok_or_else(|| "Select an exact native edge or vertex".to_string())
                .and_then(|hit| self.select(input.session, hit));
            match result {
                Ok(()) => return true,
                Err(error) => self.error = Some(error),
            }
        }
        false
    }
    pub(super) fn select(&mut self, session: &Session, hit: Hit) -> Result<(), String> {
        if let Some(active) = &mut self.active {
            active.selected = None;
            active.request = None;
        }
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
            .ok_or("Current native source geometry is unavailable")?;
        if hit.stamp != scene.stamp() {
            return Err("Selected geometry is stale".into());
        }
        let instance = scene
            .reference_instances()
            .iter()
            .find(|instance| instance.id == hit.instance && instance.geometry == hit.geometry);
        let isolated = active.inspected_source == Some(hit.geometry)
            && hit.instance == hit.geometry
            && scene.geometries().contains_key(&hit.geometry);
        if instance.is_none() && !isolated {
            return Err("Selected source is not in this canonical native scene".into());
        }
        if instance.is_some_and(|instance| instance.transform != Transform::default())
            || context.pose != Transform::default()
        {
            return Err("Placed occurrence references require the occurrence-aware reference adapter. Select an unplaced source in the sketch definition context".into());
        }
        if !matches!(
            session
                .document()
                .get(hit.geometry)
                .map(|entity| &entity.data),
            Some(EntityData::Feature { .. })
        ) {
            return Err("Select a canonical source feature".into());
        }
        let snapshot = context
            .snapshots
            .get(&hit.geometry)
            .ok_or("Current source topology is unavailable")?;
        if snapshot
            .elements
            .iter()
            .find(|element| element.key == hit.key)
            .is_none_or(|element| element.geometry.is_none())
        {
            return Err("This source has no supported exact analytic descriptor. Projection supports points, straight lines, circles and circular arcs; plane intersection supports finite straight edges".into());
        }
        let intent = GeometryIntent::capture(snapshot, hit.key)?;
        let request = match active.purpose {
            Purpose::Capture => Some(
                CaptureRequest::new(
                    session.document(),
                    session.revision(),
                    CaptureInput {
                        sketch: context.sketch,
                        source: hit.geometry,
                        intent,
                        method: active.method,
                        behavior: active.behavior,
                    },
                    &context.snapshots,
                )
                .map_err(|e| e.to_string())?,
            ),
            Purpose::Repair(reference) => {
                let binding = sketch_external::read(session.document(), reference)
                    .map_err(|e| e.to_string())?;
                if binding.intent.kind != hit.key.kind {
                    return Err("Replacement must have the same topology kind".into());
                }
                None
            }
        };
        let active = self.active.as_mut().expect("checked active");
        active.selected = Some(hit);
        active.request = request;
        self.error = None;
        Ok(())
    }
    pub(crate) fn prepare(&self, session: &Session, channel: Channel) -> Result<Proposal, String> {
        let context = self.context.as_ref().ok_or("Open a sketch")?;
        let active = self.active.as_ref().ok_or("Start a reference tool")?;
        session
            .validate_preview(&context.ticket)
            .map_err(|e| e.to_string())?;
        session
            .validate_preview(&active.ticket)
            .map_err(|e| e.to_string())?;
        let hit = active.selected.as_ref().ok_or("Select a source first")?;
        if context
            .scene
            .as_ref()
            .is_none_or(|scene| scene.stamp() != hit.stamp)
        {
            return Err("Native geometry changed. Pick again".into());
        }
        let (reference, edit, label) = match active.purpose {
            Purpose::Capture => {
                let mut request = active
                    .request
                    .clone()
                    .ok_or("Choose supported source geometry")?;
                request.input.behavior = active.behavior;
                let result = multicad_solver::prepare_external_reference(
                    session,
                    session.revision(),
                    Actor::local(channel),
                    &request,
                    &context.snapshots,
                )
                .map_err(|e| e.to_string())?;
                (result.reference, result.edit, "Capture external reference")
            }
            Purpose::Repair(reference) => {
                let edit = multicad_solver::prepare_external_repair(
                    session,
                    session.revision(),
                    Actor::local(channel),
                    &multicad_solver::ExternalRepairRequest {
                        sketch: context.sketch,
                        reference,
                        source: hit.geometry,
                        selected: hit.key,
                    },
                    &context.snapshots,
                )
                .map_err(|e| e.to_string())?;
                (reference, edit, "Repair external reference")
            }
        };
        checked(
            reference,
            edit,
            label,
            session.revision(),
            &context.snapshots,
        )
    }
    pub(super) fn remove(
        &self,
        session: &Session,
        reference: EntityId,
    ) -> Result<Proposal, String> {
        let context = self.context.as_ref().ok_or("Open a sketch")?;
        session
            .validate_preview(&context.ticket)
            .map_err(|e| e.to_string())?;
        let edit = multicad_solver::prepare_remove_external_reference(
            session,
            session.revision(),
            Actor::local(Channel::Ui),
            context.sketch,
            reference,
            &context.snapshots,
        )
        .map_err(|e| e.to_string())?;
        checked(
            reference,
            edit,
            "Remove external reference",
            session.revision(),
            &context.snapshots,
        )
    }
    pub(super) fn behavior(
        &self,
        session: &Session,
        reference: EntityId,
        behavior: Behavior,
    ) -> Result<Proposal, String> {
        let context = self.context.as_ref().ok_or("Open a sketch")?;
        session
            .validate_preview(&context.ticket)
            .map_err(|e| e.to_string())?;
        if context.scene.is_none() {
            return Err(
                "Rebuild current native source geometry before changing reference behavior".into(),
            );
        }
        let edit = multicad_solver::prepare_external_behavior(
            session,
            session.revision(),
            Actor::local(Channel::Ui),
            context.sketch,
            reference,
            behavior,
            &context.snapshots,
        )
        .map_err(|e| e.to_string())?;
        checked(
            reference,
            edit,
            "Change reference behavior",
            session.revision(),
            &context.snapshots,
        )
    }
    fn unavailable(&self) -> String {
        if self
            .context
            .as_ref()
            .is_some_and(|context| context.pose != Transform::default())
        {
            return "Placed sketch contexts require the occurrence-aware reference adapter".into();
        }
        "Rebuild the current native model before selecting external geometry".into()
    }
    pub fn protected(
        document: &multicad_core::document::Document,
        sketch: EntityId,
        id: EntityId,
    ) -> bool {
        sketch_external::bindings(document, sketch).is_ok_and(|bindings| {
            bindings.iter().any(|binding| {
                binding.definition.output.curve() == Some(id)
                    || binding.definition.output.points().contains(&id)
            })
        })
    }
}
fn scope(session: &Session, sketch: EntityId) -> Result<(Prepared, ContentHash), String> {
    let entity = session.document().get(sketch).ok_or("Sketch is missing")?;
    let EntityData::Sketch { definition, .. } = &entity.data else {
        return Err("Select a sketch".into());
    };
    Source::from_definition(definition).map_err(|e| e.to_string())?;
    let ticket = session
        .prepare(Request {
            expected_revision: session.revision(),
            actor: Actor::local(Channel::Ui),
            label: "Inspect external sketch reference".into(),
            edits: vec![],
        })
        .map_err(|e| e.to_string())?;
    let hash = ContentHash::document(session.document())?;
    Ok((ticket, hash))
}
fn checked(
    reference: EntityId,
    edit: SketchEdit,
    label: &'static str,
    revision: u64,
    snapshots: &BTreeMap<EntityId, TopologySnapshot>,
) -> Result<Proposal, String> {
    if edit.preview.is_none() {
        return Err(format!(
            "Reference edit cannot solve the consumer sketch: {:?}",
            edit.evaluation.diagnostics.state
        ));
    }
    let report = multicad_core::references::inspect(
        edit.preview.as_ref().expect("checked preview").document(),
        revision,
        snapshots,
    );
    Ok(Proposal {
        report,
        reference,
        edit,
        label,
    })
}

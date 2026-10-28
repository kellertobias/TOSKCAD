use super::inspection::{AssemblyInspectionBackendFactory, InspectionPanel};
use super::{
    AnchorPick, AnchorSide, AssemblyOutcome, AssemblySelection, AssemblyTool,
    forms::{Form, Operation},
};
use crate::{
    egui,
    inspector_actions::{self, Button, Owner},
    viewport::{Scene, SceneBackend, SceneStamp, VisibilityFilter},
};
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    document::{EntityData, EntityId},
    model,
    project_file::{ContentHash, ProjectResources},
    references::{self, Resolution},
};
use std::{collections::BTreeMap, sync::Arc};
struct Scope {
    guard: Prepared,
    source: ContentHash,
    resources: ProjectResources,
    channel: Channel,
}
struct Candidate {
    prepared: Prepared,
    source: ContentHash,
    stamp: SceneStamp,
    version: u64,
    selected: Vec<EntityId>,
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
pub struct AssemblyController {
    form: Option<Form>,
    scope: Option<Scope>,
    backend: Option<Box<dyn SceneBackend>>,
    candidate: Option<Candidate>,
    handoff: Option<Handoff>,
    generation: u64,
    error: Option<String>,
    base_scene: Option<Arc<Scene>>,
    query_factory: Option<AssemblyInspectionBackendFactory>,
    inspection: Option<InspectionPanel>,
}
impl AssemblyController {
    pub fn set_query_factory(&mut self, factory: AssemblyInspectionBackendFactory) {
        if let Some(p) = &mut self.inspection {
            p.invalidate();
        }
        self.query_factory = Some(factory);
    }
    pub fn has_query_factory(&self) -> bool {
        self.query_factory.is_some()
    }
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
        self.form.as_ref().is_some_and(|f| f.tool.authoring())
    }
    pub fn label(&self) -> &'static str {
        self.form.as_ref().map_or("Assembly", |f| f.tool.label())
    }
    pub fn pending(&self) -> bool {
        self.candidate.as_ref().is_some_and(|c| c.pending)
            || self.inspection.as_ref().is_some_and(|p| p.pending)
    }
    pub fn begin(
        &mut self,
        kind: AssemblyTool,
        session: &Session,
        resources: &ProjectResources,
        scene: Option<&Arc<Scene>>,
        selection: AssemblySelection,
        channel: Channel,
    ) -> Result<(), String> {
        let form = Form::new(kind, session.document(), selection.clone())?;
        self.open(session, resources, scene, form, channel)?;
        if matches!(kind, AssemblyTool::Interference | AssemblyTool::MotionRange) {
            self.inspection = Some(InspectionPanel::new(
                kind == AssemblyTool::MotionRange,
                session.document(),
                self.base_scene.as_ref(),
                &selection,
                self.query_factory.as_ref(),
            ));
        }
        Ok(())
    }
    pub fn begin_existing(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: Option<&Arc<Scene>>,
        entity: EntityId,
        channel: Channel,
    ) -> Result<(), String> {
        let form = Form::existing(session.document(), entity)?;
        self.open(session, resources, scene, form, channel)
    }
    fn open(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: Option<&Arc<Scene>>,
        form: Form,
        channel: Channel,
    ) -> Result<(), String> {
        if self.active() {
            return Err("Apply or cancel the active assembly tool first".into());
        }
        if form.tool.authoring() && !self.has_backend() {
            return Err("The native assembly preview provider is unavailable".into());
        }
        let guard = session
            .prepare(Request {
                expected_revision: session.revision(),
                actor: Actor::local(channel),
                label: "Open assembly controls".into(),
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
        self.base_scene = scene
            .filter(|s| {
                s.is_full_model()
                    && s.stamp().document == session.document().id()
                    && s.stamp().revision == session.revision()
                    && s.model_evaluation().is_some_and(|i| {
                        Some(i.source_hash) == self.scope.as_ref().map(|s| s.source)
                    })
            })
            .cloned();
        if let Err(error) = self.preserve_placement(session) {
            self.cancel();
            return Err(error);
        }
        Ok(())
    }
    fn current(&self, session: &Session, resources: &ProjectResources) -> Result<(), String> {
        let scope = self.scope.as_ref().ok_or("No assembly tool is active")?;
        session.validate_preview(&scope.guard).map_err(|_| {
            "The source session changed; cancel and reopen this assembly tool".to_string()
        })?;
        if &scope.resources != resources
            || ContentHash::document(session.document())? != scope.source
        {
            return Err(
                "The source or native resources changed; cancel and reopen this assembly tool"
                    .into(),
            );
        }
        Ok(())
    }
    pub fn cancel(&mut self) {
        if let Some(b) = &mut self.backend {
            b.cancel();
        }
        self.form = None;
        self.scope = None;
        self.candidate = None;
        self.handoff = None;
        self.error = None;
        self.base_scene = None;
        if let Some(p) = &mut self.inspection {
            p.invalidate();
        }
        self.inspection = None;
    }
    pub fn preview(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let result = if self.inspection.is_some() {
            self.current(session, resources).and_then(|_| {
                self.inspection
                    .as_mut()
                    .expect("inspection panel")
                    .request(session)
            })
        } else {
            self.request_preview(session, resources, channel)
        };
        if let Err(error) = &result {
            self.error = Some(error.clone());
            self.candidate = None;
        } else {
            self.error = None;
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
        let form = self.form.as_mut().ok_or("No assembly tool is active")?;
        form.pick = None;
        let prepared = form.prepare(session, Actor::local(channel))?;
        let selected = prepared
            .instances
            .iter()
            .chain(&prepared.assemblies)
            .chain(&prepared.joints)
            .copied()
            .chain(prepared.pose)
            .collect();
        let source = ContentHash::document(prepared.prepared.document())?;
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("Assembly preview generation limit reached")?;
        let stamp = SceneStamp {
            document: session.document().id(),
            revision: session.revision(),
            generation: self.generation,
        };
        self.backend
            .as_mut()
            .ok_or("The native assembly preview provider is unavailable")?
            .request_evaluation(
                prepared.prepared.document(),
                resources,
                stamp,
                &Default::default(),
            )?;
        self.candidate = Some(Candidate {
            prepared: prepared.prepared,
            source,
            stamp,
            version: form.version,
            selected,
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
            self.base_scene = None;
            self.error = Some(error);
            if let Some(p) = &mut self.inspection {
                p.invalidate();
            }
        }
        if self.current(session, resources).is_ok()
            && let Some(p) = &mut self.inspection
        {
            p.poll(session);
        }
        for _ in 0..16 {
            let Some(reply) = self.backend.as_mut().and_then(|b| b.poll()) else {
                break;
            };
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
                    return Err("Assembly inputs changed; preview them again".into());
                }
                let scene = reply.result?;
                if !scene.is_full_model()
                    || scene.stamp() != candidate.stamp
                    || !scene
                        .model_evaluation()
                        .is_some_and(|i| i.source_hash == candidate.source)
                {
                    return Err("Assembly Apply requires a complete native evaluation of these exact candidate inputs".into());
                }
                super::proof::validate(
                    candidate.prepared.document(),
                    candidate.stamp.revision,
                    &scene,
                )?;
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
        self.dirty()
            && self.form.as_ref().is_some_and(|f| f.pick.is_none())
            && self.current(session, resources).is_ok()
            && self.error.is_none()
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
    ) -> Result<AssemblyOutcome, String> {
        self.current(session, resources)?;
        if !self.can_apply(session, resources) {
            return Err(
                "Wait for a successful native preview of the current assembly inputs".into(),
            );
        }
        let candidate = self.candidate.take().expect("checked candidate");
        if ContentHash::document(candidate.prepared.document())? != candidate.source {
            return Err("Assembly candidate content changed; preview again".into());
        }
        let label = self.label();
        let scene = candidate.scene.expect("checked native scene");
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
        Ok(AssemblyOutcome::Applied {
            selected: candidate.selected,
            label,
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
    pub fn take_handoff(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
    ) -> Option<Arc<Scene>> {
        let scene = self.display_scene(session, resources);
        self.handoff = None;
        scene
    }
    pub fn canonical_ready(&mut self, session: &Session) {
        if self.handoff.as_ref().is_some_and(|h| {
            h.document == session.document().id() && h.revision <= session.revision()
        }) {
            self.handoff = None;
        }
    }
    pub fn anchor_pick(&self) -> Option<AnchorPick> {
        self.form.as_ref()?.anchor_pick()
    }
    pub fn set_anchor_input(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        side: AnchorSide,
        input: model::AlignmentInput,
    ) -> Result<(), String> {
        self.current(session, resources)?;
        let pick = self
            .anchor_pick()
            .filter(|p| p.side == side)
            .ok_or("Select an anchor lane before picking a native frame")?;
        let Some(EntityData::Instance { component, .. }) =
            session.document().get(pick.instance).map(|e| &e.data)
        else {
            return Err("The chosen occurrence is missing".into());
        };
        if *component != input.source
            || !scene.is_full_model()
            || scene.stamp().document != session.document().id()
            || scene.stamp().revision != session.revision()
            || scene
                .model_evaluation()
                .is_none_or(|i| Some(i.source_hash) != self.scope.as_ref().map(|s| s.source))
        {
            return Err(
                "Choose a current component-local native frame on the indicated occurrence".into(),
            );
        }
        let snapshots = scene.reference_snapshots();
        let snapshot = snapshots
            .get(&input.source)
            .ok_or("The component's local native topology is unavailable")?;
        if input.intent.attachment.is_none()
            || !matches!(
                references::resolve(&input.intent, snapshot, session.revision()),
                Resolution::Resolved { frame: Some(_), .. }
            )
        {
            return Err("Choose an explicitly named, resolvable native frame".into());
        }
        let form = self.form.as_mut().expect("active form");
        let Operation::Joint(request) = &mut form.operation else {
            return Err("No relationship is being edited".into());
        };
        let anchor = match side {
            AnchorSide::Moving => &mut request.parameters.moving,
            AnchorSide::Target => &mut request.parameters.target,
        };
        if let model::JointAnchor::Geometry { reference } = anchor {
            request.references.retain(|r| r.intent.id != *reference);
        }
        *anchor = model::JointAnchor::Geometry {
            reference: input.intent.id,
        };
        request.references.push(input);
        form.pick = None;
        form.changed();
        self.candidate = None;
        self.error = None;
        Ok(())
    }
    fn preserve_placement(&mut self, session: &Session) -> Result<(), String> {
        let Some(form) = &mut self.form else {
            return Ok(());
        };
        if !form.preserve_placement {
            return Ok(());
        }
        form.preserve_placement = false;
        let Operation::Joint(request) = &mut form.operation else {
            return Ok(());
        };
        if request.owner.is_nil() {
            return Ok(());
        }
        let snapshots = self
            .base_scene
            .as_ref()
            .map(|s| s.reference_snapshots())
            .unwrap_or_default();
        let moving = model::resolve_instance_pose_with_snapshots(
            session.document(),
            request.owner,
            session.parameters(),
            session.revision(),
            &snapshots,
        )
        .map_err(|e| e.to_string())?;
        let parent = request
            .parameters
            .parent
            .map(|id| {
                model::resolve_instance_pose_with_snapshots(
                    session.document(),
                    id,
                    session.parameters(),
                    session.revision(),
                    &snapshots,
                )
            })
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let local = moving
            .then(parent.inverse().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        request.parameters.moving = Default::default();
        request.parameters.target = model::JointAnchor::Frame {
            frame: Box::new(model::PrimitiveFrame {
                origin: local.translation_mm.map(model::length),
                u: local.vector([1., 0., 0.]),
                v: local.vector([0., 1., 0.]),
            }),
        };
        request.references.clear();
        form.fields.reset();
        Ok(())
    }
    pub fn display_leaf_offsets(
        &self,
        session: &Session,
        resources: &ProjectResources,
    ) -> Option<BTreeMap<model::OccurrencePath, [f64; 3]>> {
        self.current(session, resources).ok()?;
        let form = self.form.as_ref()?;
        if form.tool != AssemblyTool::Explode {
            return None;
        }
        let scene = self.base_scene.as_ref()?;
        let receipt = scene.assembly_evaluation()?;
        let leaves = receipt
            .leaves
            .iter()
            .map(|leaf| {
                let bounds = scene.native_instance_bounds(leaf.producer)?;
                let local = std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) / 2.);
                Some((leaf.path.clone(), leaf.pose.point(local)))
            })
            .collect::<Option<Vec<_>>>()?;
        if leaves.is_empty() {
            return Some(BTreeMap::new());
        }
        let center = std::array::from_fn::<_, 3, _>(|i| {
            leaves.iter().map(|(_, p)| p[i]).sum::<f64>() / leaves.len() as f64
        });
        Some(
            leaves
                .into_iter()
                .enumerate()
                .map(|(index, (path, p))| {
                    let d = std::array::from_fn::<_, 3, _>(|i| p[i] - center[i]);
                    let n = d.iter().map(|v| v * v).sum::<f64>().sqrt();
                    let axis = if n > 1e-12 {
                        d.map(|v| v / n)
                    } else {
                        let mut a = [0.; 3];
                        a[index % 3] = if index % 2 == 0 { 1. } else { -1. };
                        a
                    };
                    (path, axis.map(|v| v * form.explode_mm))
                })
                .collect(),
        )
    }
    pub fn display_offsets(
        &self,
        session: &Session,
        resources: &ProjectResources,
    ) -> Option<BTreeMap<EntityId, [f64; 3]>> {
        self.current(session, resources).ok()?;
        let form = self.form.as_ref()?;
        if form.tool != AssemblyTool::Explode {
            return None;
        }
        let scene = self.base_scene.as_ref()?;
        let roots=session.document().entities().filter(|e|matches!(e.data,EntityData::Instance{..})).filter(|e|!session.document().entities().any(|a|matches!(&a.data,EntityData::Assembly{instances}if instances.contains(&e.id)))).filter_map(|e|scene.instance_bounds(e.id).map(|b|(e.id,b))).collect::<Vec<_>>();
        if roots.is_empty() {
            return Some(BTreeMap::new());
        }
        let centers = roots
            .iter()
            .map(|(_, b)| std::array::from_fn::<_, 3, _>(|i| (b[0][i] + b[1][i]) / 2.))
            .collect::<Vec<_>>();
        let center = std::array::from_fn::<_, 3, _>(|i| {
            centers.iter().map(|p| p[i]).sum::<f64>() / centers.len() as f64
        });
        Some(
            roots
                .iter()
                .zip(centers)
                .enumerate()
                .map(|(index, ((id, _), p))| {
                    let d = std::array::from_fn::<_, 3, _>(|i| p[i] - center[i]);
                    let n = d.iter().map(|v| v * v).sum::<f64>().sqrt();
                    let direction = if n > 1e-12 {
                        d.map(|v| v / n)
                    } else {
                        let mut d = [0.; 3];
                        d[index % 3] = if index % 2 == 0 { 1. } else { -1. };
                        d
                    };
                    (*id, direction.map(|v| v * form.explode_mm))
                })
                .collect(),
        )
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        resources: &ProjectResources,
    ) -> Result<Option<AssemblyOutcome>, String> {
        let Some(form) = &mut self.form else {
            return Ok(None);
        };
        crate::controls::inspector_layout(ui);
        ui.heading(form.tool.label());
        let changed = form.show(ui, session.document());
        if changed {
            self.candidate = None;
            self.error = None;
        }
        self.preserve_placement(session)?;
        let form = self.form.as_mut().expect("active form");
        if form.tool == AssemblyTool::Explode {
            ui.small("Display only · separate physical Part occurrences, including nested members. Inspection uses assembled geometry.");
            if ui
                .add(egui::Slider::new(&mut form.explode_mm, 0.0..=500.).text("Separation (mm)"))
                .changed()
            {
                form.changed();
            }
        }
        if let Some(p) = &mut self.inspection {
            p.show(ui, session);
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        } else if self.pending() {
            ui.spinner();
            ui.label("Checking native assembly…");
        } else if self.can_apply(session, resources) {
            ui.label("Native preview ready");
        }
        let authoring = self.dirty();
        let actions = inspector_actions::row(
            ui,
            Owner::Assembly,
            (authoring || self.inspection.is_some()).then(|| {
                Button::new(
                    "Preview",
                    !self.pending(),
                    "Check the current candidate using the native geometry engine",
                )
            }),
            Button::new(
                "Apply",
                self.can_apply(session, resources),
                "Commit this checked assembly transaction",
            ),
            Button::new(
                "Cancel",
                true,
                "Close these controls and discard uncommitted edits",
            ),
        );
        if actions.cancel {
            self.cancel();
            return Ok(Some(AssemblyOutcome::Cancelled));
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
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "native_tests.rs"]
mod native_tests;

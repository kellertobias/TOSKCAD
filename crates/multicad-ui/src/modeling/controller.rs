//! A candidate evaluation lane, separate from canonical display publication.
use super::{
    ExtrudeCapabilities, ExtrudePanel, FormStamp, ModelRequest, ModelingTool, PreviewState,
    advanced::AdvancedPanel,
    alignment_panel::AlignmentPanel,
    boolean::BooleanPanel,
    boolean_edit::BooleanEditPanel,
    edge_treatment::EdgeTreatmentPanel,
    extrude_operation::{BooleanIdentities, CutDiscovery, OperationControls},
    form::{ModelEvent, ModelForm, ToolProof},
    group_edit::GroupEditPanel,
    names::GeneratedNames,
    placement::PlacementPanel,
    primitive::PrimitivePanel,
    revolve::RevolvePanel,
    scaling::ScalePanel,
    split::SplitPanel,
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
use proof::SceneProof;
use std::sync::Arc;

struct Scope {
    /// A source-neutral transaction also binds a newly opened form to the
    /// originating Session before there is any extrusion transaction.
    session: Prepared,
    source: ContentHash,
    resources: ProjectResources,
    execution: Execution,
    adoption: Option<multicad_core::model::ExtrudeGroupDraft>,
}
#[derive(Clone, Copy)]
enum Execution {
    Create,
    Edit { feature: EntityId },
    EditGroup { group: EntityId },
    LegacyExtrusion { feature: EntityId, group: EntityId },
}
struct Candidate {
    prepared: Prepared,
    content: ContentHash,
    form: FormStamp,
    stamp: SceneStamp,
    outputs: Vec<CreatedBody>,
    instances: Vec<EntityId>,
    placement: bool,
    tools: Vec<ToolProof>,
    proof: SceneProof,
    pending: bool,
    scene: Option<Arc<Scene>>,
    requires_info: bool,
}
struct Discovery {
    cut: CutDiscovery,
    form: FormStamp,
    stamp: SceneStamp,
    channel: Channel,
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
    advanced_context: Option<advanced::AdvancedContext>,
    backend: Option<Box<dyn SceneBackend>>,
    scope: Option<Scope>,
    candidate: Option<Candidate>,
    discovery: Option<Discovery>,
    operations: Option<OperationControls>,
    identities: BooleanIdentities,
    handoff: Option<Handoff>,
    generation: u64,
    error: Option<String>,
    context_valid: bool,
    context_attempt: Option<SceneStamp>,
    context_notice: Option<String>,
    placement_interaction: super::gizmo::Interaction,
    placement_confirm: Option<FormStamp>,
    scale_interaction: super::scale_gizmo::Interaction,
    scale_confirm: Option<FormStamp>,
    scale_host_basis: Option<super::GizmoBasis>,
    operation: operation_interaction::OperationState,
    boolean_preset: Option<multicad_core::model::Operation>,
}
pub enum ModelToolOutcome {
    Applied(Vec<CreatedBody>),
    Placed {
        parts: Vec<CreatedBody>,
        instances: Vec<EntityId>,
    },
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
        match self.scope.as_ref().map(|scope| scope.execution) {
            Some(Execution::EditGroup { .. }) => "Extrusion operation edit",
            Some(Execution::Edit { .. }) => "Feature edit",
            Some(Execution::LegacyExtrusion { .. }) => "Extrusion edit",
            _ => self.form.as_ref().map_or("Modeling", ModelForm::label),
        }
    }
    pub fn editing(&self) -> bool {
        self.scope.as_ref().is_some_and(|scope| {
            matches!(
                scope.execution,
                Execution::Edit { .. }
                    | Execution::EditGroup { .. }
                    | Execution::LegacyExtrusion { .. }
            )
        })
    }
    pub fn editable(document: &multicad_core::document::Document, feature: EntityId) -> bool {
        if !matches!(
            document.get(feature).map(|entity| &entity.data),
            Some(
                EntityData::Feature {
                    suppressed: false,
                    ..
                } | EntityData::Extension { .. }
            )
        ) {
            return false;
        }

        if super::AdvancedCommand::editable(document, feature)
            || super::EdgeTreatmentCommand::editable(document, feature)
            || super::ScaleCommand::editable(document, feature)
            || super::PlacementCommand::editable(document, feature)
            || multicad_core::model::read_alignment(document, feature).is_ok()
            || multicad_core::model::read_instance_alignment(document, feature).is_ok()
        {
            return true;
        }
        matches!(document.get(feature).map(|entity|&entity.data),Some(EntityData::Feature{definition,suppressed:false,..}) if matches!(definition.type_id.as_str(),multicad_core::model::EXTRUDE_TYPE|multicad_core::model::PRIMITIVE_TYPE|multicad_core::model::REVOLVE_TYPE|multicad_core::model::BOOLEAN_TYPE|multicad_core::model::WALL_TYPE|multicad_core::model::PANEL_TYPE|multicad_core::model::PANEL_JOINT_CUT_TYPE|multicad_core::model::MACHINING_CUT_TYPE|multicad_core::model::AID_TYPE|multicad_core::model::MEMBER_TYPE|multicad_core::model::SHEET_TYPE))
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
            return Err(
                "Choose a supported, unsuppressed applied feature or owned placement driver".into(),
            );
        }
        if multicad_core::model::read_alignment(document, feature).is_ok()
            || multicad_core::model::read_instance_alignment(document, feature).is_ok()
        {
            let panel = AlignmentPanel::existing(document, feature)?;
            self.begin_form(
                session,
                resources,
                channel,
                ModelForm::Alignment(Box::new(panel)),
            )?;
            self.scope.as_mut().expect("opened scope").execution = Execution::Edit { feature };
            return Ok(());
        }
        if super::AdvancedCommand::editable(document, feature) {
            let panel = AdvancedPanel::existing(document, feature)?;
            self.begin_form(
                session,
                resources,
                channel,
                ModelForm::Advanced(Box::new(panel)),
            )?;
            self.seed_advanced_context(session);
            self.scope.as_mut().expect("opened scope").execution = Execution::Edit { feature };
            return Ok(());
        }
        if super::EdgeTreatmentCommand::editable(document, feature) {
            let panel = EdgeTreatmentPanel::existing(document, feature)?;
            self.begin_form(
                session,
                resources,
                channel,
                ModelForm::EdgeTreatment(Box::new(panel)),
            )?;
            self.scope.as_mut().expect("opened scope").execution = Execution::Edit { feature };
            return Ok(());
        }
        if super::ScaleCommand::editable(document, feature) {
            let panel = ScalePanel::existing(document, feature)?;
            self.begin_form(
                session,
                resources,
                channel,
                ModelForm::Scale(Box::new(panel)),
            )?;
            self.scope.as_mut().expect("opened scope").execution = Execution::Edit { feature };
            return Ok(());
        }
        if super::PlacementCommand::editable(document, feature) {
            let panel = PlacementPanel::existing(document, feature)?;
            self.begin_form(
                session,
                resources,
                channel,
                ModelForm::Placement(Box::new(panel)),
            )?;
            self.scope.as_mut().expect("opened scope").execution = Execution::Edit { feature };
            return Ok(());
        }
        if !Self::editable(document, feature) {
            return Err("Choose a supported, unsuppressed applied model feature".into());
        }
        if model::read_sheet(document, feature).is_ok() {
            let form = super::sheet::SheetPanel::existing(document, feature)?;
            self.begin_form(
                session,
                resources,
                channel,
                ModelForm::Sheet(Box::new(form)),
            )?;
            self.scope.as_mut().expect("opened scope").execution = Execution::Edit { feature };
            return Ok(());
        }
        if model::read_member(document, feature).is_ok() {
            let form = super::weldment::WeldmentPanel::existing(document, feature)?;
            self.begin_form(
                session,
                resources,
                channel,
                ModelForm::Weldment(Box::new(form)),
            )?;
            self.scope.as_mut().expect("opened scope").execution = Execution::Edit { feature };
            return Ok(());
        }
        if model::read_machining_cut(document, feature).is_ok() {
            let form = super::machining::MachiningPanel::existing(document, feature)?;
            return self.begin_form(
                session,
                resources,
                channel,
                ModelForm::Machining(Box::new(form)),
            );
        }
        if model::read_panel_joint_cut(document, feature).is_ok() {
            // A joint edit rebuilds every pair's cuts, so it runs as a
            // source change like a carcass edit.
            let form = super::joint::JointPanel::existing(document, feature)?;
            return self.begin_form(
                session,
                resources,
                channel,
                ModelForm::Joint(Box::new(form)),
            );
        }
        if model::read_aid(document, feature).is_ok() {
            let form = super::fdm_aid::FdmAidPanel::existing(document, feature)?;
            self.begin_form(
                session,
                resources,
                channel,
                ModelForm::FdmAid(Box::new(form)),
            )?;
            self.scope.as_mut().expect("opened scope").execution = Execution::Edit { feature };
            return Ok(());
        }
        if let Ok(panel) = model::read_panel(document, feature) {
            let form = super::cabinet::CabinetPanel::existing(document, feature)?;
            self.begin_form(
                session,
                resources,
                channel,
                ModelForm::Cabinet(Box::new(form)),
            )?;
            // A carcass edit adds and removes panels, so it runs as a source
            // change; a rectangle panel edit keeps its feature.
            if !matches!(panel.source, model::PanelSource::Carcass { .. }) {
                self.scope.as_mut().expect("opened scope").execution = Execution::Edit { feature };
            }
            return Ok(());
        }
        if let Some(group) =
            model::extrude_group_owner(document, feature).map_err(|error| error.to_string())?
        {
            return self.begin_group(session, resources, group, channel);
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
                return self.begin_legacy_extrusion(session, resources, feature, channel);
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
            model::SLAB_TYPE | model::STAIR_TYPE => {
                let body = model::read_slab(document, feature)
                    .map(|s| s.output.body)
                    .or_else(|_| model::read_stair(document, feature).map(|s| s.output.body))
                    .map_err(|error| error.to_string())?;
                ModelForm::Architecture(Box::new(super::architecture::ArchitecturePanel::existing(
                    document,
                    feature,
                    body_name(body)?,
                )?))
            }
            model::WALL_TYPE => ModelForm::Wall(Box::new(super::wall::WallPanel::existing(
                document,
                feature,
                body_name(
                    model::read_wall(document, feature)
                        .map_err(|error| error.to_string())?
                        .output
                        .body,
                )?,
            )?)),
            model::REVOLVE_TYPE => {
                let source =
                    model::read_revolve(document, feature).map_err(|error| error.to_string())?;
                ModelForm::Revolve(Box::new(RevolvePanel::new(RevolveRequest {
                    sketch: source.profile.sketch_id().unwrap_or(EntityId::nil()),
                    outputs: vec![model::RevolveOutput {
                        feature,
                        body: source.output.body,
                        name: body_name(source.output.body)?,
                        region: source
                            .profile
                            .sketch()
                            .map(|profile| profile.region.clone())
                            .unwrap_or_else(model::empty_region),
                        face: model::stored_profile_face(document, feature),
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
        self.discovery.is_some()
            || self
                .candidate
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
        )?;
        self.operations = Some(OperationControls::new(session.document()));
        self.identities = BooleanIdentities::new(session.document());
        Ok(())
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
    pub fn begin_wall(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Wall", " feature");
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Wall(Box::new(super::wall::WallPanel::new(
                session.document(),
                name,
            )?)),
        )
    }
    /// Room door, window or vertical-cutout tool on `wall` (or the first
    /// wall); a known `opening` of that wall is reopened for editing.
    pub fn begin_opening(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        tool: ModelingTool,
        wall: Option<EntityId>,
        channel: Channel,
    ) -> Result<(), String> {
        let kind = match tool {
            ModelingTool::Door => super::opening::RoomOpening::Door,
            ModelingTool::Window => super::opening::RoomOpening::Window,
            ModelingTool::Cutout => super::opening::RoomOpening::Cutout,
            _ => return Err("Choose a door, window or vertical cutout tool".into()),
        };
        let panel = super::opening::OpeningPanel::new(session.document(), kind, wall)?;
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Opening(Box::new(panel)),
        )
    }
    /// The wall feature producing a selected Part, for the Room tools.
    pub fn wall_of(document: &multicad_core::document::Document, id: EntityId) -> Option<EntityId> {
        super::opening::wall_of_part(document, id)
    }
    pub fn opening_picking(&self) -> bool {
        matches!(&self.form, Some(ModelForm::Opening(panel)) if panel.picking())
    }
    pub fn opening_viewport(
        &mut self,
        session: &Session,
        view: super::TerminationViewport<'_>,
    ) -> bool {
        let changed = match self.form.as_mut() {
            Some(ModelForm::Opening(panel)) => {
                panel.viewport(session.document(), session.revision(), view)
            }
            _ => false,
        };
        if changed {
            self.invalidate();
        }
        changed
    }
    pub fn begin_sheet(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Sheet part", "");
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Sheet(Box::new(super::sheet::SheetPanel::new(
                session.document(),
                name,
            ))),
        )
    }
    pub fn begin_machining(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Machining", "");
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Machining(Box::new(super::machining::MachiningPanel::new(name))),
        )
    }
    pub fn begin_joint(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Joint", "");
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Joint(Box::new(super::joint::JointPanel::new(name))),
        )
    }
    pub fn begin_cabinet(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Cabinet", "");
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Cabinet(Box::new(super::cabinet::CabinetPanel::new(
                session.document(),
                name,
            ))),
        )
    }
    pub fn begin_weldment(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Member", "");
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Weldment(Box::new(super::weldment::WeldmentPanel::new(
                session.document(),
                name,
            ))),
        )
    }
    pub fn begin_weld(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Weld(Box::new(super::weld_form::WeldPanel::new(
                session.document(),
            ))),
        )
    }
    pub fn begin_weld_connection(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Joint", "");
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::WeldConnection(Box::new(super::weld_connection::ConnectionPanel::new(
                session.document(),
                name,
            ))),
        )
    }
    pub fn begin_fdm_aid(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Print aid", "");
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::FdmAid(Box::new(super::fdm_aid::FdmAidPanel::new(
                session.document(),
                name,
            ))),
        )
    }
    pub fn begin_architecture(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let name = if multicad_core::model::levels(session.document())
            .next()
            .is_none()
        {
            "Ground floor".to_owned()
        } else {
            GeneratedNames::new(session.document()).numbered("Slab", " feature")
        };
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Architecture(Box::new(super::architecture::ArchitecturePanel::new(
                session.document(),
                name,
            )?)),
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
            ModelForm::Revolve(Box::new(RevolvePanel::new(request)?.acquire_axis())),
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
        if let Some(panel) = self.form.as_mut().and_then(ModelForm::extrusion_mut) {
            panel.set_picking_available(false);
        }
        self.scope = Some(Scope {
            session: scope,
            source,
            resources: resources.clone(),
            execution: Execution::Create,
            adoption: None,
        });
        self.candidate = None;
        self.discovery = None;
        self.operations = None;
        self.identities = BooleanIdentities::default();
        self.handoff = None;
        self.error = None;
        self.context_valid = false;
        self.context_attempt = None;
        self.context_notice = None;
        Ok(())
    }
    pub fn placement_available(
        document: &multicad_core::document::Document,
        revision: u64,
        scene: &Scene,
        selected: &std::collections::BTreeSet<EntityId>,
    ) -> bool {
        super::placement::available(document, revision, scene, selected)
    }
    pub fn begin_placement(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        selected: &std::collections::BTreeSet<EntityId>,
        mode: super::PlacementMode,
        channel: Channel,
    ) -> Result<(), String> {
        let panel = PlacementPanel::new(
            session.document(),
            session.revision(),
            scene,
            selected,
            mode,
        )?;
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Placement(Box::new(panel)),
        )
    }
    pub fn begin_alignment(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        selected: &std::collections::BTreeSet<EntityId>,
        channel: Channel,
    ) -> Result<(), String> {
        let panel = AlignmentPanel::new(session.document(), session.revision(), scene, selected)?;
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Alignment(Box::new(panel)),
        )
    }
    pub fn alignment_picking(&self) -> bool {
        matches!(&self.form,Some(ModelForm::Alignment(panel)) if panel.picking())
    }
    pub fn alignment_highlight(&self) -> Option<crate::viewport::Selection> {
        match &self.form {
            Some(ModelForm::Alignment(panel)) => panel.highlight(),
            _ => None,
        }
    }
    pub fn alignment_viewport(
        &mut self,
        session: &Session,
        view: super::TerminationViewport<'_>,
    ) -> bool {
        let Some(scope) = &self.scope else {
            return false;
        };
        if session.validate_preview(&scope.session).is_err() {
            return false;
        }
        let changed = match self.form.as_mut() {
            Some(ModelForm::Alignment(panel)) => {
                panel.viewport(session.document(), session.revision(), scope.source, view)
            }
            _ => false,
        };
        if changed {
            self.invalidate();
        }
        changed
    }
    pub fn boolean_available(
        document: &multicad_core::document::Document,
        revision: u64,
        scene: &Scene,
    ) -> bool {
        BooleanPanel::available(document, revision, scene).is_ok_and(|count| count >= 2)
    }
    pub fn split_available(
        document: &multicad_core::document::Document,
        revision: u64,
        scene: &Scene,
    ) -> bool {
        SplitPanel::available(document, revision, scene).is_ok_and(|count| count >= 2)
    }
    pub fn begin_split(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        selected: &std::collections::BTreeSet<EntityId>,
        channel: Channel,
    ) -> Result<(), String> {
        let panel = SplitPanel::new(session.document(), session.revision(), scene, selected)?;
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Split(Box::new(panel)),
        )
    }
    /// Operation the next Boolean form starts with; `None` restores Subtract.
    pub fn set_boolean_preset(&mut self, operation: Option<multicad_core::model::Operation>) {
        self.boolean_preset = operation;
    }
    pub fn boolean_preset(&self) -> Option<multicad_core::model::Operation> {
        self.boolean_preset
    }
    /// Operation of the open Boolean form, if one is open.
    pub fn boolean_operation(&self) -> Option<multicad_core::model::Operation> {
        match &self.form {
            Some(ModelForm::Boolean(panel)) => Some(panel.operation()),
            _ => None,
        }
    }
    pub fn begin_boolean(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        selected: &std::collections::BTreeSet<EntityId>,
        channel: Channel,
    ) -> Result<(), String> {
        let panel = BooleanPanel::new(session.document(), session.revision(), scene, selected)?
            .with_operation(
                self.boolean_preset
                    .unwrap_or(multicad_core::model::Operation::Cut),
            );
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Boolean(Box::new(panel)),
        )
    }
    pub fn cancel(&mut self) {
        self.placement_interaction = Default::default();
        self.scale_interaction = Default::default();
        self.scale_host_basis = None;
        self.placement_confirm = None;
        self.scale_confirm = None;
        self.operation = Default::default();
        self.form = None;
        self.scope = None;
        self.candidate = None;
        self.discovery = None;
        self.operations = None;
        self.identities = BooleanIdentities::default();
        self.handoff = None;
        self.error = None;
        self.context_valid = false;
        self.context_attempt = None;
        self.context_notice = None;
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
        self.placement_confirm = None;
        self.scale_confirm = None;
        self.candidate = None;
        self.discovery = None;
        if let Some(operations) = &mut self.operations {
            operations.invalidate();
        }
        self.error = None;
    }
    pub fn preview(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        self.stop_edge_treatment_picking();
        self.stop_advanced_picking();
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
        self.stop_edge_treatment_picking();
        self.stop_advanced_picking();
        if matches!(
            self.scope.as_ref().map(|scope| scope.execution),
            Some(Execution::Create)
        ) && let (Some(_), ModelRequest::Extrude(extrusion)) = (&self.operations, request)
        {
            return self.submit_extrusion(session, resources, channel, form, extrusion);
        }
        self.submit_final(session, resources, channel, form, request)
    }
    fn submit_final(
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
        // A Room opening edits the wall (or slab) the form currently names.
        let execution = match request {
            ModelRequest::Opening(command) => Execution::Edit {
                feature: command.anchor,
            },
            _ => self.scope.as_ref().expect("current scope").execution,
        };
        let (prepared, proof) = match execution {
            Execution::Create => (
                request.prepare(session, Actor::local(channel))?,
                SceneProof::Create,
            ),
            Execution::Edit { feature } => {
                let prepared = request.prepare_edit(session, Actor::local(channel), feature)?;
                if prepared
                    .outputs
                    .iter()
                    .any(|output| output.feature != feature)
                {
                    return Err("Feature edit changed its producer identity".into());
                }
                let proof = SceneProof::edited(prepared.outputs.clone(), &[], vec![]);
                (prepared, proof)
            }
            Execution::EditGroup { group } => {
                self.prepare_group(session, channel, group, request)?
            }
            Execution::LegacyExtrusion { feature, group } => {
                self.prepare_legacy_extrusion(session, channel, feature, group, request)?
            }
        };
        session
            .validate_preview(&prepared.prepared)
            .map_err(|error| error.to_string())?;
        let content = ContentHash::document(prepared.prepared.document())?;
        let tools = match execution {
            Execution::Create => request.tool_proof(prepared.prepared.document())?,
            Execution::Edit { .. }
            | Execution::EditGroup { .. }
            | Execution::LegacyExtrusion { .. } => vec![],
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
            instances: match request {
                ModelRequest::Placement(command) => command.instances(),
                ModelRequest::Scale(command) => command.instances(),
                ModelRequest::Alignment(command) => command.instances(),
                _ => vec![],
            },
            placement: matches!(
                request,
                ModelRequest::Placement(_) | ModelRequest::Scale(_) | ModelRequest::Alignment(_)
            ),
            tools,
            proof,
            pending: true,
            scene: None,
            requires_info: matches!(
                request,
                ModelRequest::ExtrudeBoolean(_)
                    | ModelRequest::Split(_)
                    | ModelRequest::Placement(_)
                    | ModelRequest::Scale(_)
                    | ModelRequest::EdgeTreatment(_)
                    | ModelRequest::Advanced(_)
                    | ModelRequest::Alignment(_)
            ) || !matches!(execution, Execution::Create),
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
            self.discovery = None;
            if let Some(operations) = &mut self.operations {
                operations.invalidate();
            }
            self.error = Some(error);
        }
        for _ in 0..16 {
            let Some(reply) = self.backend.as_mut().and_then(|backend| backend.poll()) else {
                break;
            };
            if self
                .discovery
                .as_ref()
                .is_some_and(|discovery| discovery.stamp == reply.stamp)
            {
                if let Err(error) = self.receive_discovery(session, resources, reply) {
                    self.error = Some(error);
                }
                continue;
            }
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
                if !scene.is_full_model() {
                    return Err(
                        "A source-only inspection scene cannot authorize modeling Apply".into(),
                    );
                }
                if scene.stamp() != candidate.stamp {
                    return Err("Native preview returned the wrong scene scope".into());
                }
                if let Some(info) = scene.model_evaluation() {
                    if info.source_hash != candidate.content {
                        return Err("Native preview returned evaluation metadata for different source content".into());
                    }
                } else if candidate.requires_info {
                    return Err("This modeling operation requires a native provider with source-bound evaluation metadata".into());
                }
                candidate.proof.validate(
                    candidate.prepared.document(),
                    &scene,
                    &candidate.outputs,
                )?;
                if candidate.placement {
                    proof::placements(
                        candidate.prepared.document(),
                        candidate.stamp.revision,
                        &scene,
                    )?;
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
    pub fn candidate_instances(&self) -> Vec<EntityId> {
        self.candidate
            .as_ref()
            .map(|candidate| candidate.instances.clone())
            .unwrap_or_default()
    }
    pub fn apply(
        &mut self,
        session: &mut Session,
        resources: &ProjectResources,
    ) -> Result<Vec<CreatedBody>, String> {
        let result: Result<Vec<CreatedBody>, String> = (|| {
            if self.placement_interaction.active() || self.scale_interaction.active() {
                return Err(
                    "Release the placement handle before applying its native preview".into(),
                );
            }
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
        if let Some(notice) = &self.context_notice {
            ui.small(notice);
            ui.separator();
        }
        if let Some(operations) = &mut self.operations {
            if operations.show(ui) {
                self.operation_changed();
            }
            ui.separator();
        }
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
            (_, _, Some(stamp)) if self.discovery.is_some() => PreviewState::Pending { stamp },
            _ => PreviewState::Idle,
        };
        let capabilities = if self.backend.is_some() {
            ExtrudeCapabilities::distance_bodies()
        } else {
            ExtrudeCapabilities::default()
        };
        if self
            .scope
            .as_ref()
            .is_some_and(|scope| matches!(scope.execution, Execution::Edit { .. }))
        {
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
            Some(ModelEvent::Apply) => {
                let instances = self
                    .candidate
                    .as_ref()
                    .map(|candidate| candidate.instances.clone())
                    .unwrap_or_default();
                self.apply(session, resources).map(|parts| {
                    Some(if instances.is_empty() {
                        ModelToolOutcome::Applied(parts)
                    } else {
                        ModelToolOutcome::Placed { parts, instances }
                    })
                })
            }
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

mod context;
mod edge_treatment;
mod face_inputs;
mod groups;
mod legacy;
mod operation_interaction;
mod operations;
mod placement_interaction;
pub use operation_interaction::OperationViewport;
mod scale_interaction;
pub use placement_interaction::PlacementViewportResult;
mod proof;
mod sheet;
mod termination;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod placement_tests;

#[cfg(test)]
mod face_entry_tests;
#[cfg(test)]
mod gizmo_entry_tests;

#[cfg(test)]
mod scale_tests;

mod advanced;

#[cfg(test)]
mod chamfer_tests;

#[cfg(test)]
mod advanced_tests;

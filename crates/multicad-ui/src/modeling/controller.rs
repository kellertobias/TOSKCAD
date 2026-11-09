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
    gear::GearPanel,
    group_edit::GroupEditPanel,
    mechanical_part::MechanicalPartPanel,
    names::GeneratedNames,
    placement::PlacementPanel,
    primitive::PrimitivePanel,
    revolve::RevolvePanel,
    scaling::ScalePanel,
    split::SplitPanel,
    thread::ThreadPanel,
};
use crate::{
    egui,
    viewport::{Scene, SceneBackend, SceneStamp, VisibilityFilter},
};
#[cfg(test)]
use multicad_core::commands::Request;
use multicad_core::{
    commands::{Actor, Channel, Prepared, Session, SessionStamp},
    document::{Document, EntityData, EntityId},
    model::{
        self, CreatedBody, ExtrudeRequest, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest,
        RevolveRequest, length,
    },
    project_file::{ContentHash, ProjectResources},
    sketch_regions::RegionIntent,
};
use proof::SceneProof;
use std::collections::BTreeMap;
use std::sync::Arc;

struct Scope {
    /// Read-only source context remains bound to its originating session.
    session: SessionStamp,
    source: ContentHash,
    resources: ProjectResources,
    channel: Channel,
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
    architecture_levels: std::sync::OnceLock<crate::architecture_view::Levels>,
    connection_entry_stations: std::sync::OnceLock<BTreeMap<EntityId, f64>>,
    profile_hardware_view: std::sync::OnceLock<Option<Arc<Scene>>>,
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
    /// An FDM aid feature whose native placement review gates Apply.
    aid: Option<EntityId>,
    /// Why a ready preview still cannot be applied.
    blocked: Option<String>,
}
impl Candidate {
    fn profile_hardware_view(&self) -> Option<&Arc<Scene>> {
        self.profile_hardware_view
            .get_or_init(|| {
                let scene = self.scene.as_ref()?;
                let bodies = self.outputs.iter().map(|output| output.body).collect();
                self.outputs
                    .iter()
                    .all(|output| {
                        scene.instances().iter().any(|instance| {
                            instance.id == output.body && instance.geometry == output.feature
                        })
                    })
                    .then(|| scene.presentation_subset(&bodies).ok())
                    .flatten()
            })
            .as_ref()
    }
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
    architecture_levels: std::sync::OnceLock<crate::architecture_view::Levels>,
}
#[derive(Default)]
pub struct ModelToolController {
    architecture_level: Option<Option<EntityId>>,
    form: Option<ModelForm>,
    stock_editor: Option<(
        crate::materials::ToolStock,
        Box<crate::materials::LibraryEditor>,
    )>,
    advanced_context: Option<advanced::AdvancedContext>,
    backend: Option<Box<dyn SceneBackend>>,
    scope: Option<Scope>,
    candidate: Option<Candidate>,
    auto_preview: Option<(FormStamp, f64)>,
    observed_preview_form: Option<FormStamp>,
    keyboard_confirm: Option<FormStamp>,
    preparation: preparation::Lane,
    discovery: Option<Discovery>,
    surface_inspection: Option<surface_structure::SurfaceInspection>,
    operations: Option<OperationControls>,
    identities: BooleanIdentities,
    handoff: Option<Handoff>,
    generation: u64,
    error: Option<String>,
    context_valid: bool,
    context_attempt: Option<SceneStamp>,
    context_notice: Option<String>,
    placement_interaction: super::gizmo::Interaction,
    scale_interaction: super::scale_gizmo::Interaction,
    scale_host_basis: Option<super::GizmoBasis>,
    operation: operation_interaction::OperationState,
    boolean_preset: Option<multicad_core::model::Operation>,
    fence_type: Option<EntityId>,
    connection_entries: Arc<Vec<crate::library::LibraryEntry>>,
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
    pub fn set_architecture_level(&mut self, level: Option<EntityId>) {
        self.architecture_level = Some(level);
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
    pub fn label(&self) -> &'static str {
        if let Some(form @ ModelForm::Sheet(_)) = &self.form {
            return form.label();
        }
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
    /// Find the generator owning a selected assembly, member, recipe or occurrence.
    pub fn gear_pair_for(
        document: &multicad_core::document::Document,
        id: EntityId,
    ) -> Option<EntityId> {
        let component = match document.get(id).map(|e| &e.data) {
            Some(EntityData::Instance { component, .. }) => Some(*component),
            _ => None,
        };
        let mut matches = document.entities().filter_map(|entity| {
            let pair = multicad_core::model::read_gear_pair(document, entity.id).ok()?;
            let owned = entity.id == id
                || pair.outputs.assembly == id
                || component == Some(pair.outputs.assembly)
                || pair.outputs.instances.contains(&id)
                || pair.outputs.joints.contains(&id)
                || pair.parameters.gears.contains(&id)
                || pair.parameters.gears.iter().any(|feature| {
                    multicad_core::model::read_gear(document, *feature).is_ok_and(|gear| {
                        gear.output.body == id || Some(gear.output.body) == component
                    })
                });
            owned.then_some(entity.id)
        });
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }
    pub fn editable(document: &multicad_core::document::Document, feature: EntityId) -> bool {
        if multicad_core::model::profile_connection_for(document, feature).is_some()
            || Self::screw_connection_for(document, feature).is_some()
        {
            return true;
        }
        if multicad_core::model::wood_box_owner(document, feature).is_some() {
            return true;
        }
        if multicad_core::model::compound_cutout_source(document, feature).is_some() {
            return true;
        }
        if !matches!(
            document.get(feature).map(|e| &e.data),
            Some(EntityData::Feature {
                suppressed: true,
                ..
            })
        ) && super::fence::FencePanel::target(document, feature).is_some()
        {
            return true;
        }
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
            || multicad_core::model::read_surface_structure(document, feature).is_ok()
            || multicad_core::model::read_alignment(document, feature).is_ok()
            || multicad_core::model::read_instance_alignment(document, feature).is_ok()
            || multicad_core::model::read_thread(document, feature).is_ok()
            || multicad_core::model::read_gear(document, feature).is_ok()
            || multicad_core::model::read_mechanical_part(document, feature).is_ok()
            || multicad_core::model::read_gear_pair(document, feature).is_ok()
        {
            return true;
        }
        matches!(document.get(feature).map(|entity|&entity.data),Some(EntityData::Feature{definition,suppressed:false,..}) if matches!(definition.type_id.as_str(),multicad_core::model::EXTRUDE_TYPE|multicad_core::model::PRIMITIVE_TYPE|multicad_core::model::REVOLVE_TYPE|multicad_core::model::BOOLEAN_TYPE|multicad_core::model::WALL_TYPE|multicad_core::model::SLAB_TYPE|multicad_core::model::STAIR_TYPE|multicad_core::model::PANEL_TYPE|multicad_core::model::PANEL_JOINT_CUT_TYPE|multicad_core::model::MACHINING_CUT_TYPE|multicad_core::model::LOOSE_TENON_TYPE|multicad_core::model::AID_TYPE|multicad_core::model::MEMBER_TYPE|multicad_core::model::SHEET_TYPE))
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
        if let Some(connection) = multicad_core::model::profile_connection_for(document, feature) {
            let request = model::read_profile_connection_request(document, connection)
                .map_err(|error| error.to_string())?;
            let panel =
                super::profile_connection::ProfileConnectionPanel::existing(document, request)?;
            return self.begin_form(
                session,
                resources,
                channel,
                ModelForm::ProfileConnection(Box::new(panel)),
            );
        }
        if let Some(connection) = Self::screw_connection_for(document, feature) {
            let mut panel =
                super::screw_connection::ScrewConnectionPanel::existing(document, connection)?;
            panel.set_entries(self.connection_entries.clone());
            return self.begin_form(
                session,
                resources,
                channel,
                ModelForm::ScrewConnection(Box::new(panel)),
            );
        }
        if !Self::editable(document, feature) {
            return Err(
                "Choose a supported, unsuppressed applied feature or owned placement driver".into(),
            );
        }
        if let Ok(request) = model::read_surface_structure(document, feature) {
            let panel = super::surface_structure::SurfaceStructurePanel::existing(request)?;
            self.begin_form(
                session,
                resources,
                channel,
                ModelForm::SurfaceStructure(Box::new(panel)),
            )?;
            self.scope.as_mut().expect("opened scope").execution = Execution::Edit { feature };
            return Ok(());
        }
        if let Some(source) = model::wood_box_owner(document, feature) {
            let panel = super::wood_box::WoodBoxPanel::existing(document, source)?;
            return self.begin_form(
                session,
                resources,
                channel,
                ModelForm::WoodBox(Box::new(panel)),
            );
        }
        if let Some(source) = model::compound_cutout_source(document, feature) {
            let panel = super::compound_cutout::CompoundCutoutPanel::existing(document, source)?;
            return self.begin_form(
                session,
                resources,
                channel,
                ModelForm::CompoundCutout(Box::new(panel)),
            );
        }
        if let Some(run) = super::fence::FencePanel::target(document, feature) {
            let panel = super::fence::FencePanel::existing(session, run)?;
            self.fence_type = Some(panel.preferred_type());
            return self.begin_form(
                session,
                resources,
                channel,
                ModelForm::Fence(Box::new(panel)),
            );
        }
        let cabinet_owner = model::read_panel_joint_cut(document, feature)
            .ok()
            .and_then(|c| model::read_panel_joint(document, c.joint).ok())
            .and_then(|j| j.generated_by)
            .or_else(|| {
                model::read_loose_tenon(document, feature)
                    .ok()
                    .and_then(|t| model::read_panel_joint(document, t.joint).ok())
                    .and_then(|j| j.generated_by)
            })
            .or_else(|| {
                model::read_machining_cut(document, feature)
                    .ok()
                    .and_then(|c| model::read_machining(document, c.operation).ok())
                    .and_then(|m| m.generated_by)
            });
        let feature = match cabinet_owner {
            Some(owner) => model::carcass_panels(document, owner)
                .map_err(|e| e.to_string())?
                .first()
                .map(|(_, feature, _)| *feature)
                .ok_or("Cabinet has no editable panel")?,
            None => feature,
        };
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
        if model::read_gear_pair(document, feature).is_ok() {
            let form = GearPanel::existing_pair(document, feature, session.parameters().clone())?;
            return self.begin_form(session, resources, channel, ModelForm::Gear(Box::new(form)));
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
        if model::read_panel_joint_cut(document, feature).is_ok()
            || model::read_loose_tenon(document, feature).is_ok()
        {
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
            model::MECHANICAL_PART_TYPE => {
                let source =
                    model::read_mechanical_part(document, feature).map_err(|e| e.to_string())?;
                ModelForm::MechanicalPart(Box::new(MechanicalPartPanel::new(
                    model::MechanicalPartRequest {
                        feature,
                        body: source.output.body,
                        name: body_name(source.output.body)?,
                        parameters: source.parameters,
                        provenance: source.provenance,
                    },
                    session.parameters().clone(),
                    true,
                )?))
            }
            model::GEAR_TYPE => {
                let source = model::read_gear(document, feature).map_err(|e| e.to_string())?;
                ModelForm::Gear(Box::new(GearPanel::new(
                    model::GearRequest {
                        feature,
                        body: source.output.body,
                        name: entity.name.clone(),
                        parameters: source.parameters,
                    },
                    session.parameters().clone(),
                    true,
                )?))
            }
            model::THREAD_TYPE => {
                let source = model::read_thread(document, feature).map_err(|e| e.to_string())?;
                ModelForm::Thread(Box::new(ThreadPanel::new(
                    model::ThreadRequest {
                        feature,
                        body: source.output.body,
                        name: entity.name.clone(),
                        target: source.target,
                        mode: source.mode,
                        parameters: source.parameters,
                    },
                    session.parameters().clone(),
                    vec![],
                    &Default::default(),
                    true,
                )?))
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
        self.auto_preview.is_some()
            || self.preparation.pending()
            || self
                .surface_inspection
                .as_ref()
                .is_some_and(surface_structure::SurfaceInspection::pending)
            || self.discovery.is_some()
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
        let operations = OperationControls::new(session.document(), Some(sketch));
        panel.set_through_sheet(operations.through_sheet());
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Extrude(Box::new(panel)),
        )?;
        self.operations = Some(operations);
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
            ModelingTool::Sphere => PrimitiveParameters::Sphere {
                radius: length(20.),
                frame: PrimitiveFrame::default(),
            },
            _ => return Err("Choose a primitive shape".into()),
        };
        let name = GeneratedNames::new(session.document()).numbered(kind.label(), " feature");
        let request = PrimitiveRequest::new(name, parameters);
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Primitive(Box::new(PrimitivePanel::new(request)?)),
        )?;
        // The form remains open if the preview provider reports an error.
        let _ = self.preview(session, resources, channel);
        Ok(())
    }
    pub fn begin_thread(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: Option<&Scene>,
        selected: &std::collections::BTreeSet<EntityId>,
        channel: Channel,
    ) -> Result<(), String> {
        let choices = scene
            .and_then(|scene| {
                super::body_choices::choices(session.document(), session.revision(), scene).ok()
            })
            .unwrap_or_default();
        let name = GeneratedNames::new(session.document()).numbered("Thread", " feature");
        let request = multicad_core::model::ThreadRequest::new(name, Default::default());
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Thread(Box::new(ThreadPanel::new(
                request,
                session.parameters().clone(),
                choices,
                selected,
                false,
            )?)),
        )
    }
    pub fn begin_mechanical_part(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Hardware", " feature");
        let request = model::MechanicalPartRequest::new(
            name,
            model::MechanicalPartParameters::Washer {
                bore_diameter: length(3.2),
                outside_diameter: length(7.),
                thickness: length(0.5),
                frame: Default::default(),
            },
        );
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::MechanicalPart(Box::new(MechanicalPartPanel::new(
                request,
                session.parameters().clone(),
                false,
            )?)),
        )
    }
    pub fn begin_gear(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Gear", " feature");
        let request = multicad_core::model::GearRequest::new(name, Default::default());
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Gear(Box::new(GearPanel::new(
                request,
                session.parameters().clone(),
                false,
            )?)),
        )
    }
    pub fn begin_fence(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Fence", "");
        let panel = super::fence::FencePanel::new(session, name, self.fence_type)?;
        self.fence_type = Some(panel.preferred_type());
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Fence(Box::new(panel)),
        )
    }
    pub fn select_architectural_definition(
        &mut self,
        session: &Session,
        definition: EntityId,
    ) -> Result<(), String> {
        match self.form.as_mut() {
            Some(ModelForm::Wall(panel)) => panel.select_definition(session.document(), definition),
            Some(ModelForm::Fence(panel)) => {
                panel.select_definition(session.document(), definition)
            }
            _ => Err("Open Draw Wall or Draw Fence before choosing its definition".into()),
        }
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
    /// The targets the open machining, joint or print-aid form would act on
    /// (machining targets, joint pair panels, or the aid's finished Part).
    pub fn form_targets(&self) -> Option<Vec<EntityId>> {
        match &self.form {
            Some(ModelForm::Machining(panel)) => Some(panel.target_parts().to_vec()),
            Some(ModelForm::Joint(panel)) => Some(panel.pair_parts()),
            Some(ModelForm::FdmAid(panel)) => Some(panel.target().into_iter().collect()),
            _ => None,
        }
    }
    pub fn wood_preset(&self) -> Option<crate::wood_navigation::Preset> {
        use crate::wood_navigation::Preset;
        match &self.form {
            Some(ModelForm::Machining(panel)) => panel.wood_preset(),
            Some(ModelForm::Weldment(panel)) if panel.is_batten() => Some(Preset::Batten),
            Some(ModelForm::MechanicalPart(_)) => Some(Preset::Hardware),
            _ => None,
        }
    }
    /// A joint form picks its panel pairs in the viewport.
    pub fn joint_picking(&self) -> bool {
        matches!(&self.form, Some(ModelForm::Joint(panel)) if panel.picking())
    }
    pub fn joint_viewport(
        &mut self,
        session: &Session,
        view: super::TerminationViewport<'_>,
    ) -> bool {
        let changed = match self.form.as_mut() {
            Some(ModelForm::Joint(panel)) => {
                panel.viewport(session.document(), session.revision(), view)
            }
            _ => false,
        };
        if changed {
            self.invalidate();
        }
        changed
    }
    /// A machining form places features on panel faces in the viewport.
    pub fn machining_picking(&self) -> bool {
        matches!(&self.form, Some(ModelForm::Machining(panel)) if panel.picking())
    }
    pub fn machining_viewport(
        &mut self,
        session: &Session,
        view: super::TerminationViewport<'_>,
    ) -> bool {
        let changed = match self.form.as_mut() {
            Some(ModelForm::Machining(panel)) => {
                panel.viewport(session.document(), session.revision(), view)
            }
            _ => false,
        };
        if changed {
            self.invalidate();
        }
        changed
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
    /// A wall or slab outline is shown and edited on the plan.
    pub fn outline_active(&self) -> bool {
        match &self.form {
            Some(
                ModelForm::Wall(_)
                | ModelForm::Fence(_)
                | ModelForm::CompoundCutout(_)
                | ModelForm::WoodBox(_),
            ) => true,
            Some(ModelForm::Architecture(panel)) => panel.outlines(),
            _ => false,
        }
    }
    pub fn outline_viewport(
        &mut self,
        session: &Session,
        view: super::TerminationViewport<'_>,
    ) -> bool {
        let (document, revision) = (session.document(), session.revision());
        let changed = match self.form.as_mut() {
            Some(ModelForm::Wall(panel)) => {
                panel.sync_source(session);
                panel.viewport(document, revision, view)
            }
            Some(ModelForm::Fence(panel)) => panel.viewport(session, view),
            Some(ModelForm::WoodBox(panel)) => panel.viewport(session, view),
            Some(ModelForm::CompoundCutout(panel)) => {
                panel.viewport(document, session.parameters(), view)
            }
            Some(ModelForm::Architecture(panel)) => {
                panel.viewport(document, revision, session.parameters(), view)
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
    pub(crate) fn begin_sheet_fasteners(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        target: EntityId,
        channel: Channel,
    ) -> Result<(), String> {
        let panel = super::sheet_fasteners::FastenerPanel::new(session.document(), target)?;
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::SheetFasteners(Box::new(panel)),
        )
    }
    pub fn begin_wood_preset(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        targets: Vec<EntityId>,
        channel: Channel,
        preset: crate::wood_navigation::Preset,
    ) -> Result<(), String> {
        use crate::wood_navigation::Preset;
        let mut names = GeneratedNames::new(session.document());
        let form = match preset {
            Preset::Hardware => ModelForm::MechanicalPart(Box::new(MechanicalPartPanel::new(
                model::MechanicalPartRequest::new(
                    names.numbered("Hardware", " feature"),
                    model::default_machine_screw(),
                ),
                session.parameters().clone(),
                false,
            )?)),
            Preset::Batten => {
                ModelForm::Weldment(Box::new(super::weldment::WeldmentPanel::batten(
                    session.document(),
                    names.numbered("Batten", ""),
                )))
            }
            Preset::Drill | Preset::Pocket => ModelForm::Machining(Box::new(
                super::machining::MachiningPanel::preset(
                    names.numbered(preset.label(), ""),
                    preset,
                )
                .with_targets(targets),
            )),
        };
        self.begin_form(session, resources, channel, form)
    }
    /// Drill / pocket, with its targets seeded (for example from the
    /// selected panels).
    pub fn begin_machining(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        targets: Vec<EntityId>,
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Machining", "");
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Machining(Box::new(
                super::machining::MachiningPanel::new(name).with_targets(targets),
            )),
        )
    }
    /// The System 32 shelf-pin tool: a machining form preset to shelf-pin
    /// rows on the inside faces of `targets`.
    pub fn begin_shelf_pins(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        targets: Vec<EntityId>,
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Shelf pins", "");
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Machining(Box::new(
                super::machining::MachiningPanel::system32(name).with_targets(targets),
            )),
        )
    }
    /// Panel joint, with its first pair seeded from up to two panels.
    pub fn begin_joint(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        pair: [Option<EntityId>; 2],
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Joint", "");
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Joint(Box::new(
                super::joint::JointPanel::new(name).with_pair(pair[0], pair[1]),
            )),
        )
    }
    /// The loose-tenon (Domino) tool: the joint form preset to loose tenons,
    /// with its first pair seeded from up to two selected panels.
    pub fn begin_loose_tenon(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        pair: [Option<EntityId>; 2],
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Domino joint", "");
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Joint(Box::new(
                super::joint::JointPanel::loose_tenon(name).with_pair(pair[0], pair[1]),
            )),
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
            ModelForm::Cabinet(Box::new(super::cabinet::CabinetPanel::cabinet(
                session.document(),
                name,
            ))),
        )
    }
    pub fn begin_wood_box(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Wood Box", "");
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::WoodBox(Box::new(super::wood_box::WoodBoxPanel::new(
                session.document(),
                name,
            )?)),
        )
    }
    pub fn begin_panel(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Panel", "");
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Cabinet(Box::new(super::cabinet::CabinetPanel::panel(
                session.document(),
                name,
            ))),
        )
    }
    /// Envelope dimensions of the open Panel / carcass form, for the viewport.
    pub fn cabinet_guide(
        &mut self,
        session: &Session,
    ) -> Option<super::cabinet_edit::CabinetGuide> {
        match self.form.as_mut() {
            Some(ModelForm::Cabinet(form)) => {
                form.guide(session.document(), session.revision()).cloned()
            }
            _ => None,
        }
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
    /// Tab / bridge / support, its finished Part taken from `selection`
    /// (see [`super::fdm_aid::seed_target`]).
    pub fn begin_fdm_aid(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        selection: &[EntityId],
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Print aid", "");
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::FdmAid(Box::new(super::fdm_aid::FdmAidPanel::with_selection(
                session.document(),
                name,
                selection,
            ))),
        )
    }
    /// The finished Part a new print aid would take from `selection`, and
    /// why none was taken.
    pub fn fdm_seed(
        document: &multicad_core::document::Document,
        selection: &[EntityId],
    ) -> (Option<EntityId>, Option<String>) {
        super::fdm_aid::seed_target(document, selection)
    }
    pub fn begin_planting(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Architecture(Box::new(super::architecture::ArchitecturePanel::planting(
                session.document(),
            )?)),
        )
    }
    pub fn begin_planting_existing(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        record: EntityId,
        channel: Channel,
    ) -> Result<(), String> {
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Architecture(Box::new(
                super::architecture::ArchitecturePanel::planting_existing(
                    session.document(),
                    record,
                )?,
            )),
        )
    }
    pub fn begin_stairs(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Stair", "");
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Architecture(Box::new(super::architecture::ArchitecturePanel::stairs(
                session.document(),
                name,
            )?)),
        )
    }
    pub fn begin_level(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        existing: Option<EntityId>,
        channel: Channel,
    ) -> Result<(), String> {
        let name = GeneratedNames::new(session.document()).numbered("Level", "");
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Architecture(Box::new(super::architecture::ArchitecturePanel::level(
                session.document(),
                name,
                existing,
            )?)),
        )
    }
    pub fn begin_equipment_existing(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        record: EntityId,
        channel: Channel,
    ) -> Result<(), String> {
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Architecture(Box::new(
                super::architecture::ArchitecturePanel::equipment_existing(
                    session.document(),
                    record,
                )?,
            )),
        )
    }
    pub fn begin_equipment_preset(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
        kind: multicad_core::equipment::Kind,
    ) -> Result<(), String> {
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Architecture(Box::new(
                super::architecture::ArchitecturePanel::equipment_preset(session.document(), kind)?,
            )),
        )
    }
    pub fn begin_equipment(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Architecture(Box::new(super::architecture::ArchitecturePanel::equipment(
                session.document(),
            )?)),
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
        self.begin_form_with_level_seed(session, resources, channel, form, true)
    }
    fn begin_form_with_level_seed(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
        mut form: ModelForm,
        seed_level: bool,
    ) -> Result<(), String> {
        if !self.has_backend() {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.active() {
            return Err("Apply or cancel the active modeling tool first".into());
        }
        if seed_level && let Some(level) = self.architecture_level {
            match &mut form {
                ModelForm::Wall(panel) => panel.seed_new_level(level),
                ModelForm::Opening(panel) => panel.seed_new_level(session.document(), level),
                ModelForm::Fence(panel) => panel.seed_new_level(level),
                ModelForm::Architecture(panel) => panel.seed_new_level(session.document(), level),
                _ => {}
            }
        }
        let scope = session.stamp();
        let source = session.source_hash()?;
        self.form = Some(form);
        if let Some(panel) = self.form.as_mut().and_then(ModelForm::extrusion_mut) {
            panel.set_picking_available(false);
        }
        self.surface_inspection = None;
        self.scope = Some(Scope {
            session: scope,
            source,
            resources: resources.clone(),
            channel,
            execution: Execution::Create,
            adoption: None,
        });
        self.candidate = None;
        self.auto_preview = None;
        self.observed_preview_form = None;
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
        if session.validate_stamp(&scope.session).is_err() {
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
        SplitPanel::available(document, revision, scene).is_ok_and(|count| count >= 1)
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
    pub fn begin_split_source(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        selected: &std::collections::BTreeSet<EntityId>,
        channel: Channel,
    ) -> Result<(), String> {
        let panel = SplitPanel::from_source(session.document(), selected)?;
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Split(Box::new(panel)),
        )
    }
    pub(crate) fn split_selection(
        &self,
    ) -> Option<(
        u64,
        super::SplitInputField,
        std::collections::BTreeSet<EntityId>,
    )> {
        match self.form.as_ref() {
            Some(ModelForm::Split(panel)) => Some(panel.selection()),
            _ => None,
        }
    }
    pub(crate) fn retarget_split(
        &mut self,
        document: &Document,
        selected: &std::collections::BTreeSet<EntityId>,
    ) -> Result<(), String> {
        let changed = match self.form.as_mut() {
            Some(ModelForm::Split(panel)) => panel.select(document, selected)?,
            _ => return Err("Split is not open".into()),
        };
        if changed {
            self.invalidate();
        }
        Ok(())
    }
    pub(crate) fn confirm_split(&mut self, document: &Document) -> Result<(), String> {
        match self.form.as_mut() {
            Some(ModelForm::Split(panel)) => panel.confirm(document),
            _ => Err("Split is not open".into()),
        }
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
        if let Some(backend) = &mut self.backend {
            backend.cancel();
        }
        self.preparation.cancel();
        self.placement_interaction = Default::default();
        self.scale_interaction = Default::default();
        self.scale_host_basis = None;
        self.keyboard_confirm = None;
        self.operation = Default::default();
        self.form = None;
        self.stock_editor = None;
        self.scope = None;
        self.surface_inspection = None;
        self.candidate = None;
        self.auto_preview = None;
        self.observed_preview_form = None;
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
        session.validate_stamp(&scope.session).map_err(|_| {
            "The source session changed; cancel the modeling tool and start again".to_string()
        })?;
        if &scope.resources != resources {
            return Err(
                "Project resources changed; cancel the modeling tool and start again".into(),
            );
        }
        if full && session.source_hash()? != scope.source {
            return Err(
                "The source content changed; cancel the modeling tool and start again".into(),
            );
        }
        Ok(())
    }
    fn invalidate(&mut self) {
        if let Some(backend) = &mut self.backend {
            backend.cancel();
        }
        self.preparation.cancel();
        if self
            .surface_inspection
            .as_ref()
            .is_some_and(surface_structure::SurfaceInspection::pending)
        {
            self.surface_inspection = None;
        }
        self.keyboard_confirm = None;
        self.candidate = None;
        self.auto_preview = None;
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
        if self.stock_editor.is_some() {
            return Err("Save or cancel the stock definition first".into());
        }
        self.stop_edge_treatment_picking();
        self.stop_advanced_picking();
        self.stop_surface_structure_picking();
        self.invalidate();
        self.observed_preview_form = self.stamp();
        let result: Result<(), String> = (|| {
            self.current(session, resources, true)?;
            let stamp = self.stamp().ok_or("Start a modeling tool first")?;
            let capabilities = self.capabilities();
            let form = self.form.as_mut().ok_or("Start a modeling tool first")?;
            let request = form.request(session.document(), capabilities)?;
            form.remember_preview(&request);
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
        self.observed_preview_form = Some(form);
        self.stop_edge_treatment_picking();
        self.stop_advanced_picking();
        self.stop_surface_structure_picking();
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
        if matches!(execution, Execution::Create)
            && matches!(
                request,
                ModelRequest::Thread(_)
                    | ModelRequest::Gear(_)
                    | ModelRequest::ScrewConnection(_)
                    | ModelRequest::ProfileConnection(_)
                    | ModelRequest::SheetFasteners(_)
            )
        {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("Model preview generation limit reached")?;
            return self.preparation.submit(preparation::Job {
                source: session.preparation_snapshot(),
                request: request.clone(),
                channel,
                form,
                generation: self.generation,
            });
        }
        let (prepared, proof) = match execution {
            Execution::Create => {
                let prepared = request.prepare(session, Actor::local(channel))?;
                let proof = match request {
                    ModelRequest::WoodBox(_) => {
                        let removed = session
                            .document()
                            .entities()
                            .filter_map(|entity| {
                                (matches!(entity.data, EntityData::Part { .. })
                                    && prepared.prepared.document().get(entity.id).is_none())
                                .then_some(entity.id)
                            })
                            .collect();
                        SceneProof::edited(prepared.outputs.clone(), &[], removed)
                    }
                    ModelRequest::CompoundCutout(_)
                    | ModelRequest::ProfileConnection(_)
                    | ModelRequest::SurfaceStructure(_)
                    | ModelRequest::SheetProfileCut { .. } => {
                        SceneProof::edited(prepared.outputs.clone(), &[], vec![])
                    }
                    ModelRequest::ScrewConnection(command) => SceneProof::ScrewConnection {
                        connection: command.connection(),
                        owned: command.removed_outputs(session.document())?,
                        removed: matches!(
                            command.as_ref(),
                            super::ScrewConnectionCommand::Remove { .. }
                        ),
                    },
                    ModelRequest::GearPair(command) => SceneProof::GearPair {
                        pair: command.pair(),
                        occurrence: command.occurrence(),
                    },
                    _ => SceneProof::Create,
                };
                (prepared, proof)
            }
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
        let content = ContentHash::document(prepared.prepared.document())?;
        self.publish_prepared(
            session, resources, form, request, execution, prepared, proof, content,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn publish_prepared(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        form: FormStamp,
        request: &ModelRequest,
        execution: Execution,
        prepared: model::ModelPrepared,
        proof: SceneProof,
        content: ContentHash,
    ) -> Result<(), String> {
        self.current(session, resources, true)?;
        if self.stamp() != Some(form) {
            return Err("Modeling inputs changed; preview the current values".into());
        }
        session
            .validate_preview(&prepared.prepared)
            .map_err(|error| error.to_string())?;
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
        let options = model::EvaluationOptions {
            sheet_development: match request {
                ModelRequest::Sheet(command) if command.editing => Some(command.sheet.output.body),
                ModelRequest::SheetProfileCut { definition, .. } => Some(definition.output.body),
                _ => None,
            },
            ..Default::default()
        };
        self.backend
            .as_mut()
            .ok_or("The native modeling preview provider is unavailable")?
            .request_evaluation(prepared.prepared.document(), resources, stamp, &options)?;
        let instances = match request {
            ModelRequest::Placement(command) => command.instances(),
            ModelRequest::Scale(command) => command.instances(),
            ModelRequest::Alignment(command) => command.instances(),
            ModelRequest::GearPair(command) => command.instances(prepared.prepared.document())?,
            ModelRequest::ScrewConnection(command) => command.instances(),
            _ => vec![],
        };
        self.candidate = Some(Candidate {
            prepared: prepared.prepared,
            architecture_levels: Default::default(),
            connection_entry_stations: Default::default(),
            profile_hardware_view: Default::default(),
            content,
            form,
            stamp,
            outputs: prepared.outputs,
            instances,
            placement: matches!(
                request,
                ModelRequest::Placement(_)
                    | ModelRequest::Scale(_)
                    | ModelRequest::Alignment(_)
                    | ModelRequest::GearPair(_)
                    | ModelRequest::ScrewConnection(_)
            ),
            tools,
            proof,
            pending: true,
            scene: None,
            requires_info: matches!(
                request,
                ModelRequest::ExtrudeBoolean(_)
                    | ModelRequest::Split(_)
                    | ModelRequest::SplitOperation(_)
                    | ModelRequest::Placement(_)
                    | ModelRequest::Scale(_)
                    | ModelRequest::EdgeTreatment(_)
                    | ModelRequest::Advanced(_)
                    | ModelRequest::SurfaceStructure(_)
                    | ModelRequest::Alignment(_)
                    | ModelRequest::GearPair(_)
                    | ModelRequest::ScrewConnection(_)
                    | ModelRequest::MechanicalPart(_)
                    | ModelRequest::ProfileConnection(_)
                    | ModelRequest::CompoundCutout(_)
                    | ModelRequest::WoodBox(_)
                    | ModelRequest::SheetFasteners(_)
                    | ModelRequest::SheetProfileCut { .. }
            ) || !matches!(execution, Execution::Create),
            aid: match request {
                ModelRequest::FdmAid(command) => Some(command.request.feature),
                _ => None,
            },
            blocked: None,
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
            self.invalidate();
            self.error = Some(error);
        }
        if let Some(reply) = self.preparation.poll() {
            let result = (|| {
                self.current(session, resources, true)?;
                let (prepared, content) = reply.result?;
                let proof = match &reply.request {
                    ModelRequest::ScrewConnection(command) => SceneProof::ScrewConnection {
                        connection: command.connection(),
                        owned: command.removed_outputs(session.document())?,
                        removed: matches!(
                            command.as_ref(),
                            super::ScrewConnectionCommand::Remove { .. }
                        ),
                    },
                    ModelRequest::ProfileConnection(_) => {
                        SceneProof::edited(prepared.outputs.clone(), &[], vec![])
                    }
                    ModelRequest::SheetFasteners(command) => {
                        SceneProof::SheetFasteners { part: command.part }
                    }
                    _ => SceneProof::Create,
                };
                self.publish_prepared(
                    session,
                    resources,
                    reply.form,
                    &reply.request,
                    Execution::Create,
                    prepared,
                    proof,
                    content,
                )
            })();
            if let Err(error) = result {
                self.error = Some(error);
            }
        }
        for _ in 0..16 {
            let Some(reply) = self.backend.as_mut().and_then(|backend| backend.poll()) else {
                break;
            };
            if self
                .surface_inspection
                .as_ref()
                .is_some_and(|inspection| inspection.expects(reply.stamp))
            {
                self.accept_surface_inspection(session, resources, reply);
                continue;
            }
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
                    candidate.blocked = candidate
                        .aid
                        .and_then(|aid| super::fdm_aid::placement_block(&scene, aid));
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
        !self.preview_waits_for_input()
            && !self.transform_dragging()
            && self.preview_visible(session, resources)
            && self
                .candidate
                .as_ref()
                .is_some_and(|candidate| candidate.blocked.is_none())
    }
    /// The current preview may be shown, though a placement review can
    /// still refuse to Apply it.
    fn preview_visible(&self, session: &Session, resources: &ProjectResources) -> bool {
        self.current(session, resources, false).is_ok()
            && self.error.is_none()
            && self.candidate.as_ref().is_some_and(|candidate| {
                !candidate.pending
                    && candidate.scene.is_some()
                    && self.stamp() == Some(candidate.form)
                    && session.validate_preview(&candidate.prepared).is_ok()
            })
    }
    /// The native placement review of the previewed FDM aid.
    pub(crate) fn fdm_review(&self) -> Option<super::fdm_aid::Review> {
        let candidate = self.candidate.as_ref()?;
        let scene = candidate.scene.as_ref()?;
        let aid = candidate.aid?;
        Some(super::fdm_aid::Review {
            stamp: candidate.form,
            result: match scene.fdm_review(aid) {
                Some(Ok(review)) => Ok(review.clone()),
                Some(Err(error)) => Err(error.to_owned()),
                None => Err("The native preview carried no inspection of this aid".into()),
            },
            blocked: candidate.blocked.clone(),
        })
    }
    pub fn candidate_instances(&self) -> Vec<EntityId> {
        self.candidate
            .as_ref()
            .map(|candidate| candidate.instances.clone())
            .unwrap_or_default()
    }
    pub(crate) fn architecture_preview_levels(
        &self,
        session: &Session,
        scene: &Arc<Scene>,
    ) -> Option<&crate::architecture_view::Levels> {
        if let Some(candidate) = self.candidate.as_ref()
            && candidate
                .scene
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, scene))
        {
            return Some(candidate.architecture_levels.get_or_init(|| {
                crate::architecture_view::Levels::new(
                    candidate.prepared.document(),
                    candidate.stamp.revision,
                )
            }));
        }
        let handoff = self.handoff.as_ref()?;
        if handoff.document != session.document().id()
            || handoff.revision != session.revision()
            || !Arc::ptr_eq(&handoff.scene, scene)
        {
            return None;
        }
        Some(handoff.architecture_levels.get_or_init(|| {
            crate::architecture_view::Levels::new(session.document(), scene.stamp().revision)
        }))
    }
    pub fn apply(
        &mut self,
        session: &mut Session,
        resources: &ProjectResources,
    ) -> Result<Vec<CreatedBody>, String> {
        let result: Result<Vec<CreatedBody>, String> = (|| {
            if self.transform_dragging() {
                return Err(
                    "Release the viewport handle before applying its native preview".into(),
                );
            }
            if self.scale_origin_picking() {
                return Err("Place the scale origin or cancel the pick before applying".into());
            }
            self.current(session, resources, true)?;
            if let Some(blocked) = self.candidate.as_ref().and_then(|c| c.blocked.clone()) {
                return Err(blocked);
            }
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
            let channel = candidate.prepared.actor().channel;
            session
                .commit(candidate.prepared)
                .map_err(|error| error.to_string())?;
            let mut repeat = self.form.take();
            let keep = match &mut repeat {
                Some(ModelForm::Architecture(panel)) => panel.repeat_equipment(session.document()),
                _ => false,
            };
            if !keep {
                repeat = None;
            }
            self.cancel();
            if let Some(form) = repeat {
                // This is the user's retained placement form, not a new tool:
                // keep its chosen floor and wall instead of reseeding them.
                self.begin_form_with_level_seed(session, resources, channel, form, false)?;
            }
            // Keep the same checked geometry visible while the canonical lane
            // rebuilds the new revision. It remains noninteractive/display-only.
            self.handoff = Some(Handoff {
                document: session.document().id(),
                revision: session.revision(),
                resources: resources.clone(),
                scene,
                architecture_levels: candidate.architecture_levels,
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
        if self.preview_visible(session, resources) {
            let candidate = self.candidate.as_ref()?;
            let scene = candidate.scene.as_ref()?;
            if matches!(&self.form, Some(ModelForm::ProfileConnection(panel)) if panel.hardware_only())
                && let Some(view) = candidate.profile_hardware_view()
            {
                return Some(view.clone());
            }
            return Some(scene.clone());
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
        self.candidate.as_ref().is_some_and(|candidate| {
            candidate
                .scene
                .as_ref()
                .is_some_and(|preview| Arc::ptr_eq(preview, scene))
                || candidate
                    .profile_hardware_view
                    .get()
                    .and_then(Option::as_ref)
                    .is_some_and(|preview| Arc::ptr_eq(preview, scene))
        }) || self
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
    fn transform_dragging(&self) -> bool {
        self.placement_interaction.active()
            || self.scale_interaction.active()
            || matches!(&self.form, Some(ModelForm::Primitive(panel)) if panel.dragging())
    }

    pub(crate) fn arm_confirmation(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
    ) -> bool {
        let Some(stamp) = self.stamp() else {
            return false;
        };
        if self.preview_waits_for_input()
            || self.transform_dragging()
            || !(self.can_apply(session, resources)
                || self.current(session, resources, false).is_ok()
                    && (self
                        .auto_preview
                        .is_some_and(|(scheduled, _)| scheduled == stamp)
                        || self.observed_preview_form == Some(stamp) && self.preparation.pending()
                        || self.discovery.as_ref().is_some_and(|job| job.form == stamp)
                        || self
                            .candidate
                            .as_ref()
                            .is_some_and(|candidate| candidate.form == stamp && candidate.pending)))
        {
            return false;
        }
        self.keyboard_confirm = Some(stamp);
        true
    }

    pub(crate) fn confirmation_ready(
        &self,
        session: &Session,
        resources: &ProjectResources,
    ) -> bool {
        self.keyboard_confirm.is_some()
            && self.keyboard_confirm == self.stamp()
            && self.can_apply(session, resources)
    }

    pub(crate) fn transform_highlight(&self, scene: &Scene) -> Option<crate::viewport::Selection> {
        let targets = self.placement_targets().or_else(|| self.scale_targets())?;
        let ids = if let Some(candidate) = &self.candidate
            && candidate
                .scene
                .as_ref()
                .is_some_and(|preview| std::ptr::eq(preview.as_ref(), scene))
        {
            candidate
                .outputs
                .iter()
                .map(|output| output.body)
                .chain(candidate.instances.iter().copied())
                .collect::<std::collections::BTreeSet<_>>()
        } else {
            targets.clone()
        };
        (!ids.is_empty()).then(|| crate::viewport::Selection::Parts(Arc::new(ids)))
    }

    fn preview_waits_for_input(&self) -> bool {
        self.stock_editor.is_some()
            || matches!(&self.form, Some(ModelForm::Sheet(panel)) if panel.waiting_for_edge())
            || self.scale_origin_picking()
            || self.edge_treatment_picking()
            || self.advanced_picking()
            || self.surface_structure_picking()
            || self.termination_picking()
            || self.alignment_picking()
            || self.fdm_aid_picking()
    }

    fn schedule_auto_preview(&mut self, ui: &egui::Ui, stamp: FormStamp) {
        self.invalidate();
        self.observed_preview_form = Some(stamp);
        let delay = match self.form {
            Some(
                ModelForm::Primitive(_)
                | ModelForm::Placement(_)
                | ModelForm::Scale(_)
                | ModelForm::Revolve(_),
            ) => 0.075,
            Some(ModelForm::Extrude(_))
                if self
                    .operations
                    .as_ref()
                    .is_none_or(|controls| controls.operation == model::Operation::NewBody) =>
            {
                0.075
            }
            _ => 0.30,
        };
        self.auto_preview = Some((stamp, ui.input(|input| input.time) + delay));
        ui.ctx().request_repaint();
    }

    pub(crate) fn tick_auto_preview(
        &mut self,
        ui: &egui::Ui,
        session: &Session,
        resources: &ProjectResources,
        flush: bool,
    ) {
        if !self.active()
            || self.preview_waits_for_input()
            || self.current(session, resources, false).is_err()
        {
            if self.auto_preview.take().is_some() {
                self.observed_preview_form = None;
                self.keyboard_confirm = None;
            }
            return;
        }
        let Some(stamp) = self.stamp() else { return };
        if self.observed_preview_form.is_none()
            && matches!(
                self.form,
                Some(ModelForm::Placement(_) | ModelForm::Scale(_))
            )
            && self
                .scope
                .as_ref()
                .is_some_and(|scope| matches!(scope.execution, Execution::Create))
        {
            // An untouched transform is a selection and its handles, not
            // an operation the user has chosen to apply.
            self.observed_preview_form = Some(stamp);
            return;
        }
        if self.observed_preview_form != Some(stamp) {
            self.schedule_auto_preview(ui, stamp);
        }
        let Some((scheduled, deadline)) = self.auto_preview else {
            return;
        };
        if scheduled != stamp {
            return;
        }
        let remaining = deadline - ui.input(|input| input.time);
        if flush || remaining <= 0. {
            let confirmation = self.keyboard_confirm;
            let channel = self
                .scope
                .as_ref()
                .map_or(Channel::Ui, |scope| scope.channel);
            if self.preview(session, resources, channel).is_ok() {
                self.keyboard_confirm = confirmation.filter(|&expected| expected == stamp);
            }
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(remaining));
        }
    }

    pub(crate) fn primitive_envelope(&self, session: &Session) -> Option<[[f64; 3]; 8]> {
        match self.form.as_ref()? {
            ModelForm::Primitive(panel) => panel.envelope(session),
            _ => None,
        }
    }

    pub(crate) fn primitive_corner_input_active(&self) -> bool {
        matches!(&self.form, Some(ModelForm::Primitive(panel)) if panel.corner_input_active())
    }

    pub fn primitive_viewport(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        resources: &ProjectResources,
        camera: crate::camera::Camera,
        rect: egui::Rect,
        start_allowed: bool,
    ) -> bool {
        if !matches!(self.form, Some(ModelForm::Primitive(_)))
            || self.current(session, resources, false).is_err()
        {
            return false;
        }
        let guide = !self.preview_visible(session, resources);
        let start_allowed = start_allowed && ui.is_enabled();
        let Some(ModelForm::Primitive(panel)) = &mut self.form else {
            unreachable!()
        };
        let result = panel.viewport(ui, session, camera, rect, start_allowed, guide);
        if result.changed {
            self.schedule_auto_preview(ui, self.stamp().expect("active primitive form"));
        }
        if result.confirm {
            self.arm_confirmation(session, resources);
        }
        self.tick_auto_preview(ui, session, resources, result.released || result.confirm);
        result.input_owned
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        resources: &ProjectResources,
    ) -> Result<Option<ModelToolOutcome>, String> {
        if self.stock_editor.is_some() {
            self.show_stock_editor(ui, session, resources);
            return Ok(None);
        }
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
        if matches!(self.form, Some(ModelForm::ProfileConnection(_))) {
            let available = self.preview_visible(session, resources)
                && self
                    .candidate
                    .as_ref()
                    .and_then(Candidate::profile_hardware_view)
                    .is_some();
            if let Some(ModelForm::ProfileConnection(panel)) = &mut self.form {
                panel.set_hardware_view_available(available);
            }
        }
        let stamp = self.stamp();
        let preview = match (&self.error, &self.candidate, stamp) {
            _ if self.preview_waits_for_input() => PreviewState::Idle,
            (_, _, Some(stamp)) if self.auto_preview.is_some() || self.transform_dragging() => {
                PreviewState::Pending { stamp }
            }
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
            (_, _, Some(stamp)) if self.discovery.is_some() || self.preparation.pending() => {
                PreviewState::Pending { stamp }
            }
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
        let connection_review = self
            .candidate
            .as_ref()
            .filter(|candidate| !candidate.pending)
            .and_then(|candidate| candidate.scene.as_ref())
            .and_then(|scene| scene.model_evaluation())
            .and_then(|info| match &self.form {
                Some(ModelForm::ScrewConnection(panel)) => info
                    .screw_connection_checks
                    .iter()
                    .find(|check| check.connection == panel.connection())
                    .cloned(),
                _ => None,
            });
        let connection_entries = self
            .candidate
            .as_ref()
            .filter(|candidate| !candidate.pending)
            .and_then(|candidate| {
                let scene = candidate.scene.as_ref()?;
                let Some(ModelForm::ScrewConnection(panel)) = &self.form else {
                    return None;
                };
                Some(
                    candidate
                        .connection_entry_stations
                        .get_or_init(|| {
                            let Ok(source) = model::read_screw_connection_request(
                                candidate.prepared.document(),
                                panel.connection(),
                            ) else {
                                return BTreeMap::new();
                            };
                            let snapshots = scene.reference_snapshots();
                            source
                                .receivers
                                .iter()
                                .filter_map(|receiver| {
                                    receiver
                                        .source
                                        .resolve_current(
                                            candidate.prepared.document(),
                                            session.parameters(),
                                            scene.stamp().revision,
                                            &snapshots,
                                        )
                                        .ok()
                                        .map(|resolved| (receiver.feature, resolved.station_mm))
                                })
                                .collect()
                        })
                        .clone(),
                )
            })
            .unwrap_or_default();
        if let Some(ModelForm::ScrewConnection(panel)) = self.form.as_mut() {
            panel.set_review(connection_review, connection_entries);
        }
        let review = self.fdm_review();
        if let Some(ModelForm::FdmAid(panel)) = self.form.as_mut() {
            panel.set_review(review);
        }
        if let Some(ModelForm::Wall(panel)) = self.form.as_mut() {
            panel.sync_source(session);
        }
        if let Some(ModelForm::WoodBox(panel)) = self.form.as_mut() {
            panel.sync_source(session);
        }
        if let Some(ModelForm::CompoundCutout(panel)) = self.form.as_mut() {
            panel.sync_source(session);
        }
        if let Some(ModelForm::Fence(panel)) = self.form.as_mut() {
            panel.sync_source(session);
            self.fence_type = Some(panel.preferred_type());
        }
        let event = self
            .form
            .as_mut()
            .ok_or("Start a modeling tool first")?
            .show(ui, session.document(), capabilities, preview);
        if self.scale_origin_picking() {
            self.keyboard_confirm = None;
        }
        let result = match event {
            Some(ModelEvent::CreateStock(kind)) => self
                .begin_stock_editor(session, resources, kind)
                .map(|()| None),
            Some(ModelEvent::CompoundCutout) => {
                self.invalidate();
                self.form = Some(ModelForm::CompoundCutout(Box::new(
                    super::compound_cutout::CompoundCutoutPanel::new(session.document())?,
                )));
                Ok(None)
            }
            Some(ModelEvent::Changed) => {
                if !self.preview_waits_for_input()
                    && let Some(stamp) = self.stamp()
                {
                    self.schedule_auto_preview(ui, stamp);
                } else {
                    self.invalidate();
                }
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
mod fdm;
mod groups;
mod legacy;
mod operation_interaction;
mod operations;
mod placement_interaction;
mod preparation;
mod stock;
pub use operation_interaction::OperationViewport;
mod profile_connection;
mod scale_interaction;
mod screw_connection;
#[cfg(test)]
mod screw_connection_tests;
mod surface_structure;
pub use placement_interaction::PlacementViewportResult;
#[cfg(test)]
mod architecture_tests;
#[cfg(test)]
mod compound_cutout_tests;
mod proof;
mod sheet;
mod termination;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod wood_box_tests;

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

#[cfg(test)]
mod fence_tests;
#[cfg(test)]
mod gear_tests;
#[cfg(test)]
mod thread_tests;

#[cfg(test)]
mod mechanical_part_tests;

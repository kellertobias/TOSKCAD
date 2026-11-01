//! Datum sketch interaction. All committed changes pass through the source Session.
use crate::{
    camera::Camera,
    icons::{self, Icon},
    navigation::{self, DisplayStyle},
    sketch_plane::{PlaneChoice, PlaneContext, PlanePicker, PlanePreview},
};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
    constraint_sketch::source::Source,
    document::{Document, EntityData, EntityId, Mutation},
    sketch::{self, Datum, Frame, Geometry, Sketch},
};
use std::collections::VecDeque;
#[cfg(test)]
#[path = "sketch_advanced_context_tests.rs"]
mod advanced_context_tests;
#[path = "sketch_advanced_host.rs"]
mod advanced_host;
#[path = "sketch_assembly_host.rs"]
mod assembly_host;
#[cfg(test)]
#[path = "sketch_assembly_host_tests.rs"]
mod assembly_host_tests;
#[path = "sketch_draw_modes.rs"]
mod draw_modes;
#[path = "sketch_edge_host.rs"]
mod edge_host;
#[cfg(test)]
#[path = "sketch_edge_host_tests.rs"]
mod edge_host_tests;
#[path = "sketch_external_host.rs"]
mod external_host;
#[path = "sketch_face_profiles.rs"]
mod face_profiles;
#[path = "sketch_mechanical_import_host.rs"]
mod mechanical_import_host;
#[path = "model_target_selection.rs"]
mod model_target_selection;
#[cfg(test)]
#[path = "sketch_scale_host_tests.rs"]
mod scale_host_tests;
#[path = "sketch_snap.rs"]
mod snap;
#[path = "sketch_snap_host.rs"]
mod snap_host;
#[cfg(test)]
#[path = "sketch_snap_host_tests.rs"]
mod snap_host_tests;
#[path = "sketch_symbol_host.rs"]
mod symbol_host;
#[path = "sketch_text_tool.rs"]
mod text_tool;
#[path = "sketch_tool_guide.rs"]
mod tool_guide;
#[cfg(test)]
#[path = "sketch_tool_guide_tests.rs"]
mod tool_guide_tests;
#[cfg(test)]
#[path = "sketch_tool_target_tests.rs"]
mod tool_target_tests;
#[path = "sketch_transfer_host.rs"]
mod transfer_host;
#[cfg(test)]
#[path = "sketch_transfer_host_tests.rs"]
mod transfer_host_tests;
#[path = "sketch_trim_host.rs"]
mod trim_host;
#[cfg(test)]
#[path = "sketch_trim_host_tests.rs"]
mod trim_host_tests;
#[path = "sketch_trim_worker.rs"]
mod trim_worker;

const MAX_PROFILE_CLICKS: usize = 32;
#[derive(Clone, Copy)]
struct ProfileClick {
    evaluation: crate::sketch_evaluation::EvaluationStamp,
    sketch: EntityId,
    point_mm: [f64; 2],
    extend: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawTool {
    Line,
    Rectangle,
    Circle,
    Arc,
    Polyline,
    Spline,
    RectangleCenter,
    RectangleCenterTwo,
    RectangleThree,
    CircleTwoRadius,
    CircleThree,
    Fillet,
    Chamfer,
    Text,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coordinates {
    World,
    Object,
    Custom(Datum),
}
impl Coordinates {
    fn label(self) -> &'static str {
        match self {
            Self::World => "World",
            Self::Object => "Object",
            Self::Custom(_) => "Custom datum",
        }
    }
}
struct MoveDrag {
    axis: usize,
    start: Pos2,
    screen: Vec2,
    length: f64,
}
struct ModelCanvas<'a> {
    resources: &'a multicad_core::project_file::ProjectResources,
    feed: &'a crate::viewport::SceneFeed,
}
struct SplinePointer {
    frame: Frame,
    rect: Rect,
    screen: Pos2,
    point: [f64; 2],
    anchor: Option<EntityId>,
    clicked: bool,
}
pub struct SketchEditor {
    pub selected: Option<EntityId>,
    pub editing: bool,
    pub picking: bool,
    pub tool: Option<DrawTool>,
    first: Option<[f64; 2]>,
    first_anchor: Option<EntityId>,
    arc_start: Option<([f64; 2], Option<EntityId>)>,
    arc_clockwise: bool,
    polyline_start: Option<([f64; 2], EntityId)>,
    polyline_segments: usize,
    spline_poles: Vec<([f64; 2], Option<EntityId>)>,
    last_draw_line: Option<[(EntityId, [f64; 2]); 2]>,
    construction: bool,
    /// Fillet radius or chamfer setback for the corner tools, in mm.
    corner_size_mm: f64,
    /// Text tool Inspector state (CAD-142).
    text_tool: text_tool::TextTool,
    draw_preview_id: EntityId,
    element: Option<EntityId>,
    pub camera: Camera,
    style: DisplayStyle,
    grid: bool,
    envelopes: bool,
    guides: bool,
    spacing: f64,
    snap: bool,
    geometry_snap: bool,
    snap_assist: snap::SnapAssist,
    pub moving: bool,
    pub coordinates: Coordinates,
    custom_angle_deg: f64,
    delta: [f64; 3],
    preview: Option<Prepared>,
    drag: Option<MoveDrag>,
    message: String,
    revision: u64,
    fit_requested: bool,
    constraints: crate::sketch_constraints::ConstraintEditor,
    sketch_evaluation: crate::sketch_evaluation::SketchEvaluationFeed,
    profiles: crate::sketch_selection::ProfileSelection,
    operations: crate::sketch_operations::Panel,
    external: crate::external_references::Controller,
    operation_error: Option<String>,
    trim_worker: trim_worker::Feed,
    operation_context: Option<(EntityId, crate::sketch_evaluation::EvaluationStamp)>,
    operation_guard: Option<Prepared>,
    operation_pick_view: Option<trim_host::PickView>,
    operation_report: Option<multicad_core::sketch_trim::TrimReport>,
    model: crate::modeling::ModelToolController,
    assembly: crate::assembly::AssemblyController,
    symbols: crate::symbols::SymbolController,
    artwork: crate::artwork_import::ImportController,
    mechanical: crate::mechanical_import::MechanicalImport,
    assembly_anchor: Option<assembly_host::PendingAnchor>,
    assembly_presentation: Option<assembly_host::PresentationCache>,
    assembly_pick_view: Option<assembly_host::AnchorView>,
    transfer: crate::transfer::TransferController,
    transfer_source_view: Option<transfer_host::TransferSourceView>,
    acquisition: Option<model_target_selection::Acquisition>,
    advanced_context_stamp: Option<(Option<crate::sketch_evaluation::EvaluationStamp>, usize)>,
    materials: crate::materials::MaterialsEditor,
    pub measurement: crate::inspection::MeasurementInspector,
    measure_pin_allowed: bool,
    initial_fit: crate::initial_fit::InitialFit,
    home_requested: bool,
    tree: crate::model_tree::ModelTree,
    tree_selection: std::collections::BTreeSet<EntityId>,
    /// Sketch selected as a whole (not merely open) when Extrude/Revolve began.
    whole_sketch: Option<EntityId>,
    /// Planar model faces chosen as Extrude / Revolve inputs (CAD-146).
    profile_faces: Vec<face_profiles::ProfileFacePick>,
    visibility: crate::viewport::VisibilityFilter,
    visibility_error: Option<String>,
    overlay_visibility: Option<((EntityId, u64), std::collections::BTreeSet<EntityId>)>,
    tree_scene_stamp: Option<crate::viewport::SceneStamp>,
    pending_profile_pick: Option<(crate::sketch_evaluation::ProfilePickStamp, bool)>,
    queued_profile_clicks: VecDeque<ProfileClick>,
    solid: crate::viewport::Interaction,
    repair: crate::reference_repair::RepairPanel,
    repair_set: crate::reference_repair_set::RepairSetPanel,
    evaluated: Option<multicad_core::references::ReferenceReport>,
    creation_evaluation: Option<(EntityId, multicad_core::references::ReferenceReport)>,
    pub viewport_rect: Option<Rect>,
    plane_picker: PlanePicker,
    occurrence: Option<EntityId>,
    display_scene: Option<std::sync::Arc<crate::viewport::Scene>>,
    /// A pending external change shown for review instead of the canonical
    /// scene while no tool shows its own candidate.
    review_scene: Option<std::sync::Arc<crate::viewport::Scene>>,
    context_key: Option<(crate::viewport::SceneStamp, u64, Option<EntityId>)>,
    contexts: std::collections::BTreeMap<EntityId, crate::viewport::Transform>,
}
impl Default for SketchEditor {
    fn default() -> Self {
        Self {
            selected: None,
            editing: false,
            picking: false,
            tool: None,
            first: None,
            first_anchor: None,
            arc_start: None,
            arc_clockwise: false,
            polyline_start: None,
            polyline_segments: 0,
            spline_poles: vec![],
            last_draw_line: None,
            construction: false,
            corner_size_mm: 2.,
            text_tool: Default::default(),
            draw_preview_id: EntityId::new(),
            element: None,
            camera: Camera {
                target: [0.; 3],
                ..Default::default()
            },
            style: DisplayStyle::VisibleEdges,
            grid: true,
            envelopes: true,
            guides: true,
            spacing: 10.,
            snap: true,
            geometry_snap: true,
            snap_assist: Default::default(),
            moving: false,
            coordinates: Coordinates::World,
            custom_angle_deg: 0.,
            delta: [0.; 3],
            preview: None,
            drag: None,
            message: "Choose New Sketch, or open a sketch from the document.".into(),
            revision: 0,
            fit_requested: false,
            constraints: Default::default(),
            sketch_evaluation: Default::default(),
            profiles: Default::default(),
            operations: Default::default(),
            external: Default::default(),
            operation_error: None,
            trim_worker: Default::default(),
            operation_context: None,
            operation_guard: None,
            operation_pick_view: None,
            operation_report: None,
            model: Default::default(),
            assembly: Default::default(),
            symbols: Default::default(),
            artwork: Default::default(),
            mechanical: Default::default(),
            assembly_anchor: None,
            assembly_presentation: None,
            assembly_pick_view: None,
            transfer: Default::default(),
            transfer_source_view: None,
            acquisition: None,
            advanced_context_stamp: None,
            materials: Default::default(),
            measurement: Default::default(),
            measure_pin_allowed: true,
            initial_fit: Default::default(),
            home_requested: false,
            tree: Default::default(),
            tree_selection: Default::default(),
            whole_sketch: None,
            profile_faces: Vec::new(),
            visibility: Default::default(),
            visibility_error: None,
            overlay_visibility: None,
            tree_scene_stamp: None,
            pending_profile_pick: None,
            queued_profile_clicks: VecDeque::new(),
            solid: Default::default(),
            repair: Default::default(),
            repair_set: Default::default(),
            evaluated: None,
            creation_evaluation: None,
            viewport_rect: None,
            plane_picker: Default::default(),
            occurrence: None,
            display_scene: None,
            review_scene: None,
            context_key: None,
            contexts: Default::default(),
        }
    }
}
fn commit(
    session: &mut Session,
    label: &str,
    edits: Vec<Mutation>,
    channel: Channel,
) -> Result<(), String> {
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: Actor::local(channel),
            label: label.into(),
            edits,
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}
impl SketchEditor {
    pub fn export_selection(&self, document: &Document) -> Vec<EntityId> {
        self.tree_selection
            .iter()
            .copied()
            .chain(self.solid.selected_ids())
            .filter(|id| {
                document.get(*id).is_some_and(|e| {
                    matches!(
                        e.data,
                        EntityData::Part { .. } | EntityData::Instance { .. }
                    )
                })
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn open_materials(&mut self, session: &Session) -> Result<(), String> {
        if self.has_pending_edits() {
            return Err("Apply or discard pending edits before opening materials".into());
        }
        if self.editing {
            self.finish()?;
        }
        self.materials.open(session);
        Ok(())
    }
    pub fn has_query_factory(&self) -> bool {
        self.sketch_evaluation.has_query_factory()
    }
    pub fn set_repair_set_checker(
        &mut self,
        checker: crate::reference_repair_set::RepairSetChecker,
    ) {
        self.repair_set.set_checker(checker);
    }
    pub fn has_repair_set_checker(&self) -> bool {
        self.repair_set.has_checker()
    }
    pub fn set_query_factory(&mut self, factory: crate::sketch_evaluation::RegionQueryFactory) {
        self.reset_profile_clicks();
        self.sketch_evaluation.set_query_factory(factory);
    }
    fn cancel_profile_clicks(&mut self) {
        self.pending_profile_pick = None;
        self.queued_profile_clicks.clear();
        self.sketch_evaluation.cancel_profile_pick();
    }
    fn reset_profile_clicks(&mut self) {
        if self.pending_profile_pick.is_some() || !self.queued_profile_clicks.is_empty() {
            self.profiles.clear();
        }
        self.cancel_profile_clicks();
    }
    fn clear_drawing_progress(&mut self) {
        self.reset_profile_clicks();
        self.first = None;
        self.first_anchor = None;
        self.arc_start = None;
        self.polyline_start = None;
        self.polyline_segments = 0;
        self.spline_poles.clear();
    }
    pub fn construction(&self) -> bool {
        self.construction
    }
    pub fn toggle_construction(
        &mut self,
        session: &mut Session,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.editing || self.has_pending_edits() {
            return Err("Open a sketch and apply or discard pending edits first".into());
        }
        if self.tool.is_some() || self.element.is_none() {
            self.construction = !self.construction;
            self.message = if self.construction {
                "Construction drawing on"
            } else {
                "Construction drawing off"
            }
            .into();
            return Ok(());
        }
        let id = self.selected.ok_or("Select a sketch")?;
        let (mut source, _, _) = self
            .read_source(session.document(), id, session.revision())
            .map_err(|e| e.to_string())?;
        let curve = source
            .sketch
            .curves
            .iter_mut()
            .find(|curve| Some(curve.id) == self.element)
            .ok_or("Select a curve")?;
        curve.construction = !curve.construction;
        let edit = multicad_solver::prepare_edit_with_report(
            session,
            session.revision(),
            Actor::local(channel),
            id,
            source,
            self.evaluated.as_ref(),
        )
        .map_err(|e| e.to_string())?;
        session
            .commit(
                edit.preview
                    .ok_or("Construction edit conflicts with constraints")?,
            )
            .map_err(|e| e.to_string())?;
        self.revision = session.revision();
        Ok(())
    }
    /// All interactive and exact-entry creation uses the same guarded typed
    /// command. UI state never writes a derived display facade into the source.
    pub fn draw(
        &mut self,
        session: &mut Session,
        request: &multicad_core::sketch_draw::DrawRequest,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.editing {
            return Err("Create or open a sketch first".into());
        }
        if self.has_pending_edits() {
            return Err("Apply or discard pending edits before drawing".into());
        }
        let id = self.selected.ok_or("Select a sketch")?;
        let result = multicad_solver::draw::prepare_draw(
            session,
            session.revision(),
            Actor::local(channel),
            id,
            self.evaluated.as_ref(),
            request,
        )
        .map_err(|e| e.to_string())?;
        self.last_draw_line = result.created_curves.last().and_then(|id| {
            let candidate = result.edit.evaluation.candidate.as_ref()?;
            let curve = candidate.curves.iter().find(|curve| curve.id == *id)?;
            let multicad_core::constraint_sketch::CurveKind::Line { start, end } = curve.kind
            else {
                return None;
            };
            Some([start, end].map(|id| {
                (
                    id,
                    candidate
                        .points
                        .iter()
                        .find(|p| p.id == id)
                        .expect("solved curve points")
                        .position_mm,
                )
            }))
        });
        let selected = result.created_curves.first().copied();
        session
            .commit(result.edit.preview.ok_or_else(|| {
                format!(
                    "Sketch rejected: {:?}",
                    result.edit.evaluation.diagnostics.state
                )
            })?)
            .map_err(|e| e.to_string())?;
        self.clear_drawing_progress();
        self.element = selected;
        self.revision = session.revision();
        self.message = "Geometry committed · Undo restores the previous sketch".into();
        Ok(())
    }
    pub fn artwork_import_active(&self) -> bool {
        self.artwork.active()
    }
    /// Starts a DXF/SVG import from a picked file. The open sketch, if any,
    /// becomes the default target; nothing changes until Apply.
    pub fn begin_artwork_import(&mut self, path: std::path::PathBuf) -> Result<(), String> {
        if self.has_pending_edits() {
            return Err("Apply or discard pending edits before importing artwork".into());
        }
        let open = self.selected.filter(|_| self.editing);
        self.tool = None;
        self.clear_drawing_progress();
        self.artwork.begin(path, open);
        self.message =
            "Import DXF/SVG · choose units, placement and layers in the Inspector".into();
        Ok(())
    }
    pub fn artwork_controller(&mut self) -> &mut crate::artwork_import::ImportController {
        &mut self.artwork
    }
    fn artwork_inspector(&mut self, ui: &mut egui::Ui, session: &mut Session) {
        self.artwork.poll();
        let report = self.evaluated.clone();
        match self.artwork.show(ui, session, report.as_ref()) {
            Some(crate::artwork_import::ImportOutcome::Applied { sketch, curves }) => {
                self.accept_artwork(session, sketch, curves.len())
            }
            Some(crate::artwork_import::ImportOutcome::Cancelled) => {
                self.message = "Import cancelled · document unchanged".into();
            }
            None => {}
        }
        if self.artwork.pending() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
    pub fn accept_artwork(&mut self, session: &Session, sketch: EntityId, curves: usize) {
        if !(self.editing && self.selected == Some(sketch))
            && let Err(error) = self.open_at(session.document(), sketch, session.revision())
        {
            self.message = error;
            return;
        }
        self.revision = session.revision();
        self.fit_requested = true;
        self.message = format!("Imported {curves} curves · Undo restores the previous document");
    }
    pub fn selected_solid(&self) -> Option<EntityId> {
        self.solid.selected_id()
    }
    /// Snapshot-bound profile choices for a subsequent typed solid command.
    /// Retained display geometry never supplies a manufacturing selection.
    pub fn selected_profiles(
        &self,
        document: &Document,
        revision: u64,
    ) -> Result<(EntityId, Vec<multicad_core::sketch_regions::RegionIntent>), String> {
        let id = self.selected.ok_or("Select a sketch profile")?;
        if self.pending_profile_pick.is_some_and(|(stamp, _)| {
            stamp.sketch == id
                && stamp.evaluation.document == document.id()
                && stamp.evaluation.revision == revision
        }) || self.queued_profile_clicks.iter().any(|click| {
            click.sketch == id
                && click.evaluation.document == document.id()
                && click.evaluation.revision == revision
        }) {
            return Err("Wait for the current profile selection query".into());
        }
        let presentation = self
            .sketch_evaluation
            .current(id)
            .filter(|p| p.evaluated.is_current(document, revision))
            .ok_or("Wait for current valid sketch profiles")?;
        if self.profiles.selected().is_empty() {
            return Err("Select one or more closed profile areas".into());
        }
        if self
            .profiles
            .selected()
            .iter()
            .any(|intent| presentation.regions.resolve(intent).is_none())
        {
            return Err("A selected profile boundary changed; select it again".into());
        }
        Ok((id, self.profiles.selected().to_vec()))
    }
    pub fn set_model_backend(&mut self, backend: Box<dyn crate::viewport::SceneBackend>) {
        self.model.set_backend(backend);
    }
    pub fn has_model_backend(&self) -> bool {
        self.model.has_backend()
    }
    pub fn poll_model(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
    ) -> bool {
        self.model.poll(session, resources);
        self.assembly.poll(session, resources);
        self.transfer.poll(session, resources);
        self.symbols.poll(session, resources);
        let external_pending = self.poll_external(session, resources);
        self.model.pending()
            || self.assembly.pending()
            || self.transfer.pending()
            || self.symbols.pending()
            || external_pending
    }
    pub fn model_active(&self) -> bool {
        self.model.active()
            || self.assembly.active()
            || self.transfer.active()
            || self.symbols.active()
    }
    pub fn model_label(&self) -> &'static str {
        if self.symbols.active() {
            self.symbols.label()
        } else if self.transfer.active() {
            self.transfer.label()
        } else if self.assembly.active() {
            self.assembly.label()
        } else {
            self.model.label()
        }
    }
    pub fn primitive_ready(&self) -> bool {
        self.model.has_backend()
            && !self.has_pending_edits()
            && !self.assembly.active()
            && !self.transfer.active()
            && !self.picking
    }
    /// The cabinet behind a single selected cabinet group or carcass panel.
    fn selected_carcass(
        &self,
        document: &Document,
    ) -> Option<crate::modeling::cabinet_edit::CarcassTarget> {
        if self.tree_selection.len() != 1 {
            return None;
        }
        crate::modeling::cabinet_edit::carcass_target(document, *self.tree_selection.first()?)
    }
    fn selected_gear_pair(&self, document: &Document) -> Option<EntityId> {
        let pairs: std::collections::BTreeSet<_> = self
            .tree_selection
            .iter()
            .copied()
            .chain(self.solid.selected_ids())
            .filter_map(|id| crate::modeling::ModelToolController::gear_pair_for(document, id))
            .collect();
        (pairs.len() == 1).then(|| *pairs.first().expect("one pair"))
    }
    fn selected_applied_feature(&self, document: &Document) -> Option<(EntityId, bool)> {
        if self.tree_selection.len() != 1 {
            return None;
        }
        let id = *self.tree_selection.first()?;
        match &document.get(id)?.data {
            EntityData::Feature { .. } => Some((id, false)),
            EntityData::Instance {
                component,
                placement_source,
                ..
            } => {
                if let Some(EntityData::Part { features, .. }) =
                    document.get(*component).map(|e| &e.data)
                    && let Some(feature) = features.iter().rev().find(|feature| {
                        matches!(
                            document.get(**feature).map(|e| &e.data),
                            Some(EntityData::Feature {
                                suppressed: false,
                                ..
                            })
                        )
                    })
                    && let Ok(scale) = multicad_core::model::read_scale(document, *feature)
                    && let multicad_core::model::ScaleInput::Component { owner, base, .. } =
                        scale.input
                    && owner == id
                    && match base {
                        multicad_core::model::PlacementBase::Previous { driver } => {
                            Some(driver) == *placement_source
                        }
                        multicad_core::model::PlacementBase::Literal { .. } => {
                            placement_source.is_none()
                        }
                    }
                {
                    Some((*feature, true))
                } else {
                    placement_source.map(|driver| (driver, true))
                }
            }
            EntityData::Part { features, .. } => features
                .iter()
                .rev()
                .copied()
                .find(|id| {
                    matches!(
                        document.get(*id).map(|entity| &entity.data),
                        Some(EntityData::Feature {
                            suppressed: false,
                            ..
                        })
                    )
                })
                .map(|id| (id, true)),
            _ => None,
        }
    }
    pub fn feature_edit_ready(
        &self,
        session: &Session,
        _feed: &crate::viewport::SceneFeed,
        feature: EntityId,
    ) -> bool {
        self.primitive_ready()
            && !self.editing
            && multicad_core::symbols::owned_entities(session.document())
                .is_ok_and(|ids| !ids.contains(&feature))
            && (crate::modeling::ModelToolController::editable(session.document(), feature)
                || crate::transfer::TransferController::editable(session.document(), feature))
    }
    pub fn begin_feature_edit(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        feed: &crate::viewport::SceneFeed,
        feature: EntityId,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.feature_edit_ready(session, feed, feature) {
            return Err("Select a supported readable applied feature and apply or cancel pending edits before opening its saved controls".into());
        }
        if crate::transfer::TransferController::editable(session.document(), feature) {
            return self.begin_transfer_edit(session, resources, feed, feature, channel);
        }
        self.model
            .begin_existing(session, resources, feature, channel)?;
        if feed.is_current(session.document().id(), session.revision())
            && let Some(scene) = feed.visible()
        {
            let _ = self.model.set_feature_context(session, resources, scene);
        } else {
            self.model.source_only_context(
                feed.error()
                    .unwrap_or("The current native model is not ready"),
            );
        }
        self.message = "Edit feature · change controls, then Preview the downstream model".into();
        Ok(())
    }
    pub fn placement_ready(&self, session: &Session, feed: &crate::viewport::SceneFeed) -> bool {
        self.primitive_ready()
            && !self.editing
            && feed.is_current(session.document().id(), session.revision())
            && feed.visible().is_some_and(|scene| {
                crate::modeling::ModelToolController::placement_available(
                    session.document(),
                    session.revision(),
                    scene,
                    &self.tree_selection,
                )
            })
    }
    pub fn begin_placement(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        feed: &crate::viewport::SceneFeed,
        mode: crate::modeling::PlacementMode,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.placement_ready(session, feed) {
            return Err("Select current native Parts or Instances and finish pending edits before placing them".into());
        }
        let scene = feed
            .visible()
            .cloned()
            .ok_or("Wait for the current native model")?;
        let selected = self.tree_selection.clone();
        self.finish()?;
        self.model
            .begin_placement(session, resources, &scene, &selected, mode, channel)?;
        self.message =
            "Placement · explicit pivot/axis and translation, then Preview and Apply".into();
        Ok(())
    }
    pub fn alignment_ready(&self, session: &Session, feed: &crate::viewport::SceneFeed) -> bool {
        self.tree_selection.len() == 1 && self.placement_ready(session, feed)
    }
    pub fn begin_scale(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        feed: &crate::viewport::SceneFeed,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.placement_ready(session, feed) {
            return Err(
                "Select current native Parts or Instances and finish pending edits before scaling"
                    .into(),
            );
        }
        let scene = feed
            .visible()
            .cloned()
            .ok_or("Wait for the current native model")?;
        let selected = self.tree_selection.clone();
        self.finish()?;
        self.model
            .begin_scale(session, resources, &scene, &selected, false, channel)?;
        self.message =
            "Scale · drag uniform or axis handles, or enter positive factors in the Inspector"
                .into();
        Ok(())
    }
    pub fn begin_alignment(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        feed: &crate::viewport::SceneFeed,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.alignment_ready(session, feed) {
            return Err(
                "Select one current Part or Instance to align and finish pending edits".into(),
            );
        }
        let scene = feed
            .visible()
            .cloned()
            .ok_or("Wait for current native geometry")?;
        let selected = self.tree_selection.clone();
        self.finish()?;
        self.model
            .begin_alignment(session, resources, &scene, &selected, channel)?;
        self.message =
            "Alignment · choose persistent source and target anchors in the Inspector and viewer"
                .into();
        Ok(())
    }
    pub fn boolean_ready(&self, session: &Session, feed: &crate::viewport::SceneFeed) -> bool {
        self.primitive_ready()
            && feed.is_current(session.document().id(), session.revision())
            && feed.visible().is_some_and(|scene| {
                crate::modeling::ModelToolController::boolean_available(
                    session.document(),
                    session.revision(),
                    scene,
                )
            })
    }
    pub fn split_ready(&self, session: &Session, feed: &crate::viewport::SceneFeed) -> bool {
        self.primitive_ready()
            && feed.is_current(session.document().id(), session.revision())
            && feed.visible().is_some_and(|scene| {
                crate::modeling::ModelToolController::split_available(
                    session.document(),
                    session.revision(),
                    scene,
                )
            })
    }
    pub fn begin_split(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        feed: &crate::viewport::SceneFeed,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.split_ready(session, feed) {
            return Err("Wait for two current native bodies with source-bound evaluation metadata, and finish pending edits before splitting".into());
        }
        let scene = feed
            .visible()
            .cloned()
            .ok_or("Wait for the current native model")?;
        let selected = self.tree_selection.clone();
        self.finish()?;
        self.model
            .begin_split(session, resources, &scene, &selected, channel)?;
        self.message = "Split · choose one target and tool, then Preview both new parts".into();
        Ok(())
    }
    pub fn begin_boolean(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        feed: &crate::viewport::SceneFeed,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.boolean_ready(session, feed) {
            return Err("Wait for at least two current native bodies and apply or cancel pending edits before Boolean modeling".into());
        }
        let selected = self.tree_selection.clone();
        let scene = feed
            .visible()
            .cloned()
            .ok_or("Wait for a current native scene")?;
        self.finish()?;
        self.model
            .begin_boolean(session, resources, &scene, &selected, channel)?;
        self.message = "Boolean · choose explicit targets and tools, then Preview".into();
        Ok(())
    }
    pub fn model_can_apply(
        &self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
    ) -> bool {
        if self.symbols.active() {
            self.symbols.can_apply(session, resources)
        } else if self.transfer.active() {
            self.transfer.can_apply(session, resources)
        } else if self.assembly.active() {
            self.assembly.can_apply(session, resources)
        } else {
            self.model.can_apply(session, resources)
        }
    }
    pub fn extrusion_ready(&self, document: &Document, revision: u64) -> bool {
        self.profile_model_ready(document, revision)
    }
    pub fn profile_model_ready(&self, document: &Document, revision: u64) -> bool {
        self.model.has_backend()
            && !self.has_pending_edits()
            && !self.picking
            && (self.profile_inputs(document, revision).is_ok()
                || self.profile_face_inputs(revision).is_ok())
    }
    pub fn begin_extrude(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        self.begin_profile_model(
            session,
            resources,
            crate::modeling::ModelingTool::Extrude,
            channel,
        )
    }
    pub fn begin_revolve(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        self.begin_profile_model(
            session,
            resources,
            crate::modeling::ModelingTool::Revolve,
            channel,
        )
    }
    fn begin_profile_model(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        tool: crate::modeling::ModelingTool,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.model.has_backend() {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.has_pending_edits() || self.picking {
            return Err("Apply or cancel pending edits before extruding".into());
        }
        if !self.profile_faces.is_empty() {
            return self.begin_face_profile_model(session, resources, tool, channel);
        }
        let (sketch, profiles) = self.profile_inputs(session.document(), session.revision())?;
        let guide = self.profile_guide(session.document(), session.revision(), sketch, &profiles);
        self.finish()?;
        match tool {
            crate::modeling::ModelingTool::Extrude => {
                self.model
                    .begin(session, resources, sketch, profiles, channel)?;
                if let Some(scene) = self.display_scene.as_deref() {
                    let _ = self.model.set_extrusion_context(session, resources, scene);
                }
            }
            crate::modeling::ModelingTool::Revolve => self
                .model
                .begin_revolve(session, resources, sketch, profiles, channel)?,
            _ => return Err("Choose a profile modeling tool".into()),
        }
        self.message = match guide {
            Ok(guide) => {
                self.model.set_profile_guide(guide);
                if tool == crate::modeling::ModelingTool::Revolve {
                    "Revolve · choose the axis (click a straight sketch line), then drag the angle handle or type an angle and press Enter".into()
                } else {
                    "Extrude · drag the arrow or type a signed distance, then press Enter".into()
                }
            }
            Err(error) => format!(
                "{} · viewport handle unavailable ({error}); use the Inspector, then Preview",
                tool.label()
            ),
        };
        Ok(())
    }
    pub fn preview_model(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        if self.symbols.active() {
            self.symbols.preview(session, resources, channel)
        } else if self.transfer.active() {
            self.transfer.preview(session, resources, channel)
        } else if self.assembly.active() {
            self.assembly.preview(session, resources, channel)
        } else {
            self.model.preview(session, resources, channel)
        }
    }
    pub fn apply_model(
        &mut self,
        session: &mut Session,
        resources: &multicad_core::project_file::ProjectResources,
    ) -> Result<(), String> {
        if self.symbols.active() {
            let outcome = self.symbols.apply(session, resources)?;
            self.accept_symbol_outcome(outcome);
            return Ok(());
        }
        if self.transfer.active() {
            let outcome = self.transfer.apply(session, resources)?;
            self.accept_transfer_outcome(outcome);
            return Ok(());
        }
        if self.assembly.active() {
            let outcome = self.assembly.apply(session, resources)?;
            self.accept_assembly_outcome(outcome);
            return Ok(());
        }
        let label = self.model.label();
        let instances = self.model.candidate_instances();
        let fit = !self.model.placement_active() && !self.model.scaling_active();
        let outputs = self.model.apply(session, resources)?;
        self.accept_model_outputs(outputs, label, fit);
        self.tree_selection.extend(instances);
        Ok(())
    }
    fn accept_model_outputs(
        &mut self,
        outputs: Vec<multicad_core::model::CreatedBody>,
        label: &str,
        fit: bool,
    ) {
        self.acquisition = None;
        self.profile_faces.clear();
        self.tree_selection = outputs.into_iter().map(|output| output.body).collect();
        self.selected = None;
        self.solid.clear();
        self.fit_requested |= fit;
        self.message = format!("{label} applied · one undoable source edit");
    }
    pub fn cancel_model(&mut self) {
        self.acquisition = None;
        self.profile_faces.clear();
        self.symbols.cancel();
        self.transfer.cancel();
        self.assembly.cancel();
        self.assembly_anchor = None;
        self.model.cancel();
        self.message = "Modeling cancelled · document unchanged".into();
    }
    pub fn begin_primitive(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        kind: crate::modeling::ModelingTool,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.primitive_ready() {
            return Err("Install native modeling and apply or cancel pending edits before creating a primitive".into());
        }
        self.finish()?;
        self.model
            .begin_primitive(session, resources, kind, channel)?;
        self.message = format!(
            "{} · enter dimensions in the Inspector, then Preview",
            kind.label()
        );
        Ok(())
    }
    pub fn begin_thread(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        feed: &crate::viewport::SceneFeed,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.primitive_ready() {
            return Err("Install native modeling and apply or cancel pending edits before creating a thread".into());
        }
        let scene = feed
            .visible()
            .filter(|_| feed.is_current(session.document().id(), session.revision()))
            .cloned();
        let selected = self.tree_selection.clone();
        self.finish()?;
        self.model
            .begin_thread(session, resources, scene.as_deref(), &selected, channel)?;
        self.message =
            "Thread · configure its profile, representation and axis, then Preview".into();
        Ok(())
    }
    pub fn begin_wall(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.primitive_ready() {
            return Err(
                "Install native modeling and apply or cancel pending edits before drawing a wall"
                    .into(),
            );
        }
        self.finish()?;
        self.model.begin_wall(session, resources, channel)?;
        self.message =
            "Wall · choose a type, draw the path in the viewport or enter corners and openings in the Inspector, then Preview"
                .into();
        Ok(())
    }
    pub fn begin_machining(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.primitive_ready() {
            return Err(
                "Install native modeling and apply or cancel pending edits before machining panels"
                    .into(),
            );
        }
        let document = session.document();
        let (targets, refused) =
            crate::modeling::selected_panels(document, self.tree_selection.iter().copied());
        self.finish()?;
        self.model
            .begin_machining(session, resources, targets.clone(), channel)?;
        self.message = crate::modeling::seeded_message(
            "Drill / pocket",
            document,
            &targets,
            &refused,
            "click a panel face in the viewport or tick target panels in the Inspector",
        );
        Ok(())
    }
    pub fn begin_loose_tenon(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.primitive_ready() {
            return Err(
                "Install native modeling and apply or cancel pending edits before joining panels"
                    .into(),
            );
        }
        let document = session.document();
        let (panels, mut refused) =
            crate::modeling::selected_panels(document, self.tree_selection.iter().copied());
        if panels.len() > 2 {
            refused.push(format!(
                "{} panels are selected; the first pair uses two",
                panels.len()
            ));
        }
        let pair = [panels.first().copied(), panels.get(1).copied()];
        let targets: Vec<_> = pair.iter().flatten().copied().collect();
        self.finish()?;
        self.model
            .begin_loose_tenon(session, resources, pair, channel)?;
        self.message = crate::modeling::seeded_message(
            "Domino joint",
            document,
            &targets,
            &refused,
            "click the panel whose face receives the joint, then the panel whose edge meets it",
        );
        Ok(())
    }
    pub fn begin_shelf_pins(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.primitive_ready() {
            return Err(
                "Install native modeling and apply or cancel pending edits before drilling shelf pins"
                    .into(),
            );
        }
        let document = session.document();
        let (panels, refused) =
            crate::modeling::selected_panels(document, self.tree_selection.iter().copied());
        let targets = crate::modeling::with_opposite_sides(document, panels);
        self.finish()?;
        self.model
            .begin_shelf_pins(session, resources, targets.clone(), channel)?;
        self.message = crate::modeling::seeded_message(
            "System 32 shelf pins",
            document,
            &targets,
            &refused,
            "click the inside face of each cabinet side in the viewport or tick the sides in the Inspector",
        );
        Ok(())
    }
    pub fn begin_joint(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.primitive_ready() {
            return Err(
                "Install native modeling and apply or cancel pending edits before joining panels"
                    .into(),
            );
        }
        let document = session.document();
        let (panels, mut refused) =
            crate::modeling::selected_panels(document, self.tree_selection.iter().copied());
        if panels.len() > 2 {
            refused.push(format!(
                "{} panels are selected; the first pair uses two",
                panels.len()
            ));
        }
        let pair = [panels.first().copied(), panels.get(1).copied()];
        let targets: Vec<_> = pair.iter().flatten().copied().collect();
        self.finish()?;
        self.model.begin_joint(session, resources, pair, channel)?;
        self.message = crate::modeling::seeded_message(
            "Panel joint",
            document,
            &targets,
            &refused,
            "click panel A and panel B in the viewport or choose the pairs in the Inspector",
        );
        Ok(())
    }
    pub fn begin_cabinet(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.primitive_ready() {
            return Err(
                "Install native modeling and apply or cancel pending edits before adding panels"
                    .into(),
            );
        }
        self.finish()?;
        self.model.begin_cabinet(session, resources, channel)?;
        self.fit_requested = true;
        self.message = "Cabinet · set the floor datum, panels, individual shelves and connections in the Inspector, then Preview".into();
        Ok(())
    }
    pub fn begin_panel(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.primitive_ready() {
            return Err(
                "Install native modeling and apply or cancel pending edits before adding panels"
                    .into(),
            );
        }
        self.finish()?;
        self.model.begin_panel(session, resources, channel)?;
        self.message =
            "Panel · choose stock, size and placement in the Inspector, then Preview".into();
        Ok(())
    }
    pub fn begin_sheet(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.primitive_ready() {
            return Err(
                "Install native modeling and apply or cancel pending edits before adding sheet parts"
                    .into(),
            );
        }
        self.finish()?;
        self.model.begin_sheet(session, resources, channel)?;
        self.message =
            "Base sheet · choose sheet stock and size in the Inspector, then Preview".into();
        Ok(())
    }
    pub fn begin_weldment(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.primitive_ready() {
            return Err(
                "Install native modeling and apply or cancel pending edits before adding members"
                    .into(),
            );
        }
        self.finish()?;
        self.model.begin_weldment(session, resources, channel)?;
        self.message =
            "Tube / profile · choose stock and a sketch or two points in the Inspector, then Preview"
                .into();
        Ok(())
    }
    pub fn begin_weld(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.primitive_ready() {
            return Err(
                "Install native modeling and apply or cancel pending edits before adding welds"
                    .into(),
            );
        }
        self.finish()?;
        self.model.begin_weld(session, resources, channel)?;
        self.message =
            "Weld joint · choose the connection and weld details in the Inspector, then Preview"
                .into();
        Ok(())
    }
    pub fn begin_weld_connection(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.primitive_ready() {
            return Err(
                "Install native modeling and apply or cancel pending edits before connecting members"
                    .into(),
            );
        }
        self.finish()?;
        self.model
            .begin_weld_connection(session, resources, channel)?;
        self.message =
            "Miter / cope · choose the joint and two members in the Inspector, then Preview".into();
        Ok(())
    }
    pub fn begin_fdm_aid(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.primitive_ready() {
            return Err(
                "Install native modeling and apply or cancel pending edits before adding print aids"
                    .into(),
            );
        }
        let selection: Vec<_> = self.tree_selection.iter().copied().collect();
        let (target, note) =
            crate::modeling::ModelToolController::fdm_seed(session.document(), &selection);
        self.finish()?;
        self.model
            .begin_fdm_aid(session, resources, &selection, channel)?;
        self.message = match (target, note) {
            (Some(part), _) => format!(
                "Tab / bridge / support · finished Part from the selection: {} · set the sizes in the Inspector, then Preview",
                session
                    .document()
                    .get(part)
                    .map_or(String::new(), |e| e.name.clone())
            ),
            (None, Some(note)) => format!("Tab / bridge / support · {note}"),
            (None, None) => {
                "Tab / bridge / support · choose the finished Part and sizes in the Inspector, then Preview"
                    .into()
            }
        };
        Ok(())
    }
    pub fn begin_planting(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.primitive_ready() {
            return Err("Apply or cancel pending edits before placing plants".into());
        }
        self.finish()?;
        self.model.begin_planting(session, resources, channel)?;
        if !session.document().entities().any(|e| {
            matches!(
                e.data,
                EntityData::Part { .. } | EntityData::Instance { .. }
            )
        }) {
            self.camera = Camera {
                yaw: 0.,
                pitch: std::f32::consts::FRAC_PI_2,
                scale: 0.05,
                target: [0.; 3],
                perspective: false,
            };
        }
        self.message =
            "Planting · choose a tree or shrub and click in the garden view, then Preview".into();
        Ok(())
    }
    pub fn begin_equipment(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.primitive_ready() {
            return Err("Apply or cancel pending edits before placing equipment".into());
        }
        self.finish()?;
        self.model.begin_equipment(session, resources, channel)?;
        if !session.document().entities().any(|e| {
            matches!(
                e.data,
                EntityData::Part { .. } | EntityData::Instance { .. }
            )
        }) {
            self.camera = Camera {
                yaw: 0.,
                pitch: std::f32::consts::FRAC_PI_2,
                scale: 0.05,
                target: [0.; 3],
                perspective: false,
            };
        }
        self.message =
            "Equipment · choose a family and host, click in the apartment view, then Preview"
                .into();
        Ok(())
    }
    pub fn begin_architecture(
        &mut self,
        session: &Session,
        resources: &multicad_core::project_file::ProjectResources,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.primitive_ready() {
            return Err(
                "Install native modeling and apply or cancel pending edits before placing levels"
                    .into(),
            );
        }
        self.finish()?;
        self.model.begin_architecture(session, resources, channel)?;
        self.message =
            "Levels & stairs · define a level or place a slab, stair or envelope, then Preview"
                .into();
        Ok(())
    }
    pub fn display_style(&self) -> DisplayStyle {
        self.style
    }
    pub fn set_display_style(&mut self, style: DisplayStyle) {
        self.style = style;
    }
    pub fn request_fit(&mut self) {
        self.fit_requested = true;
    }
    /// Standard orientation framing the whole model; applied on the next
    /// frame, when the current evaluated bounds are known.
    pub fn home_view(&mut self) {
        self.home_requested = true;
    }
    pub fn toggle_transform(&mut self) {
        if self.model_interaction_active() {
            self.message = "Apply or cancel the modeling tool before transforming".into();
            return;
        }
        if self.selected.is_none() {
            self.moving = false;
            self.message = "Select a sketch to transform.".into();
            return;
        }
        if self.constraints.dirty() || self.repair.has_pending() {
            self.message = "Apply or discard pending edits before transforming".into();
            return;
        }
        self.moving = !self.moving;
        self.cancel_operation();
        self.cancel_move();
        self.tool = None;
        self.clear_drawing_progress();
    }
    pub fn set_measure_pin_allowed(&mut self, allowed: bool) {
        self.measure_pin_allowed = allowed;
    }
    pub fn has_pending_edits(&self) -> bool {
        if self.materials.pending() {
            return true;
        }
        self.preview.is_some()
            || self.constraints.dirty()
            || self.repair.has_pending()
            || self.repair_set.has_pending()
            || self.tree.tool_active()
            || self.model.active()
            || self.assembly.dirty()
            || self.transfer.dirty()
            || self.symbols.dirty()
            || self.artwork.active()
            || self.mechanical.active()
            || self.acquisition.is_some()
            || self.external.active()
            || self.trim_worker.pending()
    }
    pub fn cancel(&mut self) {
        if self.measurement.active() {
            self.measurement.close();
            return;
        }
        self.acquisition = None;
        self.symbols.cancel();
        self.artwork.cancel();
        self.mechanical.cancel();
        self.transfer.cancel();
        self.assembly.cancel();
        self.assembly_anchor = None;
        self.external.cancel();
        self.materials.cancel();
        if self.model.active() {
            self.message = "Modeling cancelled · document unchanged".into();
        }
        self.model.cancel();
        self.cancel_operation();
        self.tree.cancel_tool();
        self.repair.cancel();
        self.repair_set.discard();
        self.solid.clear();
        self.moving = false;
        self.picking = false;
        self.plane_picker.cancel();
        self.tool = None;
        self.clear_drawing_progress();
        self.snap_assist.clear();
        self.cancel_move();
    }
    fn cancel_move(&mut self) {
        self.preview = None;
        self.delta = [0.; 3];
        self.drag = None;
    }
    pub fn begin_from_selection(
        &mut self,
        session: &mut Session,
        channel: Channel,
    ) -> Result<(), String> {
        if self.has_pending_edits() {
            return Err("Apply or discard pending edits before creating a sketch".into());
        }
        self.ensure_single_selected_face()?;
        if self.selected_face_hit().is_some() {
            self.create_on_selected_face(session, "Sketch", channel)
        } else {
            self.begin();
            Ok(())
        }
    }
    fn selected_face_hit(&self) -> Option<crate::viewport::Hit> {
        self.solid
            .selected_hit()
            .filter(|hit| hit.key.kind == multicad_core::references::TopologyKind::Face)
            .cloned()
    }
    fn create_on_selected_face(
        &mut self,
        session: &mut Session,
        prefix: &str,
        channel: Channel,
    ) -> Result<(), String> {
        let hit = self
            .selected_face_hit()
            .ok_or("Select a planar face in the 3D view first")?;
        let scene = self.display_scene.clone();
        let context = PlaneContext {
            document: session.document(),
            revision: session.revision(),
            scene: scene.as_deref(),
            report: self.evaluated.as_ref(),
            occurrence: Some(hit.instance),
            section: self.solid.section.plane(),
        };
        let preview = crate::sketch_plane::selected_face(context, &hit)?;
        self.create_named(session, preview, prefix, channel)
    }
    /// Create Engraving: a sketch attached to the planar face selected in the
    /// 3D view. Unlike New sketch there is no plane-picking fallback, and it
    /// defines only the sketch, not any depth or material removal.
    pub fn begin_engraving(
        &mut self,
        session: &mut Session,
        channel: Channel,
    ) -> Result<(), String> {
        if self.has_pending_edits() {
            return Err(
                "Apply or discard pending edits before creating an engraving sketch".into(),
            );
        }
        if self.editing {
            return Err("Finish or abort the open sketch before creating an engraving".into());
        }
        self.ensure_single_selected_face()?;
        if self.selected_face_hit().is_none() {
            return Err("Select a planar face in the 3D view, then Create engraving".into());
        }
        self.create_on_selected_face(session, "Engraving", channel)?;
        self.message =
            "Editing engraving sketch on the selected face · dimensions in millimeters".into();
        Ok(())
    }
    pub fn begin(&mut self) {
        if self.has_pending_edits() {
            self.message =
                "Apply or discard pending edits before choosing a new sketch plane".into();
            return;
        }
        self.occurrence = self.solid.selected_id().or(self.occurrence);
        self.cancel();
        self.selected = None;
        self.editing = false;
        self.picking = true;
        self.message = "Select a datum, construction plane or planar face · Esc cancels".into();
    }
    pub fn create(
        &mut self,
        session: &mut Session,
        datum: Datum,
        channel: Channel,
    ) -> Result<(), String> {
        if self.has_pending_edits() {
            return Err("Apply or discard pending edits before creating a sketch".into());
        }
        let frame = datum.frame();
        self.create_on_plane(
            session,
            PlanePreview {
                choice: PlaneChoice::Datum(datum),
                local_frame: frame,
                world_frame: frame,
                options: Default::default(),
            },
            channel,
        )
    }
    fn create_on_plane(
        &mut self,
        session: &mut Session,
        preview: PlanePreview,
        channel: Channel,
    ) -> Result<(), String> {
        self.create_named(session, preview, "Sketch", channel)
    }
    fn create_named(
        &mut self,
        session: &mut Session,
        preview: PlanePreview,
        prefix: &str,
        channel: Channel,
    ) -> Result<(), String> {
        if self.has_pending_edits() {
            return Err("Apply or discard pending edits before creating a sketch".into());
        }
        let scene = self.display_scene.clone();
        let context = PlaneContext {
            document: session.document(),
            revision: session.revision(),
            scene: scene.as_deref(),
            report: self.evaluated.as_ref(),
            occurrence: self.occurrence,
            section: self.solid.section.plane(),
        };
        // Revalidate even Inspector acceptance against the current scene, not a
        // pointer hit retained from a previous revision or placement.
        let mut validator = PlanePicker::default();
        validator.choose(preview.choice.clone());
        validator.set_options(preview.options)?;
        let preview = validator
            .current_preview(context)?
            .ok_or("Select a sketch plane")?;
        let plane = match &preview.choice {
            PlaneChoice::Datum(d) => sketch::CreationPlane::Datum(*d),
            PlaneChoice::Existing(id) => sketch::CreationPlane::Existing {
                id: *id,
                report: self.evaluated.as_ref(),
            },
            PlaneChoice::Face(face) => sketch::CreationPlane::Face {
                snapshot: &face.snapshot,
                key: face.key,
                anchor: Some(&face.anchor),
            },
        };
        let count = session
            .document()
            .entities()
            .filter(|e| matches!(e.data, EntityData::Sketch { .. }))
            .count();
        let prepared = sketch::prepare_new(
            session,
            session.revision(),
            Actor::local(channel),
            &format!("{prefix} {}", count + 1),
            plane,
            preview.options,
        )
        .map_err(|e| e.to_string())?;
        let id = prepared.sketch;
        // Only prepare_new's insertion of a Plane + empty Sketch is eligible:
        // native producer geometry and placements are unchanged by this command.
        // Never reuse these snapshots after another source edit.
        let initial_report = scene
            .as_deref()
            .filter(|s| {
                s.stamp().document == session.document().id()
                    && s.stamp().revision == session.revision()
            })
            .map(|scene| {
                let revision = session.revision() + 1;
                let mut snapshots = scene.reference_snapshots();
                for snapshot in snapshots.values_mut() {
                    snapshot.revision = revision;
                }
                multicad_core::references::inspect(
                    prepared.prepared.document(),
                    revision,
                    &snapshots,
                )
            });
        let occurrence = match &preview.choice {
            PlaneChoice::Face(face) => Some(face.occurrence),
            PlaneChoice::Existing(plane) => scene
                .as_deref()
                .and_then(|scene| {
                    crate::sketch_context::occurrence(
                        session.document(),
                        *plane,
                        scene,
                        self.occurrence,
                    )
                })
                .map(|(id, _)| id),
            PlaneChoice::Datum(_) => None,
        };
        session
            .commit(prepared.prepared)
            .map_err(|e| e.to_string())?;
        self.activate_sketch(id)?;
        self.creation_evaluation = initial_report.map(|report| (session.document().id(), report));
        if let Some((_, report)) = &self.creation_evaluation {
            self.evaluated = Some(report.clone());
        }
        self.occurrence = occurrence;
        self.refresh_contexts(session.document(), session.revision());
        self.orient_to(preview.world_frame);
        self.message =
            "Sketch ready. Choose Line or Rectangle; coordinates are millimeters.".into();
        Ok(())
    }
    fn orient_to(&mut self, frame: Frame) {
        self.camera.yaw = frame.n[0].atan2(frame.n[1]) as f32;
        self.camera.pitch = frame.n[2].clamp(-1., 1.).asin() as f32;
        self.camera.target = frame.origin.map(|v| v as f32);
    }
    fn refresh_contexts(&mut self, doc: &Document, revision: u64) {
        let Some(scene) = self.display_scene.as_deref() else {
            self.contexts.clear();
            self.context_key = None;
            return;
        };
        let key = (scene.stamp(), revision, self.occurrence);
        if self.context_key == Some(key) {
            return;
        }
        self.contexts = doc
            .entities()
            .filter(|e| matches!(e.data, EntityData::Sketch { .. }))
            .filter_map(|e| {
                crate::sketch_context::occurrence(doc, e.id, scene, self.occurrence)
                    .map(|(_, pose)| (e.id, pose))
            })
            .collect();
        self.context_key = Some(key);
    }
    fn handle_tree_events(
        &mut self,
        session: &Session,
        scene: Option<&crate::viewport::Scene>,
        events: crate::model_tree::TreeEvents,
    ) {
        if let Some(selection) = events.selection {
            if !self.accept_target_selection(session.document(), &selection) {
                return;
            }
            let explicit = [&selection, &self.tree_selection]
                .into_iter()
                .find_map(|ids| {
                    (ids.len() == 1)
                        .then(|| ids.first().copied())
                        .flatten()
                        .filter(|id| {
                            matches!(
                                session.document().get(*id).map(|e| &e.data),
                                Some(EntityData::Instance { .. })
                            )
                        })
                });
            self.occurrence = explicit.or(self.occurrence);
            let sketch = if selection.len() == 1 {
                selection.first().copied().filter(|id| {
                    matches!(
                        session.document().get(*id).map(|e| &e.data),
                        Some(EntityData::Sketch { .. })
                    )
                })
            } else {
                None
            };
            self.tree_selection = selection;
            if sketch != self.selected || sketch.is_none() {
                self.editing = false;
                self.tool = None;
                self.clear_drawing_progress();
                self.element = None;
                self.constraints.clear_geometry_selection();
                self.profiles.clear();
                self.cancel_move();
            }
            self.selected = sketch;
            if sketch.is_some() {
                self.solid.clear();
            } else if let Some(scene) = scene {
                if let Err(error) = self.solid.select_source_entities(
                    session.document(),
                    session.revision(),
                    scene,
                    &self.tree_selection,
                ) {
                    self.solid.clear();
                    self.message = error;
                }
            } else {
                self.solid.clear();
            }
        }
        if let Some(id) = events.open_sketch
            && let Err(error) = self.open_at(session.document(), id, session.revision())
        {
            self.message = error;
        }
        if let Some(message) = events.message {
            self.message = message;
        }
        if let Some(error) = events.error {
            self.message = error;
        }
    }
    fn refresh_overlay_visibility(&mut self, document: &Document, revision: u64) {
        let key = (document.id(), revision);
        if self
            .overlay_visibility
            .as_ref()
            .is_some_and(|(old, _)| *old == key)
        {
            return;
        }
        // Private planting plans live in their local definition coordinates.
        // Drawings apply the occurrence pose; painting them here at the origin
        // would duplicate footprints in the ordinary 3D viewer.
        let plans = multicad_core::symbols::definitions(document)
            .unwrap_or_default()
            .into_iter()
            .map(|(id, definition)| {
                let mut plans =
                    multicad_core::plants::template_for(&definition.template, definition.root)
                        .ok()
                        .flatten()
                        .map(|plant| plant.plan)
                        .into_iter()
                        .collect::<Vec<_>>();
                if let Ok(Some(equipment)) =
                    multicad_core::equipment::template_for(&definition.template, definition.root)
                {
                    plans.push(equipment.footprint);
                    plans.extend(equipment.mark);
                }
                (id, plans)
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let private_plans = document
            .entities()
            .flat_map(|entity| {
                if let EntityData::Extension { definition, .. } = &entity.data
                    && definition.type_id == multicad_core::symbols::INSTANCE_TYPE
                {
                    if let Ok(instance) = multicad_core::symbols::read_instance(document, entity.id)
                    {
                        return plans
                            .get(&instance.definition)
                            .into_iter()
                            .flatten()
                            .filter_map(|plan| instance.ids.get(plan).copied())
                            .collect::<Vec<_>>();
                    }
                    vec![]
                } else {
                    vec![]
                }
            })
            .collect::<std::collections::BTreeSet<_>>();
        let visible = multicad_core::hierarchy::Index::new(document)
            .map(|index| {
                document
                    .entities()
                    .filter(|entity| {
                        matches!(entity.data, EntityData::Sketch { .. })
                            && !private_plans.contains(&entity.id)
                            && crate::viewport::source_visible(document, &index, entity.id)
                                .unwrap_or(false)
                    })
                    .map(|entity| entity.id)
                    .collect()
            })
            .unwrap_or_default();
        self.overlay_visibility = Some((key, visible));
    }
    fn overlay_visible(&self, id: EntityId) -> bool {
        self.editing && self.selected == Some(id)
            || self
                .overlay_visibility
                .as_ref()
                .is_some_and(|(_, visible)| visible.contains(&id))
    }
    fn enqueue_profile_click(
        &mut self,
        document: &Document,
        revision: u64,
        sketch: EntityId,
        point_mm: [f64; 2],
        extend: bool,
    ) -> Result<(), String> {
        if self.selected != Some(sketch)
            || (!self.editing && !self.acquiring_profiles())
            || self.tool.is_some()
            || self.moving
            || (self.has_pending_edits() && !self.acquiring_profiles())
        {
            return Err("Open an idle sketch before selecting profile areas".into());
        }
        let evaluation = self
            .sketch_evaluation
            .current(sketch)
            .filter(|p| p.stamp.document == document.id() && p.stamp.revision == revision)
            .ok_or("Wait for current valid sketch profiles")?
            .stamp;
        if !extend
            || self
                .pending_profile_pick
                .is_some_and(|(stamp, _)| stamp.sketch != sketch || stamp.evaluation != evaluation)
            || self
                .queued_profile_clicks
                .iter()
                .any(|click| click.sketch != sketch || click.evaluation != evaluation)
        {
            self.reset_profile_clicks();
        }
        if self.queued_profile_clicks.len() + usize::from(self.pending_profile_pick.is_some())
            >= MAX_PROFILE_CLICKS
        {
            return Err(format!(
                "Profile selection is busy ({MAX_PROFILE_CLICKS} clicks); wait, then click again"
            ));
        }
        self.queued_profile_clicks.push_back(ProfileClick {
            evaluation,
            sketch,
            point_mm,
            extend,
        });
        self.dispatch_profile_click();
        Ok(())
    }
    fn dispatch_profile_click(&mut self) {
        if self.pending_profile_pick.is_some() {
            return;
        }
        let Some(click) = self.queued_profile_clicks.pop_front() else {
            return;
        };
        if self
            .sketch_evaluation
            .current(click.sketch)
            .is_none_or(|p| p.stamp != click.evaluation)
        {
            self.reset_profile_clicks();
            self.profiles.clear();
            return;
        }
        // Every committed click, including repeated toggles at one position,
        // gets its own ordered reply. The worker may still coalesce previews.
        self.sketch_evaluation.cancel_profile_pick();
        self.pending_profile_pick = self
            .sketch_evaluation
            .request_profile_pick(click.sketch, click.point_mm)
            .map(|stamp| (stamp, click.extend));
        if self.pending_profile_pick.is_none() {
            self.reset_profile_clicks();
            self.profiles.clear();
            self.message = "Profile selection unavailable; select the areas again".into();
        }
    }
    fn poll_profile_pick(&mut self, document: &Document, revision: u64) {
        if self.pending_profile_pick.is_none() && self.queued_profile_clicks.is_empty() {
            return;
        }
        let current = self
            .selected
            .and_then(|id| self.sketch_evaluation.current(id));
        let invalid = (!self.editing && !self.acquiring_profiles())
            || self.tool.is_some()
            || self.moving
            || (self.has_pending_edits() && !self.acquiring_profiles())
            || current.is_none_or(|p| {
                p.stamp.document != document.id()
                    || p.stamp.revision != revision
                    || self.pending_profile_pick.is_some_and(|(stamp, _)| {
                        stamp.sketch != p.evaluated.sketch || stamp.evaluation != p.stamp
                    })
                    || self.queued_profile_clicks.iter().any(|click| {
                        click.sketch != p.evaluated.sketch || click.evaluation != p.stamp
                    })
            });
        if invalid {
            self.reset_profile_clicks();
            return;
        }
        self.dispatch_profile_click();
        let Some((expected, extend)) = self.pending_profile_pick else {
            return;
        };
        let reply = self
            .sketch_evaluation
            .profile_pick()
            .filter(|reply| reply.stamp == expected)
            .cloned();
        if let Some(reply) = reply {
            self.pending_profile_pick = None;
            use crate::sketch_evaluation::ProfileHit;
            match reply.result {
                Ok(ProfileHit::Inside(intent)) => {
                    if let Some(current) = self.sketch_evaluation.current(expected.sketch)
                        && current.regions.resolve(&intent).is_some()
                    {
                        self.element = None;
                        self.constraints.clear_geometry_selection();
                        self.profiles.reconcile(expected.sketch, &current.regions);
                        self.profiles.choose(intent, extend);
                        self.message = format!(
                            "{} closed profile areas selected",
                            self.profiles.selected().len()
                        );
                    }
                }
                Ok(ProfileHit::Outside) => {
                    if !extend {
                        self.element = None;
                        self.constraints.clear_geometry_selection();
                        self.profiles.clear();
                    }
                }
                Ok(ProfileHit::Boundary) => self.message =
                    "Click inside the desired area; its shared boundary is not an area selection"
                        .into(),
                Err(error) => {
                    self.reset_profile_clicks();
                    self.profiles.clear();
                    self.message = error;
                    return;
                }
            }
            self.dispatch_profile_click();
        } else if !self.sketch_evaluation.pick_pending() {
            self.reset_profile_clicks();
            self.message = "Profile selection unavailable; select the areas again".into();
        }
    }
    pub fn open(&mut self, doc: &Document, id: EntityId) -> Result<(), String> {
        if multicad_core::symbols::owned_entities(doc)
            .map_err(|e| e.to_string())?
            .contains(&id)
        {
            return Err("This sketch is generated by a pinned symbol. Edit the symbol's exposed parameters or author a new library revision.".into());
        }
        if matches!(doc.get(id).map(|e|&e.data),Some(EntityData::Sketch{definition,..}) if definition.type_id==multicad_core::constraint_sketch::source::TYPE)
        {
            Source::read_for_evaluation(doc, id, 0, None).map_err(|e| e.to_string())?;
        } else {
            Sketch::read(doc, id).map_err(|e| e.to_string())?;
        }
        self.activate_sketch(id)
    }
    pub fn open_at(&mut self, doc: &Document, id: EntityId, revision: u64) -> Result<(), String> {
        if multicad_core::symbols::owned_entities(doc)
            .map_err(|e| e.to_string())?
            .contains(&id)
        {
            return Err("This sketch is generated by a pinned symbol. Edit the symbol's exposed parameters or author a new library revision.".into());
        }
        if self.tree_selection.len() == 1
            && let Some(instance) = self.tree_selection.first().filter(|id| {
                matches!(
                    doc.get(**id).map(|e| &e.data),
                    Some(EntityData::Instance { .. })
                )
            })
        {
            self.occurrence = Some(*instance);
        }
        self.refresh_contexts(doc, revision);
        let frame = self
            .read_sketch(doc, id, revision)
            .map(|(_, frame, _)| frame)
            .or_else(|_| {
                self.read_source(doc, id, revision)
                    .map(|(_, frame, _)| self.display_frame(id, frame))
            })
            .map_err(|e| e.to_string())?;
        self.activate_sketch(id)?;
        self.orient_to(frame);
        Ok(())
    }
    fn activate_sketch(&mut self, id: EntityId) -> Result<(), String> {
        if self.has_pending_edits() {
            return Err("Apply or discard pending edits before opening a sketch".into());
        }
        self.cancel();
        self.selected = Some(id);
        self.tree_selection = std::collections::BTreeSet::from([id]);
        self.editing = true;
        self.element = None;
        self.message = "Editing sketch · dimensions in millimeters".into();
        Ok(())
    }
    pub fn finish(&mut self) -> Result<(), String> {
        if self.has_pending_edits() {
            return Err("Apply or discard pending sketch edits before finishing".into());
        }
        self.cancel();
        self.editing = false;
        self.moving = false;
        self.message = "Sketch saved in the document. Select it to inspect or reopen.".into();
        Ok(())
    }
    /// Leave sketch editing and drop only what is not yet in the document:
    /// the active tool, unapplied constraint values, previews and pending
    /// controls. Committed sketch edits stay; Undo removes them.
    pub fn abort(&mut self) -> Result<(), String> {
        if !self.editing && !self.picking {
            return Err("No sketch is being edited".into());
        }
        self.cancel();
        self.constraints.discard();
        self.repair.cancel();
        self.repair_set.discard();
        self.trim_worker.cancel();
        if self.has_pending_edits() {
            return Err("Apply or discard the pending sketch controls before aborting".into());
        }
        self.editing = false;
        self.moving = false;
        self.message =
            "Sketch editing aborted · unapplied changes discarded, committed edits kept".into();
        Ok(())
    }
    pub fn set_tool(&mut self, tool: DrawTool) -> Result<(), String> {
        if self.has_pending_edits() {
            return Err("Apply or discard pending edits before drawing".into());
        }
        if !self.editing {
            return Err("Create or open a sketch first".into());
        }
        self.cancel_move();
        self.cancel_operation();
        self.external.cancel();
        self.moving = false;
        self.tool = Some(tool);
        self.clear_drawing_progress();
        self.snap_assist.clear();
        self.set_tool_status();
        Ok(())
    }
    pub fn operation_ready(&self) -> bool {
        self.editing && self.selected.is_some() && !self.picking && !self.has_pending_edits()
    }
    pub fn begin_operation(
        &mut self,
        tool: crate::sketch_operations::OperationTool,
    ) -> Result<(), String> {
        if !self.operation_ready() {
            return Err(
                "Open a sketch and apply or discard pending edits before choosing an edit tool"
                    .into(),
            );
        }
        self.tool = None;
        self.moving = false;
        self.cancel_move();
        self.clear_drawing_progress();
        self.operations.begin(tool);
        self.operation_report = None;
        self.operation_context = None;
        self.operation_guard = None;
        self.operation_pick_view = None;
        self.external.cancel();
        self.operation_error = None;
        self.message = format!(
            "{} · select geometry and preview the edit in Inspector",
            tool.label()
        );
        Ok(())
    }
    fn prepare_operation(
        &mut self,
        session: &Session,
        request: crate::sketch_operations::OperationRequest,
    ) -> Result<(), String> {
        let id = self.selected.ok_or("Select an active sketch")?;
        if !self.operation_ready() {
            return Err("Apply or discard the current preview first".into());
        }
        let tool = self.operations.tool().ok_or("Choose a sketch edit tool")?;
        if let crate::sketch_operations::OperationRequest::Trim(ref trim, policy) = request {
            let (source, _, _) = self
                .read_source(session.document(), id, session.revision())
                .map_err(|e| e.to_string())?;
            if self.exact_operation(&source, trim) {
                return self.request_exact_operation(session, trim.clone(), policy);
            }
        }
        let actor = Actor::local(Channel::Ui);
        use crate::sketch_operations::OperationRequest;
        let edit = match request {
            OperationRequest::Transform(request) => multicad_solver::prepare_transform(
                session,
                session.revision(),
                actor,
                id,
                self.evaluated.as_ref(),
                &request,
            )
            .map(|result| result.edit)
            .map_err(|e| e.to_string())?,
            OperationRequest::Pattern(request) => multicad_solver::prepare_pattern(
                session,
                session.revision(),
                actor,
                id,
                self.evaluated.as_ref(),
                &request,
            )
            .map(|result| result.edit)
            .map_err(|e| e.to_string())?,
            OperationRequest::Trim(request, policy) => {
                let result = multicad_solver::prepare_trim(
                    session,
                    session.revision(),
                    actor,
                    id,
                    self.evaluated.as_ref(),
                    &request,
                    policy,
                )
                .map_err(|e| e.to_string())?;
                self.operation_report = Some(result.report);
                result.edit
            }
        };
        self.adopt_operation_edit(session, id, tool, edit)
    }
    pub fn can_finish_spline(&self) -> bool {
        self.editing && self.tool == Some(DrawTool::Spline) && self.spline_poles.len() >= 4
    }
    pub fn finish_spline(
        &mut self,
        session: &mut Session,
        closed: bool,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.can_finish_spline() {
            return Err("Place at least four spline control poles".into());
        }
        let poles = self
            .spline_poles
            .iter()
            .map(|(position, anchor)| {
                anchor.map_or(
                    multicad_core::sketch_draw::PointInput::Position(*position),
                    multicad_core::sketch_draw::PointInput::Existing,
                )
            })
            .collect();
        let basis = multicad_core::constraint_sketch::SplineBasis::open_uniform(
            3,
            self.spline_poles.len() + usize::from(closed),
        )
        .map_err(|e| e.to_string())?;
        self.draw(
            session,
            &multicad_core::sketch_draw::DrawRequest {
                input: multicad_core::sketch_draw::DrawInput::BSpline {
                    poles,
                    basis,
                    closed,
                },
                construction: self.construction,
            },
            channel,
        )
    }
    fn spline_pointer(
        &mut self,
        painter: &egui::Painter,
        session: &mut Session,
        input: SplinePointer,
    ) {
        let camera = self.camera;
        let project = |point| {
            let p = camera.project(input.frame.point(point).map(|v| v as f32), input.rect);
            Pos2::new(p[0], p[1])
        };
        let close = self
            .spline_poles
            .first()
            .is_some_and(|(point, _)| project(*point).distance(input.screen) <= 8.);
        let mut positions: Vec<_> = self.spline_poles.iter().map(|(point, _)| *point).collect();
        positions.push(if close {
            self.spline_poles[0].0
        } else {
            input.point
        });
        for pair in positions.windows(2) {
            if let Some(segment) = camera.segment(
                input.frame.point(pair[0]),
                input.frame.point(pair[1]),
                input.rect,
            ) {
                painter.extend(egui::Shape::dashed_line(
                    &segment,
                    Stroke::new(0.8, Color32::from_gray(135)),
                    4.,
                    3.,
                ));
            }
        }
        for (point, _) in &self.spline_poles {
            painter.circle_stroke(project(*point), 3.5, Stroke::new(1., Color32::LIGHT_YELLOW));
        }
        if let Ok(basis) =
            multicad_core::constraint_sketch::SplineBasis::open_uniform(3, positions.len())
        {
            let curve = Geometry::BSpline {
                id: self.draw_preview_id,
                poles: positions,
                basis,
            };
            for [a, b] in curve.segments() {
                if let Some(segment) =
                    camera.segment(input.frame.point(a), input.frame.point(b), input.rect)
                {
                    painter.line_segment(segment, Stroke::new(1.5, Color32::LIGHT_YELLOW));
                }
            }
        }
        if input.clicked {
            if close {
                if self.can_finish_spline() {
                    if let Err(error) = self.finish_spline(session, true, Channel::Ui) {
                        self.message = error;
                    }
                } else {
                    self.message = "Place at least four poles before closing the spline".into();
                }
            } else if self.spline_poles.len() >= 255 {
                self.message = "Spline control pole budget reached · Enter finishes".into();
            } else {
                self.spline_poles.push((input.point, input.anchor));
                self.set_tool_status();
            }
        }
    }
    pub fn add(
        &mut self,
        session: &mut Session,
        tool: DrawTool,
        a: [f64; 2],
        b: [f64; 2],
        channel: Channel,
    ) -> Result<(), String> {
        if !self.editing {
            return Err("Create or open a sketch first".into());
        }
        if self.has_pending_edits() {
            return Err("Apply or discard pending edits before drawing".into());
        }
        use multicad_core::sketch_draw::{DrawInput, DrawRequest, PointInput};
        let point = PointInput::Position;
        let input = match tool {
            DrawTool::Line | DrawTool::Polyline => DrawInput::Line {
                start: point(a),
                end: point(b),
            },
            DrawTool::Rectangle => DrawInput::Rectangle {
                a: point(a),
                b: point(b),
            },
            DrawTool::Circle => DrawInput::Circle {
                center: point(a),
                radius_mm: (b[0] - a[0]).hypot(b[1] - a[1]),
            },
            DrawTool::Arc => return Err("An arc needs a center, start and end point".into()),
            DrawTool::Spline => {
                return Err("A cubic spline needs at least four control poles".into());
            }
            DrawTool::RectangleCenter => DrawInput::CenterRectangle {
                center: a,
                corner: point(b),
            },
            DrawTool::RectangleCenterTwo
            | DrawTool::RectangleThree
            | DrawTool::CircleTwoRadius
            | DrawTool::CircleThree => {
                return Err(format!("{} needs three points", tool.label()));
            }
            DrawTool::Fillet | DrawTool::Chamfer => {
                return Err(format!("{} needs a corner point", tool.label()));
            }
            DrawTool::Text => {
                return Err("Text is placed by a click in the sketch view".into());
            }
        };
        self.draw(
            session,
            &DrawRequest {
                input,
                construction: self.construction,
            },
            channel,
        )
    }
    pub fn delete(&mut self, session: &mut Session, channel: Channel) -> Result<(), String> {
        if self.has_pending_edits() {
            return Err("Apply or discard pending edits before deleting geometry".into());
        }
        let id = self.selected.ok_or("Select a sketch")?;
        let element = self
            .element
            .ok_or("Select a line or rectangle in the Inspector")?;
        if let Ok((mut source, _, _)) = self.read_source(session.document(), id, session.revision())
        {
            if !source.sketch.curves.iter().any(|c| c.id == element) {
                return Err("Select a curve to delete".into());
            }
            source.sketch.curves.retain(|c| c.id != element);
            source
                .sketch
                .constraints
                .retain(|c| !c.kind.references().contains(&element));
            let edit = multicad_solver::prepare_edit_with_report(
                session,
                session.revision(),
                Actor::local(channel),
                id,
                source,
                self.evaluated.as_ref(),
            )
            .map_err(|e| e.to_string())?;
            session
                .commit(
                    edit.preview
                        .ok_or("Deleting this curve did not produce a valid sketch")?,
                )
                .map_err(|e| e.to_string())?;
        } else {
            let edits = sketch::remove_geometry(session.document(), id, element)
                .map_err(|e| e.to_string())?;
            commit(session, "Delete sketch geometry", edits, channel)?;
        }
        self.element = None;
        Ok(())
    }
    pub fn dimension(
        &mut self,
        session: &mut Session,
        value: Option<f64>,
        unit: &str,
        channel: Channel,
    ) -> Result<(), String> {
        if !self.editing {
            return Err("Create or open a sketch first".into());
        }
        if self.has_pending_edits() {
            return Err("Apply or discard pending edits before adding a dimension".into());
        }
        let id = self.selected.ok_or("Select a sketch")?;
        let element = self
            .element
            .ok_or("Select a curve in the Inspector first")?;
        if let Some(value) = value {
            self.constraints
                .begin_dimension_exact(session, id, element, value, unit)?;
        } else {
            self.constraints.begin_dimension(session, id, element)?;
        }
        self.tool = None;
        self.clear_drawing_progress();
        self.moving = false;
        if let Some(value) = value {
            let (mut source, _, _) = self
                .read_source(session.document(), id, session.revision())
                .map_err(|e| e.to_string())?;
            source.add_constraint(self.constraints.dimension_kind(value, unit)?);
            let edit = multicad_solver::prepare_edit_with_report(
                session,
                session.revision(),
                Actor::local(channel),
                id,
                source,
                self.evaluated.as_ref(),
            )
            .map_err(|e| e.to_string())?;
            session
                .commit(
                    edit.preview
                        .ok_or("The dimension conflicts with existing constraints")?,
                )
                .map_err(|e| e.to_string())?;
            self.message = "Dimension committed · Undo restores the previous source".into();
        } else {
            self.message = "Dimension · choose the size in Inspector, then Add and Apply".into();
        }
        Ok(())
    }
    fn basis(&self, doc: &Document, revision: u64) -> Result<Frame, String> {
        match self.coordinates {
            Coordinates::World => Ok(Datum::Xy.frame()),
            Coordinates::Object => self
                .selected
                .ok_or_else(|| "Select a sketch".to_string())
                .and_then(|id| {
                    self.read_source(doc, id, revision)
                        .map(|(_, f, _)| self.display_frame(id, f))
                        .map_err(|e| e.to_string())
                }),
            Coordinates::Custom(d) => {
                let mut frame = d.frame();
                let (s, c) = self.custom_angle_deg.to_radians().sin_cos();
                let (u, v) = (frame.u, frame.v);
                frame.u = std::array::from_fn(|i| u[i] * c + v[i] * s);
                frame.v = std::array::from_fn(|i| v[i] * c - u[i] * s);
                Ok(frame)
            }
        }
    }
    fn prepare_move(&mut self, session: &Session) -> Result<(), String> {
        if self.repair.has_pending() || self.constraints.dirty() {
            return Err("Apply or discard pending edits before transforming".into());
        }
        let id = self.selected.ok_or("Select a sketch to move")?;
        if self.waiting_for_sketch_pose(id) {
            return Err(
                "Wait for the selected sketch occurrence placement before transforming".into(),
            );
        }
        let world_delta = self
            .basis(session.document(), session.revision())?
            .vector(self.delta);
        let delta = self.sketch_pose(id).map_or(world_delta, |pose| {
            std::array::from_fn(|axis| {
                let mut basis = [0.; 3];
                basis[axis] = 1.;
                sketch::dot(pose.vector(basis), world_delta)
            })
        });
        let edits = sketch::translate_with_report(
            session.document(),
            id,
            session.revision(),
            self.evaluated.as_ref(),
            delta,
        )
        .map_err(|e| e.to_string())?;
        self.preview = Some(
            session
                .prepare(Request {
                    expected_revision: session.revision(),
                    actor: Actor::local(Channel::Ui),
                    label: "Move sketch".into(),
                    edits,
                })
                .map_err(|e| e.to_string())?,
        );
        Ok(())
    }
    fn apply_move(&mut self, session: &mut Session) -> Result<(), String> {
        if self.repair.has_pending() || self.constraints.dirty() {
            return Err("Apply or discard pending edits before transforming".into());
        }
        let preview = self.preview.take().ok_or("Preview a move first")?;
        session.commit(preview).map_err(|e| e.to_string())?;
        self.delta = [0.; 3];
        self.message = "Moved sketch in one transaction".into();
        Ok(())
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        sketch_mode: bool,
        feed: &mut crate::viewport::SceneFeed,
    ) {
        self.show_project(ui, session, sketch_mode, feed, &Default::default());
    }
    pub fn set_review_scene(&mut self, scene: Option<std::sync::Arc<crate::viewport::Scene>>) {
        self.review_scene = scene;
    }
    pub fn review_scene_shown(&self) -> bool {
        self.review_scene.is_some()
    }
    pub fn show_project(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        sketch_mode: bool,
        feed: &mut crate::viewport::SceneFeed,
        resources: &multicad_core::project_file::ProjectResources,
    ) {
        self.reconcile_acquisition(session);
        self.measurement.tick(session);
        if self.measurement.pending() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        self.model.poll(session, resources);
        self.assembly.poll(session, resources);
        self.transfer.poll(session, resources);
        self.symbols.poll(session, resources);
        if feed.is_current(session.document().id(), session.revision()) {
            self.model.canonical_ready(session);
            self.assembly.canonical_ready(session);
            self.transfer.canonical_ready(session);
            self.symbols.canonical_ready(session);
            if let Some(scene) = feed.visible() {
                self.model.refresh_source_context(session, resources, scene);
            }
        }
        if self.transfer.active()
            && let Err(error) = self.transfer.refresh_scene(
                session,
                resources,
                feed.visible()
                    .filter(|_| feed.is_current(session.document().id(), session.revision())),
            )
        {
            self.message = error;
        }
        if self.model.pending()
            || self.assembly.pending()
            || self.transfer.pending()
            || self.symbols.pending()
        {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        if self
            .creation_evaluation
            .as_ref()
            .is_some_and(|(doc, report)| {
                *doc != session.document().id() || report.revision != session.revision()
            })
        {
            self.creation_evaluation = None;
        }
        self.evaluated = feed
            .references()
            .filter(|e| {
                e.stamp.document == session.document().id()
                    && e.stamp.revision == session.revision()
            })
            .map(|e| e.report.clone())
            .or_else(|| {
                self.creation_evaluation
                    .as_ref()
                    .map(|(_, report)| report.clone())
            });
        if self.evaluated.is_none() {
            self.repair.cancel();
        }
        self.constraints.set_evaluation(self.evaluated.clone());
        let reference_context = feed
            .references()
            .filter(|e| {
                e.stamp.document == session.document().id()
                    && e.stamp.revision == session.revision()
            })
            .map(|e| (e.stamp, e.report.clone()));
        if reference_context.is_none()
            && let Some((_, report)) = &self.creation_evaluation
        {
            self.sketch_evaluation.sync_creation_report(
                session.document(),
                session.revision(),
                report.clone(),
            );
        } else {
            self.sketch_evaluation
                .sync(session.document(), session.revision(), reference_context);
        }
        self.sketch_evaluation.poll();
        self.poll_exact_operation(session);
        if let Err(error) = self.refresh_advanced_sketch_context(session) {
            self.message = error;
        }
        if self.sketch_evaluation.pending() || self.trim_worker.pending() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        self.constraints.set_current(
            self.selected
                .and_then(|id| self.sketch_evaluation.current(id))
                .map(|p| p.evaluated.clone()),
        );
        let mut scene = feed
            .visible()
            .filter(|s| s.stamp().document == session.document().id())
            .cloned();
        self.visibility_error = None;
        scene = scene.and_then(|scene| {
            match self
                .visibility
                .apply(&scene, session.document(), session.revision())
            {
                Ok(view) => Some(view),
                Err(error) => {
                    self.visibility_error = Some(error);
                    None
                }
            }
        });
        self.tree_selection
            .retain(|id| *id == session.document().id() || session.document().get(*id).is_some());
        if let Some(current) = scene
            .as_ref()
            .filter(|scene| scene.stamp().revision == session.revision())
            && self.tree_scene_stamp != Some(current.stamp())
        {
            let keep_topology = self.solid.has_topology_selection()
                && self.tree_scene_stamp.is_some_and(|old| {
                    old.document == current.stamp().document
                        && old.revision == current.stamp().revision
                });
            if !keep_topology
                && !self.editing
                && let Err(error) = self.solid.select_source_entities(
                    session.document(),
                    session.revision(),
                    current,
                    &self.tree_selection,
                )
            {
                self.message = error;
            }
            self.tree_scene_stamp = Some(current.stamp());
        }
        self.display_scene = scene.clone();
        self.refresh_contexts(session.document(), session.revision());
        self.refresh_external(session, feed);
        if self.revision != session.revision() {
            self.cancel_move();
            self.clear_drawing_progress();
            self.revision = session.revision();
            self.tree_selection.retain(|id| {
                *id == session.document().id() || session.document().get(*id).is_some()
            });
            if self.selected.is_some_and(|id| {
                !matches!(
                    session.document().get(id).map(|e| &e.data),
                    Some(EntityData::Sketch { .. })
                )
            }) {
                self.selected = None;
                self.editing = false;
                self.moving = false;
                self.tool = None;
            }
        }
        if !self.measurement.active() {
            self.materials.show(ui.ctx(), session);
        }
        self.poll_profile_pick(session.document(), session.revision());
        if self.pending_profile_pick.is_some() || !self.queued_profile_clicks.is_empty() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        egui::Panel::left("source-tree")
            .default_size(225.)
            .resizable(true)
            .show(ui, |ui| {
                let options = crate::model_tree::TreeOptions {
                    enabled: (!self.has_pending_edits() || self.acquisition.is_some())
                        && !self.measurement.active(),
                    allow_open_sketch: sketch_mode && self.acquisition.is_none(),
                };
                let events = if self.acquisition.is_some() {
                    self.tree
                        .show_targets(ui, session, &self.tree_selection, options)
                } else {
                    self.tree.show(ui, session, &self.tree_selection, options)
                };
                self.handle_tree_events(session, scene.as_deref(), events);
                if !self.measurement.active() {
                    self.advance_model_targets(session, resources, feed, false);
                }
                if sketch_mode
                    && self.selected.is_some()
                    && !self.editing
                    && ui
                        .add_enabled(
                            !self.has_pending_edits(),
                            egui::Button::new("Open selected sketch"),
                        )
                        .clicked()
                {
                    let id = self.selected.expect("selected");
                    if let Err(error) = self.open_at(session.document(), id, session.revision()) {
                        self.message = error;
                    }
                }
            });
        crate::controls::inspector_panel(ui,"source-inspector")
            .show(ui, |ui| {
                crate::controls::inspector_layout(ui);
                ui.heading("Inspector");
                ui.separator();
                use crate::inspector_actions::Owner;
                let owner=if self.measurement.active(){None}
                    else if self.symbols.active(){Some(Owner::Symbols)}
                    else if self.artwork.active() || self.mechanical.active(){Some(Owner::Import)}
                    else if self.transfer.active(){Some(Owner::Transfer)}
                    else if self.assembly.active(){Some(Owner::Assembly)}
                    else if self.model.active(){Some(Owner::Model)}
                    else if self.acquisition.is_some(){Some(Owner::Targets)}
                    else if self.tree.tool_active(){Some(Owner::Tree)}
                    else if self.repair.has_pending() || self.repair_set.has_pending(){Some(Owner::Repair)}
                    else if self.external.active(){Some(Owner::External)}
                    else if self.constraints.dirty(){Some(Owner::Constraints)}
                    else if self.picking{Some(Owner::Plane)}
                    else if self.moving{Some(Owner::Move)}
                    else if self.operations.tool().is_some(){Some(Owner::Operation)}
                    else if self.tool.is_some_and(|tool|tool.guide(0,0).finish_action){Some(Owner::Drawing)}else{None};
                let accent=if sketch_mode{crate::registry::Mode::Sketch.color()}else{crate::registry::Mode::Model.color()};
                crate::inspector_actions::scope(ui,owner,accent,|ui| {
                        if self.measurement.active(){let allow=self.measure_pin_allowed && !self.has_pending_edits();self.measurement.show(ui,session,allow);if let Some((notice,_))=self.measurement.take_notice(){self.message=notice;}return;}
                        if self.symbols.active(){
                            if let Some(outcome)=self.symbols.show(ui,session,resources,true){self.accept_symbol_outcome(outcome);}
                            if self.symbols.pending(){ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));}
                            return;
                        }
                        if self.artwork.active(){
                            self.artwork_inspector(ui,session);
                            return;
                        }
                        if self.mechanical.active(){
                            self.mechanical_import_inspector(ui,session);
                            return;
                        }
                        if self.target_inspector(ui, session, resources, feed) { return; }
                        if self.transfer.active() {
                            match self.transfer.show(ui,session,resources) {
                                Ok(Some(outcome))=>self.accept_transfer_outcome(outcome),
                                Err(error)=>self.message=error,
                                Ok(None)=>{}
                            }
                            if self.transfer.pending(){ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));}
                            return;
                        }
                        if self.assembly.active() {
                            self.assembly_anchor_controls(ui,session,resources,feed);
                            match self.assembly.show(ui,session,resources) {
                                Ok(Some(outcome))=>self.accept_assembly_outcome(outcome),
                                Err(error)=>self.message=error,
                                Ok(None)=>{}
                            }
                            if self.assembly.pending(){ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));}
                            return;
                        }
                        if self.model.active() {
                            let label=self.model.label();
                            let fit = !self.model.placement_active() && !self.model.scaling_active();
                            match self.model.show(ui,session,resources) {
                                Ok(Some(crate::modeling::ModelToolOutcome::Applied(outputs)))=>self.accept_model_outputs(outputs,label,fit),
                                Ok(Some(crate::modeling::ModelToolOutcome::Placed{parts,instances}))=>{self.accept_model_outputs(parts,label,fit);self.tree_selection.extend(instances);},
                                Ok(Some(crate::modeling::ModelToolOutcome::Cancelled))=>{self.acquisition=None;self.message="Modeling cancelled · document unchanged".into();},
                                Err(error)=>self.message=error,
                                Ok(None)=>{}
                            }
                            if self.model.pending() {ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));}
                            return;
                        }
                        if let Some(error)=&self.visibility_error {ui.colored_label(Color32::LIGHT_YELLOW,error);}
                        let hierarchy_tool=self.tree.tool_active();
                        if self.tree.tool_active() || !self.editing {
                            let options=crate::model_tree::TreeOptions{enabled:self.preview.is_none() && !self.constraints.dirty() && !self.repair.has_pending(),allow_open_sketch:sketch_mode};
                            let events=self.tree.inspector(ui,session,&self.tree_selection,options);
                            self.handle_tree_events(session,scene.as_deref(),events);
                        }
                        if hierarchy_tool {return;}
                        if let Some(pair)=self.selected_gear_pair(session.document()) {
                            let ready=self.feature_edit_ready(session,feed,pair);
                            if ui.add_enabled(ready,egui::Button::new("Edit gear pair")).on_hover_text("Edit both mating recipes together, or change their shared drive and layout").clicked()
                                && let Err(error)=self.begin_feature_edit(session,resources,feed,pair,Channel::Ui) {self.message=error;}
                            if self.model_active() {return;}
                        }
                        self.assembly_edit_controls(ui,session,resources,feed);
                        if self.assembly.active(){return;}
                        self.symbol_edit_controls(ui,session,resources,feed);
                        if self.symbols.active(){return;}
                        if let Some(error)=feed.error() {ui.colored_label(Color32::LIGHT_RED,error);if ui.button("Retry model update").on_hover_text("Rebuild the current source while retaining the previous visible model").clicked(){feed.retry();}}
                        else if feed.refining() {ui.small("Refining display…");}
                        else if feed.pending() {ui.small("Rebuilding model…");}
                        if self.sketch_evaluation.pending() {ui.small("Solving sketches…");}
                        if let Some(error)=self.sketch_evaluation.error() {ui.colored_label(Color32::LIGHT_RED,error);}
                        if let Some(failure)=self.selected.and_then(|id|self.sketch_evaluation.errors().get(&id)) {ui.colored_label(Color32::LIGHT_RED,&failure.message);}
                        if let Some(error)=feed.quality_error() {ui.colored_label(Color32::LIGHT_YELLOW,error);if ui.button("Retry display refinement").clicked(){feed.retry_quality();}}
                        if let Some(evaluation)=feed.references() {
                            self.repair.show(ui,session,evaluation);
                            let failed=self.sketch_evaluation.errors().keys().copied().collect();
                            self.repair_set.show(ui,session,resources,evaluation,&failed);
                        }
                        if self.repair.has_pending() {self.tool=None;self.clear_drawing_progress();self.moving=false;self.cancel_move();}
                        if self.editing && let Some(id)=self.selected {
                            self.external_inspector(ui,session,id,resources);
                            if self.external.active(){return;}
                        }
                        if let Some(target)=self.selected_carcass(session.document()) {
                            let ready=self.feature_edit_ready(session,feed,target.feature);
                            if ui.add_enabled(ready,egui::Button::new(format!("Edit cabinet \u{201c}{}\u{201d}",target.name))).on_hover_text("Reopen this cabinet's saved envelope, construction, back and shelf settings; Preview shows the whole cabinet before one undoable Apply").on_disabled_hover_text("Requires an installed native modeling provider and no pending tool").clicked()
                                && let Err(error)=self.begin_feature_edit(session,resources,feed,target.feature,Channel::Ui) {self.message=error;}
                            if self.model_active() {return;}
                        }
                        if let Some((feature,latest))=self.selected_applied_feature(session.document()) {
                            let ready=self.feature_edit_ready(session,feed,feature);
                            if ui.add_enabled(ready,egui::Button::new(if latest {"Edit latest feature"}else{"Edit feature"})).on_hover_text("Reopen this producer's saved controls or its recorded operation group; Preview validates the complete downstream model before Apply").on_disabled_hover_text("Requires an installed native modeling provider, a supported readable source feature, and no pending tool").clicked()
                                && let Err(error)=self.begin_feature_edit(session,resources,feed,feature,Channel::Ui) {self.message=error;}
                            if self.model_active() {return;}
                        }
                        self.solid.inspector(ui,session.document());
                        if !sketch_mode && !self.editing {
                            let mut selection=self.tree_selection.clone();
                            selection.extend(self.solid.selected_ids());
                            let selection:Vec<_>=selection.into_iter().collect();
                            let enabled=self.preview.is_none() && !self.constraints.dirty() && !self.repair.has_pending();
                            self.materials.inspector(ui,session,&selection,enabled);
                        }
                        if !self.editing && !self.picking && !self.moving && !self.has_pending_edits() && self.solid.quick_actions(ui,session,feed.is_current(session.document().id(),session.revision())) {
                            self.fit_requested=true;
                        }
                        if self.picking && sketch_mode {
                            let scene = self.display_scene.clone();
                            let context = PlaneContext { document: session.document(), revision: session.revision(), scene: scene.as_deref().filter(|s| s.stamp().revision == session.revision()), report: self.evaluated.as_ref(), occurrence: self.occurrence, section: self.solid.section.plane() };
                            let chosen = self.plane_picker.show_inspector(ui,context);
                            if self.plane_picker.take_cancel_requested() { self.cancel(); }
                            if let Some(preview) = chosen && let Err(error) = self.create_on_plane(session,preview,Channel::Ui) {self.message=error;}
                        }
                        if self.editing && let Some(tool)=self.tool {
                            ui.strong(tool.label());
                            self.tool_guide_ui(ui, tool);
                            if ui.checkbox(&mut self.construction,"Construction geometry").on_hover_text("Construction curves constrain the sketch but are excluded from solid profiles").changed() {self.message=if self.construction {"Construction drawing on"}else{"Construction drawing off"}.into();}
                            if tool==DrawTool::Arc {ui.checkbox(&mut self.arc_clockwise,"Clockwise arc").on_hover_text("Direction around the sketch's positive normal");}
                            if tool.corner() {
                                self.corner_size_ui(ui, tool);
                            }
                            if tool == DrawTool::Text {
                                self.text_tool_ui(ui, session);
                            }
                            if tool==DrawTool::Spline {
                                ui.small(format!("{} control poles",self.spline_poles.len()));
                                if ui.add_enabled(!self.spline_poles.is_empty(),egui::Button::new("Remove last pole")).on_hover_text("Remove only the last uncommitted control pole").clicked() {self.spline_poles.pop();}
                            }
                            if tool==DrawTool::Spline {
                                use crate::inspector_actions::{self,Button,Owner};
                                let clicked=inspector_actions::row(ui,Owner::Drawing,None,
                                    Button::new("Finish spline",self.can_finish_spline(),"Commit the pending open spline; needs at least four control poles"),
                                    Button::new("Cancel drawing",true,"Discard the uncommitted control poles and end the tool; existing geometry stays"));
                                if clicked.apply && let Err(error)=self.finish_spline(session,false,Channel::Ui){self.message=error;}
                                if clicked.cancel{self.cancel();}
                            }
                            ui.separator();
                        }
                        if let Some(id) = self.selected {
                            let view = self
                                .preview
                                .as_ref()
                                .map(|p| p.document())
                                .or_else(|| self.constraints.preview_document())
                                .unwrap_or(session.document());
                            match self.read_sketch(view, id,session.revision()) {
                                Ok((source, frame, _)) => {
                                    if self.element.is_some_and(|id|!source.geometry.iter().any(|geometry|geometry.id()==id)) {self.element=None;self.constraints.clear_geometry_selection();}
                                    ui.strong(&session.document().get(id).expect("read").name);
                                    ui.label(format!("{} elements · mm", source.geometry.len()));
                                    ui.small(format!(
                                        "Origin {:.2}, {:.2}, {:.2}",
                                        frame.origin[0], frame.origin[1], frame.origin[2]
                                    ));
                                    ui.separator();
                                    for g in &source.geometry {
                                        let label = match g {
                                            Geometry::Line { a, b, .. } => {
                                                format!(
                                                    "Line · {:.2} mm",
                                                    (b[0] - a[0]).hypot(b[1] - a[1])
                                                )
                                            }
                                            Geometry::Rectangle { a, b, .. } => format!(
                                                "Rectangle · {:.2} × {:.2}",
                                                (b[0] - a[0]).abs(),
                                                (b[1] - a[1]).abs()
                                            ),
                                            Geometry::Circle { radius_mm, .. } => {
                                                format!("Circle · R {radius_mm:.2} mm")
                                            }
                                            Geometry::BSpline {poles,basis,..}=>format!("Spline · degree {} · {} poles",basis.degree,poles.len()),
                                            Geometry::Arc { center, a, .. } => format!(
                                                "Arc · R {:.2} mm",
                                                (a[0] - center[0]).hypot(a[1] - center[1])
                                            ),
                                        };
                                        let response = ui.selectable_label(self.element == Some(g.id()) || self.constraints.selected(g.id()) || self.operations.curve_selection().contains(&g.id()), label);
                                        if response.clicked() {
                                            if self.operations.curve_pick_lane().is_some() {
                                                let point = g.segments().first().map_or([0.; 2], |s| s[0]);
                                                let modifiers = crate::selection_input::click_modifiers(ui, &response);
                                                if let Err(error) = self.pick_operation_curve(session, g.id(), point, modifiers.shift || modifiers.command, false) { self.message = error; }
                                                continue;
                                            }
                                            self.element = Some(g.id());
                                            let modifiers = crate::selection_input::click_modifiers(ui, &response);
                                            if !self.constraints.select_geometry(g.id(),Some(g.id()),modifiers.shift || modifiers.command) {self.element=None;}
                                            self.profiles.clear();
                                            self.tool = None;
                                            self.clear_drawing_progress();
                                        }
                                    }
                                    if let Some(presentation)=self.sketch_evaluation.current(id) && self.profiles.show(ui,id,&presentation.regions) {
                                            self.element=None;self.constraints.clear_geometry_selection();
                                            self.cancel_profile_clicks();
                                    }
                                    if self.editing
                                        && ui
                                            .add_enabled(
                                                self.element.is_some_and(|element|!crate::external_references::Controller::protected(session.document(),id,element)),
                                                egui::Button::new("Delete selected element"),
                                            )
                                            .on_disabled_hover_text("Select authored geometry to delete. External outputs follow their source; use Remove reference in External references instead")
                                            .clicked()
                                        && let Err(e) = self.delete(session, Channel::Ui)
                                    {
                                        self.message = e;
                                    }
                                    if self.editing && !self.moving {
                                        if self.operations.tool().is_some() && let Ok((canonical,_,_))=self.read_source(session.document(),id,session.revision()) {
                                            let enabled=self.operation_ready() && self.sketch_evaluation.current(id).is_some();
                                            if let Some(request)=self.operations.show(ui,&canonical,self.constraints.selection(),enabled) && let Err(error)=self.prepare_operation(session,request) {self.operation_error=Some(error.clone());self.message=error;}
                                            if let Some(error)=&self.operation_error {ui.colored_label(Color32::LIGHT_RED,error);}
                                            self.show_operation_report(ui);
                                        }
                                        let before = session.revision();
                                        let selection_before=self.constraints.selection().to_vec();
                                        ui.add_enabled_ui(!self.repair.has_pending(),|ui|self.constraints.show(ui, session, id, self.element));
                                        if self.constraints.selection()!=selection_before {
                                            self.profiles.clear();
                                            self.cancel_profile_clicks();
                                            self.message=format!("{} selected sketch entities · Shift-click adds or removes",self.constraints.selection().len());
                                        }
                                        if session.revision() != before {
                                            self.message =
                                                "Sketch edit applied · one undoable source edit"
                                                    .into();
                                        }
                                        if self.constraints.dirty() {
                                            self.tool = None;
                                            self.clear_drawing_progress();
                                        }
                                    }
                                    if self.moving {
                                        ui.separator();
                                        ui.strong("Move sketch");
                                        if matches!(self.coordinates, Coordinates::Custom(_)) {
                                            ui.horizontal(|ui| {
                                                ui.label("Custom axis angle");
                                                if ui
                                                    .add(
                                                        egui::DragValue::new(
                                                            &mut self.custom_angle_deg,
                                                        )
                                                        .suffix("°")
                                                        .range(-360.0..=360.0),
                                                    )
                                                    .changed()
                                                {
                                                    self.cancel_move();
                                                }
                                            });
                                        }
                                        ui.label(format!(
                                            "{} coordinates · mm",
                                            self.coordinates.label()
                                        ));
                                        let mut changed = false;
                                        for (i, label) in
                                            ["X / U", "Y / V", "Z / N"].into_iter().enumerate()
                                        {
                                            ui.horizontal(|ui| {
                                                ui.label(label);
                                                changed |= ui
                                                    .add(
                                                        egui::DragValue::new(&mut self.delta[i])
                                                            .speed(0.5),
                                                    )
                                                    .changed();
                                            });
                                        }
                                        if changed && let Err(e) = self.prepare_move(session) {
                                            self.preview = None;
                                            self.message = e;
                                        }
                                        use crate::inspector_actions::{self,Button,Owner};
                                        let clicked=inspector_actions::row(ui,Owner::Move,None,
                                            Button::new("Apply move",self.preview.is_some(),"Commit the prepared sketch move in one Undo step"),
                                            Button::new("Cancel move",true,"Discard the uncommitted move"));
                                        if clicked.apply && let Err(e)=self.apply_move(session){self.message=e;}
                                        if clicked.cancel{self.cancel_move();}

                                    }
                                }
                                Err(e) => {
                                    ui.colored_label(Color32::LIGHT_RED, e.to_string());
                                    if self.editing && !self.moving {
                                        self.constraints.show(ui,session,id,self.element);
                                    }
                                }
                            }
                        }
                    });
            });
        self.refresh_overlay_visibility(session.document(), session.revision());
        let mut current = feed.is_current(session.document().id(), session.revision());
        let mut repair_view = false;
        let mut model_view = false;
        let mut assembly_view = false;
        let mut exploded_view = false;
        let mut anchor_view = false;
        let mut transfer_source_view = false;
        self.solid.highlight_reference(None);
        match self.assembly_anchor_scene(session, feed) {
            Ok(Some(source)) => {
                scene = Some(source);
                current = false;
                anchor_view = true;
            }
            Ok(None) => {}
            Err(error) => self.message = error,
        }
        match self.transfer_pick_scene(session, resources) {
            Ok(Some(source)) => {
                scene = Some(source);
                current = false;
                transfer_source_view = true;
            }
            Ok(None) => {}
            Err(error) => self.message = error,
        }
        if !self.measurement.active()
            && self.transfer.picker().is_none()
            && let Some(candidate) = self.transfer.display_scene(session, resources)
        {
            scene = Some(candidate);
            current = false;
            model_view = true;
        }
        if !self.measurement.active()
            && self.assembly.anchor_pick().is_none()
            && let Some(candidate) = self.assembly.display_scene(session, resources)
        {
            scene = Some(candidate);
            current = false;
            model_view = true;
            assembly_view = true;
        }
        if !self.measurement.active()
            && let Some(candidate) = self.symbols.display_scene(session, resources)
        {
            scene = Some(candidate);
            current = false;
            model_view = true;
            assembly_view = true;
        }
        if !self.measurement.active()
            && !self.model.termination_picking()
            && !self.model.alignment_picking()
            && !self.model.edge_treatment_picking()
            && !self.model.advanced_picking()
            && !assembly_view
            && let Some(candidate) = self.model.display_scene(session, resources)
        {
            scene = Some(candidate);
            current = false;
            model_view = true;
        }
        if !self.measurement.active() && self.model.edge_treatment_picking() {
            match self.edge_source_scene(session, feed) {
                Ok(source) => {
                    scene = Some(source);
                    current = false;
                }
                Err(error) => self.message = error,
            }
        }
        if !self.measurement.active() && self.model.advanced_picking() {
            match self.advanced_source_scene(session, feed) {
                Ok(source) => {
                    scene = Some(source);
                    current = false;
                }
                Err(error) => self.message = error,
            }
        }
        if !self.measurement.active()
            && !model_view
            && let Some(review) = &self.review_scene
        {
            scene = Some(review.clone());
            current = false;
            model_view = true;
        }
        if !self.measurement.active()
            && !model_view
            && self.repair.viewing_candidates()
            && let Some(evaluation) = feed.references()
            && evaluation.stamp.revision == session.revision()
            && let Some((source, key)) = self.repair.highlight()
            && let Ok(candidate) = evaluation.candidate.isolated_source(source)
        {
            scene = Some(candidate);
            repair_view = true;
            current = false;
            use crate::viewport::Selection;
            use multicad_core::references::TopologyKind;
            self.solid.highlight_reference(Some(match key.kind {
                TopologyKind::Face => Selection::Face {
                    instance: source,
                    ordinal: key.ordinal,
                },
                TopologyKind::Edge => Selection::Edge {
                    instance: source,
                    ordinal: key.ordinal,
                },
                TopologyKind::Vertex => Selection::Vertex {
                    instance: source,
                    ordinal: key.ordinal,
                },
            }));
        }
        if !self.measurement.active()
            && !model_view
            && let Some(source) = self.external.scene()
        {
            scene = Some(source);
            current = false;
        }
        if !self.measurement.active()
            && let Some(canonical) = &scene
        {
            match self.assembly_exploded_scene(session, resources, canonical) {
                Ok(Some(view)) => {
                    scene = Some(view);
                    current = false;
                    exploded_view = true;
                }
                Ok(None) => {}
                Err(error) => self.message = error,
            }
        }
        let model_bounds = scene.as_ref().and_then(|scene| scene.world_bounds());
        if std::mem::take(&mut self.home_requested) {
            self.camera = crate::initial_fit::home(
                self.camera,
                model_bounds,
                self.viewport_rect.unwrap_or(Rect::NOTHING),
            );
            self.initial_fit.settle();
        }
        if !transfer_source_view
            && !anchor_view
            && let Some(camera) = self.initial_fit.update(
                session.document().id(),
                model_bounds,
                self.camera,
                self.viewport_rect,
            )
        {
            self.camera = camera;
            ui.ctx().request_repaint();
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::from_rgb(21, 27, 35)))
            .show(ui, |ui| {
                self.canvas_with_model(
                    ui,
                    session,
                    sketch_mode,
                    scene.clone(),
                    current,
                    Some(ModelCanvas { resources, feed }),
                )
            });
        if repair_view && let Some(rect) = self.viewport_rect {
            ui.painter().text(
                rect.left_top() + Vec2::new(14., 12.),
                Align2::LEFT_TOP,
                "Reference repair · current source preview",
                FontId::proportional(12.),
                Color32::LIGHT_YELLOW,
            );
        }
        if model_view
            && self.model_active()
            && let Some(rect) = self.viewport_rect
        {
            ui.painter().text(
                rect.left_top() + Vec2::new(14., 12.),
                Align2::LEFT_TOP,
                format!("{} preview · not applied", self.model_label()),
                FontId::proportional(12.),
                Color32::LIGHT_YELLOW,
            );
        }
        if exploded_view && let Some(rect) = self.viewport_rect {
            ui.painter().text(
                rect.left_top() + Vec2::new(14., 12.),
                Align2::LEFT_TOP,
                "Exploded display · inspection uses assembled geometry",
                FontId::proportional(12.),
                Color32::LIGHT_YELLOW,
            );
        }
        if anchor_view && let Some(rect) = self.viewport_rect {
            ui.painter().text(
                rect.left_top() + Vec2::new(14., 12.),
                Align2::LEFT_TOP,
                "Relationship anchor · component source view",
                FontId::proportional(12.),
                Color32::LIGHT_YELLOW,
            );
        }
        if transfer_source_view && let Some(rect) = self.viewport_rect {
            ui.painter().text(
                rect.left_top() + Vec2::new(14., 12.),
                Align2::LEFT_TOP,
                "Transfer reference · component-local source view",
                FontId::proportional(12.),
                Color32::LIGHT_YELLOW,
            );
        }
        if let (Some(scene), Some(rect)) = (&scene, self.viewport_rect)
            && !model_view
            && let Some(bounds) = scene.world_bounds()
            && let Some(quality) =
                crate::viewport::DisplayQuality::for_view(self.camera, rect, bounds)
        {
            feed.note_quality(quality, ui.input(|i| i.time));
        }
    }
    fn read_sketch(
        &self,
        doc: &Document,
        id: EntityId,
        revision: u64,
    ) -> Result<(Sketch, sketch::Frame, EntityId), multicad_core::document::DocumentError> {
        if self.waiting_for_sketch_pose(id) {
            return Err(multicad_core::document::DocumentError::Invalid(
                "Waiting for the selected sketch occurrence's current native placement".into(),
            ));
        }
        let explicit_placement =
            self.selected == Some(id) && self.external.editing_pose().is_some();
        if !explicit_placement
            && self.contexts.contains_key(&id)
            && self.display_scene.as_ref().is_some_and(|scene| {
                scene.stamp().document != doc.id()
                    || scene.stamp().revision != revision
                    || scene.model_evaluation().is_some_and(|info| {
                        multicad_core::project_file::ContentHash::document(doc).ok()
                            != Some(info.source_hash)
                    })
            })
            && !self
                .creation_evaluation
                .as_ref()
                .is_some_and(|(owner, report)| *owner == doc.id() && report.revision == revision)
        {
            return Err(multicad_core::document::DocumentError::Invalid(
                "Waiting for the current placed sketch frame".into(),
            ));
        }
        if let Some(evaluated) = self.semantic_geometry(doc, id, revision) {
            let geometry = if self
                .constraints
                .preview_document()
                .is_some_and(|preview| std::ptr::eq(preview, doc))
            {
                self.constraints.semantic_preview_geometry()
            } else {
                self.sketch_evaluation
                    .current(id)
                    .map(|p| p.geometry.as_ref())
            }
            .ok_or_else(|| {
                multicad_core::document::DocumentError::Invalid(
                    "Sketch has no current valid geometry".into(),
                )
            })?
            .to_vec();
            let frame = if self
                .preview
                .as_ref()
                .is_some_and(|preview| std::ptr::eq(preview.document(), doc))
            {
                self.read_source(doc, id, revision)?.1
            } else {
                evaluated.frame
            };
            return Ok((
                Sketch {
                    u: evaluated.canonical().u,
                    geometry,
                },
                self.display_frame(id, frame),
                evaluated.plane,
            ));
        }
        if doc.get(id).is_some_and(|e|matches!(&e.data,EntityData::Sketch{definition,..} if definition.type_id==multicad_core::constraint_sketch::source::TYPE)) && !self.checked_literal_preview(doc,id) {
            return Err(multicad_core::document::DocumentError::Invalid("Waiting for a current valid sketch solution".into()));
        }
        let (source, frame, plane) = if let Some(report) = &self.evaluated {
            Sketch::read_evaluated(doc, id, revision, report)
        } else {
            Sketch::read(doc, id)
        }?;
        Ok((source, self.display_frame(id, frame), plane))
    }
    fn sketch_pose(&self, id: EntityId) -> Option<crate::viewport::Transform> {
        if self.waiting_for_sketch_pose(id) {
            return None;
        }
        (self.selected == Some(id))
            .then(|| self.external.editing_pose())
            .flatten()
            .or_else(|| self.contexts.get(&id).copied())
    }
    fn waiting_for_sketch_pose(&self, id: EntityId) -> bool {
        self.selected == Some(id) && self.external.waiting_for_editing_pose()
    }
    fn display_frame(&self, id: EntityId, frame: Frame) -> Frame {
        if let Some(pose) = self.sketch_pose(id) {
            Frame {
                origin: pose.point(frame.origin),
                u: pose.vector(frame.u),
                v: pose.vector(frame.v),
                n: pose.vector(frame.n),
            }
        } else {
            frame
        }
    }
    fn checked_literal_preview(&self, doc: &Document, id: EntityId) -> bool {
        self.constraints.preview_sketch() == Some(id)
            && self.constraints.semantic_preview().is_none()
            && self
                .constraints
                .preview_document()
                .is_some_and(|preview| std::ptr::eq(preview, doc))
    }
    fn semantic_geometry(
        &self,
        doc: &Document,
        id: EntityId,
        revision: u64,
    ) -> Option<&multicad_solver::EvaluatedSketch> {
        if self
            .constraints
            .preview_document()
            .is_some_and(|preview| std::ptr::eq(preview, doc))
        {
            return self
                .constraints
                .semantic_preview()
                .filter(|e| e.sketch == id);
        }
        self.sketch_evaluation
            .current(id)
            .filter(|p| p.stamp.document == doc.id() && p.stamp.revision == revision)
            .map(|p| p.evaluated.as_ref())
    }
    /// Transient solved points are used only for display and snaps. Source edits
    /// always start from `read_source`, which retains authored guesses/formulas.
    fn read_solved_source(
        &self,
        doc: &Document,
        id: EntityId,
        revision: u64,
    ) -> Result<(Source, Frame, EntityId), multicad_core::document::DocumentError> {
        if let Some(e) = self.semantic_geometry(doc, id, revision) {
            return e
                .solved()
                .cloned()
                .map(|sketch| {
                    (
                        Source {
                            u: e.canonical().u,
                            sketch,
                        },
                        e.frame,
                        e.plane,
                    )
                })
                .ok_or_else(|| {
                    multicad_core::document::DocumentError::Invalid(
                        "Sketch has no current solution".into(),
                    )
                });
        }
        if doc.get(id).is_some_and(|e|matches!(&e.data,EntityData::Sketch{definition,..} if definition.type_id==multicad_core::constraint_sketch::source::TYPE)) && !self.checked_literal_preview(doc,id) {
            return Err(multicad_core::document::DocumentError::Invalid("Waiting for a current valid sketch solution".into()));
        }
        self.read_source(doc, id, revision)
    }
    fn read_source(
        &self,
        doc: &Document,
        id: EntityId,
        revision: u64,
    ) -> Result<(Source, Frame, EntityId), multicad_core::document::DocumentError> {
        Source::read_for_evaluation(doc, id, revision, self.evaluated.as_ref())
    }
    #[cfg(test)]
    fn canvas(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        sketch_mode: bool,
        scene: Option<std::sync::Arc<crate::viewport::Scene>>,
        current: bool,
    ) {
        self.canvas_with_model(ui, session, sketch_mode, scene, current, None);
    }
    fn canvas_with_model(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        sketch_mode: bool,
        scene: Option<std::sync::Arc<crate::viewport::Scene>>,
        current: bool,
        model_context: Option<ModelCanvas<'_>>,
    ) {
        let sketch_mode = sketch_mode && !self.measurement.active();
        let edge_face_picking = self.model.edge_treatment_pick_kind()
            == Some(multicad_core::references::TopologyKind::Face);
        self.refresh_overlay_visibility(session.document(), session.revision());
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        self.viewport_rect = Some(rect);
        crate::camera_gestures::navigate(ui, &response, &mut self.camera);
        let painter = ui.painter().with_clip_rect(rect);
        let project = |p: [f64; 3], camera: &Camera| {
            let p = camera.project(p.map(|v| v as f32), rect);
            Pos2::new(p[0], p[1])
        };
        if self.grid {
            for [a, b] in crate::viewport::floor_grid(self.camera, rect, self.spacing) {
                if let Some(segment) = self.camera.segment(a, b, rect) {
                    painter.line_segment(segment, Stroke::new(0.5, Color32::from_gray(47)));
                }
            }
        }
        for (axis, color) in [
            ([100., 0., 0.], Color32::from_rgb(233, 91, 102)),
            ([0., 100., 0.], Color32::from_rgb(92, 199, 132)),
            ([0., 0., 100.], Color32::from_rgb(96, 157, 245)),
        ] {
            if let Some(segment) = self.camera.segment([0.; 3], axis, rect) {
                painter.line_segment(segment, Stroke::new(1., color));
            }
        }
        if self.measurement.active() {
            self.measurement.pointer(
                session,
                crate::inspection::MeasurementViewport {
                    scene: scene.as_ref(),
                    camera: self.camera,
                    rect,
                    pointer: response.hover_pos(),
                    clicked: response.clicked(),
                    section: self.solid.section.plane(),
                },
            );
            self.solid.highlight_reference(
                scene
                    .as_ref()
                    .and_then(|s| self.measurement.highlights(session, s)),
            );
        }
        if !self.measurement.active() && self.model.termination_picking() {
            let selected = self.model.termination_viewport(
                session,
                crate::modeling::TerminationViewport {
                    scene: self.display_scene.as_deref(),
                    report: self.evaluated.as_ref(),
                    camera: self.camera,
                    rect,
                    pointer: response.hover_pos(),
                    clicked: response.clicked(),
                    painter: &painter,
                    section: self.solid.section.plane(),
                },
            );
            self.solid
                .highlight_reference(self.model.termination_highlight());
            if selected {
                self.message =
                    "Extrusion end selected · Preview validates its exact current geometry".into();
            }
        }
        if !self.measurement.active() && self.model.alignment_picking() {
            if self.model.alignment_viewport(
                session,
                crate::modeling::TerminationViewport {
                    scene: self.display_scene.as_deref(),
                    report: self.evaluated.as_ref(),
                    camera: self.camera,
                    rect,
                    pointer: response.hover_pos(),
                    clicked: response.clicked(),
                    painter: &painter,
                    section: self.solid.section.plane(),
                },
            ) {
                self.message =
                    "Alignment anchor selected · Preview validates the exact current references"
                        .into();
            }
            self.solid
                .highlight_reference(self.model.alignment_highlight());
        }
        if !self.measurement.active()
            && self.model.opening_picking()
            && self.model.opening_viewport(
                session,
                crate::modeling::TerminationViewport {
                    scene: self.display_scene.as_deref(),
                    report: self.evaluated.as_ref(),
                    camera: self.camera,
                    rect,
                    pointer: response.hover_pos(),
                    clicked: response.clicked(),
                    painter: &painter,
                    section: self.solid.section.plane(),
                },
            )
        {
            self.message =
                "Wall location picked · check the fit in the Inspector, then Preview".into();
        }
        if !self.measurement.active()
            && self.model.fdm_aid_active()
            && let Some(message) = self.model.fdm_aid_viewport(
                session,
                crate::modeling::TerminationViewport {
                    scene: self.display_scene.as_deref(),
                    report: self.evaluated.as_ref(),
                    camera: self.camera,
                    rect,
                    pointer: response.hover_pos(),
                    clicked: response.clicked(),
                    painter: &painter,
                    section: self.solid.section.plane(),
                },
            )
        {
            self.message = message;
        }
        if !self.measurement.active()
            && self.model.outline_active()
            && self.model.outline_viewport(
                session,
                crate::modeling::TerminationViewport {
                    scene: self.display_scene.as_deref(),
                    report: self.evaluated.as_ref(),
                    camera: self.camera,
                    rect,
                    pointer: response.hover_pos(),
                    clicked: response.clicked(),
                    painter: &painter,
                    section: self.solid.section.plane(),
                },
            )
        {
            self.message =
                "Outline changed · check the dimensions in the Inspector, then Preview".into();
        }
        let cabinet_guide = self.model.cabinet_guide(session);
        if !self.measurement.active()
            && self.model.machining_picking()
            && self.model.machining_viewport(
                session,
                crate::modeling::TerminationViewport {
                    scene: self.display_scene.as_deref(),
                    report: self.evaluated.as_ref(),
                    camera: self.camera,
                    rect,
                    pointer: response.hover_pos(),
                    clicked: response.clicked(),
                    painter: &painter,
                    section: self.solid.section.plane(),
                },
            )
        {
            self.message =
                "Panel face placed · check the axes and offsets in the viewport and Inspector, then Preview"
                    .into();
        }
        if !self.measurement.active()
            && self.model.joint_picking()
            && self.model.joint_viewport(
                session,
                crate::modeling::TerminationViewport {
                    scene: self.display_scene.as_deref(),
                    report: self.evaluated.as_ref(),
                    camera: self.camera,
                    rect,
                    pointer: response.hover_pos(),
                    clicked: response.clicked(),
                    painter: &painter,
                    section: self.solid.section.plane(),
                },
            )
        {
            self.message =
                "Joint panel picked · check the pair, mortises and tenons in the viewport and Inspector, then Preview"
                    .into();
        }
        if !self.measurement.active() && self.external.active() {
            if self.external.pointer(crate::external_references::Pointer {
                camera: self.camera,
                rect,
                position: response.hover_pos(),
                clicked: response.clicked(),
                section: self.solid.section.plane(),
                session,
            }) {
                self.message = "Exact source selected · Preview the reference in Inspector".into();
            }
            self.solid.highlight_reference(self.external.highlight());
        }
        if !self.measurement.active()
            && let Some(ref context) = model_context
        {
            self.assembly_viewport(&response, session, context.resources, context.feed, rect);
            self.transfer_viewport(&response, session, context.resources, rect);
            self.advanced_viewport(
                ui,
                &response,
                session,
                context.resources,
                context.feed,
                rect,
            );
            if edge_face_picking {
                self.edge_viewport(
                    ui,
                    &response,
                    session,
                    context.resources,
                    context.feed,
                    rect,
                );
            }
        }
        if let Some(scene) = &scene {
            let selectable = !self.measurement.active()
                && !self.assembly.active()
                && !self.transfer.active()
                && current
                && !self.editing
                && !self.picking
                && !self.moving
                && !self.model.sheet_edge_picking()
                && !self.has_pending_edits();
            self.solid.show(
                ui,
                &response,
                scene.clone(),
                self.camera,
                self.style,
                selectable,
            );
            if response.clicked() && selectable {
                self.tree_selection = self.solid.selected_ids();
                if let Some(instance) = self
                    .solid
                    .hovered_instance()
                    .filter(|id| self.tree_selection.contains(id))
                {
                    self.occurrence = Some(instance);
                }
            }
            if self.solid.selected_id().is_some() && !self.editing {
                self.selected = None;
            }
        }
        if let Some(scene) = &scene {
            if self.envelopes {
                self.paint_envelopes(&painter, scene, rect);
            }
            if self.guides {
                self.paint_guides(&painter, scene, rect);
            }
        }
        if let Some(guide) = &cabinet_guide {
            guide.paint(&painter, self.camera, rect);
        }
        if !self.measurement.active()
            && self.model.sheet_edge_picking()
            && self.model.sheet_viewport(
                session,
                crate::modeling::TerminationViewport {
                    scene: scene.as_deref(),
                    report: self.evaluated.as_ref(),
                    camera: self.camera,
                    rect,
                    pointer: response.hover_pos(),
                    clicked: response.clicked(),
                    painter: &painter,
                    section: self.solid.section.plane(),
                },
            )
        {
            self.message =
                "Sheet flange edge chosen · adjust it in the Inspector, then Preview".into();
        }
        if !self.measurement.active()
            && let Some(context) = model_context
        {
            if self.model.edge_treatment_picking() && !edge_face_picking {
                self.edge_viewport(
                    ui,
                    &response,
                    session,
                    context.resources,
                    context.feed,
                    rect,
                );
            }
            let mut owned = false;
            if self.model.placement_active() || self.model.scaling_active() {
                let scaling = self.model.scaling_active();
                let label = if scaling { "Scale" } else { "Move / Copy" };
                if scaling {
                    self.model
                        .set_scale_copy_modifier(ui.input(|i| i.modifiers.alt));
                } else {
                    self.model
                        .set_placement_copy_modifier(ui.input(|i| i.modifiers.alt));
                }
                let canonical_scene = context
                    .feed
                    .visible()
                    .filter(|_| {
                        context
                            .feed
                            .is_current(session.document().id(), session.revision())
                    })
                    .map(|s| s.as_ref());
                let placement_result = match self.placement_basis(session) {
                    Ok(basis) if scaling => self.model.scale_viewport(
                        ui,
                        session,
                        context.resources,
                        crate::modeling::ScaleViewport {
                            scene: canonical_scene,
                            camera: self.camera,
                            rect,
                            basis,
                            start_allowed: response.hovered()
                                || response.dragged_by(egui::PointerButton::Primary),
                        },
                    ),
                    Ok(basis) => self.model.placement_viewport(
                        ui,
                        session,
                        context.resources,
                        crate::modeling::PlacementViewport {
                            scene: context
                                .feed
                                .visible()
                                .filter(|_| {
                                    context
                                        .feed
                                        .is_current(session.document().id(), session.revision())
                                })
                                .map(|s| s.as_ref()),
                            camera: self.camera,
                            rect,
                            basis,
                            start_allowed: response.hovered()
                                || response.dragged_by(egui::PointerButton::Primary),
                        },
                    ),
                    Err(error) => {
                        self.message = error;
                        Ok(crate::modeling::PlacementViewportResult::default())
                    }
                };
                match placement_result {
                    Ok(result) => {
                        owned = result.input_owned;
                        match result.outcome {
                            Some(crate::modeling::ModelToolOutcome::Applied(parts)) => {
                                self.accept_model_outputs(parts, label, false)
                            }
                            Some(crate::modeling::ModelToolOutcome::Placed {
                                parts,
                                instances,
                            }) => {
                                self.accept_model_outputs(parts, label, false);
                                self.tree_selection.extend(instances);
                            }
                            Some(crate::modeling::ModelToolOutcome::Cancelled) => {
                                self.cancel_model()
                            }
                            None => {}
                        }
                    }
                    Err(error) => {
                        owned = true;
                        self.message = error;
                    }
                }
                if self.model.pending() {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(16));
                }
            }
            if self.model.operation_gizmo_active() {
                owned |= self.operation_viewport(ui, &response, session, context.resources, rect);
            }
            if !owned {
                self.target_viewport(
                    ui,
                    &response,
                    session,
                    context.resources,
                    context.feed,
                    rect,
                );
            }
        }
        self.measurement.paint(
            &painter,
            rect,
            self.camera,
            session,
            scene.as_ref(),
            self.solid.section.plane(),
        );
        self.assembly_clearance_paint(&painter, rect, session, scene.as_ref());
        if !self.measurement.active()
            && scene.is_none()
            && response.clicked()
            && !self.editing
            && !self.picking
            && !self.moving
            && !self.has_pending_edits()
        {
            self.tree_selection.clear();
            self.selected = None;
        }
        if self.picking && sketch_mode {
            let context = PlaneContext {
                document: session.document(),
                revision: session.revision(),
                scene: scene.as_deref().filter(|_| current),
                report: self.evaluated.as_ref(),
                occurrence: self.occurrence,
                section: self.solid.section.plane(),
            };
            self.plane_picker
                .update_hover(response.hover_pos(), self.camera, rect, context);
            self.plane_picker
                .paint_preview(&painter, self.camera, rect, context);
            if response.clicked() {
                let chosen = self.plane_picker.click_hovered(context);
                match chosen {
                    Ok(_) => {
                        self.message =
                            "Plane selected · adjust offset or angle, then Use plane".into()
                    }
                    Err(error) => self.message = error,
                }
            }
        }
        let doc = self
            .preview
            .as_ref()
            .map(|p| p.document())
            .or_else(|| self.constraints.preview_document())
            .unwrap_or(session.document());
        let mut points = cabinet_guide
            .as_ref()
            .map_or_else(Vec::new, |g| g.fit_points());
        let model_view = scene
            .as_ref()
            .is_some_and(|scene| self.model.owns_display_scene(scene));
        if (self.measurement.active()
            || model_view
            || self.external.active()
            || self.selected.is_none())
            && let Some(scene) = &scene
            && let Some([low, high]) = self
                .solid
                .selected_bounds(scene)
                .or_else(|| {
                    let mut bounds: Option<[[f64; 3]; 2]> = None;
                    for id in &self.tree_selection {
                        if let Some(selected) = scene.instance_bounds(*id) {
                            if let Some(current) = &mut bounds {
                                for axis in 0..3 {
                                    current[0][axis] = current[0][axis].min(selected[0][axis]);
                                    current[1][axis] = current[1][axis].max(selected[1][axis]);
                                }
                            } else {
                                bounds = Some(selected);
                            }
                        }
                    }
                    bounds
                })
                .or_else(|| scene.world_bounds())
        {
            for corner in 0..8 {
                points.push(std::array::from_fn(|i| {
                    if corner & (1 << i) == 0 {
                        low[i] as f32
                    } else {
                        high[i] as f32
                    }
                }));
            }
        }
        let affected = self.constraints.affected();
        for e in doc.entities().filter(|e| {
            !self.measurement.active()
                && !self.repair.viewing_candidates()
                && self.overlay_visible(e.id)
        }) {
            if let Ok((source, frame, _)) = self.read_sketch(doc, e.id, session.revision()) {
                if self.selected == Some(e.id)
                    && (self.editing || self.acquiring_profiles())
                    && self.tool.is_none()
                    && !self.moving
                    && !self.external.active()
                    && (!self.has_pending_edits() || self.acquiring_profiles())
                    && let Some(current) = self.sketch_evaluation.current(e.id)
                    && current.stamp.document == session.document().id()
                    && current.stamp.revision == session.revision()
                {
                    let hover = response
                        .hover_pos()
                        .and_then(|p| self.camera.on_plane(p, rect, frame));
                    for fill in current.fills.iter() {
                        let selected = self.profiles.selected().contains(&fill.intent);
                        if selected || hover.is_some_and(|p| fill.contains(p)) {
                            fill.paint(&painter, self.camera, rect, frame, selected);
                        }
                    }
                }
                let constrained = self.read_solved_source(doc, e.id, session.revision()).ok();
                let conflicted_curves: std::collections::BTreeSet<_> = constrained
                    .as_ref()
                    .into_iter()
                    .flat_map(|(source, _, _)| source.sketch.curves.iter())
                    .filter(|c| match c.kind {
                        multicad_core::constraint_sketch::CurveKind::Line { start, end } => {
                            affected.contains(&start) || affected.contains(&end)
                        }
                        multicad_core::constraint_sketch::CurveKind::Circle { center, .. } => {
                            affected.contains(&center)
                        }
                        multicad_core::constraint_sketch::CurveKind::Arc {
                            center,
                            start,
                            end,
                            ..
                        } => [center, start, end].iter().any(|id| affected.contains(id)),
                        multicad_core::constraint_sketch::CurveKind::BSpline {
                            ref poles, ..
                        } => poles.iter().any(|id| affected.contains(id)),
                    })
                    .map(|c| c.id)
                    .collect();
                for g in &source.geometry {
                    let construction = constrained.as_ref().is_some_and(|(source, _, _)| {
                        source
                            .sketch
                            .curves
                            .iter()
                            .any(|curve| curve.id == g.id() && curve.construction)
                    });
                    let mut dash_offset = 0.;
                    let selected = self.selected == Some(e.id);
                    let conflicted =
                        affected.contains(&g.id()) || conflicted_curves.contains(&g.id());
                    let color = if selected && conflicted {
                        Color32::LIGHT_RED
                    } else if selected
                        && (self.element == Some(g.id())
                            || self.constraints.selected(g.id())
                            || self.operations.curve_target() == Some(g.id())
                            || self.operations.curve_boundaries().contains(&g.id())
                            || self.profiles.boundary_selected(g.id()))
                    {
                        Color32::from_rgb(255, 195, 80)
                    } else if selected {
                        Color32::from_rgb(100, 225, 220)
                    } else {
                        Color32::from_rgb(148, 163, 181)
                    };
                    for [a, b] in g.segments() {
                        let a = frame.point(a);
                        let b = frame.point(b);
                        if !model_view
                            && !self.external.active()
                            && self.solid.selected_id().is_none()
                            && self.selected.is_none_or(|id| id == e.id)
                        {
                            points.extend([a.map(|v| v as f32), b.map(|v| v as f32)]);
                        }
                        if let Some(segment) = self.camera.segment(a, b, rect) {
                            let stroke = Stroke::new(if selected { 2. } else { 1. }, color);
                            if construction {
                                painter.extend(egui::Shape::dashed_line_with_offset(
                                    &segment,
                                    stroke,
                                    &[5.],
                                    &[4.],
                                    dash_offset,
                                ));
                                dash_offset = (dash_offset + segment[0].distance(segment[1])) % 9.;
                            } else {
                                painter.line_segment(segment, stroke);
                            }
                        }
                    }
                }
                if self.selected == Some(e.id)
                    && self.editing
                    && let Some((source, _, _)) = constrained.as_ref()
                {
                    for point in &source.sketch.points {
                        let p = project(frame.point(point.position_mm), &self.camera);
                        let color = if self.constraints.selected(point.id) {
                            Color32::YELLOW
                        } else if !self.constraints.free(point.id) {
                            Color32::from_rgb(100, 205, 140)
                        } else {
                            Color32::from_rgb(95, 175, 200)
                        };
                        painter.circle_filled(p, 2.5, color);
                    }
                }
            } else if let Some(retained) = self.sketch_evaluation.last_valid(e.id) {
                // Retained geometry is a visual aid only. All interaction paths
                // use read_sketch/current and therefore reject this stale view.
                let frame = self.display_frame(e.id, retained.evaluated.frame);
                for geometry in retained.geometry.iter() {
                    for [a, b] in geometry.segments() {
                        if let Some(segment) =
                            self.camera.segment(frame.point(a), frame.point(b), rect)
                        {
                            painter.line_segment(segment, Stroke::new(1., Color32::from_gray(90)));
                        }
                    }
                }
            }
        }
        self.begin_snap_frame();
        if sketch_mode
            && self.editing
            && !self.has_pending_edits()
            && !self.moving
            && let (Some(id), Some(tool)) = (self.selected, self.tool)
            && let Ok((_, frame, _)) = self.read_sketch(session.document(), id, session.revision())
            && let Some(pointer) = response.hover_pos()
            && let Some(raw) = self.camera.on_plane(pointer, rect, frame)
        {
            use multicad_core::sketch_draw::{DrawInput, DrawRequest, PointInput};
            // Geometry targets and assistive lines win over the optional grid.
            let (mut p, mut anchor) = self.snap_pointer(ui, session, id, frame, rect, pointer, raw);
            // Closing a chain is an explicit vertex target, independent of the
            // optional grid/near-point snapping preference.
            if tool == DrawTool::Polyline
                && let Some((stored, start_id)) = self.polyline_start
            {
                let current = self
                    .read_solved_source(session.document(), id, session.revision())
                    .ok()
                    .and_then(|(source, _, _)| {
                        source
                            .sketch
                            .points
                            .iter()
                            .find(|p| p.id == start_id)
                            .map(|p| p.position_mm)
                    })
                    .unwrap_or(stored);
                if project(frame.point(current), &self.camera).distance(pointer) <= 8. {
                    p = current;
                    anchor = Some(start_id);
                }
            }
            if tool == DrawTool::Arc
                && let (Some(center), Some((start, _))) = (self.first, self.arc_start)
            {
                let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                let direction = [p[0] - center[0], p[1] - center[1]];
                let length = direction[0].hypot(direction[1]);
                if length > 1e-9 {
                    if (length - radius).abs() > 1e-7 {
                        anchor = None;
                    }
                    p = std::array::from_fn(|i| center[i] + direction[i] * radius / length);
                }
            }
            self.snap_assist.settle(p);
            let cursor = project(frame.point(p), &self.camera);
            painter.circle_stroke(
                cursor,
                if anchor.is_some() { 5. } else { 4. },
                Stroke::new(
                    1.,
                    if anchor.is_some() {
                        Color32::YELLOW
                    } else {
                        Color32::WHITE
                    },
                ),
            );
            if tool.multi_point() {
                self.mode_pointer(
                    &painter,
                    session,
                    tool,
                    draw_modes::ModePointer {
                        frame,
                        rect,
                        screen: pointer,
                        point: p,
                        anchor,
                        clicked: response.clicked(),
                    },
                );
            } else if tool == DrawTool::Spline {
                self.spline_pointer(
                    &painter,
                    session,
                    SplinePointer {
                        frame,
                        rect,
                        screen: pointer,
                        point: p,
                        anchor,
                        clicked: response.clicked(),
                    },
                );
            } else {
                let point = |position, anchor: Option<EntityId>| {
                    anchor
                        .map(PointInput::Existing)
                        .unwrap_or(PointInput::Position(position))
                };
                if let Some(a) = self.first {
                    let ghost = match tool {
                        DrawTool::Line | DrawTool::Polyline => Geometry::Line {
                            id: self.draw_preview_id,
                            a,
                            b: p,
                        },
                        DrawTool::Rectangle => Geometry::Rectangle {
                            id: self.draw_preview_id,
                            a,
                            b: p,
                        },
                        DrawTool::Circle => Geometry::Circle {
                            id: self.draw_preview_id,
                            center: a,
                            radius_mm: (p[0] - a[0]).hypot(p[1] - a[1]),
                        },
                        DrawTool::Spline => unreachable!("Spline uses the control-pole gesture"),
                        DrawTool::RectangleCenter
                        | DrawTool::RectangleCenterTwo
                        | DrawTool::RectangleThree
                        | DrawTool::CircleTwoRadius
                        | DrawTool::CircleThree
                        | DrawTool::Fillet
                        | DrawTool::Chamfer
                        | DrawTool::Text => {
                            unreachable!("multi-click modes use their own gesture")
                        }
                        DrawTool::Arc => {
                            if let Some((start, _)) = self.arc_start {
                                Geometry::Arc {
                                    id: self.draw_preview_id,
                                    center: a,
                                    a: start,
                                    b: p,
                                    clockwise: self.arc_clockwise,
                                }
                            } else {
                                Geometry::Circle {
                                    id: self.draw_preview_id,
                                    center: a,
                                    radius_mm: (p[0] - a[0]).hypot(p[1] - a[1]),
                                }
                            }
                        }
                    };
                    for [a, b] in ghost.segments() {
                        if let Some(segment) =
                            self.camera.segment(frame.point(a), frame.point(b), rect)
                        {
                            painter.line_segment(segment, Stroke::new(1., Color32::LIGHT_YELLOW));
                        }
                    }
                }
                if response.clicked() {
                    if let Some(a) = self.first {
                        if tool == DrawTool::Arc && self.arc_start.is_none() {
                            if (p[0] - a[0]).hypot(p[1] - a[1]) >= 1e-7 {
                                self.arc_start = Some((p, anchor));
                                self.set_tool_status();
                            }
                        } else {
                            let start = point(a, self.first_anchor);
                            let end = point(p, anchor);
                            let closing = tool == DrawTool::Polyline
                                && self
                                    .polyline_start
                                    .is_some_and(|(_, id)| anchor == Some(id));
                            let input = match tool {
                                DrawTool::Line | DrawTool::Polyline => {
                                    DrawInput::Line { start, end }
                                }
                                DrawTool::Rectangle => DrawInput::Rectangle { a: start, b: end },
                                DrawTool::Circle => DrawInput::Circle {
                                    center: start,
                                    radius_mm: (p[0] - a[0]).hypot(p[1] - a[1]),
                                },
                                DrawTool::Spline => {
                                    unreachable!("Spline uses the control-pole gesture")
                                }
                                DrawTool::RectangleCenter
                                | DrawTool::RectangleCenterTwo
                                | DrawTool::RectangleThree
                                | DrawTool::CircleTwoRadius
                                | DrawTool::CircleThree
                                | DrawTool::Fillet
                                | DrawTool::Chamfer
                                | DrawTool::Text => {
                                    unreachable!("multi-click modes use their own gesture")
                                }
                                DrawTool::Arc => {
                                    let (b, anchor) = self.arc_start.expect("arc start chosen");
                                    DrawInput::Arc {
                                        center: start,
                                        start: point(b, anchor),
                                        end,
                                        clockwise: self.arc_clockwise,
                                    }
                                }
                            };
                            let first = self.polyline_start;
                            let segments = self.polyline_segments;
                            if closing && segments < 2 {
                                self.message =
                                    "A closed polyline needs at least three vertices".into();
                            } else if let Err(error) = self.draw(
                                session,
                                &DrawRequest {
                                    input,
                                    construction: self.construction,
                                },
                                Channel::Ui,
                            ) {
                                self.message = error;
                            } else if tool == DrawTool::Polyline
                                && !closing
                                && let Some([start, end]) = self.last_draw_line
                            {
                                self.polyline_start = first.or(Some((start.1, start.0)));
                                self.polyline_segments = segments + 1;
                                self.first = Some(end.1);
                                self.first_anchor = Some(end.0);
                                self.set_committed_status(tool);
                            } else {
                                self.set_committed_status(tool);
                            }
                        }
                    } else {
                        self.first = Some(p);
                        self.first_anchor = anchor;
                        self.set_tool_status();
                    }
                }
            }
        }
        if sketch_mode
            && self.editing
            && self.operations.curve_pick_lane().is_some()
            && self.tool.is_none()
            && !self.external.active()
            && !self.moving
            && !self.has_pending_edits()
            && response.clicked()
            && let Some(id) = self.selected
            && let Ok((_, frame, _)) = self.read_sketch(session.document(), id, session.revision())
            && let Some(presentation) = self.sketch_evaluation.current(id).cloned()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            if let Some(crate::sketch_selection::Hit::Curve(curve)) =
                crate::sketch_selection::pick_geometry(
                    pointer,
                    self.camera,
                    rect,
                    frame,
                    &[],
                    &presentation.geometry,
                )
                && let Some(point) = self.camera.on_plane(pointer, rect, frame)
            {
                let modifiers = crate::selection_input::click_modifiers(ui, &response);
                if let Err(error) = self.pick_operation_curve(
                    session,
                    curve,
                    point,
                    modifiers.shift || modifiers.command,
                    true,
                ) {
                    self.message = error;
                }
            } else {
                self.message = "Click a current solved curve for the active operation pick".into();
            }
        }
        if sketch_mode
            && self.editing
            && self.operations.curve_pick_lane().is_none()
            && self.tool.is_none()
            && !self.external.active()
            && !self.moving
            && !self.has_pending_edits()
            && response.clicked()
            && let Some(id) = self.selected
            && let Ok((_, frame, _)) = self.read_sketch(session.document(), id, session.revision())
            && let Some(presentation) = self.sketch_evaluation.current(id).cloned()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            use crate::sketch_selection::{self, Hit};
            let hit = presentation.evaluated.solved().and_then(|solved| {
                sketch_selection::pick_geometry(
                    pointer,
                    self.camera,
                    rect,
                    frame,
                    &solved.points,
                    &presentation.geometry,
                )
            });
            let modifiers = crate::selection_input::click_modifiers(ui, &response);
            let extend = modifiers.shift || modifiers.command;
            // CAD-149: a plain click on a letter selects its text element and
            // opens it in the Text tool; Shift-click still picks profiles.
            let text_curve = match hit {
                Some(Hit::Curve(curve)) => Some(curve),
                _ => None,
            };
            let opened = !extend
                && !matches!(hit, Some(Hit::Point(_)))
                && self.select_text_at(
                    session,
                    text_curve,
                    self.camera.on_plane(pointer, rect, frame),
                );
            match hit {
                _ if opened => {}
                Some(Hit::Point(point)) => {
                    self.reset_profile_clicks();
                    self.element = None;
                    self.constraints.select_geometry(point, None, extend);
                    self.profiles.clear();
                    self.message = format!(
                        "{} selected sketch entities · Shift-click adds or removes",
                        self.constraints.selection().len()
                    );
                }
                Some(Hit::Curve(curve)) => {
                    self.reset_profile_clicks();
                    self.element = Some(curve);
                    if !self.constraints.select_geometry(curve, Some(curve), extend) {
                        self.element = None;
                    }
                    self.profiles.clear();
                    self.message = format!(
                        "{} selected sketch entities · Shift-click adds or removes",
                        self.constraints.selection().len()
                    );
                }
                Some(Hit::Region(region)) => {
                    self.reset_profile_clicks();
                    self.element = None;
                    self.constraints.clear_geometry_selection();
                    self.profiles.reconcile(id, &presentation.regions);
                    self.profiles.choose(region, extend);
                    self.message = format!(
                        "{} closed profile areas selected",
                        self.profiles.selected().len()
                    );
                }
                None => {
                    if let Some(point) = self.camera.on_plane(pointer, rect, frame) {
                        if let Err(error) = self.enqueue_profile_click(
                            session.document(),
                            session.revision(),
                            id,
                            point,
                            extend,
                        ) {
                            self.message = error;
                        }
                        if self.pending_profile_pick.is_some() {
                            ui.ctx()
                                .request_repaint_after(std::time::Duration::from_millis(16));
                        }
                    }
                }
            }
        }
        self.paint_snap_aids(&painter, session, rect);
        if !self.measurement.active() && self.moving && self.selected.is_some() {
            self.gizmo(ui, session, rect);
        }
        if sketch_mode || scene.is_none() {
            painter.text(
                rect.left_top() + Vec2::new(16., 14.),
                Align2::LEFT_TOP,
                &self.message,
                FontId::proportional(12.),
                Color32::from_gray(175),
            );
        }
        egui::Area::new(ui.id().with("orientation"))
            .movable(false)
            .pivot(Align2::RIGHT_TOP)
            .fixed_pos(rect.right_top() - Vec2::new(8., -4.))
            .show(ui.ctx(), |ui| {
                navigation::orientation(ui, &mut self.camera);
            });
        let mut coordinates = self.coordinates;
        let mut moving = self.moving;
        let mut grid = self.grid;
        let mut envelopes = self.envelopes;
        let mut guides = self.guides;
        let mut spacing = self.spacing;
        let mut snap = self.snap;
        let mut geometry_snap = self.geometry_snap;
        egui::Area::new(ui.id().with("camera-toolbar"))
            .movable(false)
            .pivot(Align2::RIGHT_BOTTOM)
            .fixed_pos(rect.right_bottom() - Vec2::splat(12.))
            .show(ui.ctx(), |ui| {
                let actions = navigation::toolbar_with_tools(
                    ui,
                    &mut self.camera,
                    &mut self.style,
                    |ui| {
                        ui.checkbox(&mut grid, "XY grid");
                        ui.horizontal(|ui| {
                            ui.label("Spacing · mm");
                            ui.add(egui::DragValue::new(&mut spacing).range(0.01..=10000.));
                        });
                        ui.checkbox(&mut snap, "Snap sketch points to grid spacing");
                        ui.checkbox(&mut geometry_snap, "Snap to geometry and assistive lines");
                        ui.checkbox(&mut envelopes, "Service envelopes");
                        ui.checkbox(&mut guides, "Drawing guides");
                    },
                    |ui| {
                        self.solid.filter_control(ui);
                        self.solid
                            .section
                            .control(ui, scene.as_ref().and_then(|s| s.world_bounds()));
                        let transform = ui.add_enabled_ui((self.model.placement_active() || self.model.scaling_active() || (self.selected.is_some() && !self.model.active() && self.acquisition.is_none())) && !self.measurement.active(),|ui|icons::compact_button(
                            ui,
                            Icon::Transform,
                            &format!(
                                "Transform gizmo · {} coordinates · right-click to choose axes",
                                coordinates.label()
                            ),
                        )).inner.on_disabled_hover_text("Select a sketch to transform");
                        if transform.clicked() && !self.model.placement_active() && !self.model.scaling_active() {
                            moving = !moving;
                        }
                        transform.context_menu(|ui| {
                            if self.measurement.active() { ui.disable(); }
                            ui.strong("Transform coordinates");
                            ui.selectable_value(&mut coordinates, Coordinates::World, "World");
                            ui.selectable_value(&mut coordinates, Coordinates::Object, "Object");
                            for d in Datum::ALL {
                                ui.selectable_value(
                                    &mut coordinates,
                                    Coordinates::Custom(d),
                                    format!("Custom {} datum", d.label()),
                                );
                            }
                        });
                    },
                );
                if actions.home {
                    self.home_requested = true;
                    ui.ctx().request_repaint();
                }
                if actions.fit || self.fit_requested {
                    self.fit_requested = false;
                    self.camera.fit(&points, rect);
                    ui.ctx().request_repaint();
                }
            });
        if (self.constraints.dirty() || self.model.active()) && moving {
            moving = false;
            self.message = "Apply or discard constraint edits before transforming".into();
        }
        if self.coordinates != coordinates || self.moving != moving {
            self.cancel_move();
            self.tool = None;
            self.clear_drawing_progress();
        }
        self.coordinates = coordinates;
        self.moving = moving;
        self.grid = grid;
        self.envelopes = envelopes;
        self.guides = guides;
        self.spacing = spacing;
        self.snap = snap;
        if self.geometry_snap != geometry_snap {
            self.snap_assist.clear();
        }
        self.geometry_snap = geometry_snap;
    }
    /// Dashed service-envelope boxes over the model. They follow the
    /// target's evaluated pose, so they stay visible while it is placed.
    fn paint_envelopes(&self, painter: &egui::Painter, scene: &crate::viewport::Scene, rect: Rect) {
        let stroke = Stroke::new(1.2, Color32::from_rgb(240, 170, 60));
        for corners in scene.envelope_boxes().values() {
            for k in 0..4 {
                for (a, b) in [(k, (k + 1) % 4), (k + 4, (k + 1) % 4 + 4), (k, k + 4)] {
                    if let Some(segment) = self.camera.segment(corners[a], corners[b], rect) {
                        painter.extend(egui::Shape::dashed_line(&segment, stroke, 6., 4.));
                    }
                }
            }
        }
    }
    /// Dashed drawing guide frames: plan cut planes, section lines and
    /// elevation directions authored in the model.
    fn paint_guides(&self, painter: &egui::Painter, scene: &crate::viewport::Scene, rect: Rect) {
        let stroke = Stroke::new(1.2, Color32::from_rgb(80, 200, 220));
        for corners in scene.guide_outlines().values().flatten() {
            for k in 0..4 {
                if let Some(segment) = self.camera.segment(corners[k], corners[(k + 1) % 4], rect) {
                    painter.extend(egui::Shape::dashed_line(&segment, stroke, 10., 5.));
                }
            }
        }
    }
    fn gizmo(&mut self, ui: &mut egui::Ui, session: &mut Session, rect: Rect) {
        let Some(id) = self.selected else {
            return;
        };
        let Ok((_, frame, _)) = self.read_sketch(session.document(), id, session.revision()) else {
            return;
        };
        let Ok(basis) = self.basis(session.document(), session.revision()) else {
            return;
        };
        let camera = self.camera;
        let project = |p: [f64; 3]| {
            let p = camera.project(p.map(|v| v as f32), rect);
            Pos2::new(p[0], p[1])
        };
        let start = project(frame.origin);
        let length = (62. / camera.scale) as f64;
        for (axis, color) in [
            Color32::LIGHT_RED,
            Color32::LIGHT_GREEN,
            Color32::LIGHT_BLUE,
        ]
        .into_iter()
        .enumerate()
        {
            let mut along = [0.; 3];
            along[axis] = length;
            let vector = basis.vector(along);
            let end = project(std::array::from_fn(|i| frame.origin[i] + vector[i]));
            let screen = end - start;
            if screen.length() < 8. {
                continue;
            }
            ui.painter()
                .line_segment([start, end], Stroke::new(2.5, color));
            let r = ui.interact(
                Rect::from_center_size(end, Vec2::splat(18.)),
                ui.id().with(("move-axis", axis)),
                Sense::drag(),
            );
            ui.painter()
                .circle_filled(end, if r.hovered() { 6. } else { 4. }, color);
            r.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!("Move along {} axis", ["X", "Y", "Z"][axis]),
                )
            });
            if r.drag_started()
                && let Some(pointer) = r.interact_pointer_pos()
            {
                self.drag = Some(MoveDrag {
                    axis,
                    start: pointer - r.drag_delta(),
                    screen,
                    length,
                });
            }
            if r.dragged()
                && let (Some(drag), Some(pointer)) = (&self.drag, r.interact_pointer_pos())
                && drag.axis == axis
            {
                self.delta = [0.; 3];
                self.delta[axis] = ((pointer - drag.start).dot(drag.screen)
                    / drag.screen.length_sq()) as f64
                    * drag.length;
                if self.snap {
                    self.delta[axis] = (self.delta[axis] / self.spacing).round() * self.spacing;
                }
                if let Err(e) = self.prepare_move(session) {
                    self.preview = None;
                    self.message = e;
                }
            }
            if r.drag_stopped() && self.drag.as_ref().is_some_and(|d| d.axis == axis) {
                if self.preview.is_some()
                    && let Err(e) = self.apply_move(session)
                {
                    self.message = e;
                }
                self.drag = None;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::sketch_regions::{
        AnalyticQueries, CurveHits, Edge, ExactCurve, PointLocation, QueryError, RegionQueries,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };

    #[test]
    fn fit_frames_native_revolve_preview_and_handoff_instead_of_the_selected_source_sketch() {
        use crate::viewport::{
            Geometry as DisplayGeometry, Instance, Scene, SceneBackend, SceneReply, SceneStamp,
            Transform,
        };
        use multicad_core::{model, parameters, references::Origin};
        use multicad_kernel::{
            CurvedEdge2d, CurvedProfile, GeometryPolicy, MeshOptions, PlaneFrame, RevolutionAxis,
            Solid,
        };
        struct RevolveBackend(Option<SceneReply>);
        impl SceneBackend for RevolveBackend {
            fn request(&mut self, document: &Document, stamp: SceneStamp) -> Result<(), String> {
                let feature=document.entities().find(|entity|matches!(&entity.data,EntityData::Feature{definition,..} if definition.type_id==model::REVOLVE_TYPE)).unwrap();
                let definition = model::read_revolve(document, feature.id).unwrap();
                let (source, frame, _) =
                    Source::read(document, definition.profile.sketch_id().unwrap()).unwrap();
                let geometry = source.display_geometry().unwrap();
                let edges = geometry
                    .iter()
                    .map(|geometry| match geometry {
                        Geometry::Line { a, b, .. } => CurvedEdge2d::Line { a: *a, b: *b },
                        _ => panic!("Straight profile fixture"),
                    })
                    .collect();
                let origins = geometry
                    .iter()
                    .map(|geometry| Origin {
                        source: geometry.id(),
                        role: "curve".into(),
                    })
                    .collect::<Vec<_>>();
                let resolved = definition
                    .parameters
                    .resolve(&parameters::evaluate(document).unwrap())
                    .unwrap();
                let solid = Solid::revolve_curved_profile_named(
                    &CurvedProfile {
                        frame: PlaneFrame {
                            origin: frame.origin,
                            u: frame.u,
                            v: frame.v,
                        },
                        outer: edges,
                        holes: vec![],
                    },
                    RevolutionAxis {
                        origin_mm: frame.point(resolved.axis_origin_mm),
                        direction: std::array::from_fn(|axis| {
                            frame.u[axis] * resolved.axis_direction[0]
                                + frame.v[axis] * resolved.axis_direction[1]
                        }),
                    },
                    resolved.angle_rad,
                    feature.id,
                    &origins,
                    GeometryPolicy::default(),
                )
                .unwrap();
                let mesh = solid.mesh(MeshOptions::default()).unwrap();
                let display = std::sync::Arc::new(
                    DisplayGeometry::new(
                        feature.id,
                        mesh.vertices.as_chunks::<3>().0.to_vec(),
                        mesh.indices,
                        mesh.face_indices,
                        solid
                            .topology_snapshot(feature.id, stamp.revision)
                            .unwrap()
                            .elements,
                    )
                    .unwrap(),
                );
                self.0 = Some(SceneReply {
                    stamp,
                    result: Ok(std::sync::Arc::new(
                        Scene::new(
                            stamp,
                            vec![display],
                            vec![Instance {
                                id: definition.output.body,
                                geometry: feature.id,
                                transform: Transform::default(),
                                color: [137, 174, 199, 255],
                            }],
                        )
                        .unwrap(),
                    )),
                });
                Ok(())
            }
            fn poll(&mut self) -> Option<SceneReply> {
                self.0.take()
            }
        }
        let (sketch, edits) = Source::create(Datum::Xz, "Twenty millimeter lathe profile").unwrap();
        let mut document = Document::new("Preview fit")
            .unwrap()
            .edited(&edits)
            .unwrap();
        let (mut source, _, plane) = Source::read(&document, sketch).unwrap();
        let mut datum = document.get(plane).unwrap().clone();
        let EntityData::Plane { origin_mm, .. } = &mut datum.data else {
            panic!()
        };
        *origin_mm = [0., -100., 0.];
        document.replace(datum).unwrap();
        source.add_rectangle([10., 0.], [30., 20.]).unwrap();
        document = document
            .edited(&source.replace(&document, sketch).unwrap())
            .unwrap();
        let mut session = Session::new(document).unwrap();
        let mut editor = SketchEditor::default();
        editor.open_at(session.document(), sketch, 0).unwrap();
        settle(&mut editor, &session);
        let regions = editor
            .sketch_evaluation
            .current(sketch)
            .unwrap()
            .regions
            .regions
            .iter()
            .map(|region| region.intent())
            .collect();
        editor.finish().unwrap();
        editor.set_model_backend(Box::new(RevolveBackend(None)));
        editor
            .model
            .begin_revolve(&session, &Default::default(), sketch, regions, Channel::Ui)
            .unwrap();
        // A new revolve needs an explicit axis; use the sketch V axis.
        editor.model.choose_revolve_axis([0., 0.], 90.);
        editor
            .preview_model(&session, &Default::default(), Channel::Ui)
            .unwrap();
        editor.poll_model(&session, &Default::default());
        let scene = editor
            .model
            .display_scene(&session, &Default::default())
            .unwrap();
        let [low, high] = scene.world_bounds().unwrap();
        assert!(
            high[0] - low[0] > 58.,
            "Native result must be larger than its 20mm source profile"
        );
        let context = egui::Context::default();
        let check_fit = |editor: &mut SketchEditor,
                         session: &mut Session,
                         scene: std::sync::Arc<Scene>| {
            for perspective in [false, true] {
                editor.home_view();
                editor.camera.perspective = perspective;
                editor.request_fit();
                context
                    .run_ui(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(
                                Pos2::ZERO,
                                Vec2::new(1000., 700.),
                            )),
                            ..Default::default()
                        },
                        |ui| editor.canvas(ui, session, false, Some(scene.clone()), false),
                    )
                    .drop_without_applying_deltas();
                let rect = editor.viewport_rect.unwrap();
                for instance in scene.instances() {
                    for point in scene.geometries()[&instance.geometry].vertices() {
                        let pixel = editor.camera.project(
                            instance.transform.point(*point).map(|value| value as f32),
                            rect,
                        );
                        assert!(
                            pixel[2] > 0.
                                && rect.shrink(8.).contains(Pos2::new(pixel[0], pixel[1])),
                            "Fit clipped a native preview vertex in perspective={perspective}: {pixel:?} in {rect:?}"
                        );
                    }
                }
                let [low, high] = scene.world_bounds().unwrap();
                for axis in 0..3 {
                    assert!(
                        (editor.camera.target[axis] - (low[axis] + high[axis]) as f32 * 0.5).abs()
                            < 1e-3,
                        "Fit stayed centered on the source instead of the displayed solid"
                    );
                }
            }
        };
        assert_eq!(editor.selected, Some(sketch));
        check_fit(&mut editor, &mut session, scene.clone());
        editor
            .apply_model(&mut session, &Default::default())
            .unwrap();
        // A source-tree selection during the render-only handoff must not make
        // Fit ignore the solid while canonical publication catches up.
        editor.selected = Some(sketch);
        let handoff = editor
            .model
            .display_scene(&session, &Default::default())
            .unwrap();
        assert!(std::sync::Arc::ptr_eq(&scene, &handoff));
        check_fit(&mut editor, &mut session, handoff);
    }

    #[test]
    fn tool_first_extrude_accepts_viewport_profiles_and_blocks_legacy_sketch_move() {
        let mut h = ProfileHarness::new();
        h.editor.set_model_backend(Box::new(
            crate::modeling::native_fixture::NativeBackend::new(),
        ));
        let before = h.session.document().clone();
        let revision = h.session.revision();
        let feed = crate::viewport::SceneFeed::default();
        h.editor
            .activate_model_tool(
                &h.session,
                &Default::default(),
                &feed,
                crate::modeling::ModelingTool::Extrude,
                Channel::Ui,
            )
            .unwrap();
        assert!(h.editor.awaiting_model_targets());
        assert!(!h.editor.editing);
        h.editor.toggle_transform();
        assert!(!h.editor.moving);
        let rect = h.editor.viewport_rect.unwrap();
        let frame = h
            .editor
            .read_sketch(h.session.document(), h.editor.selected.unwrap(), revision)
            .unwrap()
            .1;
        for (point, shift) in [([15., 15.], false), ([65., 15.], true)] {
            let p = h
                .editor
                .camera
                .project(frame.point(point).map(|v| v as f32), rect);
            let pos = Pos2::new(p[0], p[1]);
            let modifiers = egui::Modifiers {
                shift,
                ..Default::default()
            };
            for pressed in [true, false] {
                h.ctx
                    .run_ui(
                        input(vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers,
                            },
                        ]),
                        |ui| {
                            let (_, response) =
                                ui.allocate_exact_size(rect.size(), Sense::click_and_drag());
                            h.editor.target_viewport(
                                ui,
                                &response,
                                &h.session,
                                &Default::default(),
                                &feed,
                                rect,
                            );
                        },
                    )
                    .drop_without_applying_deltas();
            }
            h.drain();
        }
        assert_eq!(
            h.editor
                .selected_profiles(h.session.document(), revision)
                .unwrap()
                .1
                .len(),
            2
        );
        h.editor
            .advance_model_targets(&h.session, &Default::default(), &feed, true);
        assert!(h.editor.model_active());
        assert!(!h.editor.awaiting_model_targets());
        h.editor.cancel_model();
        assert_eq!(h.session.document(), &before);
        assert_eq!(h.session.revision(), revision);
    }
    #[test]
    fn extrusion_captures_checked_profiles_and_owns_the_existing_tool_gate() {
        struct UnavailableReplyBackend;
        impl crate::viewport::SceneBackend for UnavailableReplyBackend {
            fn request(
                &mut self,
                _: &Document,
                _: crate::viewport::SceneStamp,
            ) -> Result<(), String> {
                Ok(())
            }
            fn poll(&mut self) -> Option<crate::viewport::SceneReply> {
                None
            }
        }
        let mut harness = ProfileHarness::new();
        let sketch = harness.editor.selected.unwrap();
        let before = harness.session.document().clone();
        let revision = harness.session.revision();
        let presentation = harness.editor.sketch_evaluation.current(sketch).unwrap();
        let profiles = presentation
            .regions
            .regions
            .iter()
            .map(|region| region.intent())
            .collect::<Vec<_>>();
        harness
            .editor
            .profiles
            .reconcile(sketch, &presentation.regions);
        for profile in &profiles {
            harness.editor.profiles.choose(profile.clone(), true);
        }
        assert!(
            harness
                .editor
                .begin_extrude(&harness.session, &Default::default(), Channel::Ui)
                .unwrap_err()
                .contains("unavailable")
        );
        assert!(harness.editor.editing);
        harness
            .editor
            .set_model_backend(Box::new(UnavailableReplyBackend));
        assert!(
            harness
                .editor
                .extrusion_ready(harness.session.document(), revision)
        );
        harness
            .editor
            .begin_extrude(&harness.session, &Default::default(), Channel::Ui)
            .unwrap();
        assert!(harness.editor.model_active());
        assert!(!harness.editor.editing);
        assert!(harness.editor.has_pending_edits());
        assert_eq!(
            harness
                .editor
                .selected_profiles(harness.session.document(), revision)
                .unwrap(),
            (sketch, profiles)
        );
        assert!(harness.editor.finish().is_err());
        assert!(harness.editor.set_tool(DrawTool::Line).is_err());
        harness.editor.toggle_transform();
        assert!(!harness.editor.moving);
        assert!(
            harness
                .editor
                .apply_model(&mut harness.session, &Default::default())
                .is_err()
        );
        harness
            .editor
            .preview_model(&harness.session, &Default::default(), Channel::CommandLine)
            .unwrap();
        assert!(harness.editor.model.pending());
        harness.editor.cancel();
        assert!(!harness.editor.has_pending_edits());
        assert_eq!(harness.session.document(), &before);
        assert_eq!(harness.session.revision(), revision);
    }

    struct ProfileGate {
        block_next: AtomicBool,
        started: mpsc::SyncSender<[f64; 2]>,
        release: Mutex<mpsc::Receiver<()>>,
    }
    struct BlockedProfileQueries(Arc<ProfileGate>);
    impl RegionQueries for BlockedProfileQueries {
        fn intersections(
            &self,
            a: &ExactCurve,
            b: &ExactCurve,
            t: f64,
        ) -> Result<CurveHits, QueryError> {
            AnalyticQueries.intersections(a, b, t)
        }
        fn self_intersections(&self, c: &ExactCurve, t: f64) -> Result<CurveHits, QueryError> {
            AnalyticQueries.self_intersections(c, t)
        }
        fn signed_area(&self, edges: &[Edge], t: f64) -> Result<f64, QueryError> {
            AnalyticQueries.signed_area(edges, t)
        }
        fn classify_boundary(
            &self,
            edges: &[Edge],
            point: [f64; 2],
            t: f64,
        ) -> Result<PointLocation, QueryError> {
            if self.0.block_next.swap(false, Ordering::AcqRel) {
                self.0.started.send(point).unwrap();
                self.0
                    .release
                    .lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(3))
                    .map_err(|e| QueryError {
                        message: e.to_string(),
                    })?;
            }
            AnalyticQueries.classify_boundary(edges, point, t)
        }
    }
    struct ProfileHarness {
        session: Session,
        editor: SketchEditor,
        ctx: egui::Context,
        gate: Arc<ProfileGate>,
        started: mpsc::Receiver<[f64; 2]>,
        release: mpsc::SyncSender<()>,
    }
    impl ProfileHarness {
        fn new() -> Self {
            let (started_tx, started) = mpsc::sync_channel(1);
            let (release, release_rx) = mpsc::sync_channel(1);
            let gate = Arc::new(ProfileGate {
                block_next: AtomicBool::new(false),
                started: started_tx,
                release: Mutex::new(release_rx),
            });
            let mut session = session();
            let mut editor = SketchEditor::default();
            let service = gate.clone();
            editor.set_query_factory(Arc::new(move || {
                Ok(Box::new(BlockedProfileQueries(service.clone())))
            }));
            editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
            for (a, b) in [([0., 0.], [30., 30.]), ([50., 0.], [80., 30.])] {
                editor
                    .add(&mut session, DrawTool::Rectangle, a, b, Channel::Ui)
                    .unwrap();
            }
            editor.tool = None;
            settle(&mut editor, &session);
            let ctx = egui::Context::default();
            ctx.all_styles_mut(|style| style.animation_time = 0.);
            ctx.run_ui(input(vec![]), |ui| {
                editor.canvas(ui, &mut session, true, None, false)
            })
            .drop_without_applying_deltas();
            Self {
                session,
                editor,
                ctx,
                gate,
                started,
                release,
            }
        }
        fn click(&mut self, point: [f64; 2], modifiers: egui::Modifiers) {
            self.ctx
                .run_ui(input(vec![]), |ui| {
                    self.editor.canvas(ui, &mut self.session, true, None, false)
                })
                .drop_without_applying_deltas();
            let id = self.editor.selected.unwrap();
            let frame = self
                .editor
                .read_sketch(self.session.document(), id, self.session.revision())
                .unwrap()
                .1;
            let p = self.editor.camera.project(
                frame.point(point).map(|v| v as f32),
                self.editor.viewport_rect.unwrap(),
            );
            let pos = Pos2::new(p[0], p[1]);
            for pressed in [true, false] {
                let mut events = vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    },
                ];
                if !pressed {
                    events.push(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
                }
                // Intentionally do not settle or poll: this models clicks that
                // arrive while an exact classification is still in flight.
                self.ctx
                    .run_ui(input(events), |ui| {
                        self.editor.canvas(ui, &mut self.session, true, None, false)
                    })
                    .drop_without_applying_deltas();
            }
        }
        fn block_click(&mut self, point: [f64; 2], modifiers: egui::Modifiers) {
            self.gate.block_next.store(true, Ordering::Release);
            self.click(point, modifiers);
            let observed = self
                .started
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap();
            assert!((observed[0] - point[0]).abs() < 1e-5 && (observed[1] - point[1]).abs() < 1e-5);
        }
        fn drain(&mut self) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            loop {
                self.editor.sketch_evaluation.poll();
                self.editor
                    .poll_profile_pick(self.session.document(), self.session.revision());
                if self.editor.pending_profile_pick.is_none()
                    && self.editor.queued_profile_clicks.is_empty()
                    && !self.editor.sketch_evaluation.pending()
                {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "Profile click sequence did not drain"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        fn selected(&self) -> Vec<multicad_core::sketch_regions::RegionIntent> {
            self.editor
                .selected_profiles(self.session.document(), self.session.revision())
                .unwrap()
                .1
        }
        fn intent(&self, point: [f64; 2]) -> multicad_core::sketch_regions::RegionIntent {
            self.editor
                .sketch_evaluation
                .current(self.editor.selected.unwrap())
                .unwrap()
                .regions
                .regions
                .iter()
                .find(|r| r.contains(point).unwrap())
                .unwrap()
                .intent()
        }
        fn inspector_frame(
            &mut self,
            feed: &mut crate::viewport::SceneFeed,
            events: Vec<egui::Event>,
        ) -> egui::FullOutput {
            let mut output = self.ctx.run_ui(input(events), |ui| {
                self.editor.show(ui, &mut self.session, true, feed);
            });
            output.textures_delta.clear();
            output
        }
        fn inspector_click(&mut self, feed: &mut crate::viewport::SceneFeed, label: &str) {
            let output = self.inspector_frame(feed, vec![]);
            let pos = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("Missing Inspector label {label}"));
            for pressed in [true, false] {
                self.inspector_frame(
                    feed,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        }
    }
    fn settle(editor: &mut SketchEditor, session: &Session) {
        if let Some(report) = editor
            .evaluated
            .clone()
            .filter(|r| r.revision == session.revision())
        {
            editor.sketch_evaluation.sync_creation_report(
                session.document(),
                session.revision(),
                report,
            );
        } else {
            editor
                .sketch_evaluation
                .sync(session.document(), session.revision(), None);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while editor.sketch_evaluation.pending() {
            editor.sketch_evaluation.poll();
            assert!(
                std::time::Instant::now() < deadline,
                "Sketch worker timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        editor.poll_profile_pick(session.document(), session.revision());
    }
    fn session() -> Session {
        Session::new(Document::new("Sketch tests").unwrap()).unwrap()
    }
    #[test]
    fn bound_sketch_display_tracks_parameters_and_rejects_retained_geometry_for_edits() {
        use multicad_core::{
            constraint_sketch::{ConstraintKind, Dimension, binding::DimensionInput},
            document::Entity,
        };
        let mut session = session();
        let mut editor = SketchEditor::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        let id = editor.selected.unwrap();
        editor
            .draw(
                &mut session,
                &multicad_core::sketch_draw::DrawRequest {
                    input: multicad_core::sketch_draw::DrawInput::Circle {
                        center: multicad_core::sketch_draw::PointInput::Position([10., 10.]),
                        radius_mm: 3.,
                    },
                    construction: false,
                },
                Channel::Ui,
            )
            .unwrap();
        let radius = Entity::new(
            "radius",
            EntityData::Parameter {
                expression: "8".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        let parameter = radius.id;
        commit(
            &mut session,
            "Radius parameter",
            vec![Mutation::Insert(radius)],
            Channel::Ui,
        )
        .unwrap();
        let mut source = editor
            .read_source(session.document(), id, session.revision())
            .unwrap()
            .0;
        let constraint = source.add_constraint(ConstraintKind::Radius {
            curve: source.sketch.curves[0].id,
            dimension: Dimension::Literal {
                value: 3.,
                unit: "mm".into(),
            },
        });
        source = multicad_core::constraint_sketch::binding::draft_dimension(
            session.document(),
            &source,
            constraint,
            &DimensionInput::Expression {
                text: "=$radius".into(),
                unit: "mm".into(),
            },
            false,
        )
        .unwrap();
        let prepared = multicad_solver::prepare_edit(
            &session,
            session.revision(),
            Actor::local(Channel::Ui),
            id,
            source,
        )
        .unwrap()
        .preview
        .unwrap();
        session.commit(prepared).unwrap();
        let canonical = session.document().get(id).unwrap().clone();
        let refresh = |editor: &mut SketchEditor, session: &Session| {
            editor
                .sketch_evaluation
                .sync(session.document(), session.revision(), None);
            let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while editor.sketch_evaluation.pending() {
                editor.sketch_evaluation.poll();
                assert!(std::time::Instant::now() < until, "Sketch worker timed out");
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        };
        refresh(&mut editor, &session);
        let sketch = editor
            .read_sketch(session.document(), id, session.revision())
            .unwrap()
            .0;
        assert!(
            matches!(sketch.geometry[0],Geometry::Circle{radius_mm,..} if (radius_mm-8.).abs()<1e-8)
        );
        editor.delta = [5., 6., 7.];
        editor.prepare_move(&session).unwrap();
        let (_, frame, _) = editor
            .read_sketch(
                editor.preview.as_ref().unwrap().document(),
                id,
                session.revision(),
            )
            .unwrap();
        assert_eq!(
            frame.origin,
            [5., 6., 7.],
            "Move preview retained the canonical plane pose"
        );
        assert_eq!(session.document().get(id).unwrap(), &canonical);
        editor.cancel_move();
        let mut changed = session.document().get(parameter).unwrap().clone();
        let EntityData::Parameter { expression, .. } = &mut changed.data else {
            panic!("parameter")
        };
        *expression = "-5".into();
        commit(
            &mut session,
            "Invalid radius",
            vec![Mutation::Replace(changed)],
            Channel::Ui,
        )
        .unwrap();
        assert!(
            editor
                .read_sketch(session.document(), id, session.revision())
                .is_err(),
            "Old revision remained selectable before worker dispatch"
        );
        refresh(&mut editor, &session);
        assert!(editor.sketch_evaluation.current(id).is_none());
        assert!(editor.sketch_evaluation.last_valid(id).is_some());
        assert!(
            editor
                .read_sketch(session.document(), id, session.revision())
                .is_err()
        );
        assert!(
            editor
                .read_solved_source(session.document(), id, session.revision())
                .is_err()
        );
        editor
            .open_at(session.document(), id, session.revision())
            .unwrap();
        assert!(
            editor
                .read_source(session.document(), id, session.revision())
                .is_ok(),
            "Failed solve prevented source repair"
        );
        assert_eq!(session.document().get(id).unwrap(), &canonical);
    }
    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000., 700.))),
            events,
            ..Default::default()
        }
    }
    #[test]
    fn face_selection_cancel_commit_and_reentry_keep_local_source_and_placed_frame() {
        use crate::{sketch_plane::*, viewport::*};
        use multicad_core::{document::*, references::*};
        use multicad_kernel::{MeshOptions, Solid};
        use std::{collections::BTreeMap, sync::Arc};
        let producer = Entity::new(
            "Panel source",
            EntityData::Feature {
                definition: Definition {
                    type_id: "test.box".into(),
                    schema_version: 1,
                    data: serde_json::json!({}),
                },
                inputs: vec![],
                suppressed: false,
            },
        );
        let part = Entity::new(
            "Panel",
            EntityData::Part {
                features: vec![producer.id],
                material: None,
                tags: Default::default(),
            },
        );
        let placement = Placement {
            translation_mm: [140., 25., 40.],
            rotation_xyzw: [
                0.,
                0.,
                std::f64::consts::FRAC_1_SQRT_2,
                std::f64::consts::FRAC_1_SQRT_2,
            ],
        };
        let instance = Entity::new(
            "Placed panel",
            EntityData::Instance {
                component: part.id,
                placement,
                placement_source: None,
            },
        );
        let mut document = Document::new("Plane entry integration").unwrap();
        for entity in [producer.clone(), part, instance.clone()] {
            document.insert(entity).unwrap();
        }
        let mut session = Session::new(document).unwrap();
        let before = session.document().clone();
        let native = Solid::box_named([100., 60., 18.], producer.id, Default::default()).unwrap();
        let mesh = native.mesh(MeshOptions::default()).unwrap();
        let geometry = Arc::new(
            crate::viewport::Geometry::new(
                producer.id,
                mesh.vertices.as_chunks::<3>().0.to_vec(),
                mesh.indices,
                mesh.face_indices,
                native.topology_snapshot(producer.id, 0).unwrap().elements,
            )
            .unwrap(),
        );
        let pose = Transform::from_placement(placement).unwrap();
        let scene = |revision, generation| {
            Arc::new(
                Scene::new(
                    SceneStamp {
                        document: before.id(),
                        revision,
                        generation,
                    },
                    vec![geometry.clone()],
                    vec![Instance {
                        id: instance.id,
                        geometry: producer.id,
                        transform: pose,
                        color: [140, 165, 190, 255],
                    }],
                )
                .unwrap(),
            )
        };
        // Selection-first entry must consume the existing face, without a second
        // plane picker click or an Inspector confirmation. Create engraving
        // takes the same face path but never falls back to plane picking.
        for engraving in [false, true] {
            let mut direct_session = Session::new(before.clone()).unwrap();
            let mut direct = SketchEditor {
                display_scene: Some(scene(0, 1)),
                ..Default::default()
            };
            let enter = |editor: &mut SketchEditor, session: &mut Session| {
                if engraving {
                    editor.begin_engraving(session, Channel::Ui)
                } else {
                    editor.begin_from_selection(session, Channel::Ui)
                }
            };
            if engraving {
                let error = enter(&mut direct, &mut direct_session).unwrap_err();
                assert!(error.contains("Select a planar face"), "{error}");
                assert!(!direct.picking && !direct.editing);
                assert_eq!(direct_session.document(), &before);
                assert_eq!(direct_session.revision(), 0);
            }
            direct.camera.target = [110., 75., 58.];
            direct.camera.pitch = std::f32::consts::FRAC_PI_2;
            direct.camera.yaw = 0.;
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000., 700.));
            let point = direct
                .camera
                .project(pose.point([30., 20., 18.]).map(|v| v as f32), rect);
            let point = Pos2::new(point[0], point[1]);
            let ctx = egui::Context::default();
            for events in [
                vec![],
                vec![],
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            ] {
                ctx.run_ui(input(events), |ui| {
                    direct.canvas(ui, &mut direct_session, false, Some(scene(0, 1)), true)
                })
                .drop_without_applying_deltas();
            }
            assert!(direct.solid.selected_hit().is_some(), "real face selection");
            let mut stale = Session::new(before.clone()).unwrap();
            stale
                .apply(Request {
                    expected_revision: 0,
                    actor: Actor::local(Channel::Ui),
                    label: "Rename".into(),
                    edits: vec![Mutation::RenameDocument("Changed".into())],
                })
                .unwrap();
            let stale_source = stale.document().clone();
            assert!(
                enter(&mut direct, &mut stale)
                    .unwrap_err()
                    .contains("stale")
            );
            assert_eq!(stale.document(), &stale_source);
            enter(&mut direct, &mut direct_session).unwrap();
            let expected = if engraving { "Engraving 1" } else { "Sketch 1" };
            assert!(
                direct_session
                    .document()
                    .entities()
                    .any(|e| matches!(e.data, EntityData::Sketch { .. }) && e.name == expected),
                "{expected}"
            );
            if engraving {
                assert!(enter(&mut direct, &mut direct_session).is_err());
                assert_eq!(direct_session.revision(), 1);
            }
            assert!(direct.editing && !direct.picking);
            assert_eq!(direct_session.revision(), 1);
            assert_eq!(direct.occurrence, Some(instance.id));
            assert_eq!(
                direct_session
                    .document()
                    .entities()
                    .filter(|e| matches!(e.data, EntityData::Sketch { .. }))
                    .count(),
                1
            );
        }
        let mut editor = SketchEditor {
            display_scene: Some(scene(0, 1)),
            ..Default::default()
        };
        editor.begin();
        assert!(editor.picking && !editor.editing && editor.tool.is_none());
        editor.cancel();
        assert_eq!(session.document(), &before);
        assert_eq!(session.revision(), 0);
        editor.begin();
        editor.camera.target = [110., 75., 58.];
        editor.camera.pitch = std::f32::consts::FRAC_PI_2;
        editor.camera.yaw = 0.;
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000., 700.));
        let point = editor
            .camera
            .project(pose.point([30., 20., 18.]).map(|v| v as f32), rect);
        let point = Pos2::new(point[0], point[1]);
        let ctx = egui::Context::default();
        for events in [
            vec![],
            vec![],
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            ctx.run_ui(input(events), |ui| {
                editor.canvas(ui, &mut session, true, Some(scene(0, 1)), true)
            })
            .drop_without_applying_deltas();
        }
        assert!(editor.picking && !editor.editing && editor.tool.is_none());
        assert_eq!(
            session.document(),
            &before,
            "A face click only selects a source-neutral preview"
        );
        let get_preview = |editor: &SketchEditor| {
            editor
                .plane_picker
                .current_preview(PlaneContext {
                    document: &before,
                    revision: 0,
                    scene: editor.display_scene.as_deref(),
                    report: None,
                    occurrence: None,
                    section: None,
                })
                .unwrap()
                .unwrap()
        };
        let selected = get_preview(&editor);
        assert!(matches!(selected.choice, PlaneChoice::Face(_)));
        editor.display_scene = Some(scene(0, 2));
        assert!(
            editor
                .create_on_plane(&mut session, selected.clone(), Channel::Ui)
                .is_err()
        );
        assert_eq!(session.document(), &before);
        editor.display_scene = Some(scene(0, 1));
        editor
            .plane_picker
            .set_options(sketch::PlaneOptions {
                normal_offset_mm: 3.,
                angle_rad: std::f64::consts::FRAC_PI_2,
            })
            .unwrap();
        let selected = get_preview(&editor);
        editor
            .create_on_plane(&mut session, selected, Channel::Ui)
            .unwrap();
        assert_eq!(session.revision(), 1);
        assert!(editor.editing && !editor.picking);
        let id = editor.selected.unwrap();
        // Empty-sketch creation leaves producer geometry unchanged; its trusted
        // creation-only report permits drawing without waiting for a worker.
        assert_eq!(editor.evaluated.as_ref().unwrap().revision, 1);
        let (_, local, plane) = editor.read_source(session.document(), id, 1).unwrap();
        settle(&mut editor, &session);
        let (_, world, _) = editor.read_sketch(session.document(), id, 1).unwrap();
        for (a, b) in world.origin.into_iter().zip(pose.point(local.origin)) {
            assert!((a - b).abs() < 1e-8);
        }
        assert_eq!(local.origin, [0., 0., 21.]);
        let EntityData::Plane {
            anchor: Some(anchor),
            ..
        } = &session.document().get(plane).unwrap().data
        else {
            panic!("face attachment missing")
        };
        assert_eq!(anchor.target, producer.id);
        editor
            .add(
                &mut session,
                DrawTool::Rectangle,
                [10., 15.],
                [30., 25.],
                Channel::Ui,
            )
            .unwrap();
        let report = inspect(
            session.document(),
            2,
            &BTreeMap::from([(
                producer.id,
                native.topology_snapshot(producer.id, 2).unwrap(),
            )]),
        );
        let (source, _, _) = Source::read_evaluated(session.document(), id, 2, &report).unwrap();
        assert_eq!(source.sketch.points[0].position_mm, [10., 15.]);
        editor.finish().unwrap();
        assert!(!editor.editing);
        editor.evaluated = Some(report);
        editor.display_scene = Some(scene(2, 4));
        editor.open_at(session.document(), id, 2).unwrap();
        settle(&mut editor, &session);
        assert_eq!(
            editor.read_sketch(session.document(), id, 2).unwrap().1,
            world
        );
        session.undo(2, Actor::local(Channel::Ui)).unwrap();
        session.undo(3, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(
            session.document(),
            &before,
            "Creation and drawing each have one undo step"
        );
    }
    #[test]
    fn moving_a_placed_datum_sketch_in_world_axes_writes_the_inverse_local_delta() {
        use multicad_core::document::Placement;
        let mut session = session();
        let mut editor = SketchEditor::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        let id = editor.selected.unwrap();
        let pose = crate::viewport::Transform::from_placement(Placement {
            translation_mm: [140., 25., 40.],
            rotation_xyzw: [
                0.,
                0.,
                std::f64::consts::FRAC_1_SQRT_2,
                std::f64::consts::FRAC_1_SQRT_2,
            ],
        })
        .unwrap();
        editor.contexts.insert(id, pose);
        editor.coordinates = Coordinates::World;
        editor.delta = [10., 20., 3.];
        let before = session.document().clone();
        editor.prepare_move(&session).unwrap();
        assert_eq!(session.document(), &before);
        editor.apply_move(&mut session).unwrap();
        let (_, local, _) = Source::read(session.document(), id).unwrap();
        settle(&mut editor, &session);
        let (_, world, _) = editor
            .read_sketch(session.document(), id, session.revision())
            .unwrap();
        for (actual, expected) in local.origin.into_iter().zip([20., -10., 3.]) {
            assert!((actual - expected).abs() < 1e-8);
        }
        for (actual, expected) in world.origin.into_iter().zip([150., 45., 43.]) {
            assert!((actual - expected).abs() < 1e-8);
        }
        let basis = editor
            .basis(session.document(), session.revision())
            .unwrap();
        assert_eq!(basis, Datum::Xy.frame());
        editor.coordinates = Coordinates::Object;
        let basis = editor
            .basis(session.document(), session.revision())
            .unwrap();
        for (actual, expected) in basis.u.into_iter().zip(pose.vector(local.u)) {
            assert!((actual - expected).abs() < 1e-8);
        }
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(session.document(), &before);
    }
    fn gesture_frame(
        ctx: &egui::Context,
        editor: &mut SketchEditor,
        session: &mut Session,
        events: Vec<egui::Event>,
    ) {
        settle(editor, session);
        ctx.run_ui(input(events), |ui| {
            editor.canvas(ui, session, true, None, false)
        })
        .drop_without_applying_deltas();
    }
    fn click_sketch(
        ctx: &egui::Context,
        editor: &mut SketchEditor,
        session: &mut Session,
        point: [f64; 2],
    ) {
        click_sketch_modifiers(ctx, editor, session, point, egui::Modifiers::NONE);
    }
    fn click_sketch_modifiers(
        ctx: &egui::Context,
        editor: &mut SketchEditor,
        session: &mut Session,
        point: [f64; 2],
        modifiers: egui::Modifiers,
    ) {
        settle(editor, session);
        let (_, frame, _) = editor
            .read_sketch(
                session.document(),
                editor.selected.unwrap(),
                session.revision(),
            )
            .unwrap();
        let screen = editor.camera.project(
            frame.point(point).map(|v| v as f32),
            editor.viewport_rect.unwrap(),
        );
        let screen = Pos2::new(screen[0], screen[1]);
        for pressed in [true, false] {
            let mut events = vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(screen),
                egui::Event::PointerButton {
                    pos: screen,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                },
            ];
            if !pressed {
                // Native input can deliver modifier key-up after mouse-up in
                // the same frame; selection belongs to the click event.
                events.push(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
            }
            let raw = input(events);
            settle(editor, session);
            ctx.run_ui(raw, |ui| editor.canvas(ui, session, true, None, false))
                .drop_without_applying_deltas();
        }
        settle(editor, session);
    }
    #[test]
    fn rapid_profile_clicks_apply_in_order_and_repeated_toggles_are_not_coalesced() {
        let mut h = ProfileHarness::new();
        let before = h.session.document().clone();
        let a = h.intent([15., 15.]);
        let b = h.intent([65., 15.]);
        h.block_click([15., 15.], egui::Modifiers::NONE);
        h.click([65., 15.], egui::Modifiers::SHIFT);
        assert_eq!(h.editor.queued_profile_clicks.len(), 1);
        assert!(matches!(
            h.started.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert!(
            h.editor
                .selected_profiles(h.session.document(), h.session.revision())
                .is_err()
        );
        h.release.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while h.editor.sketch_evaluation.profile_pick().is_none() {
            h.editor.sketch_evaluation.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!h.editor.sketch_evaluation.pending());
        // A has finished, but the UI has not applied its reply or submitted B.
        // Dispatching B must schedule another repaint even from this idle feed.
        h.gate.block_next.store(true, Ordering::Release);
        let output = h.inspector_frame(&mut crate::viewport::SceneFeed::default(), vec![]);
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
                <= std::time::Duration::from_millis(16)
        );
        let b_point = h
            .started
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        assert!((b_point[0] - 65.).abs() < 1e-5);
        assert!(
            h.editor
                .selected_profiles(h.session.document(), h.session.revision())
                .is_err()
        );
        h.release.send(()).unwrap();
        h.drain();
        assert_eq!(h.selected(), vec![a.clone(), b.clone()]);
        h.block_click([15., 15.], egui::Modifiers::SHIFT);
        h.click([15., 15.], egui::Modifiers::SHIFT);
        assert_eq!(h.editor.queued_profile_clicks.len(), 1);
        h.release.send(()).unwrap();
        h.drain();
        assert_eq!(h.selected(), vec![b, a]);
        assert_eq!(h.session.document(), &before);
    }
    #[test]
    fn plain_profile_replacement_cancels_an_unfinished_additive_sequence() {
        let mut h = ProfileHarness::new();
        let b = h.intent([65., 15.]);
        h.block_click([15., 15.], egui::Modifiers::NONE);
        h.click([65., 15.], egui::Modifiers::SHIFT);
        let obsolete = h.editor.pending_profile_pick.unwrap().0;
        h.click([65., 15.], egui::Modifiers::NONE);
        assert!(h.editor.queued_profile_clicks.is_empty());
        assert_ne!(h.editor.pending_profile_pick.unwrap().0, obsolete);
        assert!(
            h.editor
                .selected_profiles(h.session.document(), h.session.revision())
                .is_err()
        );
        h.release.send(()).unwrap();
        h.drain();
        assert_eq!(h.selected(), vec![b]);
    }
    #[test]
    fn profile_click_queue_is_bounded_and_rejected_click_has_visible_feedback() {
        let mut h = ProfileHarness::new();
        let a = h.intent([15., 15.]);
        h.block_click([15., 15.], egui::Modifiers::NONE);
        for _ in 1..MAX_PROFILE_CLICKS {
            h.click([15., 15.], egui::Modifiers::SHIFT);
        }
        assert_eq!(h.editor.queued_profile_clicks.len(), MAX_PROFILE_CLICKS - 1);
        h.click([65., 15.], egui::Modifiers::SHIFT);
        assert_eq!(h.editor.queued_profile_clicks.len(), MAX_PROFILE_CLICKS - 1);
        assert!(h.editor.message.contains("wait, then click again"));
        let mut output = h.ctx.run_ui(input(vec![]), |ui| {
            h.editor.canvas(ui, &mut h.session, true, None, false)
        });
        output.textures_delta.clear();
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(text) if text.galley.text().contains("wait, then click again"))));
        assert!(
            h.editor
                .selected_profiles(h.session.document(), h.session.revision())
                .is_err()
        );
        h.release.send(()).unwrap();
        h.drain();
        // One replacement plus31 toggles leaves no selection. The rejected B
        // click never silently replaces or joins the accepted sequence.
        assert!(h.editor.profiles.selected().is_empty());
        assert!(!h.editor.profiles.selected().contains(&a));
    }
    #[test]
    fn source_tool_and_geometry_resets_cancel_profile_sequences_without_reviving_old_selection() {
        for reset in 0..3 {
            let mut h = ProfileHarness::new();
            h.click([15., 15.], egui::Modifiers::NONE);
            h.drain();
            assert_eq!(h.selected().len(), 1);
            h.block_click([65., 15.], egui::Modifiers::NONE);
            h.click([15., 15.], egui::Modifiers::SHIFT);
            match reset {
                0 => {
                    commit(
                        &mut h.session,
                        "Unrelated source edit",
                        vec![Mutation::Insert(multicad_core::document::Entity::new(
                            "Group",
                            EntityData::Group {},
                        ))],
                        Channel::Ui,
                    )
                    .unwrap();
                    h.editor
                        .poll_profile_pick(h.session.document(), h.session.revision());
                }
                1 => h.editor.set_tool(DrawTool::Line).unwrap(),
                _ => h.click([0., 0.], egui::Modifiers::NONE),
            }
            assert!(h.editor.pending_profile_pick.is_none());
            assert!(h.editor.queued_profile_clicks.is_empty());
            assert!(
                h.editor.profiles.selected().is_empty(),
                "Reset {reset} revived the pre-click profile"
            );
            assert!(
                h.editor
                    .selected_profiles(h.session.document(), h.session.revision())
                    .is_err()
            );
            h.release.send(()).unwrap();
            h.drain();
            assert!(h.editor.profiles.selected().is_empty());
            if reset == 2 {
                assert_eq!(h.editor.constraints.selection().len(), 1);
            }
        }
    }
    #[test]
    fn inspector_profile_selection_overrides_all_pending_canvas_clicks() {
        let mut h = ProfileHarness::new();
        let b = h.intent([65., 15.]);
        let presentation = h
            .editor
            .sketch_evaluation
            .current(h.editor.selected.unwrap())
            .unwrap();
        let (index, region) = presentation
            .regions
            .regions
            .iter()
            .enumerate()
            .find(|(_, r)| r.intent() == b)
            .unwrap();
        let label = format!(
            "Area {} · {:.2} mm² · {} holes",
            index + 1,
            region.area_mm2,
            region.holes.len()
        );
        h.block_click([15., 15.], egui::Modifiers::NONE);
        h.click([65., 15.], egui::Modifiers::SHIFT);
        let mut feed = crate::viewport::SceneFeed::default();
        h.inspector_click(&mut feed, "Profiles · 2 closed areas");
        h.inspector_click(&mut feed, &label);
        assert!(h.editor.pending_profile_pick.is_none());
        assert!(h.editor.queued_profile_clicks.is_empty());
        assert_eq!(h.selected(), vec![b.clone()]);
        h.release.send(()).unwrap();
        h.drain();
        assert_eq!(h.selected(), vec![b]);
    }
    #[test]
    fn changed_reference_publication_at_same_revision_cannot_replay_queued_clicks() {
        let mut h = ProfileHarness::new();
        let original = h.session.document().clone();
        h.block_click([15., 15.], egui::Modifiers::NONE);
        h.click([65., 15.], egui::Modifiers::SHIFT);
        let old = h.editor.pending_profile_pick.unwrap().0.evaluation;
        let report = multicad_core::references::inspect(
            h.session.document(),
            h.session.revision(),
            &Default::default(),
        );
        h.editor.sketch_evaluation.sync_creation_report(
            h.session.document(),
            h.session.revision(),
            report,
        );
        h.release.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while h.editor.sketch_evaluation.pending() {
            h.editor.sketch_evaluation.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let current = h
            .editor
            .sketch_evaluation
            .current(h.editor.selected.unwrap())
            .unwrap()
            .stamp;
        assert_eq!(current.document, old.document);
        assert_eq!(current.revision, old.revision);
        assert_ne!(current, old);
        h.editor
            .poll_profile_pick(h.session.document(), h.session.revision());
        assert!(h.editor.pending_profile_pick.is_none());
        assert!(h.editor.queued_profile_clicks.is_empty());
        assert!(h.editor.profiles.selected().is_empty());
        assert!(
            h.editor
                .selected_profiles(h.session.document(), h.session.revision())
                .is_err()
        );
        assert_eq!(h.session.document(), &original);
    }
    #[test]
    fn pending_profile_click_cannot_supply_the_previous_manufacturing_selection() {
        let mut session = session();
        let mut editor = SketchEditor::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        for (a, b) in [([0., 0.], [30., 30.]), ([50., 0.], [80., 30.])] {
            editor
                .add(&mut session, DrawTool::Rectangle, a, b, Channel::Ui)
                .unwrap();
        }
        editor.tool = None;
        settle(&mut editor, &session);
        let id = editor.selected.unwrap();
        let current = editor.sketch_evaluation.current(id).unwrap();
        editor.profiles.reconcile(id, &current.regions);
        let previous = current
            .regions
            .regions
            .iter()
            .find(|region| region.contains([15., 15.]).unwrap())
            .unwrap()
            .intent();
        editor.profiles.choose(previous.clone(), false);
        assert!(
            editor
                .selected_profiles(session.document(), session.revision())
                .is_ok()
        );
        let before = session.document().clone();
        let stamp = editor
            .sketch_evaluation
            .request_profile_pick(id, [65., 15.])
            .unwrap();
        editor.pending_profile_pick = Some((stamp, false));
        assert_eq!(editor.profiles.selected(), std::slice::from_ref(&previous));
        assert_eq!(
            editor.selected_profiles(session.document(), session.revision()),
            Err("Wait for the current profile selection query".into())
        );
        settle(&mut editor, &session);
        let (_, chosen) = editor
            .selected_profiles(session.document(), session.revision())
            .unwrap();
        assert_eq!(chosen.len(), 1);
        assert_ne!(chosen[0], previous);
        let mut altered = before.clone();
        let mut entity = altered.get(id).unwrap().clone();
        entity.name = "Changed source without a new revision".into();
        altered.replace(entity).unwrap();
        assert_eq!(altered.id(), session.document().id());
        assert!(
            editor
                .selected_profiles(&altered, session.revision())
                .is_err(),
            "A same-ID/revision document clone bypassed canonical source validation"
        );
        assert_eq!(session.document(), &before);
    }
    #[test]
    fn pointer_selection_is_additive_and_profile_intents_never_silently_acquire_new_holes() {
        let mut session = session();
        let mut editor = SketchEditor::default();
        let ctx = egui::Context::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        editor
            .add(
                &mut session,
                DrawTool::Rectangle,
                [0., 0.],
                [80., 60.],
                Channel::Ui,
            )
            .unwrap();
        editor
            .draw(
                &mut session,
                &multicad_core::sketch_draw::DrawRequest {
                    input: multicad_core::sketch_draw::DrawInput::Circle {
                        center: multicad_core::sketch_draw::PointInput::Position([20., 20.]),
                        radius_mm: 6.,
                    },
                    construction: false,
                },
                Channel::Ui,
            )
            .unwrap();
        editor.tool = None;
        gesture_frame(&ctx, &mut editor, &mut session, vec![]);
        click_sketch(&ctx, &mut editor, &mut session, [60., 40.]);
        assert_eq!(
            editor
                .selected_profiles(session.document(), session.revision())
                .unwrap()
                .1
                .len(),
            1
        );
        click_sketch_modifiers(
            &ctx,
            &mut editor,
            &mut session,
            [23., 20.],
            egui::Modifiers::SHIFT,
        );
        assert_eq!(
            editor
                .selected_profiles(session.document(), session.revision())
                .unwrap()
                .1
                .len(),
            2
        );
        let before = session.document().clone();
        click_sketch(&ctx, &mut editor, &mut session, [40., 0.]);
        click_sketch_modifiers(
            &ctx,
            &mut editor,
            &mut session,
            [40., 60.],
            egui::Modifiers::SHIFT,
        );
        assert_eq!(editor.constraints.selection().len(), 2);
        assert!(editor.profiles.selected().is_empty());
        click_sketch_modifiers(
            &ctx,
            &mut editor,
            &mut session,
            [40., 60.],
            egui::Modifiers::SHIFT,
        );
        assert_eq!(editor.constraints.selection().len(), 1);
        assert!(
            editor.element.is_none(),
            "Toggled-off curve stayed a quick-action target"
        );
        click_sketch(&ctx, &mut editor, &mut session, [0., 0.]);
        assert_eq!(editor.constraints.selection().len(), 1);
        assert!(
            editor.element.is_none(),
            "A point became a curve action target"
        );
        assert_eq!(session.document(), &before, "Selection edited source");
        click_sketch(&ctx, &mut editor, &mut session, [60., 40.]);
        editor
            .draw(
                &mut session,
                &multicad_core::sketch_draw::DrawRequest {
                    input: multicad_core::sketch_draw::DrawInput::Circle {
                        center: multicad_core::sketch_draw::PointInput::Position([50., 30.]),
                        radius_mm: 4.,
                    },
                    construction: false,
                },
                Channel::Ui,
            )
            .unwrap();
        assert!(
            editor
                .selected_profiles(session.document(), session.revision())
                .is_err(),
            "Old profiles were available while source was pending"
        );
        settle(&mut editor, &session);
        assert!(
            editor
                .selected_profiles(session.document(), session.revision())
                .is_err(),
            "New hole silently changed the manufacturing profile"
        );
    }
    #[test]
    fn spline_poles_preview_without_source_edits_then_snap_into_an_exact_closed_profile() {
        let mut session = session();
        let mut editor = SketchEditor::default();
        editor.set_query_factory(std::sync::Arc::new(|| {
            Ok(Box::new(multicad_kernel::NativeRegionQueries::default()))
        }));
        let ctx = egui::Context::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        editor.set_tool(DrawTool::Spline).unwrap();
        editor.snap = false;
        gesture_frame(&ctx, &mut editor, &mut session, vec![]);
        let before = session.document().clone();
        for point in [[0., 0.], [0., 30.], [40., 30.], [40., 0.]] {
            click_sketch(&ctx, &mut editor, &mut session, point);
            assert_eq!(session.document(), &before);
        }
        assert!(editor.can_finish_spline());
        editor
            .finish_spline(&mut session, false, Channel::CommandLine)
            .unwrap();
        assert_eq!(session.revision(), 2);
        let id = editor.selected.unwrap();
        let source = editor
            .read_source(session.document(), id, session.revision())
            .unwrap()
            .0;
        let multicad_core::constraint_sketch::CurveKind::BSpline { poles, basis } =
            &source.sketch.curves[0].kind
        else {
            panic!("Spline missing")
        };
        assert_eq!(basis.degree, 3);
        let ends = [poles[0], *poles.last().unwrap()];
        assert_eq!(source.sketch.points.len(), 4);
        editor.set_tool(DrawTool::Line).unwrap();
        editor.snap = true;
        click_sketch(&ctx, &mut editor, &mut session, [40., 0.]);
        click_sketch(&ctx, &mut editor, &mut session, [0., 0.]);
        let source = editor
            .read_source(session.document(), id, session.revision())
            .unwrap()
            .0;
        assert_eq!(
            source.sketch.points.len(),
            4,
            "Endpoint snaps added disconnected poles"
        );
        assert!(
            matches!(source.sketch.curves[1].kind,multicad_core::constraint_sketch::CurveKind::Line{start,end} if [start,end]==[ends[1],ends[0]])
        );
        settle(&mut editor, &session);
        assert_eq!(
            editor
                .sketch_evaluation
                .current(id)
                .unwrap()
                .regions
                .regions
                .len(),
            1
        );
        editor.tool = None;
        click_sketch(&ctx, &mut editor, &mut session, [20., 10.]);
        assert_eq!(
            editor
                .selected_profiles(session.document(), session.revision())
                .unwrap()
                .1
                .len(),
            1
        );
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        let restored = editor
            .read_source(session.document(), id, session.revision())
            .unwrap()
            .0;
        assert_eq!(restored.sketch.curves[0], source.sketch.curves[0]);
        assert_eq!(restored.sketch.curves.len(), 1);
        editor.set_tool(DrawTool::Spline).unwrap();
        let before = session.document().clone();
        click_sketch(&ctx, &mut editor, &mut session, [60., 0.]);
        editor.cancel();
        assert_eq!(
            session.document(),
            &before,
            "Cancel committed an unfinished spline"
        );
    }
    #[test]
    fn closing_a_pointer_spline_reuses_the_first_pole_identity_in_one_transaction() {
        let mut session = session();
        let mut editor = SketchEditor::default();
        editor.set_query_factory(std::sync::Arc::new(|| {
            Ok(Box::new(multicad_kernel::NativeRegionQueries::default()))
        }));
        let ctx = egui::Context::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        editor.set_tool(DrawTool::Spline).unwrap();
        editor.snap = false;
        gesture_frame(&ctx, &mut editor, &mut session, vec![]);
        for point in [[0., 0.], [0., 30.], [40., 30.], [40., 0.], [0., 0.]] {
            click_sketch(&ctx, &mut editor, &mut session, point);
        }
        assert_eq!(session.revision(), 2);
        let id = editor.selected.unwrap();
        let source = editor.read_source(session.document(), id, 2).unwrap().0;
        let multicad_core::constraint_sketch::CurveKind::BSpline { poles, .. } =
            &source.sketch.curves[0].kind
        else {
            panic!("Spline missing")
        };
        assert_eq!(poles.len(), 5);
        assert_eq!(poles.first(), poles.last());
        assert_eq!(source.sketch.points.len(), 4);
        assert_eq!(
            editor
                .sketch_evaluation
                .current(id)
                .unwrap()
                .regions
                .regions
                .len(),
            1
        );
    }
    #[test]
    fn selected_geometry_operations_preview_apply_and_undo_through_the_same_inspector() {
        use crate::sketch_operations::{OperationRequest, OperationTool};
        use multicad_core::sketch_transform::{Transform, TransformOptions, TransformRequest};
        let mut session = session();
        let mut editor = SketchEditor::default();
        let ctx = egui::Context::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        editor
            .draw(
                &mut session,
                &multicad_core::sketch_draw::DrawRequest {
                    input: multicad_core::sketch_draw::DrawInput::Circle {
                        center: multicad_core::sketch_draw::PointInput::Position([10., 10.]),
                        radius_mm: 3.,
                    },
                    construction: false,
                },
                Channel::Ui,
            )
            .unwrap();
        editor.tool = None;
        gesture_frame(&ctx, &mut editor, &mut session, vec![]);
        click_sketch(&ctx, &mut editor, &mut session, [13., 10.]);
        let selection = editor.constraints.selection().iter().copied().collect();
        let id = editor.selected.unwrap();
        let before = session.document().clone();
        editor.begin_operation(OperationTool::Move).unwrap();
        editor
            .prepare_operation(
                &session,
                OperationRequest::Transform(TransformRequest {
                    selection,
                    transform: Transform::Translate {
                        delta_mm: [20., 5.],
                    },
                    options: TransformOptions::default(),
                }),
            )
            .unwrap();
        assert_eq!(
            session.document(),
            &before,
            "Operation preview changed source"
        );
        assert!(editor.has_pending_edits());
        assert!(editor.set_tool(DrawTool::Line).is_err());
        let preview = editor
            .read_sketch(
                editor.constraints.preview_document().unwrap(),
                id,
                session.revision(),
            )
            .unwrap()
            .0;
        assert!(
            matches!(preview.geometry[0],Geometry::Circle{center,radius_mm,..} if (center[0]-30.).abs()<1e-6 && (center[1]-15.).abs()<1e-6 && (radius_mm-3.).abs()<1e-8)
        );
        editor.constraints.apply(&mut session).unwrap();
        assert_eq!(session.revision(), 3);
        let source = editor
            .read_source(session.document(), id, session.revision())
            .unwrap()
            .0;
        assert!(
            matches!(source.sketch.curves[0].kind,multicad_core::constraint_sketch::CurveKind::Circle{radius_mm,..} if radius_mm==3.)
        );
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(
            session.document(),
            &before,
            "Apply required more than one undo"
        );
    }
    #[test]
    fn pointer_circle_uses_radius_then_endpoint_snaps_reuse_actual_point_ids() {
        use multicad_core::constraint_sketch::CurveKind;
        let mut session = session();
        let mut editor = SketchEditor::default();
        let ctx = egui::Context::default();
        editor.create(&mut session, Datum::Xz, Channel::Ui).unwrap();
        editor.snap = false;
        editor.set_tool(DrawTool::Circle).unwrap();
        gesture_frame(&ctx, &mut editor, &mut session, vec![]);
        gesture_frame(&ctx, &mut editor, &mut session, vec![]);
        let before = session.document().clone();
        click_sketch(&ctx, &mut editor, &mut session, [10., 10.]);
        assert_eq!(
            session.document(),
            &before,
            "The center click is only UI state"
        );
        click_sketch(&ctx, &mut editor, &mut session, [25., 10.]);
        let id = editor.selected.unwrap();
        let (source, frame, _) = Source::read(session.document(), id).unwrap();
        let CurveKind::Circle { center, radius_mm } = source.sketch.curves[0].kind else {
            panic!("circle missing")
        };
        assert!((radius_mm - 15.).abs() < 1e-5);
        assert_eq!(frame.n, Datum::Xz.frame().n);
        assert_eq!(source.sketch.points.len(), 1);
        editor.snap = true;
        editor.set_tool(DrawTool::Line).unwrap();
        click_sketch(&ctx, &mut editor, &mut session, [10.2, 10.2]);
        click_sketch(&ctx, &mut editor, &mut session, [40., 10.]);
        let (source, _, _) = Source::read(session.document(), id).unwrap();
        assert_eq!(
            source.sketch.points.len(),
            2,
            "Snapping joins the authored center point"
        );
        let CurveKind::Line { start, .. } = source.sketch.curves[1].kind else {
            panic!("line missing")
        };
        assert_eq!(start, center);
    }
    #[test]
    fn pointer_rectangle_and_circle_modes_commit_once_and_cancel_keeps_source() {
        use multicad_core::constraint_sketch::CurveKind;
        let mut session = session();
        let mut editor = SketchEditor::default();
        let ctx = egui::Context::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        editor.snap = false;
        let cases: [(DrawTool, &[[f64; 2]], usize); 4] = [
            (
                DrawTool::RectangleThree,
                &[[0., 0.], [20., 10.], [10., 30.]],
                4,
            ),
            (DrawTool::RectangleCenter, &[[50., 0.], [60., 8.]], 4),
            (
                DrawTool::CircleThree,
                &[[100., 0.], [110., 10.], [120., 0.]],
                1,
            ),
            (
                DrawTool::CircleTwoRadius,
                &[[150., 0.], [158., 0.], [154., 3.]],
                1,
            ),
        ];
        for (tool, clicks, curves) in cases {
            editor.set_tool(tool).unwrap();
            gesture_frame(&ctx, &mut editor, &mut session, vec![]);
            gesture_frame(&ctx, &mut editor, &mut session, vec![]);
            let before = session.document().clone();
            // Partial gestures are UI state; cancelling never commits.
            for point in &clicks[..clicks.len() - 1] {
                click_sketch(&ctx, &mut editor, &mut session, *point);
            }
            assert_eq!(session.document(), &before, "{tool:?} committed early");
            editor.cancel();
            assert_eq!(session.document(), &before);
            editor.set_tool(tool).unwrap();
            let id = editor.selected.unwrap();
            let count = Source::read(session.document(), id)
                .unwrap()
                .0
                .sketch
                .curves
                .len();
            let revision = session.revision();
            let undone = session.document().clone();
            for point in clicks {
                click_sketch(&ctx, &mut editor, &mut session, *point);
            }
            assert_eq!(
                session.revision(),
                revision + 1,
                "{tool:?}: one transaction"
            );
            let (source, _, _) = Source::read(session.document(), id).unwrap();
            assert_eq!(source.sketch.curves.len(), count + curves, "{tool:?}");
            if tool == DrawTool::CircleTwoRadius {
                let CurveKind::Circle { center, radius_mm } = source.sketch.curves[count].kind
                else {
                    panic!("circle expected")
                };
                let center = source
                    .sketch
                    .points
                    .iter()
                    .find(|p| p.id == center)
                    .unwrap()
                    .position_mm;
                assert!((center[0] - 154.).abs() < 1e-4 && (center[1] - 3.).abs() < 1e-4);
                assert!((radius_mm - 5.).abs() < 1e-4);
            }
            session
                .undo(session.revision(), Actor::local(Channel::Ui))
                .unwrap();
            assert_eq!(session.document(), &undone, "{tool:?}: one undo restores");
            for point in clicks {
                click_sketch(&ctx, &mut editor, &mut session, *point);
            }
        }
    }
    #[test]
    fn pointer_text_places_once_edits_in_inspector_and_refuses_without_change() {
        use multicad_core::constraint_sketch::text::TextOperation;
        let mut session = session();
        let mut editor = SketchEditor::default();
        let ctx = egui::Context::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        editor.snap = false;
        let id = editor.selected.unwrap();
        editor.set_tool(DrawTool::Text).unwrap();
        editor.text_tool.draft.text = "ABO".into();
        editor.text_tool.draft.height_mm = 8.;
        gesture_frame(&ctx, &mut editor, &mut session, vec![]);
        gesture_frame(&ctx, &mut editor, &mut session, vec![]);
        let before = session.document().clone();
        click_sketch(&ctx, &mut editor, &mut session, [10., 5.]);
        assert_ne!(session.document(), &before, "{}", editor.message);
        let texts = |session: &Session| {
            Source::read_for_evaluation(session.document(), id, session.revision(), None)
                .unwrap()
                .0
                .sketch
                .texts
        };
        let placed = texts(&session);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].style.text, "ABO");
        assert_eq!(placed[0].style.height_mm, 8.);
        assert!((placed[0].style.position_mm[0] - 10.).abs() < 1e-6);
        assert!((placed[0].style.position_mm[1] - 5.).abs() < 1e-6);
        assert_eq!(editor.text_tool.editing, Some(placed[0].id));
        let after_place = session.document().clone();
        // An Inspector edit is one further transaction.
        let mut style = editor.text_tool.draft.clone();
        style.text = "ABD".into();
        style.height_mm = 12.;
        editor
            .apply_text(
                &mut session,
                TextOperation::Edit {
                    text: placed[0].id,
                    style,
                },
            )
            .unwrap();
        assert_eq!(texts(&session)[0].style.text, "ABD");
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(
            session.document(),
            &after_place,
            "one undo restores the edit"
        );
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(
            session.document(),
            &before,
            "one undo removes the placement"
        );
        // Empty text and unsupported characters are refused without change.
        for (text, needle) in [("  ", "empty"), ("A\u{263A}", "not supported")] {
            editor.text_tool.draft.text = text.into();
            click_sketch(&ctx, &mut editor, &mut session, [0., 0.]);
            assert_eq!(session.document(), &before, "{text:?}");
            assert!(editor.message.contains(needle), "{}", editor.message);
        }
        assert!(DrawTool::Text.parse(&[], false).is_err());
    }
    #[test]
    fn clicking_a_letter_selects_its_text_and_opens_it_for_editing() {
        use multicad_core::constraint_sketch::text::{SketchText, TextAnchor, TextFont};
        let mut session = session();
        let mut editor = SketchEditor::default();
        let ctx = egui::Context::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        editor.snap = false;
        let id = editor.selected.unwrap();
        editor.set_tool(DrawTool::Text).unwrap();
        assert_eq!(editor.text_tool.draft.anchor, TextAnchor::Center);
        editor.text_tool.draft.text = "HI\nH".into();
        editor.text_tool.draft.height_mm = 10.;
        editor.text_tool.draft.font = TextFont::HackRegular;
        gesture_frame(&ctx, &mut editor, &mut session, vec![]);
        gesture_frame(&ctx, &mut editor, &mut session, vec![]);
        click_sketch(&ctx, &mut editor, &mut session, [20., 10.]);
        let texts = |session: &Session| {
            Source::read_for_evaluation(session.document(), id, session.revision(), None)
                .unwrap()
                .0
                .sketch
                .texts
        };
        let placed: Vec<SketchText> = texts(&session);
        assert_eq!(placed.len(), 1, "{}", editor.message);
        let at = placed[0].style.position_mm;
        assert!(
            (at[0] - 20.).abs() < 1e-6 && (at[1] - 10.).abs() < 1e-6,
            "{at:?}"
        );
        assert_eq!(placed[0].style.anchor, TextAnchor::Center);
        // A point inside the first H's left stem.
        let h = &placed[0].contours().unwrap()[0];
        let xs = h.poles.iter().map(|p| p[0]);
        let ys = h.poles.iter().map(|p| p[1]);
        let (x0, x1) = (
            xs.clone().fold(f64::MAX, f64::min),
            xs.fold(f64::MIN, f64::max),
        );
        let (y0, y1) = (
            ys.clone().fold(f64::MAX, f64::min),
            ys.fold(f64::MIN, f64::max),
        );
        let stem = [x0 + (x1 - x0) * 0.08, (y0 + y1) / 2.];
        assert!(placed[0].contains(stem));
        // With no tool active, a click on the letter opens the text.
        editor.tool = None;
        editor.text_tool.editing = None;
        let before = session.document().clone();
        click_sketch(&ctx, &mut editor, &mut session, stem);
        assert_eq!(editor.tool, Some(DrawTool::Text), "{}", editor.message);
        assert_eq!(editor.text_tool.editing, Some(placed[0].id));
        assert_eq!(editor.text_tool.draft, placed[0].style);
        assert_eq!(session.document(), &before, "selection changes nothing");
        // With the Text tool active, clicking a letter selects instead of
        // placing a second text; empty space still places one.
        editor.text_tool.editing = None;
        click_sketch(&ctx, &mut editor, &mut session, stem);
        assert_eq!(texts(&session).len(), 1);
        assert_eq!(editor.text_tool.editing, Some(placed[0].id));
        click_sketch(&ctx, &mut editor, &mut session, [-60., -40.]);
        assert_eq!(texts(&session).len(), 2, "{}", editor.message);
    }
    #[test]
    fn pointer_corner_fillet_and_chamfer_apply_once_and_misses_change_nothing() {
        use multicad_core::constraint_sketch::CurveKind;
        let mut session = session();
        let mut editor = SketchEditor::default();
        let ctx = egui::Context::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        editor.snap = false;
        editor
            .add(
                &mut session,
                DrawTool::Rectangle,
                [0., 0.],
                [40., 20.],
                Channel::Ui,
            )
            .unwrap();
        let id = editor.selected.unwrap();
        for (tool, corner, arcs, lines) in [
            (DrawTool::Fillet, [40., 0.], 1, 6),
            (DrawTool::Chamfer, [0., 20.], 1, 9),
        ] {
            editor.set_tool(tool).unwrap();
            editor.corner_size_mm = 4.;
            gesture_frame(&ctx, &mut editor, &mut session, vec![]);
            gesture_frame(&ctx, &mut editor, &mut session, vec![]);
            let before = session.document().clone();
            // A click away from any two-line corner is refused.
            click_sketch(&ctx, &mut editor, &mut session, [20., 10.]);
            assert_eq!(session.document(), &before);
            assert!(editor.message.contains("corner"), "{}", editor.message);
            click_sketch(&ctx, &mut editor, &mut session, corner);
            assert_ne!(session.document(), &before, "{tool:?} did not apply");
            let (source, _, _) = Source::read(session.document(), id).unwrap();
            let count = |arc: bool| {
                source
                    .sketch
                    .curves
                    .iter()
                    .filter(|c| matches!(c.kind, CurveKind::Arc { .. }) == arc)
                    .count()
            };
            assert_eq!(count(true), arcs.min(1), "{tool:?}");
            assert_eq!(count(false), lines, "{tool:?}");
            session
                .undo(session.revision(), Actor::local(Channel::Ui))
                .unwrap();
            assert_eq!(session.document(), &before, "{tool:?}: one undo restores");
            click_sketch(&ctx, &mut editor, &mut session, corner);
        }
    }
    #[test]
    fn pointer_arc_center_start_end_projects_radius_and_cancel_never_commits_partial_arc() {
        use multicad_core::constraint_sketch::CurveKind;
        let mut session = session();
        let mut editor = SketchEditor::default();
        let ctx = egui::Context::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        editor.snap = false;
        editor.set_tool(DrawTool::Arc).unwrap();
        gesture_frame(&ctx, &mut editor, &mut session, vec![]);
        gesture_frame(&ctx, &mut editor, &mut session, vec![]);
        let before = session.document().clone();
        click_sketch(&ctx, &mut editor, &mut session, [0., 0.]);
        click_sketch(&ctx, &mut editor, &mut session, [10., 0.]);
        assert_eq!(session.document(), &before);
        editor.cancel();
        assert_eq!(session.document(), &before);
        editor.set_tool(DrawTool::Arc).unwrap();
        for point in [[0., 0.], [10., 0.], [5., 20.]] {
            click_sketch(&ctx, &mut editor, &mut session, point);
        }
        let id = editor.selected.unwrap();
        let (source, _, _) = Source::read(session.document(), id).unwrap();
        let CurveKind::Arc {
            center,
            start,
            end,
            clockwise,
        } = source.sketch.curves[0].kind
        else {
            panic!("arc missing")
        };
        let point = |id| {
            source
                .sketch
                .points
                .iter()
                .find(|p| p.id == id)
                .unwrap()
                .position_mm
        };
        let c = point(center);
        let a = point(start);
        let b = point(end);
        assert!(!clockwise);
        assert!(((a[0] - c[0]).hypot(a[1] - c[1]) - 10.).abs() < 1e-5);
        assert!(((b[0] - c[0]).hypot(b[1] - c[1]) - 10.).abs() < 1e-5);
        assert!(((b[1] - c[1]).atan2(b[0] - c[0]) - 20_f64.atan2(5.)).abs() < 1e-5);
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(session.document(), &before);
        session
            .redo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(Source::read(session.document(), id).unwrap().0, source);
    }
    #[test]
    fn pointer_polyline_closes_shared_vertices_even_with_snapping_off_and_undo_retains_identity() {
        let mut session = session();
        let mut editor = SketchEditor::default();
        let ctx = egui::Context::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        editor.snap = false;
        editor.set_tool(DrawTool::Polyline).unwrap();
        gesture_frame(&ctx, &mut editor, &mut session, vec![]);
        gesture_frame(&ctx, &mut editor, &mut session, vec![]);
        for point in [[0., 0.], [30., 0.], [20., 20.], [0., 0.]] {
            click_sketch(&ctx, &mut editor, &mut session, point);
        }
        let id = editor.selected.unwrap();
        let (source, _, _) = Source::read(session.document(), id).unwrap();
        assert_eq!(source.sketch.points.len(), 3);
        assert_eq!(source.sketch.curves.len(), 3);
        assert!(editor.first.is_none());
        let regions = multicad_core::sketch_regions::extract(&source);
        assert!(regions.diagnostics.is_empty());
        assert_eq!(regions.regions.len(), 1);
        assert!((regions.regions[0].area_mm2 - 300.).abs() < 1e-4);
        let before = session.document().clone();
        click_sketch(&ctx, &mut editor, &mut session, [60., 40.]);
        editor.cancel();
        assert_eq!(session.document(), &before);
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(
            Source::read(session.document(), id)
                .unwrap()
                .0
                .sketch
                .curves
                .len(),
            2
        );
        session
            .redo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(Source::read(session.document(), id).unwrap().0, source);
    }
    #[test]
    fn construction_toggle_distinguishes_active_drawing_from_selected_curve_edit() {
        let mut session = session();
        let mut editor = SketchEditor::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        editor.set_tool(DrawTool::Line).unwrap();
        let before = session.document().clone();
        editor
            .toggle_construction(&mut session, Channel::Ui)
            .unwrap();
        assert!(editor.construction());
        assert_eq!(session.document(), &before);
        editor
            .add(
                &mut session,
                DrawTool::Line,
                [0., 0.],
                [20., 0.],
                Channel::Ui,
            )
            .unwrap();
        let id = editor.selected.unwrap();
        let (source, _, _) = Source::read(session.document(), id).unwrap();
        assert!(source.sketch.curves[0].construction);
        assert!(
            multicad_core::sketch_regions::extract(&source)
                .diagnostics
                .is_empty()
        );
        editor.tool = None;
        editor
            .toggle_construction(&mut session, Channel::Ui)
            .unwrap();
        assert!(
            !Source::read(session.document(), id)
                .unwrap()
                .0
                .sketch
                .curves[0]
                .construction
        );
        session
            .undo(session.revision(), Actor::local(Channel::Ui))
            .unwrap();
        assert_eq!(Source::read(session.document(), id).unwrap().0, source);
    }
    #[test]
    fn unavailable_transform_does_not_trap_model_picking() {
        use crate::viewport::{self, Instance, Scene, SceneStamp, Transform};
        use multicad_core::document::*;
        use multicad_kernel::{MeshOptions, Solid};
        use std::sync::Arc;
        let producer = Entity::new(
            "Box source",
            EntityData::Feature {
                definition: Definition {
                    type_id: "test.box".into(),
                    schema_version: 1,
                    data: serde_json::json!({}),
                },
                inputs: vec![],
                suppressed: false,
            },
        );
        let part = Entity::new(
            "Box",
            EntityData::Part {
                features: vec![producer.id],
                material: None,
                tags: Default::default(),
            },
        );
        let instance = Entity::new(
            "Box instance",
            EntityData::Instance {
                component: part.id,
                placement: Default::default(),
                placement_source: None,
            },
        );
        let mut doc = Document::new("Picking after unavailable transform").unwrap();
        for entity in [producer.clone(), part, instance.clone()] {
            doc.insert(entity).unwrap();
        }
        let mut session = Session::new(doc).unwrap();
        let before = session.document().clone();
        let native = Solid::box_named([100., 60., 18.], producer.id, Default::default()).unwrap();
        let mesh = native.mesh(MeshOptions::default()).unwrap();
        let geometry = Arc::new(
            viewport::Geometry::new(
                producer.id,
                mesh.vertices.as_chunks::<3>().0.to_vec(),
                mesh.indices,
                mesh.face_indices,
                native.topology_snapshot(producer.id, 0).unwrap().elements,
            )
            .unwrap(),
        );
        let scene = Arc::new(
            Scene::new(
                SceneStamp {
                    document: session.document().id(),
                    revision: 0,
                    generation: 1,
                },
                vec![geometry],
                vec![Instance {
                    id: instance.id,
                    geometry: producer.id,
                    transform: Transform::default(),
                    color: [140, 165, 190, 255],
                }],
            )
            .unwrap(),
        );
        let ctx = egui::Context::default();
        let mut editor = SketchEditor::default();
        editor.toggle_transform(); // There is no selected sketch or part-move provider.
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000., 700.));
        let point = editor.camera.project([50., 30., 18.], rect);
        let point = Pos2::new(point[0], point[1]);
        let mut frame = |events| {
            settle(&mut editor, &session);
            ctx.run_ui(input(events), |ui| {
                editor.canvas(ui, &mut session, false, Some(scene.clone()), true)
            })
            .drop_without_applying_deltas()
        };
        frame(vec![]);
        frame(vec![]);
        for pressed in [true, false] {
            frame(vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        assert_eq!(editor.selected_solid(), Some(instance.id));
        assert_eq!(session.document(), &before);
    }
    #[test]
    fn attached_sketch_opens_and_edits_only_with_a_current_resolved_frame() {
        use multicad_core::{document::*, references::*};
        use multicad_kernel::Solid;
        use std::collections::BTreeMap;
        let panel = Entity::new(
            "Panel",
            EntityData::Feature {
                definition: Definition {
                    type_id: "test.panel".into(),
                    schema_version: 1,
                    data: serde_json::json!({}),
                },
                inputs: vec![],
                suppressed: false,
            },
        );
        let native = Solid::box_named([100., 60., 18.], panel.id, Default::default()).unwrap();
        let snapshot = native.topology_snapshot(panel.id, 0).unwrap();
        let top = snapshot
            .elements
            .iter()
            .find(|e| {
                e.key.kind == TopologyKind::Face && e.origins.iter().any(|o| o.role == "cap.end")
            })
            .unwrap();
        let mut intent = GeometryIntent::capture(&snapshot, top.key).unwrap();
        intent.attachment = Some(Attachment {
            anchor: top
                .origins
                .iter()
                .find(|o| o.role == "cap.end")
                .unwrap()
                .clone(),
            offset_mm: [0.; 3],
            angle_rad: 0.,
        });
        let mut link = Link::new(panel.id, "face");
        link.geometry = Some(intent);
        let plane = Entity::new(
            "Attached plane",
            EntityData::Plane {
                origin_mm: [999.; 3],
                normal: [0., 0., 1.],
                anchor: Some(link),
            },
        );
        let source = Source {
            u: [1., 0., 0.],
            sketch: Default::default(),
        };
        let sketch = Entity::new(
            "Attached sketch",
            EntityData::Sketch {
                plane: plane.id,
                definition: source.definition().unwrap(),
                inputs: vec![],
            },
        );
        let mut doc = Document::new("Attached editing").unwrap();
        for entity in [panel, plane.clone(), sketch.clone()] {
            doc.insert(entity).unwrap();
        }
        let mut session = Session::new(doc).unwrap();
        let mut editor = SketchEditor::default();
        assert!(editor.open_at(session.document(), sketch.id, 0).is_err());
        editor.evaluated = Some(inspect(
            session.document(),
            0,
            &BTreeMap::from([(snapshot.owner, snapshot)]),
        ));
        editor.open_at(session.document(), sketch.id, 0).unwrap();
        settle(&mut editor, &session);
        assert_eq!(
            editor
                .read_sketch(session.document(), sketch.id, 0)
                .unwrap()
                .1
                .origin,
            [0., 0., 18.]
        );
        editor
            .add(
                &mut session,
                DrawTool::Rectangle,
                [5., 5.],
                [15., 15.],
                Channel::Ui,
            )
            .unwrap();
        assert_eq!(session.revision(), 1);
        assert_eq!(session.document().get(plane.id), Some(&plane));
        assert!(editor.open_at(session.document(), sketch.id, 1).is_err());
        let before = session.document().clone();
        assert!(
            editor
                .add(
                    &mut session,
                    DrawTool::Line,
                    [0., 0.],
                    [10., 0.],
                    Channel::Ui
                )
                .is_err()
        );
        assert_eq!(session.document(), &before);
    }
    #[test]
    fn pending_constraint_draft_blocks_drawing_switching_finishing_and_transform() {
        let mut session = session();
        let mut editor = SketchEditor::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        let id = editor.selected.unwrap();
        editor
            .add(
                &mut session,
                DrawTool::Rectangle,
                [0., 0.],
                [80., 60.],
                Channel::Ui,
            )
            .unwrap();
        let (mut source, _, _) = Source::read(session.document(), id).unwrap();
        let edge = source.sketch.curves[0].id;
        source.add_constraint(multicad_core::constraint_sketch::ConstraintKind::Length {
            line: edge,
            dimension: multicad_core::constraint_sketch::Dimension::Literal {
                value: 90.,
                unit: "mm".into(),
            },
        });
        editor.constraints.prepare(&session, id, source);
        assert!(
            editor
                .read_sketch(
                    editor.constraints.preview_document().unwrap(),
                    id,
                    session.revision()
                )
                .is_ok(),
            "A checked literal preview disappeared while its canonical revision remained unchanged"
        );
        assert!(
            editor
                .read_solved_source(
                    editor.constraints.preview_document().unwrap(),
                    id,
                    session.revision()
                )
                .is_ok()
        );
        let before = session.document().clone();
        assert!(editor.has_pending_edits());
        assert!(
            editor
                .add(
                    &mut session,
                    DrawTool::Line,
                    [0., 0.],
                    [10., 0.],
                    Channel::Ui
                )
                .is_err()
        );
        assert!(editor.finish().is_err());
        assert!(editor.create(&mut session, Datum::Xy, Channel::Ui).is_err());
        assert!(editor.open(session.document(), id).is_err());
        editor.toggle_transform();
        assert!(!editor.moving);
        assert_eq!(session.document(), &before);
        editor.constraints.discard();
        editor.finish().unwrap();
        assert!(!editor.editing);
    }
    #[test]
    fn cancel_is_source_neutral_and_move_preview_is_one_undoable_transaction() {
        let mut s = session();
        let mut editor = SketchEditor::default();
        editor.begin();
        editor.cancel();
        assert_eq!(s.revision(), 0);
        assert!(s.document().entities().next().is_none());
        editor.create(&mut s, Datum::Xz, Channel::Ui).unwrap();
        let id = editor.selected.unwrap();
        let before = s.document().clone();
        editor.coordinates = Coordinates::Object;
        editor.delta = [10., 20., 30.];
        editor.prepare_move(&s).unwrap();
        assert_eq!(s.document(), &before);
        assert_eq!(
            Sketch::read(editor.preview.as_ref().unwrap().document(), id)
                .unwrap()
                .1
                .origin,
            [10., -30., 20.]
        );
        editor.cancel_move();
        assert_eq!(s.document(), &before);
        editor.delta = [10., 20., 30.];
        editor.prepare_move(&s).unwrap();
        editor.apply_move(&mut s).unwrap();
        assert_eq!(s.revision(), 2);
        s.undo(2, Actor::local(Channel::Ui)).unwrap();
        assert_eq!(s.document(), &before);
        editor.coordinates = Coordinates::Custom(Datum::Yz);
        editor.delta = [1., 2., 3.];
        editor.prepare_move(&s).unwrap();
        assert_eq!(
            Sketch::read(editor.preview.as_ref().unwrap().document(), id)
                .unwrap()
                .1
                .origin,
            [3., 1., 2.]
        );
        editor.custom_angle_deg = 90.;
        editor.prepare_move(&s).unwrap();
        let rotated = Sketch::read(editor.preview.as_ref().unwrap().document(), id)
            .unwrap()
            .1
            .origin;
        for (actual, expected) in rotated.into_iter().zip([3., -2., 1.]) {
            assert!((actual - expected).abs() < 1e-9);
        }
        s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
        assert!(editor.apply_move(&mut s).is_err());
        assert!(s.document().entities().next().is_none());
    }
    #[test]
    fn two_pointer_clicks_draw_rectangle_on_active_plane() {
        let ctx = egui::Context::default();
        let mut s = session();
        let mut editor = SketchEditor::default();
        editor.create(&mut s, Datum::Xy, Channel::Ui).unwrap();
        editor.set_tool(DrawTool::Rectangle).unwrap();
        let id = editor.selected.unwrap();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000., 700.));
        let a = editor.camera.project([20., 20., 0.], rect);
        let b = editor.camera.project([80., 60., 0.], rect);
        let mut frame = |events| {
            settle(&mut editor, &s);
            ctx.run_ui(input(events), |ui| {
                editor.canvas(ui, &mut s, true, None, false)
            })
            .drop_without_applying_deltas();
        };
        frame(vec![]);
        for p in [Pos2::new(a[0], a[1]), Pos2::new(b[0], b[1])] {
            for pressed in [true, false] {
                frame(vec![
                    egui::Event::PointerMoved(p),
                    egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
        }
        let (source, _, _) = Sketch::read(s.document(), id).unwrap();
        assert_eq!(source.geometry.len(), 4);
        let (constrained, _, _) = Source::read(s.document(), id).unwrap();
        assert_eq!(constrained.sketch.points.len(), 4);
        assert_eq!(constrained.sketch.constraints.len(), 4);
        assert_eq!(source.geometry[0].segments()[0], [[20., 20.], [80., 20.]]);
        assert_eq!(s.revision(), 2);
    }
    #[test]
    fn dragging_gizmo_commits_one_source_move() {
        let ctx = egui::Context::default();
        let mut session = session();
        let mut editor = SketchEditor::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        editor.moving = true;
        let id = editor.selected.unwrap();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000., 700.));
        let length = (62. / editor.camera.scale) as f64;
        let end = editor.camera.project([length as f32, 0., 0.], rect);
        let start = Pos2::new(end[0], end[1]);
        let finish = start + Vec2::new(30., 0.);
        let mut frame = |events| {
            settle(&mut editor, &session);
            ctx.run_ui(input(events), |ui| {
                editor.canvas(ui, &mut session, true, None, false)
            })
            .drop_without_applying_deltas();
        };
        frame(vec![]);
        frame(vec![]);
        frame(vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        frame(vec![egui::Event::PointerMoved(finish)]);
        frame(vec![egui::Event::PointerButton {
            pos: finish,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(session.revision(), 2);
        let origin = Sketch::read(session.document(), id).unwrap().1.origin;
        assert_eq!(origin, [10., 0., 0.]);
    }
    #[test]
    fn sketch_draft_is_queued_above_real_solid_rendering_without_source_edits() {
        use crate::viewport::{self, Instance, Scene, SceneStamp, Transform};
        use multicad_kernel::{MeshOptions, Solid};
        use std::sync::Arc;
        let ctx = egui::Context::default();
        let mut session = session();
        let mut editor = SketchEditor::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        editor.set_tool(DrawTool::Rectangle).unwrap();
        editor.snap = false;
        let id = EntityId::new();
        let solid = Solid::box_named([100., 60., 18.], id, Default::default()).unwrap();
        let mesh = solid.mesh(MeshOptions::default()).unwrap();
        let geometry = Arc::new(
            viewport::Geometry::new(
                id,
                mesh.vertices.as_chunks::<3>().0.to_vec(),
                mesh.indices,
                mesh.face_indices,
                solid
                    .topology_snapshot(id, session.revision())
                    .unwrap()
                    .elements,
            )
            .unwrap(),
        );
        let scene = Arc::new(
            Scene::new(
                SceneStamp {
                    document: session.document().id(),
                    revision: session.revision(),
                    generation: 1,
                },
                vec![geometry],
                vec![Instance {
                    id: EntityId::new(),
                    geometry: id,
                    transform: Transform::default(),
                    color: [140, 160, 180, 255],
                }],
            )
            .unwrap(),
        );
        let before = session.document().clone();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000., 700.));
        let a = editor.camera.project([20., 20., 0.], rect);
        let b = editor.camera.project([80., 60., 0.], rect);
        let mut frame = |events| {
            settle(&mut editor, &session);
            ctx.run_ui(input(events), |ui| {
                editor.canvas(ui, &mut session, true, Some(scene.clone()), true)
            })
        };
        frame(vec![]).drop_without_applying_deltas();
        let p = Pos2::new(a[0], a[1]);
        for pressed in [true, false] {
            frame(vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ])
            .drop_without_applying_deltas();
        }
        let output = frame(vec![egui::Event::PointerMoved(Pos2::new(b[0], b[1]))]);
        let callback = output
            .shapes
            .iter()
            .position(|s| matches!(s.shape, egui::Shape::Callback(_)))
            .expect("Solid render callback missing");
        let preview=output.shapes.iter().position(|s|matches!(s.shape,egui::Shape::LineSegment{stroke,..} if stroke.color==Color32::LIGHT_YELLOW)).expect("Sketch draft missing");
        assert!(preview > callback, "Draft was painted below the model");
        output.drop_without_applying_deltas();
        assert_eq!(session.document(), &before);
        assert!(editor.first.is_some());
    }
    #[test]
    fn trackpad_navigation_changes_camera_without_editing_canonical_sketch() {
        let ctx = egui::Context::default();
        let mut editor = SketchEditor::default();
        let mut session = session();
        let canonical = session.document().clone();
        let mut frame = |events| {
            settle(&mut editor, &session);
            ctx.run_ui(input(events), |ui| {
                editor.canvas(ui, &mut session, true, None, false)
            })
            .drop_without_applying_deltas();
        };
        frame(vec![]);
        frame(vec![egui::Event::PointerMoved(Pos2::new(450., 350.))]);
        frame(vec![egui::Event::Zoom(1.5)]);
        for (unit, modifiers) in [
            (egui::MouseWheelUnit::Point, egui::Modifiers::NONE),
            (egui::MouseWheelUnit::Line, egui::Modifiers::NONE),
            (egui::MouseWheelUnit::Point, egui::Modifiers::ALT),
        ] {
            frame(vec![egui::Event::MouseWheel {
                unit,
                delta: Vec2::new(12., 8.),
                phase: egui::TouchPhase::Move,
                modifiers,
            }]);
        }
        assert_ne!(editor.camera, Camera::default());
        assert_eq!(session.document(), &canonical);
        assert_eq!(session.revision(), 0);
    }
    #[test]
    fn middle_mouse_drag_still_pans_the_view_without_editing_source() {
        let ctx = egui::Context::default();
        let mut editor = SketchEditor::default();
        let mut session = session();
        let before = editor.camera;
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000., 700.));
        let p = before.target;
        let initial = before.project(p, rect);
        let mut frame = |events| {
            settle(&mut editor, &session);
            ctx.run_ui(input(events), |ui| {
                editor.canvas(ui, &mut session, true, None, false)
            })
            .drop_without_applying_deltas();
        };
        frame(vec![]);
        frame(vec![]);
        let start = egui::pos2(450., 350.);
        let end = start + Vec2::new(30., 20.);
        frame(vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Middle,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        frame(vec![egui::Event::PointerMoved(end)]);
        frame(vec![egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Middle,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        let actual = editor.camera.project(p, rect);
        assert!((actual[0] - initial[0] - 30.).abs() < 0.01);
        assert!((actual[1] - initial[1] - 20.).abs() < 0.01);
        assert_eq!(session.revision(), 0);
    }
    mod operation_selection_tests;
}

#[cfg(test)]
#[path = "sketch_editor/feature_edit_tests.rs"]
mod feature_edit_tests;

#[cfg(test)]
#[path = "sketch_editor/cabinet_edit_tests.rs"]
mod cabinet_edit_tests;

#[cfg(test)]
#[path = "sketch_editor/open_fit_tests.rs"]
mod open_fit_tests;

#[cfg(test)]
mod inspector_tests;

#[cfg(test)]
mod planting_overlay_tests {
    use super::*;
    #[test]
    fn private_plant_plan_is_not_painted_at_the_origin_but_user_sketches_remain_visible() {
        let actor = multicad_core::commands::Actor::local(Channel::Ui);
        let mut session = Session::new(Document::new("Plant overlays").unwrap()).unwrap();
        let request = multicad_core::plants::PlantPlaceRequest::new(
            &session,
            multicad_core::plants::generic_definition(multicad_core::plants::PlantKind::Tree)
                .unwrap(),
            "Tree".into(),
            6000.,
            4000.,
            Default::default(),
        )
        .unwrap();
        session
            .commit(
                multicad_core::plants::prepare_place(&session, 0, actor.clone(), &request).unwrap(),
            )
            .unwrap();
        let plan = multicad_core::plants::resolve(
            session.document(),
            session.parameters(),
            request.placement.record,
        )
        .unwrap()
        .unwrap()
        .plan;
        let (sketch, edits) = multicad_core::constraint_sketch::source::Source::create(
            multicad_core::sketch::Datum::Xy,
            "User sketch",
        )
        .unwrap();
        session
            .apply(multicad_core::commands::Request {
                expected_revision: session.revision(),
                actor,
                label: "User sketch".into(),
                edits,
            })
            .unwrap();
        let mut editor = SketchEditor::default();
        editor.refresh_overlay_visibility(session.document(), session.revision());
        assert!(!editor.overlay_visible(plan));
        assert!(editor.overlay_visible(sketch));
    }
}

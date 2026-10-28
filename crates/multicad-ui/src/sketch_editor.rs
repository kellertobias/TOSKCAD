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
    draw_preview_id: EntityId,
    element: Option<EntityId>,
    pub camera: Camera,
    style: DisplayStyle,
    grid: bool,
    spacing: f64,
    snap: bool,
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
    operation_error: Option<String>,
    tree: crate::model_tree::ModelTree,
    tree_selection: std::collections::BTreeSet<EntityId>,
    visibility: crate::viewport::VisibilityFilter,
    visibility_error: Option<String>,
    overlay_visibility: Option<((EntityId, u64), std::collections::BTreeSet<EntityId>)>,
    tree_scene_stamp: Option<crate::viewport::SceneStamp>,
    pending_profile_pick: Option<(crate::sketch_evaluation::ProfilePickStamp, bool)>,
    queued_profile_clicks: VecDeque<ProfileClick>,
    solid: crate::viewport::Interaction,
    repair: crate::reference_repair::RepairPanel,
    evaluated: Option<multicad_core::references::ReferenceReport>,
    creation_evaluation: Option<(EntityId, multicad_core::references::ReferenceReport)>,
    pub viewport_rect: Option<Rect>,
    plane_picker: PlanePicker,
    occurrence: Option<EntityId>,
    display_scene: Option<std::sync::Arc<crate::viewport::Scene>>,
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
            draw_preview_id: EntityId::new(),
            element: None,
            camera: Camera {
                target: [0.; 3],
                ..Default::default()
            },
            style: DisplayStyle::VisibleEdges,
            grid: true,
            spacing: 10.,
            snap: true,
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
            operation_error: None,
            tree: Default::default(),
            tree_selection: Default::default(),
            visibility: Default::default(),
            visibility_error: None,
            overlay_visibility: None,
            tree_scene_stamp: None,
            pending_profile_pick: None,
            queued_profile_clicks: VecDeque::new(),
            solid: Default::default(),
            repair: Default::default(),
            evaluated: None,
            creation_evaluation: None,
            viewport_rect: None,
            plane_picker: Default::default(),
            occurrence: None,
            display_scene: None,
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
    pub fn has_query_factory(&self) -> bool {
        self.sketch_evaluation.has_query_factory()
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
    pub fn set_display_style(&mut self, style: DisplayStyle) {
        self.style = style;
    }
    pub fn request_fit(&mut self) {
        self.fit_requested = true;
    }
    pub fn home_view(&mut self) {
        self.camera = Camera {
            target: [0.; 3],
            ..Default::default()
        };
    }
    pub fn toggle_transform(&mut self) {
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
        self.operations.cancel();
        self.cancel_move();
        self.tool = None;
        self.clear_drawing_progress();
    }
    pub fn has_pending_edits(&self) -> bool {
        self.preview.is_some()
            || self.constraints.dirty()
            || self.repair.has_pending()
            || self.tree.tool_active()
    }
    pub fn cancel(&mut self) {
        self.operations.cancel();
        self.tree.cancel_tool();
        self.repair.cancel();
        self.solid.clear();
        self.moving = false;
        self.picking = false;
        self.plane_picker.cancel();
        self.tool = None;
        self.clear_drawing_progress();
        self.cancel_move();
    }
    fn cancel_move(&mut self) {
        self.preview = None;
        self.delta = [0.; 3];
        self.drag = None;
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
            &format!("Sketch {}", count + 1),
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
        let visible = multicad_core::hierarchy::Index::new(document)
            .map(|index| {
                document
                    .entities()
                    .filter(|entity| {
                        matches!(entity.data, EntityData::Sketch { .. })
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
            || !self.editing
            || self.tool.is_some()
            || self.moving
            || self.has_pending_edits()
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
        let invalid = !self.editing
            || self.tool.is_some()
            || self.moving
            || self.has_pending_edits()
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
        if matches!(doc.get(id).map(|e|&e.data),Some(EntityData::Sketch{definition,..}) if definition.type_id==multicad_core::constraint_sketch::source::TYPE)
        {
            Source::read_for_evaluation(doc, id, 0, None).map_err(|e| e.to_string())?;
        } else {
            Sketch::read(doc, id).map_err(|e| e.to_string())?;
        }
        self.activate_sketch(id)
    }
    pub fn open_at(&mut self, doc: &Document, id: EntityId, revision: u64) -> Result<(), String> {
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
    pub fn set_tool(&mut self, tool: DrawTool) -> Result<(), String> {
        if self.has_pending_edits() {
            return Err("Apply or discard pending edits before drawing".into());
        }
        if !self.editing {
            return Err("Create or open a sketch first".into());
        }
        self.cancel_move();
        self.operations.cancel();
        self.moving = false;
        self.tool = Some(tool);
        self.clear_drawing_progress();
        self.message = match tool {
            DrawTool::Circle => "Click the center, then the radius · Esc cancels",
            DrawTool::Arc => "Click the center, start, then end · Esc cancels",
            DrawTool::Polyline => {
                "Click connected vertices; click the first vertex to close · Esc ends"
            }
            DrawTool::Spline => {
                "Click control poles · Enter finishes · click the first pole to close · Esc cancels"
            }
            _ => "Click the first point, then the second point · Esc cancels",
        }
        .into();
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
            OperationRequest::Trim(request, policy) => multicad_solver::prepare_trim(
                session,
                session.revision(),
                actor,
                id,
                self.evaluated.as_ref(),
                &request,
                policy,
            )
            .map(|result| result.edit)
            .map_err(|e| e.to_string())?,
        };
        let ready = edit.preview.is_some();
        self.constraints.accept_edit(
            session,
            id,
            edit,
            &format!("Apply {}", tool.label().to_lowercase()),
        )?;
        self.clear_drawing_progress();
        self.operation_error = None;
        self.message = if ready {
            format!(
                "{} preview · Apply commits one undoable edit, Discard keeps the original",
                tool.label()
            )
        } else {
            format!(
                "{} preview has unresolved constraints · inspect diagnostics or Discard",
                tool.label()
            )
        };
        Ok(())
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
                self.message = format!(
                    "{} control poles · Enter finishes · click the first pole to close",
                    self.spline_poles.len()
                );
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
        let world_delta = self
            .basis(session.document(), session.revision())?
            .vector(self.delta);
        let delta = self.contexts.get(&id).map_or(world_delta, |pose| {
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
        if self.sketch_evaluation.pending() {
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
                    enabled: !self.has_pending_edits(),
                    allow_open_sketch: sketch_mode,
                };
                let events = self.tree.show(ui, session, &self.tree_selection, options);
                self.handle_tree_events(session, scene.as_deref(), events);
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
        egui::Panel::right("source-inspector")
            .default_size(250.)
            .resizable(true)
            .show(ui, |ui| {
                crate::controls::inspector_layout(ui);
                ui.heading("Inspector");
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("sketch-inspector-content")
                    .show(ui, |ui| {
                        if let Some(error)=&self.visibility_error {ui.colored_label(Color32::LIGHT_YELLOW,error);}
                        let hierarchy_tool=self.tree.tool_active();
                        if self.tree.tool_active() || !self.editing {
                            let options=crate::model_tree::TreeOptions{enabled:self.preview.is_none() && !self.constraints.dirty() && !self.repair.has_pending(),allow_open_sketch:sketch_mode};
                            let events=self.tree.inspector(ui,session,&self.tree_selection,options);
                            self.handle_tree_events(session,scene.as_deref(),events);
                        }
                        if hierarchy_tool {return;}
                        if let Some(error)=feed.error() {ui.colored_label(Color32::LIGHT_RED,error);if ui.button("Retry model update").on_hover_text("Rebuild the current source while retaining the previous visible model").clicked(){feed.retry();}}
                        else if feed.refining() {ui.small("Refining display…");}
                        else if feed.pending() {ui.small("Rebuilding model…");}
                        if self.sketch_evaluation.pending() {ui.small("Solving sketches…");}
                        if let Some(error)=self.sketch_evaluation.error() {ui.colored_label(Color32::LIGHT_RED,error);}
                        if let Some(failure)=self.selected.and_then(|id|self.sketch_evaluation.errors().get(&id)) {ui.colored_label(Color32::LIGHT_RED,&failure.message);}
                        if let Some(error)=feed.quality_error() {ui.colored_label(Color32::LIGHT_YELLOW,error);if ui.button("Retry display refinement").clicked(){feed.retry_quality();}}
                        if let Some(evaluation)=feed.references() {
                            self.repair.show(ui,session,evaluation);
                        }
                        if self.repair.has_pending() {self.tool=None;self.clear_drawing_progress();self.moving=false;self.cancel_move();}
                        self.solid.inspector(ui,session.document());
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
                            ui.small(tool.usage());
                            if ui.checkbox(&mut self.construction,"Construction geometry").on_hover_text("Construction curves constrain the sketch but are excluded from solid profiles").changed() {self.message=if self.construction {"Construction drawing on"}else{"Construction drawing off"}.into();}
                            if tool==DrawTool::Arc {ui.checkbox(&mut self.arc_clockwise,"Clockwise arc").on_hover_text("Direction around the sketch's positive normal");}
                            if tool==DrawTool::Spline {
                                ui.small(format!("{} control poles",self.spline_poles.len()));
                                if ui.add_enabled(self.can_finish_spline(),egui::Button::new("Finish spline · Enter")).on_hover_text("Create an open cubic spline as one undoable edit").clicked() && let Err(error)=self.finish_spline(session,false,Channel::Ui) {self.message=error;}
                                if ui.add_enabled(!self.spline_poles.is_empty(),egui::Button::new("Remove last pole")).on_hover_text("Remove only the last uncommitted control pole").clicked() {self.spline_poles.pop();}
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
                                        let response = ui.selectable_label(self.element == Some(g.id()) || self.constraints.selected(g.id()), label);
                                        if response.clicked() {
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
                                                self.element.is_some(),
                                                egui::Button::new("Delete selected element"),
                                            )
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
                                        ui.horizontal(|ui| {
                                            if ui
                                                .add_enabled(
                                                    self.preview.is_some(),
                                                    egui::Button::new("Apply move"),
                                                )
                                                .clicked()
                                                && let Err(e) = self.apply_move(session)
                                            {
                                                self.message = e;
                                            }
                                            if ui.button("Cancel move").clicked() {
                                                self.cancel_move();
                                            }
                                        });
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
        self.solid.highlight_reference(None);
        if self.repair.viewing_candidates()
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
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::from_rgb(21, 27, 35)))
            .show(ui, |ui| {
                self.canvas(ui, session, sketch_mode, scene.clone(), current)
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
        if let (Some(scene), Some(rect)) = (&scene, self.viewport_rect)
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
        if self.contexts.contains_key(&id)
            && self.display_scene.as_ref().is_some_and(|scene| {
                scene.stamp().document != doc.id() || scene.stamp().revision != revision
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
    fn display_frame(&self, id: EntityId, frame: Frame) -> Frame {
        if let Some(pose) = self.contexts.get(&id) {
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
    fn canvas(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        sketch_mode: bool,
        scene: Option<std::sync::Arc<crate::viewport::Scene>>,
        current: bool,
    ) {
        self.refresh_overlay_visibility(session.document(), session.revision());
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        self.viewport_rect = Some(rect);
        if response.dragged_by(egui::PointerButton::Secondary) {
            self.camera.orbit(ui.input(|i| i.pointer.delta()));
        }
        if response.dragged_by(egui::PointerButton::Middle) {
            self.camera.pan(ui.input(|i| i.pointer.delta()));
        }
        if response.hovered() {
            self.camera.zoom(ui.input(|i| i.smooth_scroll_delta.y));
        }
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
        if let Some(scene) = &scene {
            let selectable = current
                && !self.editing
                && !self.picking
                && !self.moving
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
        if scene.is_none()
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
        let mut points = vec![];
        if self.selected.is_none()
            && let Some(scene) = &scene
            && let Some([low, high]) = self
                .solid
                .selected_bounds(scene)
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
        for e in doc
            .entities()
            .filter(|e| !self.repair.viewing_candidates() && self.overlay_visible(e.id))
        {
            if let Ok((source, frame, _)) = self.read_sketch(doc, e.id, session.revision()) {
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
                        if self.solid.selected_id().is_none()
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
        if sketch_mode
            && self.editing
            && !self.has_pending_edits()
            && !self.moving
            && let (Some(id), Some(tool)) = (self.selected, self.tool)
            && let Ok((_, frame, _)) = self.read_sketch(session.document(), id, session.revision())
            && let Some(pointer) = response.hover_pos()
            && let Some(mut p) = self.camera.on_plane(pointer, rect, frame)
        {
            use multicad_core::sketch_draw::{DrawInput, DrawRequest, PointInput};
            let mut anchor = None;
            if self.snap {
                if let Ok((source, _, _)) =
                    self.read_solved_source(session.document(), id, session.revision())
                {
                    let nearest = source
                        .sketch
                        .points
                        .iter()
                        .filter_map(|point| {
                            let screen = project(frame.point(point.position_mm), &self.camera);
                            let distance = screen.distance(pointer);
                            (distance <= 8.).then_some((distance, point.id, point.position_mm))
                        })
                        .min_by(|a, b| a.0.total_cmp(&b.0));
                    if let Some((_, id, point)) = nearest {
                        anchor = Some(id);
                        p = point;
                    }
                }
                if anchor.is_none() {
                    p = p.map(|v| (v / self.spacing).round() * self.spacing);
                }
            }
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
            if tool == DrawTool::Spline {
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
                                self.message =
                                    "Choose the arc end · endpoint follows the chosen radius"
                                        .into();
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
                                self.message="Continue the polyline · click its first vertex to close · Esc ends".into();
                            }
                        }
                    } else {
                        self.first = Some(p);
                        self.first_anchor = anchor;
                    }
                }
            }
        }
        if sketch_mode
            && self.editing
            && self.tool.is_none()
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
            match hit {
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
        if self.moving && self.selected.is_some() {
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
        let mut spacing = self.spacing;
        let mut snap = self.snap;
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
                        ui.checkbox(&mut snap, "Snap sketch points to spacing");
                    },
                    |ui| {
                        self.solid.filter_control(ui);
                        self.solid
                            .section
                            .control(ui, scene.as_ref().and_then(|s| s.world_bounds()));
                        let transform = ui.add_enabled_ui(self.selected.is_some(),|ui|icons::compact_button(
                            ui,
                            Icon::Transform,
                            &format!(
                                "Transform gizmo · {} coordinates · right-click to choose axes",
                                coordinates.label()
                            ),
                        )).inner.on_disabled_hover_text("Select a sketch to transform");
                        if transform.clicked() {
                            moving = !moving;
                        }
                        transform.context_menu(|ui| {
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
                    self.camera = Camera {
                        target: [0.; 3],
                        ..Default::default()
                    };
                }
                if actions.fit || self.fit_requested {
                    self.fit_requested = false;
                    self.camera.fit(&points, rect);
                }
            });
        if self.constraints.dirty() && moving {
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
        self.spacing = spacing;
        self.snap = snap;
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
}

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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawTool {
    Line,
    Rectangle,
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
pub struct SketchEditor {
    pub selected: Option<EntityId>,
    pub editing: bool,
    pub picking: bool,
    pub tool: Option<DrawTool>,
    first: Option<[f64; 2]>,
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
    pub fn selected_solid(&self) -> Option<EntityId> {
        self.solid.selected_id()
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
        self.cancel_move();
        self.tool = None;
        self.first = None;
    }
    pub fn has_pending_edits(&self) -> bool {
        self.preview.is_some() || self.constraints.dirty() || self.repair.has_pending()
    }
    pub fn cancel(&mut self) {
        self.repair.cancel();
        self.solid.clear();
        self.moving = false;
        self.picking = false;
        self.plane_picker.cancel();
        self.tool = None;
        self.first = None;
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
    pub fn open(&mut self, doc: &Document, id: EntityId) -> Result<(), String> {
        Sketch::read(doc, id).map_err(|e| e.to_string())?;
        self.activate_sketch(id)
    }
    pub fn open_at(&mut self, doc: &Document, id: EntityId, revision: u64) -> Result<(), String> {
        self.occurrence = self.solid.selected_id().or(self.occurrence);
        self.refresh_contexts(doc, revision);
        let (_, frame, _) = self
            .read_sketch(doc, id, revision)
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
        self.moving = false;
        self.tool = Some(tool);
        self.first = None;
        self.message = "Click the first point, then the second point · Esc cancels".into();
        Ok(())
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
        let id = self.selected.ok_or("Select a sketch")?;
        let gid = if let Ok((mut source, _, _)) =
            self.read_source(session.document(), id, session.revision())
        {
            let gid = match tool {
                DrawTool::Line => {
                    let start = source.add_point(a);
                    let end = source.add_point(b);
                    source.add_line(start, end)
                }
                DrawTool::Rectangle => source.add_rectangle(a, b).map_err(|e| e.to_string())?,
            };
            let edit = multicad_solver::prepare_edit(
                session,
                session.revision(),
                Actor::local(channel),
                id,
                source,
            )
            .map_err(|e| e.to_string())?;
            let preview = edit.preview.ok_or_else(|| {
                format!("Sketch rejected: {:?}", edit.evaluation.diagnostics.state)
            })?;
            session.commit(preview).map_err(|e| e.to_string())?;
            gid
        } else {
            let gid = EntityId::new();
            let geometry = match tool {
                DrawTool::Line => Geometry::Line { id: gid, a, b },
                DrawTool::Rectangle => Geometry::Rectangle { id: gid, a, b },
            };
            let edits =
                sketch::append(session.document(), id, geometry).map_err(|e| e.to_string())?;
            commit(
                session,
                match tool {
                    DrawTool::Line => "Draw line",
                    DrawTool::Rectangle => "Draw rectangle",
                },
                edits,
                channel,
            )?;
            gid
        };
        self.first = None;
        self.element = Some(gid);
        self.message = "Geometry committed · Undo restores the previous sketch".into();
        Ok(())
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
            let edit = multicad_solver::prepare_edit(
                session,
                session.revision(),
                Actor::local(channel),
                id,
                source,
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
        self.constraints.begin_dimension(session, id, element)?;
        self.tool = None;
        self.first = None;
        self.moving = false;
        if let Some(value) = value {
            let (mut source, _, _) = self
                .read_source(session.document(), id, session.revision())
                .map_err(|e| e.to_string())?;
            source.add_constraint(self.constraints.dimension_kind(value, unit)?);
            let edit = multicad_solver::prepare_edit(
                session,
                session.revision(),
                Actor::local(channel),
                id,
                source,
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
                    self.read_sketch(doc, id, revision)
                        .map(|(_, f, _)| f)
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
        let edits = sketch::translate(session.document(), id, delta).map_err(|e| e.to_string())?;
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
        let mut scene = feed
            .visible()
            .filter(|s| s.stamp().document == session.document().id())
            .cloned();
        self.display_scene = scene.clone();
        self.refresh_contexts(session.document(), session.revision());
        if self.revision != session.revision() {
            self.cancel_move();
            self.first = None;
            self.revision = session.revision();
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
        egui::Panel::left("source-tree").default_size(205.).resizable(true).show(ui, |ui| {
            ui.heading("Parts & sketches");
            ui.separator();
            if let Some(scene)=&scene&&ui.add_enabled_ui(!self.has_pending_edits(),|ui|self.solid.tree(ui,session.document(),scene)).inner {self.selected=None;self.editing=false;self.tool=None;self.first=None;}
            let mut open = None;
            for e in session.document().entities() {
                if matches!(&e.data, EntityData::Sketch { definition, .. } if definition.type_id == sketch::TYPE || definition.type_id == multicad_core::constraint_sketch::source::TYPE) {
                    let response = ui.selectable_label(self.selected == Some(e.id), &e.name);
                    if response.clicked() {
                        if self.selected!=Some(e.id) && self.has_pending_edits(){self.message="Apply or discard pending edits before switching sketches".into();continue;}
                        if self.selected != Some(e.id) { self.editing = false; self.tool = None; self.first = None; }
                        self.selected = Some(e.id);
                        self.solid.clear();
                        self.element = None;
                        self.cancel_move();
                    }
                    if response.double_clicked() && sketch_mode { open = Some(e.id); }
                }
            }
            if let Some(id) = open && let Err(error) = self.open_at(session.document(), id,session.revision()) { self.message = error; }
            if sketch_mode && self.selected.is_some() && !self.editing && ui.button("Open selected sketch").clicked() {
                let id = self.selected.expect("selected");
                if let Err(error) = self.open_at(session.document(), id,session.revision()) { self.message = error; }
            }
        });
        egui::Panel::right("source-inspector")
            .default_size(250.)
            .resizable(true)
            .show(ui, |ui| {
                ui.heading("Inspector");
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("sketch-inspector-content")
                    .show(ui, |ui| {
                        if let Some(error)=feed.error() {ui.colored_label(Color32::LIGHT_RED,error);if ui.button("Retry model update").on_hover_text("Rebuild the current source while retaining the previous visible model").clicked(){feed.retry();}}
                        else if feed.refining() {ui.small("Refining display…");}
                        else if feed.pending() {ui.small("Rebuilding model…");}
                        if let Some(error)=feed.quality_error() {ui.colored_label(Color32::LIGHT_YELLOW,error);if ui.button("Retry display refinement").clicked(){feed.retry_quality();}}
                        if let Some(evaluation)=feed.references() {
                            self.repair.show(ui,session,evaluation);
                        }
                        if self.repair.has_pending() {self.tool=None;self.first=None;self.moving=false;self.cancel_move();}
                        self.solid.inspector(ui,session.document());
                        if !self.editing && !self.picking && !self.moving && !self.has_pending_edits() && self.solid.quick_actions(ui,session,feed.is_current(session.document().id(),session.revision())) {
                            self.fit_requested=true;
                        }
                        if self.picking && sketch_mode {
                            let scene = self.display_scene.clone();
                            let context = PlaneContext { document: session.document(), revision: session.revision(), scene: scene.as_deref().filter(|s| s.stamp().revision == session.revision()), report: self.evaluated.as_ref(), occurrence: self.occurrence };
                            let chosen = self.plane_picker.show_inspector(ui,context);
                            if self.plane_picker.take_cancel_requested() { self.cancel(); }
                            if let Some(preview) = chosen && let Err(error) = self.create_on_plane(session,preview,Channel::Ui) {self.message=error;}
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
                                            Geometry::Arc { center, a, .. } => format!(
                                                "Arc · R {:.2} mm",
                                                (a[0] - center[0]).hypot(a[1] - center[1])
                                            ),
                                        };
                                        if ui
                                            .selectable_label(self.element == Some(g.id()), label)
                                            .clicked()
                                        {
                                            self.element = Some(g.id());
                                            self.tool = None;
                                            self.first = None;
                                        }
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
                                        let before = session.revision();
                                        ui.add_enabled_ui(!self.repair.has_pending(),|ui|self.constraints.show(ui, session, id, self.element));
                                        if session.revision() != before {
                                            self.message =
                                                "Constraints applied · one undoable source edit"
                                                    .into();
                                        }
                                        if self.constraints.dirty() {
                                            self.tool = None;
                                            self.first = None;
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
                                }
                            }
                        }
                    });
            });
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
        let (source, mut frame, plane) = if let Some(report) = &self.evaluated {
            Sketch::read_evaluated(doc, id, revision, report)
        } else {
            Sketch::read(doc, id)
        }?;
        if let Some(pose) = self.contexts.get(&id) {
            frame = Frame {
                origin: pose.point(frame.origin),
                u: pose.vector(frame.u),
                v: pose.vector(frame.v),
                n: pose.vector(frame.n),
            };
        }
        Ok((source, frame, plane))
    }
    fn read_source(
        &self,
        doc: &Document,
        id: EntityId,
        revision: u64,
    ) -> Result<(Source, Frame, EntityId), multicad_core::document::DocumentError> {
        if let Some(report) = &self.evaluated {
            Source::read_evaluated(doc, id, revision, report)
        } else {
            Source::read(doc, id)
        }
    }
    fn canvas(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        sketch_mode: bool,
        scene: Option<std::sync::Arc<crate::viewport::Scene>>,
        current: bool,
    ) {
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
            self.solid.show(
                ui,
                &response,
                scene.clone(),
                self.camera,
                self.style,
                current && !self.editing && !self.picking && !self.moving,
            );
            if self.solid.selected_id().is_some() && !self.editing {
                self.selected = None;
            }
        }
        if self.picking && sketch_mode {
            let context = PlaneContext {
                document: session.document(),
                revision: session.revision(),
                scene: scene.as_deref().filter(|_| current),
                report: self.evaluated.as_ref(),
                occurrence: self.occurrence,
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
        for e in doc.entities().filter(|_| !self.repair.viewing_candidates()) {
            if let Ok((source, frame, _)) = self.read_sketch(doc, e.id, session.revision()) {
                let constrained = if let Some(report) = &self.evaluated {
                    Source::read_evaluated(doc, e.id, session.revision(), report).ok()
                } else {
                    Source::read(doc, e.id).ok()
                };
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
                    })
                    .map(|c| c.id)
                    .collect();
                for g in &source.geometry {
                    let selected = self.selected == Some(e.id);
                    let conflicted =
                        affected.contains(&g.id()) || conflicted_curves.contains(&g.id());
                    let color = if selected && conflicted {
                        Color32::LIGHT_RED
                    } else if selected
                        && (self.element == Some(g.id()) || self.constraints.selected(g.id()))
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
                            painter.line_segment(
                                segment,
                                Stroke::new(if selected { 2. } else { 1. }, color),
                            );
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
            if self.snap {
                p = p.map(|v| (v / self.spacing).round() * self.spacing);
            }
            let cursor = project(frame.point(p), &self.camera);
            painter.circle_stroke(cursor, 4., Stroke::new(1., Color32::WHITE));
            if let Some(a) = self.first {
                let g = match tool {
                    DrawTool::Line => Geometry::Line {
                        id: EntityId::new(),
                        a,
                        b: p,
                    },
                    DrawTool::Rectangle => Geometry::Rectangle {
                        id: EntityId::new(),
                        a,
                        b: p,
                    },
                };
                for [a, b] in g.segments() {
                    if let Some(segment) = self.camera.segment(frame.point(a), frame.point(b), rect)
                    {
                        painter.line_segment(segment, Stroke::new(1., Color32::LIGHT_YELLOW));
                    }
                }
            }
            if response.clicked() {
                if let Some(a) = self.first {
                    if let Err(e) = self.add(session, tool, a, p, Channel::Ui) {
                        self.message = e;
                    }
                } else {
                    self.first = Some(p);
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
            self.first = None;
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
    fn session() -> Session {
        Session::new(Document::new("Sketch tests").unwrap()).unwrap()
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
            dimension: multicad_core::constraint_sketch::Dimension {
                value: 90.,
                unit: "mm".into(),
            },
        });
        editor.constraints.prepare(&session, id, source);
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

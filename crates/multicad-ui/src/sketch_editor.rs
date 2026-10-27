//! Datum sketch interaction. All committed changes pass through the source Session.
use crate::{
    camera::Camera,
    icons::{self, Icon},
    navigation::{self, AxisView, DisplayStyle},
};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use multicad_core::{
    commands::{Actor, Channel, Prepared, Request, Session},
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
            style: DisplayStyle::Wire,
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
    pub fn has_move_preview(&self) -> bool {
        self.preview.is_some()
    }
    pub fn cancel(&mut self) {
        self.picking = false;
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
        self.cancel();
        self.editing = false;
        self.picking = true;
        self.message = "Select a datum plane in the viewer · Esc cancels".into();
    }
    pub fn create(
        &mut self,
        session: &mut Session,
        datum: Datum,
        channel: Channel,
    ) -> Result<(), String> {
        let count = session
            .document()
            .entities()
            .filter(|e| matches!(e.data, EntityData::Sketch { .. }))
            .count();
        let (id, edits) =
            sketch::create(datum, &format!("Sketch {}", count + 1)).map_err(|e| e.to_string())?;
        commit(session, "New datum sketch", edits, channel)?;
        self.open(session.document(), id)?;
        self.camera.orient(match datum {
            Datum::Xy => AxisView::PositiveZ,
            Datum::Xz => AxisView::NegativeY,
            Datum::Yz => AxisView::PositiveX,
        });
        self.camera.target = [0.; 3];
        self.message =
            "Sketch ready. Choose Line or Rectangle; coordinates are millimeters.".into();
        Ok(())
    }
    pub fn open(&mut self, doc: &Document, id: EntityId) -> Result<(), String> {
        Sketch::read(doc, id).map_err(|e| e.to_string())?;
        self.cancel();
        self.selected = Some(id);
        self.editing = true;
        self.element = None;
        Ok(())
    }
    pub fn finish(&mut self) {
        self.cancel();
        self.editing = false;
        self.moving = false;
        self.message = "Sketch saved in the document. Select it to inspect or reopen.".into();
    }
    pub fn set_tool(&mut self, tool: DrawTool) -> Result<(), String> {
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
        let id = self.selected.ok_or("Select a sketch")?;
        let gid = EntityId::new();
        let geometry = match tool {
            DrawTool::Line => Geometry::Line { id: gid, a, b },
            DrawTool::Rectangle => Geometry::Rectangle { id: gid, a, b },
        };
        let edits = sketch::append(session.document(), id, geometry).map_err(|e| e.to_string())?;
        commit(
            session,
            match tool {
                DrawTool::Line => "Draw line",
                DrawTool::Rectangle => "Draw rectangle",
            },
            edits,
            channel,
        )?;
        self.first = None;
        self.element = Some(gid);
        self.message = "Geometry committed · Undo restores the previous sketch".into();
        Ok(())
    }
    pub fn delete(&mut self, session: &mut Session, channel: Channel) -> Result<(), String> {
        let id = self.selected.ok_or("Select a sketch")?;
        let element = self
            .element
            .ok_or("Select a line or rectangle in the Inspector")?;
        let edits =
            sketch::remove_geometry(session.document(), id, element).map_err(|e| e.to_string())?;
        commit(session, "Delete sketch geometry", edits, channel)?;
        self.element = None;
        Ok(())
    }
    fn basis(&self, doc: &Document) -> Result<Frame, String> {
        match self.coordinates {
            Coordinates::World => Ok(Datum::Xy.frame()),
            Coordinates::Object => self
                .selected
                .ok_or_else(|| "Select a sketch".to_string())
                .and_then(|id| {
                    Sketch::read(doc, id)
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
        let id = self.selected.ok_or("Select a sketch to move")?;
        let delta = self.basis(session.document())?.vector(self.delta);
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
        let preview = self.preview.take().ok_or("Preview a move first")?;
        session.commit(preview).map_err(|e| e.to_string())?;
        self.delta = [0.; 3];
        self.message = "Moved sketch in one transaction".into();
        Ok(())
    }
    pub fn show(&mut self, ui: &mut egui::Ui, session: &mut Session, sketch_mode: bool) {
        if self.revision != session.revision() {
            self.cancel_move();
            self.first = None;
            self.revision = session.revision();
            if self
                .selected
                .is_some_and(|id| Sketch::read(session.document(), id).is_err())
            {
                self.selected = None;
                self.editing = false;
                self.tool = None;
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.cancel();
        }
        egui::Panel::left("source-tree").default_size(205.).resizable(true).show(ui, |ui| {
            ui.heading("Parts & sketches");
            ui.separator();
            let mut open = None;
            for e in session.document().entities() {
                if matches!(&e.data, EntityData::Sketch { definition, .. } if definition.type_id == sketch::TYPE) {
                    let response = ui.selectable_label(self.selected == Some(e.id), &e.name);
                    if response.clicked() {
                        if self.selected != Some(e.id) { self.editing = false; self.tool = None; self.first = None; }
                        self.selected = Some(e.id);
                        self.element = None;
                        self.cancel_move();
                    }
                    if response.double_clicked() && sketch_mode { open = Some(e.id); }
                }
            }
            if let Some(id) = open && let Err(error) = self.open(session.document(), id) { self.message = error; }
            if sketch_mode && self.selected.is_some() && !self.editing && ui.button("Open selected sketch").clicked() {
                let id = self.selected.expect("selected");
                if let Err(error) = self.open(session.document(), id) { self.message = error; }
            }
        });
        egui::Panel::right("source-inspector")
            .default_size(250.)
            .resizable(true)
            .show(ui, |ui| {
                ui.heading("Inspector");
                ui.separator();
                if let Some(id) = self.selected {
                    match Sketch::read(session.document(), id) {
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
                                        format!("Line · {:.2} mm", (b[0] - a[0]).hypot(b[1] - a[1]))
                                    }
                                    Geometry::Rectangle { a, b, .. } => format!(
                                        "Rectangle · {:.2} × {:.2}",
                                        (b[0] - a[0]).abs(),
                                        (b[1] - a[1]).abs()
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
                            if self.moving {
                                ui.separator();
                                ui.strong("Move sketch");
                                if matches!(self.coordinates, Coordinates::Custom(_)) {
                                    ui.horizontal(|ui| {
                                        ui.label("Custom axis angle");
                                        if ui
                                            .add(
                                                egui::DragValue::new(&mut self.custom_angle_deg)
                                                    .suffix("°")
                                                    .range(-360.0..=360.0),
                                            )
                                            .changed()
                                        {
                                            self.cancel_move();
                                        }
                                    });
                                }
                                ui.label(format!("{} coordinates · mm", self.coordinates.label()));
                                let mut changed = false;
                                for (i, label) in
                                    ["X / U", "Y / V", "Z / N"].into_iter().enumerate()
                                {
                                    ui.horizontal(|ui| {
                                        ui.label(label);
                                        changed |= ui
                                            .add(
                                                egui::DragValue::new(&mut self.delta[i]).speed(0.5),
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
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::from_rgb(21, 27, 35)))
            .show(ui, |ui| self.canvas(ui, session, sketch_mode));
    }
    fn canvas(&mut self, ui: &mut egui::Ui, session: &mut Session, sketch_mode: bool) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
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
            let step = self.spacing.max(0.01);
            for i in -50..=50 {
                let value = i as f64 * step;
                for (a, b) in [
                    ([value, -50. * step, 0.], [value, 50. * step, 0.]),
                    ([-50. * step, value, 0.], [50. * step, value, 0.]),
                ] {
                    if let Some(segment) = self.camera.segment(a, b, rect) {
                        painter.line_segment(segment, Stroke::new(0.5, Color32::from_gray(47)));
                    }
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
        if self.picking && sketch_mode {
            let hovered = response.hover_pos().and_then(|pointer| {
                Datum::ALL
                    .into_iter()
                    .filter_map(|datum| {
                        let frame = datum.frame();
                        let point = self.camera.on_plane(pointer, rect, frame)?;
                        if point.iter().any(|v| !(10.0..=80.0).contains(v)) {
                            return None;
                        }
                        let depth = self
                            .camera
                            .project(frame.point(point).map(|v| v as f32), rect)[2];
                        Some((datum, depth))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(datum, _)| datum)
            });
            let mut chosen = None;
            for (datum, color) in [
                (Datum::Xy, Color32::from_rgb(90, 150, 220)),
                (Datum::Xz, Color32::from_rgb(90, 190, 125)),
                (Datum::Yz, Color32::from_rgb(220, 100, 100)),
            ] {
                let frame = datum.frame();
                let corners = [[10., 10.], [80., 10.], [80., 80.], [10., 80.]]
                    .map(|p| project(frame.point(p), &self.camera));
                let center = corners.iter().fold(Vec2::ZERO, |a, p| a + p.to_vec2()) / 4.;
                let label = Pos2::new(center.x, center.y);
                let r = ui.interact(
                    Rect::from_center_size(label, Vec2::new(54., 28.)),
                    ui.id().with(datum.label()),
                    Sense::click(),
                );
                painter.add(egui::Shape::convex_polygon(
                    corners.to_vec(),
                    color.gamma_multiply(if r.hovered() || hovered == Some(datum) {
                        0.45
                    } else {
                        0.2
                    }),
                    Stroke::new(1.5, color),
                ));
                painter.rect_filled(r.rect, 4, color.gamma_multiply(0.7));
                painter.text(
                    label,
                    Align2::CENTER_CENTER,
                    datum.label(),
                    FontId::proportional(13.),
                    Color32::WHITE,
                );
                r.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        true,
                        format!("Choose {} sketch plane", datum.label()),
                    )
                });
                if r.clicked() {
                    chosen = Some(datum);
                }
            }
            if let Some(datum) = chosen.or_else(|| if response.clicked() { hovered } else { None })
                && let Err(e) = self.create(session, datum, Channel::Ui)
            {
                self.message = e;
            }
        }
        let doc = self
            .preview
            .as_ref()
            .map(|p| p.document())
            .unwrap_or(session.document());
        let mut points = vec![];
        for e in doc.entities() {
            if let Ok((source, frame, _)) = Sketch::read(doc, e.id) {
                for g in &source.geometry {
                    let selected = self.selected == Some(e.id);
                    let color = if selected && self.element == Some(g.id()) {
                        Color32::from_rgb(255, 195, 80)
                    } else if selected {
                        Color32::from_rgb(100, 225, 220)
                    } else {
                        Color32::from_rgb(148, 163, 181)
                    };
                    for [a, b] in g.segments() {
                        let a = frame.point(a);
                        let b = frame.point(b);
                        points.extend([a.map(|v| v as f32), b.map(|v| v as f32)]);
                        if let Some(segment) = self.camera.segment(a, b, rect) {
                            painter.line_segment(
                                segment,
                                Stroke::new(if selected { 2. } else { 1. }, color),
                            );
                        }
                    }
                }
            }
        }
        if sketch_mode
            && self.editing
            && !self.moving
            && let (Some(id), Some(tool)) = (self.selected, self.tool)
            && let Ok((_, frame, _)) = Sketch::read(session.document(), id)
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
        painter.text(
            rect.left_top() + Vec2::new(16., 14.),
            Align2::LEFT_TOP,
            &self.message,
            FontId::proportional(12.),
            Color32::from_gray(175),
        );
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
                        if icons::compact_button(ui, Icon::Link, "Toggle transform gizmo").clicked()
                        {
                            moving = !moving;
                        }
                        egui::ComboBox::from_id_salt("coordinates")
                            .selected_text(coordinates.label())
                            .width(82.)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut coordinates, Coordinates::World, "World");
                                ui.selectable_value(
                                    &mut coordinates,
                                    Coordinates::Object,
                                    "Object",
                                );
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
                if actions.fit {
                    self.camera.fit(&points, rect);
                }
            });
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
        let Ok((_, frame, _)) = Sketch::read(session.document(), id) else {
            return;
        };
        let Ok(basis) = self.basis(session.document()) else {
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
            ctx.run_ui(input(events), |ui| editor.canvas(ui, &mut s, true))
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
        assert_eq!(source.geometry.len(), 1);
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
            ctx.run_ui(input(events), |ui| editor.canvas(ui, &mut session, true))
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
}

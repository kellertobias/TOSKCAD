//! Viewport placement for text notes and sheet drafting. A tool starts a
//! pending placement that the sheet shows before anything enters the page
//! draft: clicks place points (snapped to existing drafting points, or with
//! Shift kept horizontal/vertical), Escape or Cancel discards the pending
//! geometry, and completion adds it to the draft like any other annotation,
//! so Apply commits it as one undoable page edit. Committed drafting shows
//! drag handles synchronized with the Inspector's numeric fields.
use super::*;
use multicad_core::drawings::{Annotation, AnnotationKind, DraftGeometry};

/// Snap radius on paper.
pub const SNAP_MM: f64 = 2.0;
/// Unsnapped points are rounded to this paper grid.
pub const GRID_MM: f64 = 0.5;

#[derive(Clone, Debug, PartialEq)]
pub enum Placement {
    /// A new text note: not on the page until "Add note".
    Note { position: [f64; 2], text: String },
    /// Sheet drafting points collected so far.
    Draft {
        tool: DrawingTool,
        points: Vec<[f64; 2]>,
    },
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    let d = sub(a, b);
    d[0].hypot(d[1])
}
fn angle_deg(center: [f64; 2], p: [f64; 2]) -> f64 {
    let d = sub(p, center);
    d[1].atan2(d[0]).to_degrees()
}
fn on_circle(center: [f64; 2], r: f64, deg: f64) -> [f64; 2] {
    let t = deg.to_radians();
    [center[0] + r * t.cos(), center[1] + r * t.sin()]
}

/// Points a drafting tool needs (hatch: at least three, finished explicitly).
pub fn required_points(tool: DrawingTool) -> Option<usize> {
    match tool {
        DrawingTool::DraftLine | DrawingTool::DraftCircle => Some(2),
        DrawingTool::DraftArc | DrawingTool::DraftDimension => Some(3),
        _ => None,
    }
}
pub fn is_drafting(tool: DrawingTool) -> bool {
    matches!(
        tool,
        DrawingTool::DraftLine
            | DrawingTool::DraftCircle
            | DrawingTool::DraftArc
            | DrawingTool::Hatch
            | DrawingTool::DraftDimension
    )
}
/// What to click next.
pub fn prompt(tool: DrawingTool, placed: usize) -> &'static str {
    match (tool, placed) {
        (DrawingTool::DraftLine, 0) => "Sheet line: click the start point",
        (DrawingTool::DraftLine, _) => {
            "Sheet line: click the end point (Shift: horizontal/vertical)"
        }
        (DrawingTool::DraftCircle, 0) => "Sheet circle: click the center",
        (DrawingTool::DraftCircle, _) => "Sheet circle: click a point on the circle",
        (DrawingTool::DraftArc, 0) => "Sheet arc: click the center",
        (DrawingTool::DraftArc, 1) => "Sheet arc: click the start point (sets the radius)",
        (DrawingTool::DraftArc, _) => "Sheet arc: click the end point",
        (DrawingTool::Hatch, n) if n < 3 => "Hatch: click the boundary corners",
        (DrawingTool::Hatch, _) => {
            "Hatch: click more corners, then click the first corner or press Enter to finish"
        }
        (DrawingTool::DraftDimension, 0) => "Sheet dimension: click the first point",
        (DrawingTool::DraftDimension, 1) => "Sheet dimension: click the second point",
        (DrawingTool::DraftDimension, _) => "Sheet dimension: click where the dimension line goes",
        _ => "Click the sheet",
    }
}
/// The annotation the points describe, if they are enough and valid.
pub fn build(tool: DrawingTool, points: &[[f64; 2]]) -> Option<AnnotationKind> {
    let draft = |geometry, line_width_mm| AnnotationKind::Draft {
        geometry,
        line_width_mm,
    };
    match (tool, points) {
        (DrawingTool::DraftLine, [a, b, ..]) if dist(*a, *b) > 1e-6 => {
            Some(draft(DraftGeometry::Line { a: *a, b: *b }, 0.35))
        }
        (DrawingTool::DraftCircle, [c, p, ..]) if dist(*c, *p) >= 0.01 => Some(draft(
            DraftGeometry::Circle {
                center: *c,
                radius_mm: dist(*c, *p),
            },
            0.35,
        )),
        (DrawingTool::DraftArc, [c, s, rest @ ..]) if dist(*c, *s) >= 0.01 => {
            let start = angle_deg(*c, *s);
            let sweep = match rest.first() {
                Some(e) => (angle_deg(*c, *e) - start).rem_euclid(360.),
                None => 90.,
            };
            let sweep = if sweep < 1e-6 { 360. } else { sweep };
            Some(draft(
                DraftGeometry::Arc {
                    center: *c,
                    radius_mm: dist(*c, *s),
                    start_deg: start,
                    sweep_deg: sweep,
                },
                0.35,
            ))
        }
        (DrawingTool::Hatch, boundary) if boundary.len() >= 3 => Some(draft(
            DraftGeometry::Hatch {
                boundary: boundary.to_vec(),
                angle_deg: 45.,
                spacing_mm: 2.,
            },
            0.18,
        )),
        (DrawingTool::DraftDimension, [a, b, rest @ ..]) if dist(*a, *b) > 1e-6 => {
            let along = sub(*b, *a);
            let l = along[0].hypot(along[1]);
            let normal = [along[1] / l, -along[0] / l];
            let offset = rest.first().map_or(6., |o| {
                let d = sub(*o, *a);
                d[0] * normal[0] + d[1] * normal[1]
            });
            Some(AnnotationKind::DraftDimension {
                a: *a,
                b: *b,
                offset_mm: offset,
                precision: 1,
            })
        }
        _ => None,
    }
}
/// Editable defining points of committed sheet drafting.
pub fn handles(kind: &AnnotationKind) -> Vec<[f64; 2]> {
    match kind {
        AnnotationKind::Draft { geometry, .. } => match geometry {
            DraftGeometry::Line { a, b } => vec![*a, *b],
            DraftGeometry::Circle { center, radius_mm } => {
                vec![*center, [center[0] + radius_mm, center[1]]]
            }
            DraftGeometry::Arc {
                center,
                radius_mm,
                start_deg,
                sweep_deg,
            } => vec![
                *center,
                on_circle(*center, *radius_mm, *start_deg),
                on_circle(*center, *radius_mm, start_deg + sweep_deg),
            ],
            DraftGeometry::Hatch { boundary, .. } => boundary.clone(),
        },
        AnnotationKind::DraftDimension {
            a, b, offset_mm, ..
        } => {
            let along = sub(*b, *a);
            let l = along[0].hypot(along[1]).max(1e-9);
            let n = [along[1] / l, -along[0] / l];
            let mid = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
            vec![
                *a,
                *b,
                [mid[0] + n[0] * offset_mm, mid[1] + n[1] * offset_mm],
            ]
        }
        _ => vec![],
    }
}
/// Move one handle. Moving a circle or arc center moves the whole curve;
/// the other handles change radius, angles or the dimension offset.
pub fn set_handle(kind: &mut AnnotationKind, index: usize, p: [f64; 2]) {
    match kind {
        AnnotationKind::Draft { geometry, .. } => match geometry {
            DraftGeometry::Line { a, b } => *[a, b][index.min(1)] = p,
            DraftGeometry::Circle { center, radius_mm } => match index {
                0 => *center = p,
                _ => *radius_mm = dist(*center, p).max(0.01),
            },
            DraftGeometry::Arc {
                center,
                radius_mm,
                start_deg,
                sweep_deg,
            } => match index {
                0 => *center = p,
                1 => {
                    let end = *start_deg + *sweep_deg;
                    *radius_mm = dist(*center, p).max(0.01);
                    *start_deg = angle_deg(*center, p);
                    let s = (end - *start_deg).rem_euclid(360.);
                    *sweep_deg = if s < 1e-6 { 360. } else { s };
                }
                _ => {
                    let s = (angle_deg(*center, p) - *start_deg).rem_euclid(360.);
                    *sweep_deg = if s < 1e-6 { 360. } else { s };
                }
            },
            DraftGeometry::Hatch { boundary, .. } => {
                if let Some(v) = boundary.get_mut(index) {
                    *v = p;
                }
            }
        },
        AnnotationKind::DraftDimension {
            a, b, offset_mm, ..
        } => match index {
            0 => *a = p,
            1 => *b = p,
            _ => {
                if let Some(AnnotationKind::DraftDimension { offset_mm: o, .. }) =
                    build(DrawingTool::DraftDimension, &[*a, *b, p])
                {
                    *offset_mm = o;
                }
            }
        },
        _ => {}
    }
}

impl DrawingEditor {
    pub fn placement(&self) -> Option<&Placement> {
        self.placement.as_ref()
    }
    pub fn placing(&self) -> bool {
        self.placement.is_some()
    }
    /// The pending note's text, for the Inspector and automation.
    pub fn pending_note_text(&mut self) -> Option<&mut String> {
        match &mut self.placement {
            Some(Placement::Note { text, .. }) => Some(text),
            _ => None,
        }
    }
    /// Start a pending note at a visible default position and focus its text.
    pub(super) fn begin_note(&mut self) -> Result<(), String> {
        let page = self.draft.as_ref().ok_or("Add a page first")?;
        let margin = page.settings.margin_mm;
        self.selected_view = None;
        self.selected_annotation = None;
        self.section_pick = None;
        self.placement = Some(Placement::Note {
            position: [margin + 5., margin + 5.],
            text: String::new(),
        });
        self.focus_note = true;
        Ok(())
    }
    pub(super) fn begin_draft(&mut self, tool: DrawingTool) -> Result<(), String> {
        self.draft.as_ref().ok_or("Add a page first")?;
        self.selected_view = None;
        self.selected_annotation = None;
        self.section_pick = None;
        self.placement = Some(Placement::Draft {
            tool,
            points: vec![],
        });
        Ok(())
    }
    pub fn placement_prompt(&self) -> Option<String> {
        Some(match self.placement.as_ref()? {
            Placement::Note { .. } => {
                "New text note: type its text, click the sheet to place it, then Add note. Escape cancels."
                    .into()
            }
            Placement::Draft { tool, points } => {
                format!("{}. Escape cancels.", prompt(*tool, points.len()))
            }
        })
    }
    pub fn cancel_placement(&mut self) {
        self.placement = None;
        self.focus_note = false;
    }
    pub(crate) fn cancel_canvas_tool(&mut self) -> bool {
        let section = self.section_pick.take().is_some();
        let placement = self.placing();
        self.cancel_placement();
        section || placement
    }
    /// Drafting points on the page and pending, for snapping.
    fn snap_targets(&self) -> Vec<[f64; 2]> {
        let mut targets: Vec<_> = self
            .draft
            .iter()
            .flat_map(|p| &p.annotations)
            .flat_map(|a| handles(&a.kind))
            .collect();
        if let Some(Placement::Draft { points, .. }) = &self.placement {
            targets.extend(points.iter().copied());
        }
        targets
    }
    /// Snap a raw paper point: to a drafting point within `SNAP_MM`, else
    /// (with `ortho`) horizontal/vertical from the previous point, then to the
    /// paper grid, and always inside the printable area.
    pub fn snap(&self, raw: [f64; 2], ortho: bool) -> [f64; 2] {
        let nearest = self
            .snap_targets()
            .into_iter()
            .filter(|t| dist(*t, raw) <= SNAP_MM)
            .min_by(|a, b| dist(*a, raw).total_cmp(&dist(*b, raw)));
        let mut p = nearest.unwrap_or_else(|| {
            let mut p = raw.map(|v| (v / GRID_MM).round() * GRID_MM);
            if ortho
                && let Some(Placement::Draft { points, .. }) = &self.placement
                && let Some(last) = points.last()
            {
                if (p[0] - last[0]).abs() >= (p[1] - last[1]).abs() {
                    p[1] = last[1];
                } else {
                    p[0] = last[0];
                }
            }
            p
        });
        if let Some(page) = &self.draft {
            let [w, h] = page.settings.dimensions();
            let m = page.settings.margin_mm;
            p = [p[0].clamp(m, w - m - 0.01), p[1].clamp(m, h - m - 0.01)];
        }
        p
    }
    /// A click on the sheet while placing (raw paper mm).
    pub fn placement_click(&mut self, raw: [f64; 2], ortho: bool) -> Result<(), String> {
        let point = self.snap(raw, ortho);
        match self.placement.as_mut().ok_or("Nothing is being placed")? {
            Placement::Note { position, .. } => {
                *position = point;
                self.focus_note = true;
                Ok(())
            }
            Placement::Draft { tool, points } => {
                let tool = *tool;
                if tool == DrawingTool::Hatch
                    && points.len() >= 3
                    && dist(points[0], point) <= SNAP_MM
                {
                    return self.finish_placement();
                }
                if points.last().is_some_and(|l| dist(*l, point) < 1e-6) {
                    return Err("Click a different point".into());
                }
                points.push(point);
                if required_points(tool) == Some(points.len()) {
                    return self.finish_placement();
                }
                Ok(())
            }
        }
    }
    /// The pending annotation as it would be with the pointer at `pointer`.
    pub fn placement_preview(&self, pointer: Option<[f64; 2]>) -> Option<AnnotationKind> {
        let Some(Placement::Draft { tool, points }) = &self.placement else {
            return None;
        };
        let mut points = points.clone();
        points.extend(pointer);
        build(*tool, &points)
    }
    /// Complete the pending note (needs text) or hatch (needs three corners).
    pub fn finish_placement(&mut self) -> Result<(), String> {
        match self.placement.clone().ok_or("Nothing is being placed")? {
            Placement::Note { position, text } => {
                if text.trim().is_empty() {
                    return Err("Type the note text before adding it".into());
                }
                let page = self.draft.as_mut().ok_or("Add a page first")?;
                page.notes.push(TextNote {
                    id: EntityId::new(),
                    position_mm: position,
                    text,
                });
            }
            Placement::Draft { tool, points } => {
                let kind = build(tool, &points).ok_or(match tool {
                    DrawingTool::Hatch => "A hatch needs at least three corners",
                    _ => "Place the remaining points first",
                })?;
                let page = self.draft.as_mut().ok_or("Add a page first")?;
                let annotation = Annotation {
                    id: EntityId::new(),
                    kind,
                };
                if let Err(error) = annotation.validate(page) {
                    return Err(error.to_string());
                }
                let id = annotation.id;
                page.annotations.push(annotation);
                self.selected_annotation = Some(id);
            }
        }
        self.placement = None;
        self.focus_note = false;
        Ok(())
    }
    /// Drag handle `index` of the selected drafting annotation (raw paper mm).
    pub fn drag_handle(&mut self, index: usize, raw: [f64; 2], ortho: bool) -> Result<(), String> {
        let id = self.selected_annotation.ok_or("Select sheet drafting")?;
        // Do not snap a handle to its own current position.
        let point = {
            let page = self.draft.as_ref().ok_or("Add a page first")?;
            let own = page
                .annotations
                .iter()
                .find(|a| a.id == id)
                .map(|a| handles(&a.kind))
                .unwrap_or_default();
            let snapped = self.snap(raw, ortho);
            if own.get(index).is_some_and(|h| dist(*h, snapped) < 1e-9)
                && dist(snapped, raw) > GRID_MM
            {
                raw.map(|v| (v / GRID_MM).round() * GRID_MM)
            } else {
                snapped
            }
        };
        let page = self.draft.as_mut().ok_or("Add a page first")?;
        let annotation = page
            .annotations
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or("The annotation is missing")?;
        set_handle(&mut annotation.kind, index, point);
        Ok(())
    }

    /// The pending note editor at the top of the Inspector.
    pub(super) fn note_inspector(&mut self, ui: &mut egui::Ui) {
        let focus = std::mem::take(&mut self.focus_note);
        let Some(Placement::Note { position, text }) = &mut self.placement else {
            return;
        };
        ui.heading("New text note");
        ui.small("Type the text, click the sheet to place it, then Add note. Escape or Cancel discards it.");
        let edit = ui.add(
            egui::TextEdit::multiline(text)
                .hint_text("Note text")
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        if focus {
            edit.request_focus();
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("X / Y mm");
            for value in position.iter_mut() {
                ui.add(egui::DragValue::new(value).speed(1.));
            }
        });
        let ready = !text.trim().is_empty();
        let mut add = false;
        let mut cancel = false;
        ui.horizontal_wrapped(|ui| {
            add = ui
                .add_enabled(ready, egui::Button::new("Add note"))
                .on_hover_text("Add this note to the page draft; Apply page commits it")
                .on_disabled_hover_text("Type the note text first")
                .clicked();
            cancel = ui
                .button("Cancel")
                .on_hover_text("Discard this pending note")
                .clicked();
        });
        if add && let Err(error) = self.finish_placement() {
            self.error = Some(error);
        }
        if cancel {
            self.cancel_placement();
        }
        ui.separator();
    }
    /// Pending drafting guidance at the top of the Inspector.
    pub(super) fn draft_placement_inspector(&mut self, ui: &mut egui::Ui) {
        let Some(Placement::Draft { tool, points }) = &self.placement else {
            return;
        };
        let (tool, count) = (*tool, points.len());
        ui.heading(tool.label());
        ui.label(prompt(tool, count));
        ui.small(format!(
            "Points snap to drafting points within {SNAP_MM} mm, otherwise to a {GRID_MM} mm grid. Shift keeps lines horizontal or vertical."
        ));
        let mut finish = false;
        let mut cancel = false;
        ui.horizontal_wrapped(|ui| {
            if tool == DrawingTool::Hatch {
                finish = ui
                    .add_enabled(count >= 3, egui::Button::new("Finish hatch"))
                    .on_hover_text("Close the boundary and add the hatch to the page draft")
                    .clicked();
            }
            cancel = ui
                .button("Cancel")
                .on_hover_text("Discard the pending geometry (Escape)")
                .clicked();
        });
        if finish && let Err(error) = self.finish_placement() {
            self.error = Some(error);
        }
        if cancel {
            self.cancel_placement();
        }
        ui.separator();
    }

    /// Sheet interaction while placing, and handles on selected drafting.
    /// Drawn and sensed on top of the canvas, after it.
    pub(super) fn placement_overlay(&mut self, ui: &mut egui::Ui, paper: egui::Rect, scale: f32) {
        let to_paper = |p: egui::Pos2| {
            let mm = (p - paper.min) / scale;
            [mm.x as f64, mm.y as f64]
        };
        let to_screen = |p: [f64; 2]| paper.min + egui::vec2(p[0] as f32, p[1] as f32) * scale;
        let accent = egui::Color32::from_rgb(30, 110, 180);
        let painter = ui.painter().with_clip_rect(paper);
        if self.placing()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.cancel_placement();
        }
        if matches!(
            self.placement,
            Some(Placement::Draft {
                tool: DrawingTool::Hatch,
                ..
            })
        ) && !ui.ctx().text_edit_focused()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
            && let Err(error) = self.finish_placement()
        {
            self.error = Some(error);
        }
        if self.placing() {
            let response = ui
                .interact(
                    paper,
                    ui.id().with("drawing-placement"),
                    egui::Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::Crosshair);
            let ortho = ui.input(|i| i.modifiers.shift);
            let hover = response.hover_pos().map(|p| self.snap(to_paper(p), ortho));
            if let Some(h) = hover {
                let s = to_screen(h);
                painter.line_segment(
                    [s - egui::vec2(6., 0.), s + egui::vec2(6., 0.)],
                    egui::Stroke::new(1., accent),
                );
                painter.line_segment(
                    [s - egui::vec2(0., 6.), s + egui::vec2(0., 6.)],
                    egui::Stroke::new(1., accent),
                );
            }
            match &self.placement {
                Some(Placement::Note { position, text }) => {
                    let s = to_screen(*position);
                    let shown = if text.trim().is_empty() {
                        "New note (type in the Inspector)"
                    } else {
                        text.lines().next().unwrap_or_default()
                    };
                    let galley = painter.layout_no_wrap(
                        shown.to_owned(),
                        egui::FontId::monospace(drawings::FONT_MM as f32 * scale),
                        accent,
                    );
                    let rect = egui::Rect::from_min_size(s, galley.size()).expand(2.);
                    painter.extend(egui::Shape::dashed_line(
                        &[
                            rect.left_top(),
                            rect.right_top(),
                            rect.right_bottom(),
                            rect.left_bottom(),
                            rect.left_top(),
                        ],
                        egui::Stroke::new(1., accent),
                        4.,
                        3.,
                    ));
                    painter.galley(s, galley, accent);
                }
                Some(Placement::Draft { points, .. }) => {
                    for p in points {
                        painter.circle_filled(to_screen(*p), 3., accent);
                    }
                    if let Some(kind) = self.placement_preview(hover) {
                        let page = self.draft.as_ref();
                        let g = match &kind {
                            AnnotationKind::DraftDimension {
                                a,
                                b,
                                offset_mm,
                                precision,
                            } => page.map(|page| {
                                drawings::draft_dimension(page, *a, *b, *offset_mm, *precision)
                            }),
                            _ => Some(drawings::sheet_graphic(&Annotation {
                                id: EntityId::new(),
                                kind,
                            })),
                        };
                        if let Some(g) = g {
                            preview_graphic(&painter, &g, to_screen, scale, accent);
                        }
                    }
                }
                None => {}
            }
            if let Some(prompt) = self.placement_prompt() {
                painter.text(
                    paper.left_top() + egui::vec2(8., 8.),
                    egui::Align2::LEFT_TOP,
                    prompt,
                    egui::FontId::proportional(12.),
                    accent,
                );
            }
            if response.clicked()
                && let Some(p) = response.interact_pointer_pos()
                && let Err(error) = self.placement_click(to_paper(p), ortho)
            {
                self.error = Some(error);
            }
            return;
        }
        // Handles on the selected sheet drafting.
        let Some(id) = self.selected_annotation else {
            return;
        };
        let Some(points) = self
            .draft
            .as_ref()
            .and_then(|p| p.annotations.iter().find(|a| a.id == id))
            .map(|a| handles(&a.kind))
        else {
            return;
        };
        let ortho = ui.input(|i| i.modifiers.shift);
        for (index, p) in points.iter().enumerate() {
            let s = to_screen(*p);
            let rect = egui::Rect::from_center_size(s, egui::vec2(9., 9.));
            let response = ui
                .interact(
                    rect,
                    ui.id().with(("drafting-handle", id, index)),
                    egui::Sense::drag(),
                )
                .on_hover_cursor(egui::CursorIcon::Grab)
                .on_hover_text("Drag to adjust; the Inspector fields update");
            painter.rect_filled(rect.shrink(1.), 1, egui::Color32::WHITE);
            painter.rect_stroke(
                rect.shrink(1.),
                1,
                egui::Stroke::new(1.2, accent),
                egui::StrokeKind::Inside,
            );
            if response.dragged_by(egui::PointerButton::Primary)
                && let Some(pointer) = response.interact_pointer_pos()
                && let Err(error) = self.drag_handle(index, to_paper(pointer), ortho)
            {
                self.error = Some(error);
            }
        }
    }
}

fn preview_graphic(
    painter: &egui::Painter,
    g: &drawings::AnnotationGraphic,
    to: impl Fn([f64; 2]) -> egui::Pos2,
    scale: f32,
    color: egui::Color32,
) {
    let stroke = egui::Stroke::new(1.2, color);
    for [a, b] in &g.lines {
        painter.extend(egui::Shape::dashed_line(&[to(*a), to(*b)], stroke, 5., 3.));
    }
    for (center, r, start, end) in &g.arcs {
        let points: Vec<_> = (0..=48)
            .map(|i| {
                let t = start + (end - start) * i as f64 / 48.;
                to([center[0] + r * t.cos(), center[1] + r * t.sin()])
            })
            .collect();
        painter.extend(egui::Shape::dashed_line(&points, stroke, 5., 3.));
    }
    if !g.text.is_empty() {
        painter.text(
            to(g.text_mm),
            egui::Align2::LEFT_BOTTOM,
            &g.text,
            egui::FontId::monospace(drawings::ANNOTATION_TEXT_MM as f32 * scale),
            color,
        );
    }
}

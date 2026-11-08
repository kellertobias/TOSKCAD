//! Viewport arrow (Extrude) and angular handle (Revolve) with an adjacent
//! numeric field. The pure drag/type/Enter rules live in `value_entry`; this
//! host turns their effects into native previews and at most one commit
//! through the ordinary transactional Apply.
use super::super::{
    gizmo::math::{Point, add, mul},
    operation_guide::{ProfileGuide, RevolveRing, extrude_axis},
    value_entry::{AngleTracker, EntryEffect, EntryInput, ValueEntry, ValueKind},
};
use super::*;
use crate::camera::Camera;
use egui::{Color32, Pos2, Rect, Stroke};

pub struct OperationViewport {
    pub camera: Camera,
    pub rect: Rect,
    /// The canvas owns the pointer (not a popup or camera control).
    pub start_allowed: bool,
}

#[derive(Clone, Copy)]
enum Handle {
    Linear { start: Point, direction: Point },
    Angular { ring: RevolveRing },
}
pub(super) struct OperationDrag {
    handle: Handle,
    camera: Camera,
    rect: Rect,
    tracker: AngleTracker,
}
#[derive(Default)]
pub(super) struct OperationState {
    pub guide: Option<ProfileGuide>,
    pub entry: Option<ValueEntry>,
    pub drag: Option<OperationDrag>,
    pub confirm: Option<FormStamp>,
    pub touched: bool,
    pub auto_previewed: Option<FormStamp>,
}

fn screen(camera: Camera, rect: Rect, p: Point) -> Option<Pos2> {
    let s = camera.project(p.map(|v| v as f32), rect);
    (s.iter().all(|v| v.is_finite()) && (!camera.perspective || s[2] > 0.1))
        .then(|| Pos2::new(s[0], s[1]))
}
fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let d = b - a;
    let t = ((p - a).dot(d) / d.length_sq().max(1e-8)).clamp(0., 1.);
    p.distance(a + d * t)
}
impl Handle {
    fn coordinate(
        &self,
        camera: Camera,
        rect: Rect,
        pointer: Pos2,
        tracker: Option<&mut AngleTracker>,
    ) -> Option<f64> {
        let ray = camera.ray(pointer, rect)?;
        match *self {
            Self::Linear { start, direction } => {
                super::super::gizmo::math::axis_parameter(ray, start, direction)
            }
            Self::Angular { ring } => {
                let point = super::super::gizmo::math::plane_point(ray, ring.center, ring.axis)?;
                let angle = ring.angle_of(point)?;
                Some(match tracker {
                    Some(tracker) => tracker.update(angle),
                    None => angle,
                })
            }
        }
    }
}

impl ModelToolController {
    /// Display guide for the accepted profiles, set by the host after opening
    /// Extrude or Revolve. Revolve also lists its straight lines as axis choices.
    pub fn set_profile_guide(&mut self, guide: ProfileGuide) {
        if let Some(ModelForm::Revolve(panel)) = &mut self.form {
            panel.set_axis_lines(guide.lines.clone());
        }
        self.operation.guide = Some(guide);
    }
    /// Extrude (new feature or feature edit) or Revolve owns a viewport gizmo.
    pub fn operation_gizmo_active(&self) -> bool {
        self.operation.guide.is_some()
            && matches!(
                self.form,
                Some(ModelForm::Extrude(_) | ModelForm::Revolve(_))
            )
    }
    /// True while a drag, typed value or Enter confirmation owns the keyboard.
    pub fn operation_entry_active(&self) -> bool {
        self.operation.drag.is_some()
            || self.operation.confirm.is_some()
            || self
                .operation
                .entry
                .as_ref()
                .is_some_and(|entry| entry.typing())
    }
    /// Explicit axis choice for a new revolve (tests and scripted hosts).
    #[cfg(test)]
    pub(crate) fn choose_revolve_axis(&mut self, origin: [f64; 2], bearing_deg: f64) {
        if let Some(ModelForm::Revolve(panel)) = &mut self.form {
            panel.set_axis(origin, bearing_deg);
        }
    }
    #[cfg(test)]
    pub(crate) fn request_for_tests(&self, session: &Session) -> Option<ModelRequest> {
        self.form
            .as_ref()?
            .request(session.document(), self.capabilities())
            .ok()
    }
    pub fn operation_value_display(&self) -> Option<String> {
        self.operation.entry.as_ref().map(ValueEntry::display)
    }

    pub fn operation_viewport(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        resources: &ProjectResources,
        input: OperationViewport,
    ) -> Result<PlacementViewportResult, String> {
        let result = self.operation_viewport_inner(ui, session, resources, input);
        if let Err(error) = &result {
            self.operation.drag = None;
            self.operation.confirm = None;
            if let Some(entry) = &mut self.operation.entry {
                entry.reopen();
            }
            self.error = Some(error.clone());
        }
        result
    }

    fn apply_entry_effect(
        &mut self,
        session: &mut Session,
        resources: &ProjectResources,
        effect: EntryEffect,
    ) -> Result<Option<ModelToolOutcome>, String> {
        let set = |form: &mut Option<ModelForm>, value: f64| match form {
            Some(ModelForm::Extrude(panel)) => panel.set_gizmo_distance(value),
            Some(ModelForm::Revolve(panel)) => panel.set_sweep_degrees(value),
            _ => {}
        };
        match effect {
            EntryEffect::None => Ok(None),
            EntryEffect::Preview(value) => {
                set(&mut self.form, value);
                self.invalidate();
                // A rejected request keeps the tool open with its message.
                let _ = self.preview(session, resources, Channel::Ui);
                self.operation.auto_previewed = self.stamp();
                Ok(None)
            }
            EntryEffect::Invalid(message) => {
                if let Some(entry) = &self.operation.entry
                    && !entry.typing()
                {
                    set(&mut self.form, entry.value());
                }
                self.invalidate();
                self.operation.auto_previewed = self.stamp();
                self.error = Some(message);
                Ok(None)
            }
            EntryEffect::Commit(value) => {
                let current = match &self.form {
                    Some(ModelForm::Extrude(panel)) => panel
                        .gizmo_distance(session.document())
                        .and_then(Result::ok),
                    Some(ModelForm::Revolve(panel)) => panel.sweep_degrees(session.document()).ok(),
                    _ => None,
                };
                if current != Some(value) {
                    set(&mut self.form, value);
                    self.invalidate();
                }
                if self.can_apply(session, resources) {
                    return self.commit_operation(session, resources).map(Some);
                }
                let stamp = self.stamp();
                let waiting = self
                    .candidate
                    .as_ref()
                    .is_some_and(|candidate| candidate.pending && Some(candidate.form) == stamp);
                if !waiting {
                    self.preview(session, resources, Channel::Ui)?;
                }
                self.operation.confirm = self.stamp();
                Ok(None)
            }
            EntryEffect::Cancel => {
                self.cancel();
                Ok(Some(ModelToolOutcome::Cancelled))
            }
        }
    }

    fn commit_operation(
        &mut self,
        session: &mut Session,
        resources: &ProjectResources,
    ) -> Result<ModelToolOutcome, String> {
        let parts = self.apply(session, resources)?;
        Ok(ModelToolOutcome::Applied(parts))
    }

    fn operation_viewport_inner(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        resources: &ProjectResources,
        input: OperationViewport,
    ) -> Result<PlacementViewportResult, String> {
        if !self.operation_gizmo_active() {
            return Ok(PlacementViewportResult::default());
        }
        self.current(session, resources, true)?;
        if let Some(expected) = self.operation.confirm {
            if self.stamp() != Some(expected) {
                return Err("Inputs changed before the commit finished; press Enter again".into());
            }
            self.poll(session, resources);
            if let Some(error) = self.error.clone() {
                return Err(error);
            }
            if self.can_apply(session, resources) {
                let outcome = self.commit_operation(session, resources)?;
                return Ok(PlacementViewportResult {
                    outcome: Some(outcome),
                    input_owned: true,
                });
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
            return Ok(PlacementViewportResult {
                input_owned: true,
                ..Default::default()
            });
        }
        let guide = self.operation.guide.clone().expect("active gizmo guide");
        let document = session.document();
        let fallback = (70. / input.camera.scale.max(1e-6)) as f64;
        // Current value and handle geometry from the shared Inspector fields.
        let (kind, value, handle) = match &self.form {
            Some(ModelForm::Extrude(panel)) => {
                let Some(value) = panel.gizmo_distance(document) else {
                    self.operation.entry = None;
                    return Ok(PlacementViewportResult::default());
                };
                let (local, offset) = panel.gizmo_axis(document)?;
                let (start, direction) = extrude_axis(&guide, local, offset)
                    .ok_or("The extrusion direction is degenerate")?;
                (
                    ValueKind::Distance,
                    value.ok(),
                    Handle::Linear { start, direction },
                )
            }
            Some(ModelForm::Revolve(panel)) => {
                let Some(axis) = panel.axis(document) else {
                    return self.revolve_axis_pick(ui, session, resources, &guide, &input);
                };
                let (origin, bearing) = axis?;
                let ring = RevolveRing::new(&guide, origin, bearing, fallback)
                    .ok_or("The revolution axis is degenerate")?;
                (
                    ValueKind::Angle,
                    panel.sweep_degrees(document).ok(),
                    Handle::Angular { ring },
                )
            }
            _ => return Ok(PlacementViewportResult::default()),
        };
        if self.operation.entry.as_ref().map(ValueEntry::kind) != Some(kind) {
            self.operation.entry = Some(ValueEntry::new(kind, value.unwrap_or(10.)));
        }
        if let (Some(entry), Some(value)) = (&mut self.operation.entry, value) {
            entry.sync_external(value);
        }
        // Show real geometry as soon as the operation has valid inputs.
        if self.candidate.is_none()
            && self.error.is_none()
            && !self.pending()
            && self.operation.auto_previewed != self.stamp()
        {
            self.operation.auto_previewed = self.stamp();
            let _ = self.preview(session, resources, Channel::Ui);
        }
        let entry_value = self.operation.entry.as_ref().map_or(0., ValueEntry::value);
        let painted = self.operation.drag.as_ref().map_or(handle, |d| d.handle);
        // Screen geometry.
        let (path, grip): (Vec<Pos2>, Option<Pos2>) = match painted {
            Handle::Linear { start, direction } => {
                let tip = add(start, mul(direction, entry_value));
                let a = screen(input.camera, input.rect, start);
                let b = screen(input.camera, input.rect, tip);
                (a.into_iter().chain(b).collect(), b)
            }
            Handle::Angular { ring } => {
                let steps = 48;
                let points = (0..=steps)
                    .filter_map(|i| {
                        screen(
                            input.camera,
                            input.rect,
                            ring.point(entry_value * i as f64 / steps as f64),
                        )
                    })
                    .collect();
                (
                    points,
                    screen(input.camera, input.rect, ring.point(entry_value)),
                )
            }
        };
        let label_text = self
            .operation
            .entry
            .as_ref()
            .map(ValueEntry::display)
            .unwrap_or_default();
        let font = egui::FontId::proportional(13.);
        let galley = ui
            .painter()
            .layout_no_wrap(label_text.clone(), font.clone(), Color32::WHITE);
        let label_rect = grip.map(|g| {
            Rect::from_min_size(
                g + egui::vec2(14., -24.),
                galley.size() + egui::vec2(12., 8.),
            )
        });
        let (pointer, press, pressed, released, down, clicked, motion) = ui.input(|i| {
            let press = i
                .events
                .iter()
                .enumerate()
                .rev()
                .find_map(|(index, event)| match event {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        ..
                    } => Some((index, *pos)),
                    _ => None,
                });
            let motion = i
                .events
                .iter()
                .skip(press.map_or(0, |(index, _)| index + 1))
                .filter_map(|event| match event {
                    egui::Event::PointerMoved(pos)
                    | egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        ..
                    } => Some(*pos),
                    _ => None,
                })
                .collect::<Vec<_>>();
            (
                i.pointer.interact_pos(),
                press.map(|(_, pos)| pos),
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.primary_down(),
                i.pointer.primary_clicked(),
                motion,
            )
        });
        let hit_point = if pressed { press.or(pointer) } else { pointer };
        let near_grip = |p: Pos2| {
            grip.is_some_and(|g| g.distance(p) < 10.)
                || path
                    .windows(2)
                    .any(|w| segment_distance(p, w[0], w[1]) < 6.)
        };
        let hovered = hit_point
            .filter(|p| input.rect.contains(*p))
            .is_some_and(near_grip);
        let on_label = hit_point.is_some_and(|p| label_rect.is_some_and(|r| r.contains(p)));
        let mut owned = self.operation.drag.is_some();
        let mut outcome = None;
        let mut effects = vec![];
        if self.operation.drag.is_none() && input.start_allowed && pressed {
            if on_label {
                self.operation.touched = true;
                owned = true;
                effects.push(EntryInput::Focus);
            } else if hovered && let Some(p) = press.or(pointer) {
                let mut tracker = AngleTracker::default();
                let coordinate = match handle {
                    Handle::Linear { .. } => handle.coordinate(input.camera, input.rect, p, None),
                    Handle::Angular { .. } => handle
                        .coordinate(input.camera, input.rect, p, None)
                        .map(|angle| {
                            tracker = AngleTracker::start(angle, 0.);
                            0.
                        }),
                };
                if let Some(coordinate) = coordinate {
                    self.operation.drag = Some(OperationDrag {
                        handle,
                        camera: input.camera,
                        rect: input.rect,
                        tracker,
                    });
                    self.operation.touched = true;
                    owned = true;
                    effects.push(EntryInput::Press(coordinate));
                }
            }
        }
        if let Some(drag) = &mut self.operation.drag
            && (down || released)
        {
            if drag.camera != input.camera || drag.rect != input.rect {
                self.operation.drag = None;
                return Err("The view changed during the drag; the gesture was cancelled".into());
            }
            let points = if motion.is_empty() {
                pointer.into_iter().collect()
            } else {
                motion
            };
            for p in points {
                let handle = drag.handle;
                if let Some(c) =
                    handle.coordinate(drag.camera, drag.rect, p, Some(&mut drag.tracker))
                {
                    effects.push(EntryInput::Drag(c));
                }
            }
        }
        if released && self.operation.drag.take().is_some() {
            effects.push(EntryInput::Release);
        }
        // Typing takes over without focusing: digits reach the gizmo whenever
        // no text field owns the keyboard.
        if !ui.ctx().text_edit_focused() && !egui::Popup::is_any_open(ui.ctx()) {
            let typing = self
                .operation
                .entry
                .as_ref()
                .is_some_and(ValueEntry::typing);
            let (text, enter, backspace, escape) = ui.input_mut(|i| {
                let text: String = i
                    .events
                    .iter()
                    .filter_map(|event| match event {
                        egui::Event::Text(text) => Some(text.clone()),
                        _ => None,
                    })
                    .collect();
                let enter = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                let backspace =
                    typing && i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace);
                let escape = typing && i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                (text, enter, backspace, escape)
            });
            if escape {
                effects.push(EntryInput::Escape);
            }
            for c in text.chars() {
                if c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '+') {
                    self.operation.touched = true;
                    effects.push(EntryInput::Char(c));
                }
            }
            if backspace {
                effects.push(EntryInput::Backspace);
            }
            // Enter commits only a value the user dragged or typed.
            if enter && self.operation.touched {
                effects.push(EntryInput::Enter);
            }
        }
        for input in effects {
            let Some(entry) = &mut self.operation.entry else {
                break;
            };
            let effect = entry.handle(input);
            if let Some(result) = self.apply_entry_effect(session, resources, effect)? {
                outcome = Some(result);
                break;
            }
        }
        if outcome.is_none() {
            self.paint_operation(ui, &input, &path, grip, label_rect, &galley, hovered);
        }
        if hovered || on_label {
            ui.ctx().set_cursor_icon(if on_label {
                egui::CursorIcon::Text
            } else {
                egui::CursorIcon::Grab
            });
        }
        if clicked && (hovered || on_label) {
            owned = true;
        }
        if self.operation.drag.is_some() || self.operation.confirm.is_some() {
            ui.ctx().request_repaint();
        }
        Ok(PlacementViewportResult {
            outcome,
            input_owned: owned || self.operation.confirm.is_some(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_operation(
        &self,
        ui: &egui::Ui,
        input: &OperationViewport,
        path: &[Pos2],
        grip: Option<Pos2>,
        label: Option<Rect>,
        galley: &std::sync::Arc<egui::Galley>,
        hovered: bool,
    ) {
        let painter = ui.painter_at(input.rect);
        let active = hovered || self.operation.drag.is_some();
        let color = if active {
            Color32::WHITE
        } else {
            Color32::from_rgb(255, 168, 38)
        };
        let stroke = Stroke::new(if active { 3. } else { 2. }, color);
        let linear = matches!(self.form, Some(ModelForm::Extrude(_)));
        if linear && path.len() == 2 {
            let (a, b) = (path[0], path[1]);
            painter.line_segment([a, b], stroke);
            let d = b - a;
            if d.length() > 4. {
                let n = d.normalized();
                let side = n.rot90() * 6.;
                painter.add(egui::Shape::convex_polygon(
                    vec![b + n * 4., b - n * 10. + side, b - n * 10. - side],
                    color,
                    Stroke::NONE,
                ));
            }
            painter.circle_stroke(a, 4., Stroke::new(1.5, color));
        } else if path.len() > 1 {
            painter.add(egui::Shape::line(path.to_vec(), stroke));
        }
        if let Some(grip) = grip {
            painter.circle_filled(grip, if active { 7. } else { 6. }, color);
        }
        if let Some(rect) = label {
            let invalid = self.error.is_some();
            painter.rect_filled(rect, 4., Color32::from_black_alpha(200));
            painter.rect_stroke(
                rect,
                4.,
                Stroke::new(1., if invalid { Color32::LIGHT_RED } else { color }),
                egui::StrokeKind::Inside,
            );
            painter.galley(
                rect.min + egui::vec2(6., 4.),
                galley.clone(),
                Color32::WHITE,
            );
            if let Some(error) = &self.error {
                painter.text(
                    rect.left_bottom() + egui::vec2(0., 4.),
                    egui::Align2::LEFT_TOP,
                    error,
                    egui::FontId::proportional(11.),
                    Color32::LIGHT_RED,
                );
            }
        }
    }

    /// Explicit revolve axis acquisition: hover highlights straight sketch
    /// lines, a click adopts one. Nothing is guessed from the profile.
    fn revolve_axis_pick(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        resources: &ProjectResources,
        guide: &ProfileGuide,
        input: &OperationViewport,
    ) -> Result<PlacementViewportResult, String> {
        let (pointer, clicked) =
            ui.input(|i| (i.pointer.interact_pos(), i.pointer.primary_clicked()));
        let painter = ui.painter_at(input.rect);
        let mut hovered = None;
        for line in &guide.lines {
            let a = screen(input.camera, input.rect, guide.frame.point(line.a));
            let b = screen(input.camera, input.rect, guide.frame.point(line.b));
            let (Some(a), Some(b)) = (a, b) else {
                continue;
            };
            let near = pointer
                .filter(|p| input.rect.contains(*p))
                .is_some_and(|p| segment_distance(p, a, b) < 6.);
            if near && hovered.is_none() {
                hovered = Some(line.clone());
            }
            painter.line_segment(
                [a, b],
                Stroke::new(
                    if near { 3. } else { 1.5 },
                    if near {
                        Color32::WHITE
                    } else {
                        Color32::from_rgb(255, 168, 38).gamma_multiply(0.7)
                    },
                ),
            );
        }
        if let Some(line) = &hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if clicked && input.start_allowed {
                if let Some(ModelForm::Revolve(panel)) = &mut self.form {
                    panel.set_axis_line(line.a, line.b)?;
                }
                self.invalidate();
                let _ = self.preview(session, resources, Channel::Ui);
                return Ok(PlacementViewportResult {
                    input_owned: true,
                    ..Default::default()
                });
            }
        }
        Ok(PlacementViewportResult {
            input_owned: hovered.is_some(),
            ..Default::default()
        })
    }
}

//! Scale handles produce authored ratio controls, never display-only transforms.
use super::{GizmoBasis, gizmo::math::*};
use crate::{camera::Camera, egui, viewport::Scene};
use egui::{Color32, Pos2, Rect, Stroke};
use multicad_core::{
    model::{self, ResolvedScale, ScaleParameters},
    parameters::EvaluatedParameters,
};
pub struct ScaleViewport<'a> {
    pub scene: Option<&'a Scene>,
    pub camera: Camera,
    pub rect: Rect,
    pub basis: GizmoBasis,
    pub start_allowed: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    Uniform,
    Axis(usize),
}
struct Shape {
    handle: Handle,
    start: Pos2,
    end: Pos2,
    color: Color32,
}
fn project(camera: Camera, rect: Rect, p: [f64; 3]) -> Option<Pos2> {
    let p = camera.project(p.map(|v| v as f32), rect);
    (p.iter().all(|v| v.is_finite()) && (!camera.perspective || p[2] > 0.1))
        .then(|| Pos2::new(p[0], p[1]))
}
fn shapes(input: &ScaleViewport<'_>, pivot: [f64; 3]) -> Vec<Shape> {
    let Some(center) = project(input.camera, input.rect, pivot) else {
        return vec![];
    };
    if !input.rect.expand(100.).contains(center) {
        return vec![];
    }
    let depth = input.camera.project(pivot.map(|v| v as f32), input.rect)[2];
    let distance = input.rect.height().max(1.) / input.camera.scale * 2.;
    let perspective = if input.camera.perspective {
        distance / depth
    } else {
        1.
    };
    let length = 82. / (input.camera.scale * perspective) as f64;
    let mut result = vec![Shape {
        handle: Handle::Uniform,
        start: center,
        end: center,
        color: Color32::WHITE,
    }];
    for (i, color) in [
        Color32::from_rgb(194, 55, 61),
        Color32::from_rgb(39, 146, 88),
        Color32::from_rgb(52, 112, 211),
    ]
    .into_iter()
    .enumerate()
    {
        if let Some(end) = project(
            input.camera,
            input.rect,
            add(pivot, mul(input.basis.axes[i], length)),
        ) && end.distance(center) > 16.
        {
            result.push(Shape {
                handle: Handle::Axis(i),
                start: center,
                end,
                color,
            });
        }
    }
    result
}
struct Drag {
    handle: Handle,
    source: ScaleParameters,
    values: ResolvedScale,
    camera: Camera,
    rect: Rect,
    basis: GizmoBasis,
    press: Pos2,
    axis_start: f64,
    last: ScaleParameters,
    changed: bool,
}
impl Drag {
    fn new(
        handle: Handle,
        source: &ScaleParameters,
        parameters: &EvaluatedParameters,
        input: &ScaleViewport<'_>,
        press: Pos2,
    ) -> Result<Self, String> {
        let values = source.resolve(parameters).map_err(|e| e.to_string())?;
        let axis_start = if let Handle::Axis(i) = handle {
            let ray = input
                .camera
                .ray(press, input.rect)
                .ok_or("No finite scale pointer ray")?;
            let start = axis_parameter(ray, values.pivot_mm, input.basis.axes[i])
                .ok_or("Scale axis is edge-on; orbit the view")?;
            if start.abs() < 1e-8 {
                return Err("Choose the square at the end of the scale axis".into());
            }
            start
        } else {
            0.
        };
        Ok(Self {
            handle,
            source: source.clone(),
            values,
            camera: input.camera,
            rect: input.rect,
            basis: input.basis,
            press,
            axis_start,
            last: source.clone(),
            changed: false,
        })
    }
    fn update(&mut self, pointer: Pos2) -> Result<Option<ScaleParameters>, String> {
        let ratio = match self.handle {
            Handle::Uniform => {
                let delta = pointer - self.press;
                ((delta.x - delta.y) as f64 / 100.).exp()
            }
            Handle::Axis(i) => {
                let ray = self
                    .camera
                    .ray(pointer, self.rect)
                    .ok_or("No finite scale pointer ray")?;
                axis_parameter(ray, self.values.pivot_mm, self.basis.axes[i])
                    .ok_or("Scale axis became edge-on")?
                    / self.axis_start
            }
        };
        let mut next = self.source.clone();
        if (ratio - 1.).abs() > 1e-12 {
            for i in 0..3 {
                if self.handle == Handle::Uniform || self.handle == Handle::Axis(i) {
                    next.factors[i] = model::ratio(self.values.factors[i] * ratio);
                }
            }
        }
        next.axes = self.basis.axes;
        next.validate().map_err(|e| e.to_string())?;
        if next == self.last {
            return Ok(None);
        }
        self.last = next.clone();
        self.changed = true;
        Ok(Some(next))
    }
}
#[derive(Default)]
pub(super) struct Interaction {
    drag: Option<Drag>,
}
pub(super) struct Gesture {
    pub input_owned: bool,
    pub started: bool,
    pub changed: Option<ScaleParameters>,
    pub released: bool,
    pub moved: bool,
    pub copy_modifier: bool,
}
impl Interaction {
    pub fn active(&self) -> bool {
        self.drag.is_some()
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        input: &ScaleViewport<'_>,
        source: &ScaleParameters,
        parameters: &EvaluatedParameters,
    ) -> Result<Gesture, String> {
        input.basis.validate()?;
        if let Some(drag) = &self.drag
            && (drag.camera != input.camera || drag.rect != input.rect || drag.basis != input.basis)
        {
            return Err("The view or scale coordinates changed; the gesture was cancelled".into());
        }
        let (pointer, press, pressed, released, down, alt, motion) = ui.input(|i| {
            // Mouse-up may precede an Option/Alt key-up in the same frame.
            // The copy intent belongs to the gesture's pointer event.
            let alt = i
                .events
                .iter()
                .rev()
                .find_map(|event| match event {
                    egui::Event::PointerButton {
                        button: egui::PointerButton::Primary,
                        modifiers,
                        ..
                    } => Some(modifiers.alt),
                    _ => None,
                })
                .unwrap_or(i.modifiers.alt);
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
            // Input can contain an entire drag between rendered frames. Keep
            // its original down position and ordered motion/rotation samples.
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
                alt,
                motion,
            )
        });
        let values = source.resolve(parameters).map_err(|e| e.to_string())?;
        if self.drag.is_some() && !down && !pressed && !released {
            return Err("The pointer gesture was interrupted; scaling was cancelled".into());
        }
        let geometry = shapes(input, values.pivot_mm);
        let hit_pointer = if self.drag.is_none() && pressed {
            press.or(pointer)
        } else {
            pointer
        };
        let hovered = hit_pointer
            .filter(|p| input.rect.contains(*p) && (input.start_allowed || self.drag.is_some()))
            .and_then(|p| {
                geometry
                    .iter()
                    .filter(|s| Rect::from_center_size(s.end, egui::vec2(16., 16.)).contains(p))
                    .min_by(|a, b| a.end.distance(p).total_cmp(&b.end.distance(p)))
                    .map(|s| s.handle)
            });
        let mut started = false;
        if self.drag.is_none()
            && input.start_allowed
            && pressed
            && let (Some(handle), Some(pointer)) = (hovered, press.or(pointer))
        {
            self.drag = Some(Drag::new(handle, source, parameters, input, pointer)?);
            started = true;
        }
        let painter = ui.painter_at(input.rect);
        for shape in geometry {
            let active = self.drag.as_ref().is_some_and(|d| d.handle == shape.handle)
                || hovered == Some(shape.handle);
            let color = if active { Color32::WHITE } else { shape.color };
            if shape.handle != Handle::Uniform {
                painter.line_segment(
                    [shape.start, shape.end],
                    Stroke::new(if active { 2.5 } else { 1.5 }, color),
                );
            }
            painter.rect_filled(
                Rect::from_center_size(shape.end, egui::vec2(8., 8.)),
                1.,
                color,
            );
            if let Handle::Axis(i) = shape.handle {
                painter.text(
                    shape.end + egui::vec2(7., -5.),
                    egui::Align2::LEFT_BOTTOM,
                    ["X", "Y", "Z"][i],
                    egui::FontId::proportional(11.),
                    color,
                );
            }
        }
        if let Some(handle) = hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            if let Some(pointer) = pointer {
                ui.interact(
                    Rect::from_center_size(pointer, egui::vec2(12., 12.)),
                    ui.id().with("scale-handle-tooltip"),
                    egui::Sense::hover(),
                )
                .on_hover_text(match handle {
                    Handle::Uniform => {
                        "Uniform scale · drag right/up to grow; Option / Alt copies; Esc cancels"
                    }
                    Handle::Axis(0) => "Scale X · positive ratios; Option / Alt copies",
                    Handle::Axis(1) => "Scale Y · positive ratios; Option / Alt copies",
                    Handle::Axis(_) => "Scale Z · positive ratios; Option / Alt copies",
                });
            }
        }
        let mut changed = None;
        if let Some(drag) = &mut self.drag
            && (down || released)
        {
            let points = if motion.is_empty() {
                pointer.into_iter().collect()
            } else {
                motion
            };
            for point in points {
                if let Some(next) = drag.update(point)? {
                    changed = Some(next);
                }
            }
        }
        let moved = self.drag.as_ref().is_some_and(|d| d.changed);
        let owned = self.drag.is_some();
        let released = owned && released;
        if released {
            self.drag = None;
        }
        Ok(Gesture {
            input_owned: owned,
            started,
            changed,
            released,
            moved,
            copy_modifier: alt,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::egui::{Context, Event, Modifiers, PointerButton};
    use multicad_core::{
        constraint_sketch::Dimension,
        document::{Document, Entity, EntityData},
        parameters::BoundExpression,
    };
    struct Harness {
        ctx: Context,
        input: ScaleViewport<'static>,
        interaction: Interaction,
        parameters: EvaluatedParameters,
    }
    impl Harness {
        fn new(basis: GizmoBasis) -> Self {
            Self {
                ctx: Default::default(),
                input: ScaleViewport {
                    scene: None,
                    camera: Camera {
                        yaw: -0.6,
                        pitch: 0.6,
                        scale: 4.,
                        target: [0.; 3],
                        perspective: false,
                    },
                    rect: Rect::from_min_max(Pos2::new(10., 10.), Pos2::new(810., 610.)),
                    basis,
                    start_allowed: true,
                },
                interaction: Default::default(),
                parameters: multicad_core::parameters::evaluate(
                    &Document::new("Scale pointer").unwrap(),
                )
                .unwrap(),
            }
        }
        fn frame(
            &mut self,
            source: &ScaleParameters,
            events: Vec<Event>,
        ) -> Result<Gesture, String> {
            let mut result = None;
            self.ctx
                .run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900., 700.))),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        result =
                            Some(
                                self.interaction
                                    .show(ui, &self.input, source, &self.parameters),
                            );
                    },
                )
                .drop_without_applying_deltas();
            result.unwrap()
        }
        fn handle(&self, source: &ScaleParameters, handle: Handle) -> Pos2 {
            let pivot = source.resolve(&self.parameters).unwrap().pivot_mm;
            shapes(&self.input, pivot)
                .into_iter()
                .find(|s| s.handle == handle)
                .unwrap()
                .end
        }
    }
    fn button(pos: Pos2, pressed: bool, alt: bool) -> Event {
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers {
                alt,
                ..Default::default()
            },
        }
    }
    #[test]
    fn coalesced_uniform_drag_uses_original_press_and_release_copy_intent() {
        let mut h = Harness::new(Default::default());
        let source = ScaleParameters::default();
        let p = h.handle(&source, Handle::Uniform);
        let end = p + egui::vec2(50., -50.);
        let g = h
            .frame(
                &source,
                vec![
                    Event::PointerMoved(p),
                    button(p, true, true),
                    Event::PointerMoved(p + egui::vec2(20., -20.)),
                    Event::PointerMoved(end),
                    button(end, false, true),
                    Event::ModifiersChanged(Modifiers::NONE),
                ],
            )
            .unwrap();
        assert!(g.started && g.input_owned && g.released && g.moved && g.copy_modifier);
        let next = g.changed.unwrap();
        for v in next.resolve(&h.parameters).unwrap().factors {
            assert!((v - std::f64::consts::E).abs() < 1e-10);
        }
        assert!(!h.interaction.active());
    }
    #[test]
    fn axis_drag_changes_only_its_ratio_and_keeps_other_bound_formulas() {
        let mut d = Document::new("Scale bound axes").unwrap();
        d.insert(Entity::new(
            "factor",
            EntityData::Parameter {
                expression: "3".into(),
                unit: "ratio".into(),
                exposed: true,
                inputs: vec![],
            },
        ))
        .unwrap();
        let bound = Dimension::Expression {
            expression: BoundExpression::bind(&d, "=$factor", "ratio", None).unwrap(),
        };
        let mut h = Harness::new(GizmoBasis {
            axes: [[0., 1., 0.], [-1., 0., 0.], [0., 0., 1.]],
        });
        h.parameters = multicad_core::parameters::evaluate(&d).unwrap();
        let source = ScaleParameters {
            factors: [model::ratio(2.), bound.clone(), model::ratio(4.)],
            ..Default::default()
        };
        let p = h.handle(&source, Handle::Axis(0));
        let center = h.handle(&source, Handle::Uniform);
        let end = center + (p - center) * 1.5;
        let g = h
            .frame(
                &source,
                vec![
                    button(p, true, false),
                    Event::PointerMoved(end),
                    button(end, false, false),
                ],
            )
            .unwrap();
        assert!(g.released && g.moved);
        let next = g.changed.unwrap();
        assert_eq!(next.factors[1], bound);
        assert_eq!(next.factors[2], source.factors[2]);
        assert_eq!(next.axes, h.input.basis.axes);
        assert!((next.resolve(&h.parameters).unwrap().factors[0] - 3.).abs() < 1e-6);
    }
    #[test]
    fn click_and_overlay_press_are_source_neutral_and_negative_drag_is_rejected() {
        let mut h = Harness::new(Default::default());
        let source = ScaleParameters::default();
        let p = h.handle(&source, Handle::Uniform);
        let g = h
            .frame(
                &source,
                vec![button(p, true, false), button(p, false, false)],
            )
            .unwrap();
        assert!(g.input_owned && g.released && !g.moved && g.changed.is_none());
        h.input.start_allowed = false;
        let g = h
            .frame(
                &source,
                vec![
                    button(p, true, false),
                    Event::PointerMoved(p + egui::vec2(100., 0.)),
                    button(p, false, false),
                ],
            )
            .unwrap();
        assert!(!g.input_owned && g.changed.is_none());
        h.input.start_allowed = true;
        let p = h.handle(&source, Handle::Axis(0));
        let center = h.handle(&source, Handle::Uniform);
        let end = center - (p - center);
        let error = h
            .frame(
                &source,
                vec![
                    button(p, true, false),
                    Event::PointerMoved(end),
                    button(end, false, false),
                ],
            )
            .err()
            .unwrap();
        assert!(error.contains("positive") || error.contains("mirror"));
    }
    #[test]
    fn captured_drag_rejects_camera_or_coordinate_changes() {
        let mut h = Harness::new(Default::default());
        let source = ScaleParameters::default();
        let p = h.handle(&source, Handle::Uniform);
        assert!(
            h.frame(&source, vec![button(p, true, false)])
                .unwrap()
                .started
        );
        h.input.camera.pitch += 0.1;
        assert!(
            h.frame(&source, vec![Event::PointerMoved(p + egui::vec2(5., 0.))])
                .is_err()
        );
    }
}

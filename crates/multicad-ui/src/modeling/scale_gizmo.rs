//! Scale handles produce authored ratio controls, never display-only transforms.
use super::{GizmoBasis, gizmo::math::*};
use crate::{
    camera::Camera,
    egui,
    viewport::{Ray, Scene},
};
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
    Plane([usize; 2]),
}
const AXES: [&str; 3] = ["X", "Y", "Z"];
const AXIS_COLORS: [Color32; 3] = [
    Color32::from_rgb(194, 55, 61),
    Color32::from_rgb(39, 146, 88),
    Color32::from_rgb(52, 112, 211),
];
impl Handle {
    fn coordinates(self, ray: Ray, pivot: Point, basis: GizmoBasis) -> Option<[f64; 2]> {
        match self {
            Self::Uniform => None,
            Self::Axis(i) => axis_parameter(ray, pivot, basis.axes[i]).map(|axis| [axis, 1.]),
            Self::Plane([i, j]) => {
                let hit = plane_point(ray, pivot, basis.axes[3 - i - j])?;
                let offset = sub(hit, pivot);
                Some([dot(offset, basis.axes[i]), dot(offset, basis.axes[j])])
            }
        }
    }

    fn label(self) -> String {
        match self {
            Self::Uniform => "Uniform scale".into(),
            Self::Axis(i) => format!("Scale {}", AXES[i]),
            Self::Plane([i, j]) => format!("Scale {}{}", AXES[i], AXES[j]),
        }
    }
}

enum Mark {
    Center,
    Axis { start: Pos2 },
    Plane { corners: [Pos2; 4] },
}
struct Shape {
    handle: Handle,
    center: Pos2,
    mark: Mark,
    color: Color32,
}
impl Shape {
    fn contains(&self, pointer: Pos2) -> bool {
        match self.mark {
            Mark::Plane { corners } => {
                let positive = (corners[1] - corners[0])
                    .rot90()
                    .dot(corners[2] - corners[1])
                    > 0.;
                (0..4).all(|i| {
                    let side = corners[(i + 1) % 4] - corners[i];
                    let distance = side.rot90().dot(pointer - corners[i]);
                    if positive {
                        distance >= 0.
                    } else {
                        distance <= 0.
                    }
                })
            }
            Mark::Center | Mark::Axis { .. } => {
                Rect::from_center_size(self.center, egui::vec2(16., 16.)).contains(pointer)
            }
        }
    }

    fn paint(self, painter: &egui::Painter, active: bool) {
        let color = if active { Color32::WHITE } else { self.color };
        let stroke = Stroke::new(if active { 2.5 } else { 1.5 }, color);
        match self.mark {
            Mark::Plane { corners } => {
                painter.add(egui::Shape::convex_polygon(
                    corners.to_vec(),
                    color.gamma_multiply(0.32),
                    stroke,
                ));
            }
            Mark::Center | Mark::Axis { .. } => {
                if let Mark::Axis { start } = self.mark {
                    painter.line_segment([start, self.center], stroke);
                }
                painter.rect_filled(
                    Rect::from_center_size(self.center, egui::vec2(8., 8.)),
                    1.,
                    color,
                );
            }
        }
        if let Handle::Axis(i) = self.handle {
            painter.text(
                self.center + egui::vec2(7., -5.),
                egui::Align2::LEFT_BOTTOM,
                AXES[i],
                egui::FontId::proportional(11.),
                color,
            );
        }
    }
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
    let mut result = Vec::new();
    for [i, j] in [[0, 1], [1, 2], [0, 2]] {
        let point = |u, v| {
            let offset = add(
                mul(input.basis.axes[i], length * u),
                mul(input.basis.axes[j], length * v),
            );
            project(input.camera, input.rect, add(pivot, offset))
        };
        let inner = 0.24;
        let outer = 0.46;
        let [Some(a), Some(b), Some(c), Some(d)] = [
            point(inner, inner),
            point(outer, inner),
            point(outer, outer),
            point(inner, outer),
        ] else {
            continue;
        };
        let u = b - a;
        let v = d - a;
        if u.rot90().dot(v).abs() > 6. * u.length().max(v.length())
            && let Some(center) = point((inner + outer) / 2., (inner + outer) / 2.)
        {
            result.push(Shape {
                handle: Handle::Plane([i, j]),
                center,
                mark: Mark::Plane {
                    corners: [a, b, c, d],
                },
                color: AXIS_COLORS[3 - i - j],
            });
        }
    }
    for (i, color) in AXIS_COLORS.into_iter().enumerate() {
        if let Some(end) = project(
            input.camera,
            input.rect,
            add(pivot, mul(input.basis.axes[i], length)),
        ) && end.distance(center) > 16.
        {
            result.push(Shape {
                handle: Handle::Axis(i),
                center: end,
                mark: Mark::Axis { start: center },
                color,
            });
        }
    }
    result.push(Shape {
        handle: Handle::Uniform,
        center,
        mark: Mark::Center,
        color: Color32::WHITE,
    });
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
    start: [f64; 2],
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
        let start = if handle == Handle::Uniform {
            [1.; 2]
        } else {
            let ray = input
                .camera
                .ray(press, input.rect)
                .ok_or("No finite scale pointer ray")?;
            let start = handle
                .coordinates(ray, values.pivot_mm, input.basis)
                .ok_or("The scale handle is edge-on. Orbit the view and try again.")?;
            if start.iter().any(|value| value.abs() < 1e-8) {
                return Err("Choose the scale handle away from the pivot".into());
            }
            start
        };
        Ok(Self {
            handle,
            source: source.clone(),
            values,
            camera: input.camera,
            rect: input.rect,
            basis: input.basis,
            press,
            start,
            last: source.clone(),
            changed: false,
        })
    }
    fn update(
        &mut self,
        pointer: Pos2,
        snap_step: Option<f64>,
    ) -> Result<Option<ScaleParameters>, String> {
        let mut ratios = [1.; 3];
        if self.handle == Handle::Uniform {
            let delta = pointer - self.press;
            ratios.fill(((delta.x - delta.y) as f64 / 100.).exp());
        } else {
            let ray = self
                .camera
                .ray(pointer, self.rect)
                .ok_or("No finite scale pointer ray")?;
            let end = self
                .handle
                .coordinates(ray, self.values.pivot_mm, self.basis)
                .ok_or("The scale handle became edge-on. Orbit the view and try again.")?;
            match self.handle {
                Handle::Axis(i) => ratios[i] = end[0] / self.start[0],
                Handle::Plane([i, j]) => {
                    ratios[i] = end[0] / self.start[0];
                    ratios[j] = end[1] / self.start[1];
                }
                Handle::Uniform => unreachable!("the uniform handle uses screen motion"),
            }
        }

        let mut next = self.source.clone();
        for (i, ratio) in ratios.into_iter().enumerate() {
            let ratio = if let Some(step) = snap_step
                && ratio.is_finite()
                && ratio > 0.
            {
                // Snap the multiplier around the original scale. This keeps
                // 1× on the grid even if the chosen step does not divide 1.
                let minimum_step = 1. - (1. / step).ceil();
                1. + ((ratio - 1.) / step).round().max(minimum_step) * step
            } else {
                ratio
            };
            if (ratio - 1.).abs() > 1e-12 || !ratio.is_finite() {
                next.factors[i] = model::ratio(self.values.factors[i] * ratio);
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
        step: f64,
    ) -> Result<Gesture, String> {
        input.basis.validate()?;
        if let Some(drag) = &self.drag
            && (drag.camera != input.camera || drag.rect != input.rect || drag.basis != input.basis)
        {
            return Err("The view or scale coordinates changed; the gesture was cancelled".into());
        }
        let (pointer, press, pressed, released, down, alt, shift, motion) = ui.input(|i| {
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
            let shift = i
                .events
                .iter()
                .rev()
                .find_map(|event| match event {
                    egui::Event::PointerButton {
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers,
                        ..
                    } => Some(modifiers.shift),
                    _ => None,
                })
                .unwrap_or(i.modifiers.shift);
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
                shift,
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
                    .filter(|shape| shape.contains(p))
                    .min_by(|a, b| a.center.distance(p).total_cmp(&b.center.distance(p)))
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
            shape.paint(&painter, active);
        }
        if let Some(handle) = hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            if let Some(pointer) = pointer {
                ui.interact(
                    Rect::from_center_size(pointer, egui::vec2(12., 12.)),
                    ui.id().with("scale-handle-tooltip"),
                    egui::Sense::hover(),
                )
                .on_hover_text(format!(
                    "{} · drag to stage. Shift steps, Option / Alt copies. Enter applies, Escape cancels.",
                    handle.label(),
                ));
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
                if let Some(next) = drag.update(point, shift.then_some(step))? {
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
        step: f64,
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
                step: 0.1,
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
                        result = Some(self.interaction.show(
                            ui,
                            &self.input,
                            source,
                            &self.parameters,
                            self.step,
                        ));
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
                .center
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
    fn each_planar_handle_tracks_two_independent_factors_and_preserves_the_third() {
        let mut document = Document::new("Scale planes").unwrap();
        document
            .insert(Entity::new(
                "unchanged",
                EntityData::Parameter {
                    expression: "3".into(),
                    unit: "ratio".into(),
                    exposed: true,
                    inputs: vec![],
                },
            ))
            .unwrap();
        let bound = Dimension::Expression {
            expression: BoundExpression::bind(&document, "=$unchanged", "ratio", None).unwrap(),
        };
        for perspective in [false, true] {
            for [i, j] in [[0, 1], [1, 2], [0, 2]] {
                let mut h = Harness::new(Default::default());
                h.input.camera.perspective = perspective;
                h.parameters = multicad_core::parameters::evaluate(&document).unwrap();
                let untouched = 3 - i - j;
                let mut source = ScaleParameters {
                    pivot: [model::length(12.), model::length(4.), model::length(-3.)],
                    ..Default::default()
                };
                source.factors[untouched] = bound.clone();
                let handle = Handle::Plane([i, j]);
                let start = h.handle(&source, handle);
                let pivot = source.resolve(&h.parameters).unwrap().pivot_mm;
                let [u, v] = handle
                    .coordinates(
                        h.input.camera.ray(start, h.input.rect).unwrap(),
                        pivot,
                        h.input.basis,
                    )
                    .unwrap();
                let end = project(
                    h.input.camera,
                    h.input.rect,
                    add(
                        pivot,
                        add(
                            mul(h.input.basis.axes[i], 1.5 * u),
                            mul(h.input.basis.axes[j], 0.75 * v),
                        ),
                    ),
                )
                .unwrap();
                let gesture = h
                    .frame(
                        &source,
                        vec![
                            button(start, true, false),
                            Event::PointerMoved(end),
                            button(end, false, false),
                        ],
                    )
                    .unwrap();
                assert!(gesture.started && gesture.released && gesture.input_owned);
                let next = gesture.changed.unwrap();
                assert_eq!(next.factors[untouched], bound);
                assert_eq!(next.pivot, source.pivot);
                let factors = next.resolve(&h.parameters).unwrap().factors;
                assert!(
                    (factors[i] - 1.5).abs() < 2e-5,
                    "{perspective} {i} {j} {factors:?}"
                );
                assert!(
                    (factors[j] - 0.75).abs() < 2e-5,
                    "{perspective} {i} {j} {factors:?}"
                );
            }
        }
    }

    #[test]
    fn only_the_face_on_plane_is_drawn_in_top_view() {
        let mut h = Harness::new(Default::default());
        h.input.camera.pitch = std::f32::consts::FRAC_PI_2;
        let planes = shapes(&h.input, [0.; 3])
            .into_iter()
            .filter_map(|shape| match shape.handle {
                Handle::Plane(pair) => Some(pair),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(planes, [[0, 1]]);
    }

    #[test]
    fn shift_steps_the_held_gesture_and_returns_to_continuous_without_pointer_motion() {
        let mut h = Harness::new(Default::default());
        h.step = 0.25;
        let mut source = ScaleParameters {
            factors: [model::ratio(1.), model::ratio(2.), model::ratio(3.)],
            ..Default::default()
        };
        let start = h.handle(&source, Handle::Uniform);
        let end = start + egui::vec2(100. * 1.37_f32.ln(), 0.);
        let snapped = [1.25, 2.5, 3.75];
        let check = |values: [f64; 3], ratio: f64| {
            for (index, factor) in values.into_iter().enumerate() {
                assert!((factor - (index + 1) as f64 * ratio).abs() < 1e-6);
            }
        };

        let gesture = h
            .frame(
                &source,
                vec![
                    Event::ModifiersChanged(Modifiers::SHIFT),
                    Event::PointerMoved(start + egui::vec2(-75., 0.)),
                ],
            )
            .unwrap();
        assert!(!gesture.input_owned && gesture.changed.is_none());
        let gesture = h
            .frame(
                &source,
                vec![
                    Event::ModifiersChanged(Modifiers::NONE),
                    button(start, true, false),
                    Event::PointerMoved(end),
                ],
            )
            .unwrap();
        source = gesture.changed.unwrap();
        check(source.resolve(&h.parameters).unwrap().factors, 1.37);

        source = h
            .frame(&source, vec![Event::ModifiersChanged(Modifiers::SHIFT)])
            .unwrap()
            .changed
            .unwrap();
        assert_eq!(source.resolve(&h.parameters).unwrap().factors, snapped);
        source = h
            .frame(&source, vec![Event::ModifiersChanged(Modifiers::NONE)])
            .unwrap()
            .changed
            .unwrap();
        check(source.resolve(&h.parameters).unwrap().factors, 1.37);

        let gesture = h
            .frame(
                &source,
                vec![
                    Event::PointerButton {
                        pos: end,
                        button: PointerButton::Primary,
                        pressed: false,
                        modifiers: Modifiers::SHIFT,
                    },
                    Event::ModifiersChanged(Modifiers::NONE),
                ],
            )
            .unwrap();
        assert!(gesture.released && gesture.input_owned && gesture.moved);
        assert_eq!(
            gesture
                .changed
                .unwrap()
                .resolve(&h.parameters)
                .unwrap()
                .factors,
            snapped
        );
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

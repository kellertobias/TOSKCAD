//! Display handles produce typed rigid-placement controls. They never edit a
//! mesh or commit a document; the controller requires a complete native receipt.
mod math;
use crate::{camera::Camera, egui, viewport::Scene};
use egui::{Color32, Pos2, Rect, Stroke};
use math::*;
use multicad_core::{model::NumericTransform, parameters::EvaluatedParameters};

/// Right-handed orthonormal world directions for World, Object or a datum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GizmoBasis {
    pub axes: [[f64; 3]; 3],
}
impl Default for GizmoBasis {
    fn default() -> Self {
        Self {
            axes: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        }
    }
}
impl GizmoBasis {
    fn validate(self) -> Result<(), String> {
        if self.axes.iter().flatten().any(|v| !v.is_finite())
            || self.axes.iter().any(|v| (dot(*v, *v) - 1.).abs() > 1e-6)
            || dot(cross(self.axes[0], self.axes[1]), self.axes[2]) < 1. - 1e-6
            || dot(self.axes[0], self.axes[1]).abs() > 1e-6
        {
            return Err("Choose an orthonormal right-handed transform coordinate system".into());
        }
        Ok(())
    }
}
pub struct PlacementViewport<'a> {
    /// Complete canonical source scene, never the noninteractive candidate.
    pub scene: Option<&'a Scene>,
    pub camera: Camera,
    pub rect: Rect,
    pub basis: GizmoBasis,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    Axis(usize),
    Plane(usize),
    Rotate(usize),
}
impl Handle {
    fn label(self) -> &'static str {
        match self {
            Self::Axis(0) => "Move X",
            Self::Axis(1) => "Move Y",
            Self::Axis(_) => "Move Z",
            Self::Plane(0) => "Move in YZ plane",
            Self::Plane(1) => "Move in ZX plane",
            Self::Plane(_) => "Move in XY plane",
            Self::Rotate(0) => "Rotate X",
            Self::Rotate(1) => "Rotate Y",
            Self::Rotate(_) => "Rotate Z",
        }
    }
}
struct HandleShape {
    handle: Handle,
    points: Vec<Pos2>,
    color: Color32,
    closed: bool,
}
impl HandleShape {
    fn distance(&self, point: Pos2) -> f32 {
        if self.closed
            && egui::epaint::PathShape::convex_polygon(
                self.points.clone(),
                Color32::WHITE,
                Stroke::NONE,
            )
            .visual_bounding_rect()
            .contains(point)
        {
            let mut signs = self
                .points
                .iter()
                .zip(self.points.iter().cycle().skip(1))
                .take(self.points.len())
                .map(|(a, b)| (*b - *a).rot90().dot(point - *a));
            let first = signs.next().unwrap_or_default();
            if signs.all(|v| v * first >= 0.) {
                return 0.;
            }
        }
        self.points
            .windows(2)
            .map(|p| segment_distance(point, p[0], p[1]))
            .fold(f32::INFINITY, f32::min)
    }
}
fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let d = b - a;
    let t = ((p - a).dot(d) / d.length_sq().max(1e-8)).clamp(0., 1.);
    p.distance(a + d * t)
}
fn projected(camera: Camera, rect: Rect, p: Point) -> Option<Pos2> {
    let p = camera.project(p.map(|v| v as f32), rect);
    (p.iter().all(|v| v.is_finite()) && (!camera.perspective || p[2] > 0.1))
        .then(|| Pos2::new(p[0], p[1]))
}
fn shapes(camera: Camera, rect: Rect, origin: Point, basis: GizmoBasis) -> Vec<HandleShape> {
    let Some(center) = projected(camera, rect, origin) else {
        return vec![];
    };
    if !rect.expand(100.).contains(center) {
        return vec![];
    }
    let depth = camera.project(origin.map(|v| v as f32), rect)[2];
    let distance = rect.height().max(1.) / camera.scale * 2.;
    let factor = if camera.perspective {
        distance / depth
    } else {
        1.
    };
    let length = 82. / (camera.scale * factor) as f64;
    let colors = [
        Color32::from_rgb(194, 55, 61),
        Color32::from_rgb(39, 146, 88),
        Color32::from_rgb(52, 112, 211),
    ];
    let mut result = vec![];
    for (i, color) in colors.into_iter().enumerate() {
        let point = |a: usize, s: f64| add(origin, mul(basis.axes[a], s * length));
        if let Some(end) = projected(camera, rect, point(i, 1.))
            && end.distance(center) > 16.
        {
            result.push(HandleShape {
                handle: Handle::Axis(i),
                points: vec![center, end],
                color,
                closed: false,
            });
        }
        let a = (i + 1) % 3;
        let b = (i + 2) % 3;
        let quad = [[0.17, 0.17], [0.34, 0.17], [0.34, 0.34], [0.17, 0.34]]
            .into_iter()
            .map(|[u, v]| {
                projected(
                    camera,
                    rect,
                    add(
                        add(origin, mul(basis.axes[a], u * length)),
                        mul(basis.axes[b], v * length),
                    ),
                )
            })
            .collect::<Option<Vec<_>>>();
        if let Some(points) = quad
            && (points[1] - points[0])
                .rot90()
                .dot(points[2] - points[0])
                .abs()
                > 8.
        {
            result.push(HandleShape {
                handle: Handle::Plane(i),
                points,
                color,
                closed: true,
            });
        }
        let ring = (0..=64)
            .map(|j| {
                let t = j as f64 / 64. * std::f64::consts::TAU;
                projected(
                    camera,
                    rect,
                    add(
                        add(origin, mul(basis.axes[a], t.cos() * length * 0.74)),
                        mul(basis.axes[b], t.sin() * length * 0.74),
                    ),
                )
            })
            .collect::<Option<Vec<_>>>();
        if let Some(points) = ring {
            result.push(HandleShape {
                handle: Handle::Rotate(i),
                points,
                color,
                closed: false,
            });
        }
    }
    result
}
struct Drag {
    handle: Handle,
    source: NumericTransform,
    values: Values,
    camera: Camera,
    rect: Rect,
    basis: GizmoBasis,
    start: Point,
    last_angle: f64,
    angle: f64,
    last: NumericTransform,
    changed: bool,
}
impl Drag {
    fn new(
        handle: Handle,
        source: &NumericTransform,
        parameters: &EvaluatedParameters,
        input: &PlacementViewport<'_>,
        pointer: Pos2,
    ) -> Result<Self, String> {
        let values = Values::read(source, parameters)?;
        let ray = input
            .camera
            .ray(pointer, input.rect)
            .ok_or("No finite pointer ray")?;
        let origin = values.origin();
        let start = match handle {
            Handle::Axis(i) => [
                axis_parameter(ray, origin, input.basis.axes[i])
                    .ok_or("This axis is edge-on; orbit the view before dragging")?,
                0.,
                0.,
            ],
            Handle::Plane(i) | Handle::Rotate(i) => {
                plane_point(ray, origin, input.basis.axes[i])
                    .ok_or("This plane is edge-on; orbit the view before dragging")?
            }
        };
        if matches!(handle, Handle::Rotate(_))
            && dot(sub(start, origin), sub(start, origin)) < 1e-12
        {
            return Err("Choose a point on the rotation ring".into());
        }
        Ok(Self {
            handle,
            source: source.clone(),
            last: source.clone(),
            values,
            camera: input.camera,
            rect: input.rect,
            basis: input.basis,
            start,
            last_angle: 0.,
            angle: 0.,
            changed: false,
        })
    }
    fn update(
        &mut self,
        pointer: Pos2,
        parameters: &EvaluatedParameters,
    ) -> Result<Option<NumericTransform>, String> {
        let ray = self
            .camera
            .ray(pointer, self.rect)
            .ok_or("No finite pointer ray")?;
        let origin = self.values.origin();
        let next = match self.handle {
            Handle::Axis(i) => {
                let end =
                    axis_parameter(ray, origin, self.basis.axes[i]).ok_or("Axis became edge-on")?;
                translated(
                    &self.source,
                    self.values,
                    mul(self.basis.axes[i], end - self.start[0]),
                )?
            }
            Handle::Plane(i) => {
                let end =
                    plane_point(ray, origin, self.basis.axes[i]).ok_or("Plane became edge-on")?;
                translated(&self.source, self.values, sub(end, self.start))?
            }
            Handle::Rotate(i) => {
                let end = plane_point(ray, origin, self.basis.axes[i])
                    .ok_or("Rotation plane became edge-on")?;
                let a = sub(self.start, origin);
                let b = sub(end, origin);
                if dot(b, b) < 1e-12 {
                    return Ok(None);
                }
                let angle = dot(self.basis.axes[i], cross(a, b)).atan2(dot(a, b));
                let mut delta = angle - self.last_angle;
                while delta > std::f64::consts::PI {
                    delta -= std::f64::consts::TAU;
                }
                while delta < -std::f64::consts::PI {
                    delta += std::f64::consts::TAU;
                }
                self.angle += delta;
                self.last_angle = angle;
                rotated(
                    &self.source,
                    self.values,
                    self.basis.axes[i],
                    self.angle.to_degrees(),
                    parameters,
                )?
            }
        };
        if next == self.last {
            return Ok(None);
        }
        self.changed = true;
        self.last = next.clone();
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
    pub changed: Option<NumericTransform>,
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
        input: &PlacementViewport<'_>,
        source: &NumericTransform,
        parameters: &EvaluatedParameters,
    ) -> Result<Gesture, String> {
        input.basis.validate()?;
        let (pointer, pressed, released, down, alt) = ui.input(|i| {
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
            (
                i.pointer.interact_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.primary_down(),
                alt,
            )
        });
        let values = Values::read(source, parameters)?;
        let draw_origin = self
            .drag
            .as_ref()
            .map_or(values.origin(), |d| d.values.origin());
        let draw_basis = self.drag.as_ref().map_or(input.basis, |d| d.basis);
        let geometry = shapes(input.camera, input.rect, draw_origin, draw_basis);
        let hovered = pointer.filter(|p| input.rect.contains(*p)).and_then(|p| {
            geometry
                .iter()
                .map(|s| (s, s.distance(p)))
                .filter(|(_, d)| *d < 7.)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(s, _)| s.handle)
        });
        let mut started = false;
        if self.drag.is_none()
            && pressed
            && let (Some(handle), Some(pointer)) = (hovered, pointer)
        {
            self.drag = Some(Drag::new(handle, source, parameters, input, pointer)?);
            started = true;
        }
        let active = self.drag.as_ref().map(|d| d.handle);
        let painter = ui.painter_at(input.rect);
        for shape in geometry {
            let highlighted = active == Some(shape.handle) || hovered == Some(shape.handle);
            let color = if highlighted {
                Color32::WHITE
            } else {
                shape.color
            };
            if shape.closed {
                painter.add(egui::Shape::convex_polygon(
                    shape.points,
                    color.gamma_multiply(0.18),
                    Stroke::new(1., color),
                ));
            } else if matches!(shape.handle, Handle::Axis(_)) {
                let end = *shape.points.last().unwrap();
                let start = shape.points[0];
                painter.arrow(
                    start,
                    end - start,
                    Stroke::new(if highlighted { 3. } else { 2. }, color),
                );
                if let Handle::Axis(i) = shape.handle {
                    painter.text(
                        end + egui::vec2(5., -5.),
                        egui::Align2::LEFT_BOTTOM,
                        ["X", "Y", "Z"][i],
                        egui::FontId::proportional(11.),
                        color,
                    );
                }
            } else {
                painter.add(egui::Shape::line(
                    shape.points,
                    Stroke::new(if highlighted { 2.5 } else { 1.5 }, color),
                ));
            }
        }
        if let Some(handle) = hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            if let Some(pointer) = pointer {
                ui.interact(
                    Rect::from_center_size(pointer, egui::vec2(12., 12.)),
                    ui.id().with("placement-handle-tooltip"),
                    egui::Sense::hover(),
                )
                .on_hover_text(format!(
                    "{} · drag to apply one exact placement; Option / Alt copies; Esc cancels",
                    handle.label()
                ));
            }
        }
        let mut changed = None;
        if let (Some(drag), Some(pointer)) = (&mut self.drag, pointer)
            && (down || released)
        {
            changed = drag.update(pointer, parameters)?;
        }
        let moved = self.drag.as_ref().is_some_and(|d| d.changed);
        let owned = self.drag.is_some();
        let released = owned && (released || !down && !pressed);
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
mod tests;

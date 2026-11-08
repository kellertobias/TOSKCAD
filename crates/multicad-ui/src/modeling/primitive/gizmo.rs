use super::{
    super::gizmo::math::{Point, add, axis_parameter, mul, plane_point, sub},
    PrimitivePanel,
};
use crate::{camera::Camera, egui};
use multicad_core::{commands::Session, model::ResolvedPrimitive, sketch::Frame};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Field {
    Origin,
    Size(usize),
    Radius,
    Height,
}

#[derive(Clone, Copy)]
enum Motion {
    Place {
        normal: Point,
        start: Point,
    },
    Resize {
        axis: Point,
        start: f64,
        length: f64,
    },
}

pub(super) struct Drag {
    field: Field,
    origin: Point,
    motion: Motion,
}

const GUIDE: egui::Color32 = egui::Color32::from_rgb(90, 190, 240);

impl PrimitivePanel {
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn viewport(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        camera: Camera,
        rect: egui::Rect,
        guide: bool,
    ) -> (bool, bool, bool) {
        let Some(shape) = self
            .request(session.document(), true)
            .ok()
            .and_then(|request| request.parameters.resolve(session.parameters()).ok())
        else {
            self.drag = None;
            return (false, false, false);
        };
        let (frame, handles) = handles(&shape, camera, rect);
        let painter = ui.painter().with_clip_rect(rect);
        if guide {
            paint_guide(&painter, camera, rect, frame, &shape);
        }
        let (pointer, pressed, down) = ui.input(|input| {
            (
                input.pointer.latest_pos(),
                input.pointer.press_origin(),
                input.pointer.primary_down(),
            )
        });
        let mut owned = self.dragging();
        for (field, axis, length) in handles {
            let point = add(frame.origin, mul(axis, length));
            let [x, y, depth] = camera.project(point.map(|n| n as f32), rect);
            let position = egui::pos2(x, y);
            if depth <= 0. || !rect.contains(position) {
                continue;
            }
            let response = ui.interact(
                egui::Rect::from_center_size(position, egui::Vec2::splat(16.)).intersect(rect),
                ui.make_persistent_id(("primitive handle", self.request.body, field)),
                egui::Sense::drag(),
            );
            let active = self.drag.as_ref().is_some_and(|drag| drag.field == field);
            let color = if active || response.hovered() {
                egui::Color32::WHITE
            } else {
                GUIDE
            };
            if field == Field::Origin {
                painter.circle_filled(position, 5., color);
            } else {
                painter.rect_filled(
                    egui::Rect::from_center_size(position, egui::Vec2::splat(9.)),
                    1.,
                    color,
                );
            }
            let label = match field {
                Field::Origin => "Drag to place the primitive",
                Field::Size(0) => "Drag to change width",
                Field::Size(1) => "Drag to change depth",
                Field::Size(_) | Field::Height => "Drag to change height",
                Field::Radius => "Drag to change radius",
            };
            owned |= response.is_pointer_button_down_on();
            if response.drag_started_by(egui::PointerButton::Primary)
                && self.drag.is_none()
                && let Some(ray) = pressed
                    .filter(|p| rect.contains(*p))
                    .and_then(|p| camera.ray(p, rect))
            {
                let motion = if field == Field::Origin {
                    camera.ray(rect.center(), rect).and_then(|center| {
                        let normal = center.direction();
                        plane_point(ray, frame.origin, normal)
                            .map(|start| Motion::Place { normal, start })
                    })
                } else {
                    axis_parameter(ray, frame.origin, axis).map(|start| Motion::Resize {
                        axis,
                        start,
                        length,
                    })
                };
                self.drag = motion.map(|motion| Drag {
                    field,
                    origin: frame.origin,
                    motion,
                });
            }
            response.on_hover_text(label);
        }
        let mut changed = false;
        if let Some(drag) = &self.drag
            && let Some(ray) = pointer.and_then(|p| camera.ray(p, rect))
        {
            owned = true;
            match drag.motion {
                Motion::Place { normal, start } => {
                    if let Some(hit) = plane_point(ray, drag.origin, normal) {
                        let origin = add(drag.origin, sub(hit, start));
                        for (index, value) in origin.into_iter().enumerate() {
                            if (value - frame.origin[index]).abs() > 1e-8 {
                                self.origin[index].set_exact_literal(value, "mm");
                                changed = true;
                            }
                        }
                    }
                }
                Motion::Resize {
                    axis,
                    start,
                    length,
                } => {
                    if let Some(hit) = axis_parameter(ray, drag.origin, axis) {
                        let value = (length + hit - start).max(0.001);
                        let (field, current) = match (drag.field, shape) {
                            (Field::Size(i), ResolvedPrimitive::Box { size_mm, .. }) => {
                                (&mut self.size[i], size_mm[i])
                            }
                            (
                                Field::Radius,
                                ResolvedPrimitive::Cylinder { radius_mm, .. }
                                | ResolvedPrimitive::Sphere { radius_mm, .. },
                            ) => (&mut self.radius, radius_mm),
                            (Field::Height, ResolvedPrimitive::Cylinder { height_mm, .. }) => {
                                (&mut self.height, height_mm)
                            }
                            _ => unreachable!("drag belongs to this primitive"),
                        };
                        if (value - current).abs() > 1e-8 {
                            field.set_exact_literal(value, "mm");
                            changed = true;
                        }
                    }
                }
            }
        }
        if changed {
            self.stamp.changed();
            self.error = None;
        }
        let released = !down && self.drag.take().is_some();
        (owned, changed, released)
    }
}

fn handles(
    shape: &ResolvedPrimitive,
    camera: Camera,
    rect: egui::Rect,
) -> (Frame, Vec<(Field, Point, f64)>) {
    let frame = match *shape {
        ResolvedPrimitive::Box { frame, .. }
        | ResolvedPrimitive::Cylinder { frame, .. }
        | ResolvedPrimitive::Sphere { frame, .. } => frame,
    };
    let span = |axis| {
        camera
            .segment(frame.origin, add(frame.origin, mul(axis, 10.)), rect)
            .map_or(0., |[a, b]| a.distance(b))
    };
    let radial = |axes: &[Point]| {
        axes.iter()
            .copied()
            .max_by(|a, b| span(*a).total_cmp(&span(*b)))
            .unwrap_or(frame.u)
    };
    let mut sizes: Vec<_> = match *shape {
        ResolvedPrimitive::Box { size_mm, .. } => [frame.u, frame.v, frame.n]
            .into_iter()
            .zip(size_mm)
            .enumerate()
            .map(|(i, (axis, length))| (Field::Size(i), axis, length))
            .collect(),
        ResolvedPrimitive::Cylinder {
            radius_mm,
            height_mm,
            ..
        } => vec![
            (Field::Radius, radial(&[frame.u, frame.v]), radius_mm),
            (Field::Height, frame.n, height_mm),
        ],
        ResolvedPrimitive::Sphere { radius_mm, .. } => {
            vec![(
                Field::Radius,
                radial(&[frame.u, frame.v, frame.n]),
                radius_mm,
            )]
        }
    };
    sizes.retain(|&(_, axis, length)| span(axis) * (length / 10.) as f32 >= 12.);
    sizes.push((Field::Origin, [0.; 3], 0.));
    (frame, sizes)
}

fn paint_guide(
    painter: &egui::Painter,
    camera: Camera,
    rect: egui::Rect,
    frame: Frame,
    shape: &ResolvedPrimitive,
) {
    let line = |a, b| {
        if let Some(segment) = camera.segment(a, b, rect) {
            painter.line_segment(segment, egui::Stroke::new(1.5, GUIDE));
        }
    };
    let circle = |center, u, v, radius| {
        let point = |step| {
            let (sin, cos) = (f64::from(step) * std::f64::consts::TAU / 32.).sin_cos();
            add(center, add(mul(u, radius * cos), mul(v, radius * sin)))
        };
        for step in 0..32 {
            line(point(step), point(step + 1));
        }
    };
    match *shape {
        ResolvedPrimitive::Box { size_mm, .. } => {
            let corner = |bits: usize| {
                add(
                    frame.origin,
                    frame.vector(std::array::from_fn(|i| {
                        if bits & (1 << i) == 0 { 0. } else { size_mm[i] }
                    })),
                )
            };
            for bits in 0..8 {
                for axis in [1, 2, 4] {
                    if bits & axis == 0 {
                        line(corner(bits), corner(bits | axis));
                    }
                }
            }
        }
        ResolvedPrimitive::Cylinder {
            radius_mm,
            height_mm,
            ..
        } => {
            let top = add(frame.origin, mul(frame.n, height_mm));
            circle(frame.origin, frame.u, frame.v, radius_mm);
            circle(top, frame.u, frame.v, radius_mm);
            for axis in [frame.u, frame.v, mul(frame.u, -1.), mul(frame.v, -1.)] {
                let offset = mul(axis, radius_mm);
                line(add(frame.origin, offset), add(top, offset));
            }
        }
        ResolvedPrimitive::Sphere { radius_mm, .. } => {
            circle(frame.origin, frame.u, frame.v, radius_mm);
            circle(frame.origin, frame.u, frame.n, radius_mm);
            circle(frame.origin, frame.v, frame.n, radius_mm);
        }
    }
}

use super::super::gizmo::math::{Point, add, axis_parameter, cross, dot, mul, plane_point, sub};
use crate::{camera::Camera, egui, viewport::Ray};
use multicad_core::{model::ResolvedPrimitive, sketch::Frame};

const GUIDE: egui::Color32 = egui::Color32::from_rgb(90, 190, 240);
const MIN_SIZE_MM: f64 = 0.001;
const HIT_RADIUS: f32 = 9.;
const DRAG_THRESHOLD: f32 = 4.;

#[derive(Default)]
pub(super) struct ResizeGizmo {
    drag: Option<Drag>,
}

#[derive(Default)]
pub(super) struct ResizeGesture {
    pub input_owned: bool,
    pub released: bool,
    pub changed: Option<ResolvedPrimitive>,
    pub clicked_corner: Option<usize>,
}

struct Drag {
    corner: usize,
    camera: Camera,
    rect: egui::Rect,
    original: ResolvedPrimitive,
    press: egui::Pos2,
    moved: bool,
    motion: Motion,
}

enum Motion {
    Box {
        pointer: WorldDrag,
        fixed: usize,
    },
    Cylinder {
        pointer: WorldDrag,
        radial: Point,
    },
    Sphere {
        radial: egui::Vec2,
        pixels_per_mm: f64,
    },
}

enum WorldDrag {
    Plane {
        point: Point,
        normal: Point,
        start: Point,
    },
    Axis {
        point: Point,
        axis: Point,
        start: f64,
    },
}

impl WorldDrag {
    fn plane(ray: Ray, point: Point, normal: Point) -> Option<Self> {
        Some(Self::Plane {
            point,
            normal,
            start: plane_point(ray, point, normal)?,
        })
    }

    fn axis(ray: Ray, point: Point, axis: Point) -> Option<Self> {
        Some(Self::Axis {
            point,
            axis,
            start: axis_parameter(ray, point, axis)?,
        })
    }

    fn delta(&self, ray: Ray) -> Option<Point> {
        match *self {
            Self::Plane {
                point,
                normal,
                start,
            } => Some(sub(plane_point(ray, point, normal)?, start)),
            Self::Axis { point, axis, start } => {
                Some(mul(axis, axis_parameter(ray, point, axis)? - start))
            }
        }
    }
}

fn frame(shape: &ResolvedPrimitive) -> Frame {
    match *shape {
        ResolvedPrimitive::Box { frame, .. }
        | ResolvedPrimitive::Cylinder { frame, .. }
        | ResolvedPrimitive::Sphere { frame, .. } => frame,
    }
}

fn high(corner: usize, axis: usize) -> bool {
    corner & (1 << axis) != 0
}

/// The low and high sides of the primitive's oriented, local envelope.
/// Bits 0, 1 and 2 select the high U, V and N sides, respectively.
pub(super) fn corners(shape: &ResolvedPrimitive) -> [Point; 8] {
    let [min, max] = match *shape {
        ResolvedPrimitive::Box { size_mm, .. } => [[0.; 3], size_mm],
        ResolvedPrimitive::Cylinder {
            radius_mm,
            height_mm,
            ..
        } => [
            [-radius_mm, -radius_mm, 0.],
            [radius_mm, radius_mm, height_mm],
        ],
        ResolvedPrimitive::Sphere { radius_mm, .. } => [[-radius_mm; 3], [radius_mm; 3]],
    };
    let frame = frame(shape);
    std::array::from_fn(|corner| {
        let local = std::array::from_fn(|axis| {
            if high(corner, axis) {
                max[axis]
            } else {
                min[axis]
            }
        });
        add(frame.origin, frame.vector(local))
    })
}

impl Drag {
    fn new(
        corner: usize,
        shape: &ResolvedPrimitive,
        camera: Camera,
        rect: egui::Rect,
        press: egui::Pos2,
        point: Point,
    ) -> Option<Self> {
        let frame = frame(shape);
        let axes = [frame.u, frame.v, frame.n];
        let ray = camera.ray(press, rect)?;
        let alignment = |axis| dot(ray.direction(), axis).abs();
        let motion = match *shape {
            ResolvedPrimitive::Box { .. } => {
                // The plane with the largest ray-normal dot is the least
                // sensitive to pointer motion and keeps the third size fixed.
                let fixed =
                    (0..3).max_by(|a, b| alignment(axes[*a]).total_cmp(&alignment(axes[*b])))?;
                Motion::Box {
                    pointer: WorldDrag::plane(ray, point, axes[fixed])?,
                    fixed,
                }
            }
            ResolvedPrimitive::Cylinder { .. } => {
                let signed = |i: usize| mul(axes[i], if high(corner, i) { 1. } else { -1. });
                let radial = mul(add(signed(0), signed(1)), std::f64::consts::FRAC_1_SQRT_2);
                let normal = cross(radial, frame.n);
                let pointer = if alignment(normal) >= 0.25 {
                    WorldDrag::plane(ray, point, normal)?
                } else {
                    let axis = if alignment(radial) < alignment(frame.n) {
                        radial
                    } else {
                        frame.n
                    };
                    WorldDrag::axis(ray, point, axis)?
                };
                Motion::Cylinder { pointer, radial }
            }
            ResolvedPrimitive::Sphere { radius_mm, .. } => {
                let center = camera.project(frame.origin.map(|coordinate| coordinate as f32), rect);
                let end = camera.project(point.map(|coordinate| coordinate as f32), rect);
                let direction = egui::vec2(end[0] - center[0], end[1] - center[1]);
                let length = direction.length();
                let (radial, pixels_per_mm) = if length >= 4. {
                    (direction / length, f64::from(length) / radius_mm)
                } else {
                    // A corner along the sightline still supports uniform
                    // scaling by a horizontal gesture.
                    let scale = f64::from(camera.scale);
                    let pixels = if camera.perspective {
                        f64::from(rect.height().max(1.)) * 2. / f64::from(center[2].max(0.1))
                    } else {
                        scale
                    };
                    (egui::vec2(1., 0.), pixels)
                };
                Motion::Sphere {
                    radial,
                    pixels_per_mm,
                }
            }
        };
        Some(Self {
            corner,
            camera,
            rect,
            original: shape.clone(),
            press,
            moved: false,
            motion,
        })
    }

    fn update(&self, position: egui::Pos2) -> Option<ResolvedPrimitive> {
        if !position.is_finite() {
            return None;
        }
        let ray = || self.camera.ray(position, self.rect);
        let result = match (&self.motion, &self.original) {
            (Motion::Box { pointer, fixed }, ResolvedPrimitive::Box { size_mm, frame }) => {
                let mut frame = *frame;
                let mut next = *size_mm;
                let delta = pointer.delta(ray()?)?;
                for (axis, direction) in [frame.u, frame.v, frame.n].into_iter().enumerate() {
                    if axis == *fixed {
                        continue;
                    }
                    let (size, shift) = move_side(
                        size_mm[axis],
                        dot(delta, direction),
                        high(self.corner, axis),
                    );
                    next[axis] = size;
                    frame.origin = add(frame.origin, mul(direction, shift));
                }
                ResolvedPrimitive::Box {
                    size_mm: next,
                    frame,
                }
            }
            (
                Motion::Cylinder { pointer, radial },
                ResolvedPrimitive::Cylinder {
                    radius_mm,
                    height_mm,
                    frame,
                },
            ) => {
                let mut frame = *frame;
                let delta = pointer.delta(ray()?)?;
                let radius_mm = (radius_mm + dot(delta, *radial) * std::f64::consts::FRAC_1_SQRT_2)
                    .max(radius_mm.min(MIN_SIZE_MM));
                let (height_mm, shift) =
                    move_side(*height_mm, dot(delta, frame.n), high(self.corner, 2));
                frame.origin = add(frame.origin, mul(frame.n, shift));
                ResolvedPrimitive::Cylinder {
                    radius_mm,
                    height_mm,
                    frame,
                }
            }
            (
                Motion::Sphere {
                    radial,
                    pixels_per_mm,
                },
                ResolvedPrimitive::Sphere { radius_mm, frame },
            ) => {
                let delta = f64::from((position - self.press).dot(*radial)) / pixels_per_mm;
                ResolvedPrimitive::Sphere {
                    radius_mm: (radius_mm + delta).max(radius_mm.min(MIN_SIZE_MM)),
                    frame: *frame,
                }
            }
            _ => return None,
        };
        Some(result)
    }
}

fn move_side(size: f64, delta: f64, high: bool) -> (f64, f64) {
    let minimum = size.min(MIN_SIZE_MM);
    if high {
        ((size + delta).max(minimum), 0.)
    } else {
        let next = (size - delta).max(minimum);
        (next, size - next)
    }
}

#[derive(Clone, Copy)]
struct Handle {
    corner: usize,
    position: egui::Pos2,
    depth: f32,
}

fn visible_handles(points: [Point; 8], camera: Camera, rect: egui::Rect) -> Vec<Handle> {
    let mut handles: Vec<_> = points
        .into_iter()
        .enumerate()
        .filter_map(|(corner, point)| {
            let [x, y, depth] = camera.project(point.map(|coordinate| coordinate as f32), rect);
            let position = egui::pos2(x, y);
            (depth.is_finite() && (!camera.perspective || depth > 0.1) && rect.contains(position))
                .then_some(Handle {
                    corner,
                    position,
                    depth,
                })
        })
        .collect();
    handles.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    handles
}

impl ResizeGizmo {
    pub(super) fn active(&self) -> bool {
        self.drag.is_some()
    }

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        shape: &ResolvedPrimitive,
        camera: Camera,
        rect: egui::Rect,
        start_allowed: bool,
        guide: bool,
    ) -> ResizeGesture {
        if self.drag.as_ref().is_some_and(|drag| {
            std::mem::discriminant(shape) != std::mem::discriminant(&drag.original)
        }) {
            self.drag = None;
        }
        let (pointer, press, release, down, crossed) = ui.input(|input| {
            let pointer = input.pointer.interact_pos();
            let mut origin = self.drag.as_ref().map(|drag| drag.press);
            let mut press = None;
            let mut release = None;
            let mut crossed = false;
            let beyond =
                |a: egui::Pos2, b: egui::Pos2| a.distance_sq(b) > DRAG_THRESHOLD * DRAG_THRESHOLD;
            // Use ordered absolute positions. The final pointer may be back
            // where it started even though this frame contains a real drag.
            for event in &input.events {
                match *event {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        ..
                    } if origin.is_none() => {
                        origin = Some(pos);
                        press = origin;
                    }
                    egui::Event::PointerMoved(pos) => {
                        crossed |= origin.is_some_and(|start| beyond(start, pos));
                    }
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        ..
                    } if origin.is_some() => {
                        release = Some(pos);
                        break;
                    }
                    _ => {}
                }
            }
            if origin.is_none() && input.pointer.primary_pressed() {
                press = input.pointer.press_origin().or(pointer);
                origin = press;
            }
            crossed |= origin
                .zip(release.or(pointer))
                .is_some_and(|(start, end)| beyond(start, end));
            (
                pointer,
                press,
                release,
                input.pointer.primary_down(),
                crossed,
            )
        });
        let points = corners(shape);
        let hit_pointer = press.or(pointer);
        let hovered = hit_pointer
            .filter(|p| start_allowed && rect.contains(*p))
            .and_then(|p| {
                visible_handles(points, camera, rect)
                    .into_iter()
                    .map(|handle| (handle.position.distance(p), handle))
                    .filter(|(distance, _)| *distance <= HIT_RADIUS)
                    .min_by(|(a, ha), (b, hb)| {
                        a.total_cmp(b).then_with(|| ha.depth.total_cmp(&hb.depth))
                    })
                    .map(|(_, handle)| handle)
            });
        if self.drag.is_none()
            && let (Some(handle), Some(press)) = (hovered, press)
        {
            self.drag = Drag::new(
                handle.corner,
                shape,
                camera,
                rect,
                press,
                points[handle.corner],
            );
        }

        if let Some(drag) = &mut self.drag {
            drag.moved |= crossed;
        }
        let changed = self
            .drag
            .as_ref()
            .filter(|drag| drag.moved && (down || release.is_some()))
            .and_then(|drag| drag.update(release.or(pointer).or(press)?))
            .filter(|next| next != shape);
        let preview = changed.as_ref().unwrap_or(shape);
        let points = corners(preview);
        let painter = ui.painter_at(rect);
        paint_envelope(&painter, points, camera, rect);
        if guide {
            paint_curves(&painter, preview, camera, rect);
        }
        let active = self.drag.as_ref().map(|drag| drag.corner);
        for handle in visible_handles(points, camera, rect) {
            let selected = active == Some(handle.corner)
                || active.is_none() && hovered.is_some_and(|hit| hit.corner == handle.corner);
            let bounds = egui::Rect::from_center_size(handle.position, egui::Vec2::splat(8.));
            painter.rect_filled(
                bounds,
                1.,
                if selected {
                    egui::Color32::WHITE
                } else {
                    GUIDE
                },
            );
        }
        if self.drag.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if let Some(handle) = hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            ui.interact(
                egui::Rect::from_center_size(handle.position, egui::Vec2::splat(2. * HIT_RADIUS))
                    .intersect(rect),
                ui.id().with("primitive-resize-tooltip"),
                egui::Sense::hover(),
            )
            .on_hover_text("Click to edit this corner · drag to resize");
        }

        let input_owned = self.drag.is_some();
        let released = input_owned && (release.is_some() || !down);
        let clicked_corner = if released {
            self.drag
                .take()
                .filter(|drag| !drag.moved && release.is_some())
                .map(|drag| drag.corner)
        } else {
            None
        };
        ResizeGesture {
            input_owned,
            released,
            changed,
            clicked_corner,
        }
    }
}

fn line(painter: &egui::Painter, camera: Camera, rect: egui::Rect, a: Point, b: Point) {
    if let Some(segment) = camera.segment(a, b, rect) {
        painter.line_segment(segment, egui::Stroke::new(1.25, GUIDE));
    }
}

fn paint_envelope(painter: &egui::Painter, points: [Point; 8], camera: Camera, rect: egui::Rect) {
    for (corner, point) in points.into_iter().enumerate() {
        for bit in [1, 2, 4] {
            if corner & bit == 0 {
                line(painter, camera, rect, point, points[corner | bit]);
            }
        }
    }
}

fn paint_curves(
    painter: &egui::Painter,
    shape: &ResolvedPrimitive,
    camera: Camera,
    rect: egui::Rect,
) {
    let frame = frame(shape);
    let circle = |center, u, v, radius| {
        let point = |step| {
            let (sin, cos) = (f64::from(step) * std::f64::consts::TAU / 32.).sin_cos();
            add(center, add(mul(u, radius * cos), mul(v, radius * sin)))
        };
        for step in 0..32 {
            line(painter, camera, rect, point(step), point(step + 1));
        }
    };
    let view = |point: Point| {
        let [x, y, _] = camera.project(point.map(|coordinate| coordinate as f32), rect);
        camera
            .ray(egui::pos2(x, y), rect)
            .map(|ray| ray.direction())
    };
    let unit = |vector: Point| {
        let length = dot(vector, vector).sqrt();
        (length > 1e-8).then(|| mul(vector, 1. / length))
    };
    match *shape {
        ResolvedPrimitive::Box { .. } => {}
        ResolvedPrimitive::Cylinder {
            radius_mm,
            height_mm,
            ..
        } => {
            let top = add(frame.origin, mul(frame.n, height_mm));
            circle(frame.origin, frame.u, frame.v, radius_mm);
            circle(top, frame.u, frame.v, radius_mm);
            if let Some(tangent) = view(add(frame.origin, mul(frame.n, height_mm * 0.5)))
                .and_then(|direction| unit(cross(direction, frame.n)))
            {
                for sign in [-1., 1.] {
                    let offset = mul(tangent, sign * radius_mm);
                    line(
                        painter,
                        camera,
                        rect,
                        add(frame.origin, offset),
                        add(top, offset),
                    );
                }
            }
        }
        ResolvedPrimitive::Sphere { radius_mm, .. } => {
            if let Some(direction) = view(frame.origin)
                && let Some(u) =
                    unit(cross(direction, frame.n)).or_else(|| unit(cross(direction, frame.u)))
            {
                circle(frame.origin, u, cross(direction, u), radius_mm);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Frame = Frame {
        origin: [0.; 3],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
        n: [0., 0., 1.],
    };

    fn rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.))
    }

    fn project(camera: Camera, point: Point) -> egui::Pos2 {
        let [x, y, _] = camera.project(point.map(|coordinate| coordinate as f32), rect());
        egui::pos2(x, y)
    }

    fn camera() -> Camera {
        Camera {
            yaw: 0.,
            pitch: std::f32::consts::FRAC_PI_2,
            scale: 3.,
            target: [0.; 3],
            perspective: false,
        }
    }

    fn near(value: f64, expected: f64) {
        assert!((value - expected).abs() < 1e-5, "{value} != {expected}");
    }

    #[test]
    fn low_box_corner_moves_the_frame_but_keeps_the_oriented_opposite_corner() {
        let frame = Frame {
            origin: [5., 7., 11.],
            u: [0., 1., 0.],
            v: [-1., 0., 0.],
            ..FRAME
        };
        let shape = ResolvedPrimitive::Box {
            size_mm: [20., 30., 40.],
            frame,
        };
        let points = corners(&shape);
        let start = points[0];
        let delta = frame.vector([2., 3., 0.]);
        let camera = camera();
        let drag = Drag::new(0, &shape, camera, rect(), project(camera, start), start).unwrap();
        let result = drag.update(project(camera, add(start, delta))).unwrap();
        let ResolvedPrimitive::Box {
            size_mm,
            frame: moved,
        } = &result
        else {
            panic!("box remains a box");
        };
        for (value, expected) in size_mm.iter().zip([18., 27., 40.]) {
            near(*value, expected);
        }
        for (value, expected) in moved.origin.into_iter().zip(add(frame.origin, delta)) {
            near(value, expected);
        }
        assert_eq!((moved.u, moved.v, moved.n), (frame.u, frame.v, frame.n));
        for (value, expected) in corners(&result)[7].into_iter().zip(points[7]) {
            near(value, expected);
        }
    }

    #[test]
    fn cylinder_corner_changes_radial_size_and_anchors_the_other_height_end() {
        let shape = ResolvedPrimitive::Cylinder {
            radius_mm: 10.,
            height_mm: 20.,
            frame: FRAME,
        };
        let points = corners(&shape);
        let camera = Camera {
            yaw: 3. * std::f32::consts::FRAC_PI_4,
            pitch: 0.,
            ..camera()
        };
        let start = points[3];
        let drag = Drag::new(3, &shape, camera, rect(), project(camera, start), start).unwrap();
        let resized = drag
            .update(project(camera, add(start, [3., 3., 4.])))
            .unwrap();
        let ResolvedPrimitive::Cylinder {
            radius_mm,
            height_mm,
            frame,
        } = resized
        else {
            panic!("cylinder remains a cylinder");
        };
        near(radius_mm, 13.);
        near(height_mm, 16.);
        near(frame.origin[2], 4.);
        near(frame.origin[2] + height_mm, 20.);
        assert_eq!(frame.origin[..2], [0., 0.]);
    }

    #[test]
    fn sphere_corner_preserves_center_and_uniform_radius_in_both_projections() {
        let shape = ResolvedPrimitive::Sphere {
            radius_mm: 10.,
            frame: FRAME,
        };
        let point = corners(&shape)[7];
        for perspective in [false, true] {
            let camera = Camera {
                perspective,
                ..camera()
            };
            let start = project(camera, point);
            let projected_radius = start - project(camera, FRAME.origin);
            let drag = Drag::new(7, &shape, camera, rect(), start, point).unwrap();
            let result = drag.update(start + projected_radius * 0.5).unwrap();
            let ResolvedPrimitive::Sphere { radius_mm, frame } = result else {
                panic!("sphere remains a sphere");
            };
            near(radius_mm, 15.);
            assert_eq!(frame, FRAME);
        }
    }

    #[test]
    fn corner_clicks_without_resizing_and_retains_drag_classification_through_release() {
        let shape = ResolvedPrimitive::Box {
            size_mm: [20., 30., 40.],
            frame: FRAME,
        };
        let camera = camera();
        let start = project(camera, corners(&shape)[7]);
        let end = start + egui::vec2(12., -6.);
        let mut gizmo = ResizeGizmo::default();
        let ctx = egui::Context::default();
        let mut frame = |shape: &ResolvedPrimitive, events, start_allowed| {
            let mut result = ResizeGesture::default();
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(rect()),
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ui| {
                    result = gizmo.show(ui, shape, camera, rect(), start_allowed, false);
                },
            )
            .drop_without_applying_deltas();
            result
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let near = start + egui::vec2(4., 0.);
        assert!(!frame(&shape, vec![egui::Event::PointerMoved(start)], true).input_owned);
        let press = frame(&shape, vec![button(start, true)], true);
        assert!(press.input_owned && !press.released && press.changed.is_none());
        assert!(press.clicked_corner.is_none());
        let motion = frame(&shape, vec![egui::Event::PointerMoved(near)], false);
        assert!(motion.input_owned && motion.changed.is_none());
        let click = frame(&shape, vec![button(near, false)], false);
        assert!(click.input_owned && click.released && click.changed.is_none());
        assert_eq!(click.clicked_corner, Some(7));

        frame(&shape, vec![button(start, true)], true);
        let drag = frame(&shape, vec![egui::Event::PointerMoved(end)], false);
        assert!(drag.input_owned && drag.clicked_corner.is_none());
        let staged = drag.changed.expect("actual drag updates the draft");
        let release = frame(&staged, vec![button(start, false)], false);
        assert!(release.input_owned && release.released && release.clicked_corner.is_none());
        assert_eq!(release.changed, Some(shape.clone()));

        let batch = frame(
            &shape,
            vec![
                button(start, true),
                egui::Event::PointerMoved(end),
                button(end, false),
            ],
            true,
        );
        assert!(batch.input_owned && batch.released && batch.changed.is_some());
        assert!(batch.clicked_corner.is_none());
        let returned = frame(
            &shape,
            vec![
                button(start, true),
                egui::Event::PointerMoved(end),
                egui::Event::PointerMoved(start),
                button(start, false),
            ],
            true,
        );
        assert!(returned.input_owned && returned.released && returned.changed.is_none());
        assert!(returned.clicked_corner.is_none());

        let batched_click = frame(&shape, vec![button(start, true), button(near, false)], true);
        assert_eq!(batched_click.clicked_corner, Some(7));
        assert!(batched_click.changed.is_none());
        assert!(!gizmo.active());
    }
}

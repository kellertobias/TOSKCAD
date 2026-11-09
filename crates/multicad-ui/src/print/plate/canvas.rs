//! Display-only geometry and pointer controls for arranged 3MF output copies.
use super::{
    Document, EntityId, PartRef, PrintPlateDefinition, PrintPlatePlacement, PrintPreview, Scene,
    Session, source_label,
};
use crate::{
    camera::Camera,
    egui::{self, Align2, Color32, FontId, Pos2, Rect, Stroke, Vec2},
    navigation::DisplayStyle,
    viewport::{self, Geometry, Instance, SceneStamp, Selection, Transform},
};
use multicad_core::{
    document::EntityData,
    exchange::PrintKey,
    model::{OccurrencePath, RigidPose},
};
use std::{collections::BTreeMap, sync::Arc};

type Bounds = [[f64; 3]; 2];
const BED: Color32 = Color32::from_rgb(37, 48, 61);
const GRID: Color32 = Color32::from_rgb(58, 72, 88);
const ACCENT: Color32 = Color32::from_rgb(95, 205, 240);
const WARNING: Color32 = Color32::from_rgb(250, 173, 88);

struct Object {
    id: EntityId,
    producer: EntityId,
    angles: [f64; 3],
    /// Geometry at the current orientation before snapping to the bed.
    oriented_bounds: Bounds,
    /// Actual (if available) or display-mesh plate bounds.
    bounds: Bounds,
}
enum Drag {
    Move {
        index: usize,
        start: Pos2,
        position: [f64; 2],
        scale: f32,
    },
    Rotate {
        index: usize,
        center: Pos2,
        last_bearing: f32,
    },
}
#[derive(Default)]
pub(super) struct View {
    stamp: Option<SceneStamp>,
    key: Option<PrintKey>,
    placements: Vec<PrintPlatePlacement>,
    objects: Vec<Option<Object>>,
    scene: Option<Arc<Scene>>,
    zoom: Option<f32>,
    pan: Vec2,
    drag: Option<Drag>,
}
impl View {
    pub(super) fn initial_position(
        &self,
        scene: Option<&Scene>,
        session: &Session,
        definition: &PrintPlateDefinition,
        added: &PrintPlatePlacement,
    ) -> [f64; 2] {
        const GAP: f64 = 5.;
        let Some(scene) = current_scene(session, scene) else {
            let offset = GAP * (definition.placements.len() + 1) as f64;
            return [offset, offset];
        };
        let mut occupied: Vec<Bounds> = definition
            .placements
            .iter()
            .filter_map(|placement| {
                let (geometry, _) = source_geometry(session.document(), scene, &placement.source)?;
                let bounds = oriented_bounds(&geometry, placement.rotation());
                Some(on_bed(bounds, placement.position_mm))
            })
            .collect();
        let Some((geometry, _)) = source_geometry(session.document(), scene, &added.source) else {
            return [GAP, GAP];
        };
        let bounds = oriented_bounds(&geometry, added.rotation());
        let size: [f64; 2] = std::array::from_fn(|axis| bounds[1][axis] - bounds[0][axis]);
        let mut x = vec![GAP];
        let mut y = vec![GAP];
        x.extend(occupied.iter().map(|bounds| bounds[1][0] + GAP));
        y.extend(occupied.iter().map(|bounds| bounds[1][1] + GAP));
        x.sort_by(f64::total_cmp);
        y.sort_by(f64::total_cmp);
        for y in y {
            for &x in &x {
                if x + size[0] > definition.bed_mm[0] || y + size[1] > definition.bed_mm[1] {
                    continue;
                }
                let footprint = on_bed(bounds, [x, y]);
                if !occupied.iter().any(|other| overlaps(other, &footprint)) {
                    return [x, y];
                }
            }
        }
        [
            GAP,
            occupied
                .drain(..)
                .map(|bounds| bounds[1][1])
                .fold(0., f64::max)
                + GAP,
        ]
    }

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        source: Option<&Scene>,
        definition: &mut PrintPlateDefinition,
        selected: &mut Option<usize>,
        preview: Option<&PrintPreview>,
    ) {
        let source = current_scene(session, source);
        self.sync(session.document(), source, definition, preview);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Fit plate").clicked() {
                self.zoom = None;
                self.pan = Vec2::ZERO;
            }
            ui.small("Drag to move · drag the round handle to rotate · scroll to zoom");
        });
        let (outside, overlap) = if preview.is_some_and(PrintPreview::can_write) {
            (None, None)
        } else {
            layout_warning(&self.objects, definition.bed_mm)
        };
        ui.allocate_ui(egui::vec2(ui.available_width(), 34.), |ui| {
            if let Some(reason) = preview.and_then(|preview| preview.blocked.as_deref()) {
                ui.colored_label(Color32::LIGHT_RED, reason);
            } else if preview.is_none() {
                if let Some(index) = outside
                    && let Some(placement) = definition.placements.get(index)
                {
                    let label = source_label(session.document(), &placement.source);
                    ui.colored_label(
                        WARNING,
                        format!("{label} is outside the bed. Move or rotate it, or increase the bed size."),
                    );
                } else if let Some([a, b]) = overlap {
                    ui.colored_label(WARNING, format!("Placements {} and {} may overlap. Move or rotate a copy and check the native preview.", a + 1, b + 1));
                }
            }
            if !definition.placements.is_empty() && self.objects.iter().any(Option::is_none) {
                ui.small(if source.is_none() {
                    "Waiting for the current native Model geometry…"
                } else {
                    "A placed source has no current native geometry. Check that Part or occurrence in Model."
                });
            }
        });
        let size = ui.available_size();
        let (_, rect) = ui.allocate_space(egui::vec2(size.x.max(1.), size.y.max(80.)));
        let response = ui.interact(
            rect,
            ui.make_persistent_id("print-plate-canvas"),
            egui::Sense::click_and_drag(),
        );
        let painter = ui.painter_at(rect);
        let mut camera = self.camera(rect, definition.bed_mm);
        if response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle)
        {
            self.pan += ui.input(|input| input.pointer.delta());
            camera = self.camera(rect, definition.bed_mm);
        }
        if let Some(pointer) = response.hover_pos() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll != 0. && scroll.is_finite() {
                let old = self.zoom.unwrap_or(1.);
                let next = (old * (scroll * 0.006).exp()).clamp(0.1, 30.);
                let anchor = pointer - rect.center();
                self.pan = anchor + (self.pan - anchor) * (next / old);
                self.zoom = Some(next);
                camera = self.camera(rect, definition.bed_mm);
            }
        }
        let changed = self.pointer(ui, &response, camera, definition, selected);
        if changed {
            self.sync(session.document(), source, definition, None);
            ui.ctx().request_repaint();
        }
        paint_bed(&painter, rect, camera, definition.bed_mm);
        if let Some(scene) = self.scene.clone() {
            let highlight = selected
                .and_then(|index| self.objects.get(index)?.as_ref())
                .map(|object| Selection::Part(object.id));
            viewport::paint_scene(
                &painter,
                rect,
                scene,
                camera,
                highlight,
                DisplayStyle::VisibleEdges,
            );
        }
        for (index, object) in self.objects.iter().enumerate() {
            let Some(object) = object else {
                continue;
            };
            let footprint = bounds_rect(object.bounds, camera, rect);
            let color =
                if outside == Some(index) || overlap.is_some_and(|pair| pair.contains(&index)) {
                    WARNING
                } else if *selected == Some(index) {
                    ACCENT
                } else {
                    Color32::from_gray(155)
                };
            painter.rect_stroke(
                footprint,
                2.,
                Stroke::new(1., color),
                egui::StrokeKind::Inside,
            );
            painter.text(
                footprint.left_top() + egui::vec2(4., 3.),
                Align2::LEFT_TOP,
                index + 1,
                FontId::proportional(12.),
                color,
            );
            if *selected == Some(index) {
                let handle = rotation_handle(footprint);
                painter.line_segment([footprint.right_top(), handle], Stroke::new(1., ACCENT));
                painter.circle_filled(handle, 10., BED);
                painter.circle_stroke(handle, 10., Stroke::new(1.3, ACCENT));
                painter.text(
                    handle,
                    Align2::CENTER_CENTER,
                    "↻",
                    FontId::proportional(14.),
                    ACCENT,
                );
            }
        }
        if definition.placements.is_empty() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Add a Part or placed occurrence in Settings",
                FontId::proportional(13.),
                Color32::LIGHT_GRAY,
            );
        }
    }

    fn camera(&self, rect: Rect, bed: [f64; 2]) -> Camera {
        let dimensions = bed.map(|dimension| dimension.max(1.) as f32);
        let fit = ((rect.width() - 54.).max(10.) / dimensions[0])
            .min((rect.height() - 54.).max(10.) / dimensions[1]);
        let scale = (fit * self.zoom.unwrap_or(1.)).clamp(0.00001, 300.);
        Camera {
            yaw: 0.,
            pitch: -std::f32::consts::FRAC_PI_2,
            scale,
            target: [
                dimensions[0] * 0.5 - self.pan.x / scale,
                dimensions[1] * 0.5 + self.pan.y / scale,
                0.,
            ],
            perspective: false,
        }
    }

    fn pointer(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        camera: Camera,
        definition: &mut PrintPlateDefinition,
        selected: &mut Option<usize>,
    ) -> bool {
        let button = egui::PointerButton::Primary;
        if response.drag_started_by(button)
            && let Some(start) = ui.input(|input| input.pointer.press_origin())
        {
            self.drag = selected
                .and_then(|index| Some((index, self.objects.get(index)?.as_ref()?)))
                .and_then(|(index, object)| {
                    let footprint = bounds_rect(object.bounds, camera, response.rect);
                    let center = footprint.center();
                    (rotation_handle(footprint).distance(start) <= 14.).then(|| Drag::Rotate {
                        index,
                        center,
                        last_bearing: bearing(start, center),
                    })
                })
                .or_else(|| {
                    let index = self.pick(start, camera, response.rect)?;
                    *selected = Some(index);
                    Some(Drag::Move {
                        index,
                        start,
                        position: definition.placements[index].position_mm,
                        scale: camera.scale,
                    })
                });
        }
        if response.clicked_by(button)
            && let Some(pointer) = response.interact_pointer_pos()
        {
            *selected = self.pick(pointer, camera, response.rect);
        }
        let mut changed = false;
        if let Some(pointer) = response.interact_pointer_pos()
            && (response.dragged_by(button) || response.drag_stopped_by(button))
        {
            match &mut self.drag {
                Some(Drag::Move {
                    index,
                    start,
                    position,
                    scale,
                }) => {
                    let delta = (pointer - *start) / *scale;
                    if let Some(placement) = definition.placements.get_mut(*index) {
                        let position = [
                            (position[0] + f64::from(delta.x)).clamp(-1_000_000., 1_000_000.),
                            (position[1] - f64::from(delta.y)).clamp(-1_000_000., 1_000_000.),
                        ];
                        changed = placement.position_mm != position;
                        placement.position_mm = position;
                    }
                }
                Some(Drag::Rotate {
                    index,
                    center,
                    last_bearing,
                }) => {
                    let angle = bearing(pointer, *center);
                    let delta = (angle - *last_bearing + 180.).rem_euclid(360.) - 180.;
                    *last_bearing = angle;
                    if delta.abs() > f32::EPSILON
                        && let Some(placement) = definition.placements.get_mut(*index)
                    {
                        let angle = placement.rotation_deg[2] + f64::from(delta);
                        placement.rotation_deg[2] = angle.rem_euclid(360.);
                        changed = true;
                    }
                }
                None => {}
            }
        }
        if response.drag_stopped_by(button) || !ui.input(|input| input.pointer.primary_down()) {
            self.drag = None;
        }
        changed
    }

    fn pick(&self, pointer: Pos2, camera: Camera, rect: Rect) -> Option<usize> {
        if let Some(hit) = camera
            .ray(pointer, rect)
            .and_then(|ray| self.scene.as_ref()?.pick_face(ray))
            && let Some(index) = self.objects.iter().position(|object| {
                object
                    .as_ref()
                    .is_some_and(|object| object.id == hit.instance)
            })
        {
            return Some(index);
        }
        self.objects.iter().rposition(|object| {
            object.as_ref().is_some_and(|object| {
                bounds_rect(object.bounds, camera, rect)
                    .expand(3.)
                    .contains(pointer)
            })
        })
    }

    fn sync(
        &mut self,
        document: &Document,
        scene: Option<&Scene>,
        definition: &PrintPlateDefinition,
        preview: Option<&PrintPreview>,
    ) {
        let stamp = scene.map(Scene::stamp);
        let key = preview.map(|preview| &preview.key);
        if self.stamp == stamp
            && self.key.as_ref() == key
            && self.placements == definition.placements
        {
            return;
        }
        let old = std::mem::take(&mut self.objects);
        let cache_current = self.stamp == stamp;
        self.stamp = stamp;
        self.key = key.cloned();
        self.placements.clone_from(&definition.placements);
        self.scene = None;
        let mut geometries = BTreeMap::new();
        let mut instances = vec![];
        self.objects = definition
            .placements
            .iter()
            .enumerate()
            .map(|(index, placement)| {
                let scene = scene?;
                let (geometry, color) = source_geometry(document, scene, &placement.source)?;
                let previous = old.get(index).and_then(Option::as_ref);
                let rotation = placement.rotation();
                let oriented = previous
                    .filter(|object| {
                        cache_current
                            && object.producer == geometry.id()
                            && object.angles == placement.rotation_deg
                    })
                    .map_or_else(
                        || oriented_bounds(&geometry, rotation),
                        |object| object.oriented_bounds,
                    );
                let mut bounds = on_bed(oriented, placement.position_mm);
                let mut pose = RigidPose {
                    rotation,
                    translation_mm: std::array::from_fn(|i| bounds[0][i] - oriented[0][i]),
                };
                if let Some(object) = preview.and_then(|preview| preview.objects.get(index))
                    && object.part == placement.source.part
                    && object.occurrence_path == placement.source.occurrence_path
                    && let Some(exact) = object.plate_pose
                {
                    bounds = object.bounds_mm;
                    pose = exact;
                }
                let id = previous.map_or_else(EntityId::new, |object| object.id);
                instances.push(Instance {
                    id,
                    geometry: geometry.id(),
                    transform: Transform::from_rigid_pose(pose).ok()?,
                    color,
                });
                let producer = geometry.id();
                geometries.entry(producer).or_insert(geometry);
                Some(Object {
                    id,
                    producer,
                    angles: placement.rotation_deg,
                    oriented_bounds: oriented,
                    bounds,
                })
            })
            .collect();
        if let Some(stamp) = stamp
            && !instances.is_empty()
        {
            self.scene = Scene::new(stamp, geometries.into_values().collect(), instances)
                .ok()
                .map(Arc::new);
        }
    }
}

fn current_scene<'a>(session: &Session, scene: Option<&'a Scene>) -> Option<&'a Scene> {
    scene.filter(|scene| {
        let stamp = scene.stamp();
        scene.is_full_model()
            && stamp.document == session.document().id()
            && stamp.revision == session.revision()
    })
}

fn source_geometry(
    document: &Document,
    scene: &Scene,
    source: &PartRef,
) -> Option<(Arc<Geometry>, [u8; 4])> {
    let producer = scene.source_producer(source.part)?;
    let occurrence = if let Some((&first, members)) = source.occurrence_path.split_first() {
        let assembly_root = source.root_assembly.filter(|root| {
            matches!(document.get(*root).map(|entity| &entity.data),
                Some(EntityData::Assembly { instances }) if instances.contains(&first))
        });
        let path = assembly_root.map_or_else(
            || OccurrencePath {
                root: first,
                members: members.to_vec(),
            },
            |root| OccurrencePath {
                root,
                members: source.occurrence_path.clone(),
            },
        );
        let leaf = scene.assembly_evaluation()?.leaves.iter().find(|leaf| {
            leaf.path == path && leaf.component == source.part && leaf.producer == producer
        })?;
        scene
            .reference_instances()
            .iter()
            .find(|instance| instance.id == leaf.path.root)?
    } else {
        scene
            .reference_instances()
            .iter()
            .find(|instance| instance.id == source.part && instance.geometry == producer)?
    };
    Some((scene.geometries().get(&producer)?.clone(), occurrence.color))
}

fn oriented_bounds(geometry: &Geometry, rotation: [[f64; 3]; 3]) -> Bounds {
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    let pose = RigidPose {
        rotation,
        ..Default::default()
    };
    for vertex in geometry.vertices() {
        let point = pose.vector(*vertex);
        for (axis, coordinate) in point.into_iter().enumerate() {
            bounds[0][axis] = bounds[0][axis].min(coordinate);
            bounds[1][axis] = bounds[1][axis].max(coordinate);
        }
    }
    bounds
}
fn on_bed(bounds: Bounds, [x, y]: [f64; 2]) -> Bounds {
    let low = [x, y, 0.];
    let high = std::array::from_fn(|i| low[i] + bounds[1][i] - bounds[0][i]);
    [low, high]
}
fn overlaps(a: &Bounds, b: &Bounds) -> bool {
    (0..2).all(|i| a[0][i].max(b[0][i]) + 1e-5 < a[1][i].min(b[1][i]))
}
fn layout_warning(
    objects: &[Option<Object>],
    bed: [f64; 2],
) -> (Option<usize>, Option<[usize; 2]>) {
    let outside = objects.iter().position(|object| {
        object.as_ref().is_some_and(|object| {
            (0..2).any(|i| object.bounds[0][i] < -1e-5 || object.bounds[1][i] > bed[i] + 1e-5)
        })
    });
    let overlap = objects.iter().enumerate().find_map(|(i, object)| {
        let object = object.as_ref()?;
        objects
            .iter()
            .enumerate()
            .skip(i + 1)
            .find_map(|(j, other)| {
                overlaps(&object.bounds, &other.as_ref()?.bounds).then_some([i, j])
            })
    });
    (outside, overlap)
}

fn bounds_rect(bounds: Bounds, camera: Camera, rect: Rect) -> Rect {
    Rect::from_two_pos(
        screen(camera, rect, [bounds[0][0], bounds[0][1]]),
        screen(camera, rect, [bounds[1][0], bounds[1][1]]),
    )
}
fn screen(camera: Camera, rect: Rect, [x, y]: [f64; 2]) -> Pos2 {
    let [x, y, _] = camera.project([x as f32, y as f32, 0.], rect);
    egui::pos2(x, y)
}
fn rotation_handle(footprint: Rect) -> Pos2 {
    footprint.right_top() + egui::vec2(12., -12.)
}
fn bearing(pointer: Pos2, center: Pos2) -> f32 {
    let offset = pointer - center;
    (-offset.y).atan2(offset.x).to_degrees()
}
fn paint_bed(painter: &egui::Painter, rect: Rect, camera: Camera, bed: [f64; 2]) {
    painter.rect_filled(rect, 4., crate::theme::SURFACE);
    let bed_rect = Rect::from_two_pos(screen(camera, rect, [0., 0.]), screen(camera, rect, bed));
    painter.rect_filled(bed_rect, 2., BED);
    let grid = painter.with_clip_rect(bed_rect.intersect(rect));
    let stroke = Stroke::new(0.7, GRID);
    let wanted = f64::from(30. / camera.scale);
    let step = 10_f64.powf(wanted.log10().ceil()).max(1.);
    for axis in 0..2 {
        let count = (bed[axis] / step).min(100.) as usize;
        for index in 1..=count {
            let mut a = [0., 0.];
            let mut b = bed;
            a[axis] = step * index as f64;
            b[axis] = a[axis];
            grid.line_segment([screen(camera, rect, a), screen(camera, rect, b)], stroke);
        }
    }
    painter.rect_stroke(
        bed_rect,
        2.,
        Stroke::new(1., Color32::from_gray(136)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        bed_rect.left_bottom() + egui::vec2(4., -4.),
        Align2::LEFT_BOTTOM,
        "0, 0",
        FontId::proportional(11.),
        Color32::LIGHT_GRAY,
    );
    painter.text(
        bed_rect.right_bottom() - egui::vec2(4., 4.),
        Align2::RIGHT_BOTTOM,
        format!("{:.0} × {:.0} mm", bed[0], bed[1]),
        FontId::proportional(11.),
        Color32::LIGHT_GRAY,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::print::PlateEditor;
    use multicad_core::{
        commands::{Actor, Channel},
        model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length},
    };

    #[test]
    fn placed_copies_drag_rotate_and_reopen_without_moving_model_parts() {
        let mut session = Session::new(Document::new("Plate test").unwrap()).unwrap();
        let actor = Actor::local(Channel::Ui);
        let parts = ["First box", "Second box"].map(|name| {
            let request = PrimitiveRequest::new(
                name,
                PrimitiveParameters::Box {
                    size: [length(20.), length(12.), length(10.)],
                    frame: PrimitiveFrame::default(),
                },
            );
            let prepared =
                model::prepare_primitive(&session, session.revision(), actor.clone(), &request)
                    .unwrap();
            session.commit(prepared.prepared).unwrap();
            request.body
        });
        let native = multicad_model::Compiler::default()
            .compile(session.document(), session.revision())
            .unwrap();
        let scene =
            crate::modeling::native_fixture::scene(session.document(), native.snapshot(), 1)
                .unwrap();
        let original_model = parts.map(|id| session.document().get(id).unwrap().clone());
        let mut editor = PlateEditor::default();
        editor.new_plate(&session).unwrap();
        let draft = editor.draft.as_mut().unwrap();
        draft.definition.bed_mm = [100., 80.];
        for (part, position_mm) in parts.into_iter().zip([[10., 12.], [55., 25.]]) {
            draft.definition.placements.push(PrintPlatePlacement {
                source: PartRef {
                    part,
                    occurrence_path: vec![],
                    root_assembly: None,
                },
                position_mm,
                rotation_deg: [0.; 3],
            });
        }
        let ctx = egui::Context::default();
        let run = |editor: &mut PlateEditor, events| {
            let mut area = None;
            let draft = editor.draft.as_mut().unwrap();
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1024., 600.))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let (rect, response) = ui
                        .allocate_exact_size(egui::vec2(420., 380.), egui::Sense::click_and_drag());
                    editor
                        .view
                        .sync(session.document(), Some(&scene), &draft.definition, None);
                    let camera = editor.view.camera(rect, draft.definition.bed_mm);
                    editor.view.pointer(
                        ui,
                        &response,
                        camera,
                        &mut draft.definition,
                        &mut editor.selected_placement,
                    );
                    area = Some((rect, camera));
                },
            )
            .drop_without_applying_deltas();
            area.unwrap()
        };
        let (rect, camera) = run(&mut editor, vec![]);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let drag = |editor: &mut PlateEditor, start, end| {
            run(editor, vec![egui::Event::PointerMoved(start)]);
            run(editor, vec![button(start, true)]);
            run(editor, vec![egui::Event::PointerMoved(end)]);
            run(editor, vec![button(end, false)]);
        };
        drag(
            &mut editor,
            screen(camera, rect, [20., 18.]),
            screen(camera, rect, [27., 22.]),
        );
        assert_eq!(editor.selected_placement, Some(0));
        let new_position = editor.definition().unwrap().placements[0].position_mm;
        assert!((new_position[0] - 17.).abs() < 1e-4);
        assert!((new_position[1] - 16.).abs() < 1e-4);
        let bounds = editor.view.objects[0].as_ref().unwrap().bounds;
        let footprint = bounds_rect(bounds, camera, rect);
        let center = footprint.center();
        let handle = rotation_handle(footprint);
        let offset = handle - center;
        drag(
            &mut editor,
            handle,
            center + egui::vec2(offset.y, -offset.x),
        );
        let angle = editor.definition().unwrap().placements[0].rotation_deg[2];
        assert!((angle - 90.).abs() < 1e-4, "{angle}");
        let id = editor.selected().unwrap();
        editor.save(&mut session, Channel::Ui).unwrap();
        let mut reopened = PlateEditor::default();
        reopened.open(&session, id).unwrap();
        assert_eq!(reopened.definition(), editor.definition());
        for entity in original_model {
            assert_eq!(session.document().get(entity.id), Some(&entity));
        }
    }
}

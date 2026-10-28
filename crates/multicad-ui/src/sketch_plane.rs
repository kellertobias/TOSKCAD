//! Source-neutral plane picking. The host commits a returned choice through the
//! shared sketch command API; display occurrences never become history parents.
use crate::{
    camera::Camera,
    viewport::{Scene, SceneStamp, SectionPlane, Transform},
};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Stroke};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    references::{
        GeometryIntent, Origin, ReferenceFrame, ReferenceReport, TopologyKey, TopologySnapshot,
    },
    sketch::{self, Datum, Frame, PlaneOptions},
};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct FacePlane {
    /// Canonical producer Feature, distinct from the displayed occurrence.
    pub producer: EntityId,
    pub occurrence: EntityId,
    pub stamp: SceneStamp,
    pub snapshot: TopologySnapshot,
    pub key: TopologyKey,
    pub anchor: Origin,
    pub local_frame: Frame,
    pub world_frame: Frame,
    pub occurrence_transform: Transform,
}
#[derive(Clone, Debug)]
pub enum PlaneChoice {
    Datum(Datum),
    Existing(EntityId),
    Face(Arc<FacePlane>),
}
#[derive(Clone, Debug)]
pub struct PlanePreview {
    pub choice: PlaneChoice,
    pub local_frame: Frame,
    pub world_frame: Frame,
    pub options: PlaneOptions,
}
#[derive(Clone, Copy)]
pub struct PlaneContext<'a> {
    pub document: &'a Document,
    pub revision: u64,
    pub scene: Option<&'a Scene>,
    pub report: Option<&'a ReferenceReport>,
    /// Preferred displayed occurrence. This is never a canonical sketch parent.
    pub occurrence: Option<EntityId>,
    pub section: Option<SectionPlane>,
}
#[derive(Default)]
pub struct PlanePicker {
    chosen: Option<PlaneChoice>,
    hovered: Option<PlaneChoice>,
    face_hover: Option<Arc<FacePlane>>,
    options: PlaneOptions,
    message: Option<String>,
    cancel_requested: bool,
}
fn from_reference(frame: ReferenceFrame) -> Frame {
    Frame {
        origin: frame.origin_mm,
        u: frame.u,
        v: frame.v,
        n: frame.normal(),
    }
}
fn label(choice: &PlaneChoice, document: &Document) -> String {
    match choice {
        PlaneChoice::Datum(datum) => format!("{} datum", datum.label()),
        PlaneChoice::Existing(id) => document
            .get(*id)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| "Missing construction plane".into()),
        PlaneChoice::Face(face) => format!(
            "Face on {}",
            document
                .get(face.occurrence)
                .or_else(|| document.get(face.producer))
                .map(|e| e.name.as_str())
                .unwrap_or("part")
        ),
    }
}
fn geometry_attached(document: &Document, plane: EntityId) -> bool {
    let mut current = plane;
    let mut visited = std::collections::BTreeSet::new();
    while visited.insert(current) {
        let Some(EntityData::Plane {
            anchor: Some(anchor),
            ..
        }) = document.get(current).map(|e| &e.data)
        else {
            return false;
        };
        if anchor.geometry.is_some() {
            return true;
        }
        if anchor.role != sketch::OFFSET_PLANE_ROLE {
            return false;
        }
        current = anchor.target;
    }
    false
}
fn preview(
    choice: &PlaneChoice,
    options: PlaneOptions,
    context: PlaneContext<'_>,
) -> Result<PlanePreview, String> {
    let (local, world) = match choice {
        PlaneChoice::Datum(datum) => (datum.frame(), datum.frame()),
        PlaneChoice::Existing(id) => {
            let frame =
                sketch::inspect_plane(context.document, *id, context.revision, context.report)
                    .map_err(|e| e.to_string())?;
            if context.scene.is_none() && geometry_attached(context.document, *id) {
                return Err(
                    "Wait for the current model before previewing the attached construction plane"
                        .into(),
                );
            }
            if context.scene.is_some_and(|scene| {
                scene.stamp().document != context.document.id()
                    || scene.stamp().revision != context.revision
            }) {
                return Err(
                    "Wait for the current model before previewing the construction plane".into(),
                );
            }
            let pose = context
                .scene
                .and_then(|scene| {
                    crate::sketch_context::occurrence(
                        context.document,
                        *id,
                        scene,
                        context.occurrence,
                    )
                })
                .map(|(_, pose)| pose);
            let world = pose.map_or(frame, |pose| {
                from_reference(pose.frame(ReferenceFrame {
                    origin_mm: frame.origin,
                    u: frame.u,
                    v: frame.v,
                }))
            });
            (frame, world)
        }
        PlaneChoice::Face(face) => {
            let scene = context
                .scene
                .ok_or("Wait for the current model before choosing a face")?;
            if face.stamp != scene.stamp()
                || face.stamp.document != context.document.id()
                || face.stamp.revision != context.revision
                || face.snapshot.owner != face.producer
                || face.snapshot.revision != context.revision
            {
                return Err(
                    "The selected face is stale; select it again on the current model".into(),
                );
            }
            if !matches!(
                context.document.get(face.producer).map(|e| &e.data),
                Some(EntityData::Feature {
                    suppressed: false,
                    ..
                })
            ) {
                return Err("The selected geometry has no active modeling feature".into());
            }
            let occurrence = scene
                .instances()
                .iter()
                .find(|i| i.id == face.occurrence && i.geometry == face.producer)
                .ok_or("The selected occurrence is no longer available")?;
            if occurrence.transform != face.occurrence_transform {
                return Err("The selected occurrence moved; select its face again".into());
            }
            GeometryIntent::capture(&face.snapshot, face.key)
                .map_err(|_| "The face cannot be referenced uniquely; choose another face")?;
            let frame = face
                .snapshot
                .elements
                .iter()
                .find(|e| e.key == face.key)
                .and_then(|e| e.frames.iter().find(|f| f.origin == face.anchor))
                .ok_or("The selected face has no planar reference frame")?
                .frame;
            frame
                .validate()
                .map_err(|_| "The selected planar frame is invalid")?;
            (
                from_reference(frame),
                from_reference(occurrence.transform.frame(frame)),
            )
        }
    };
    Ok(PlanePreview {
        choice: choice.clone(),
        local_frame: options.apply(local).map_err(|e| e.to_string())?,
        world_frame: options.apply(world).map_err(|e| e.to_string())?,
        options,
    })
}
fn face_choice(
    context: PlaneContext<'_>,
    camera: Camera,
    rect: Rect,
    pointer: Pos2,
    cached: Option<&Arc<FacePlane>>,
) -> Result<Option<PlaneChoice>, String> {
    let Some(scene) = context.scene else {
        return Ok(None);
    };
    if scene.stamp().document != context.document.id() || scene.stamp().revision != context.revision
    {
        return Err("Wait for the current model before choosing a face".into());
    }
    let Some(ray) = camera.ray(pointer, rect) else {
        return Ok(None);
    };
    let Some(hit) = scene.pick_face_section(ray, context.section) else {
        return Ok(None);
    };
    face_from_hit(context, &hit, cached)
}
/// Reuse a committed selection rather than ray-picking beneath a toolbar click.
pub fn selected_face(
    context: PlaneContext<'_>,
    hit: &crate::viewport::Hit,
) -> Result<PlanePreview, String> {
    let scene = context
        .scene
        .ok_or("Wait for the current model before creating a face sketch")?;
    if hit.stamp != scene.stamp()
        || hit.stamp.document != context.document.id()
        || hit.stamp.revision != context.revision
    {
        return Err("The selected face is stale; select it again on the current model".into());
    }
    if hit.key.kind != multicad_core::references::TopologyKind::Face {
        return Err("Select a planar face to start a face sketch".into());
    }
    let choice = face_from_hit(context, hit, None)?.ok_or("Select a current planar face")?;
    preview(&choice, PlaneOptions::default(), context)
}
fn face_from_hit(
    context: PlaneContext<'_>,
    hit: &crate::viewport::Hit,
    cached: Option<&Arc<FacePlane>>,
) -> Result<Option<PlaneChoice>, String> {
    let scene = context
        .scene
        .ok_or("Wait for the current model before choosing a face")?;
    if !matches!(
        context.document.get(hit.geometry).map(|e| &e.data),
        Some(EntityData::Feature {
            suppressed: false,
            ..
        })
    ) {
        return Err("Only geometry from an active modeling feature can host a sketch".into());
    }
    if let Some(face) = cached.filter(|face| {
        face.stamp == scene.stamp()
            && face.producer == hit.geometry
            && face.occurrence == hit.instance
            && face.key == hit.key
    }) {
        return Ok(Some(PlaneChoice::Face(face.clone())));
    }
    let snapshot = scene
        .geometry_snapshot(hit.geometry)
        .ok_or("The face source is unavailable")?;
    GeometryIntent::capture(&snapshot, hit.key)
        .map_err(|_| "This face has no unique persistent source; choose another face")?;
    let element = snapshot
        .elements
        .iter()
        .find(|e| e.key == hit.key)
        .ok_or("The face source is unavailable")?;
    if element.frames.is_empty() {
        return Err(
            "Choose a named planar face; this surface has no planar reference frame".into(),
        );
    }
    if element.frames.len() != 1 {
        return Err(
            "This face has multiple source frames; choose a construction plane instead".into(),
        );
    }
    let anchored = element.frames[0].clone();
    anchored
        .frame
        .validate()
        .map_err(|_| "The face has an invalid planar reference frame")?;
    let occurrence = scene
        .instances()
        .iter()
        .find(|i| i.id == hit.instance)
        .ok_or("The face occurrence is unavailable")?;
    Ok(Some(PlaneChoice::Face(Arc::new(FacePlane {
        producer: hit.geometry,
        occurrence: hit.instance,
        stamp: scene.stamp(),
        snapshot,
        key: hit.key,
        anchor: anchored.origin.clone(),
        local_frame: from_reference(anchored.frame),
        world_frame: from_reference(occurrence.transform.frame(anchored.frame)),
        occurrence_transform: occurrence.transform,
    }))))
}
fn planes(context: PlaneContext<'_>) -> Vec<(PlaneChoice, Frame)> {
    let mut result: Vec<_> = Datum::ALL
        .into_iter()
        .map(|d| (PlaneChoice::Datum(d), d.frame()))
        .collect();
    for entity in context.document.entities() {
        if matches!(entity.data, EntityData::Plane { .. })
            && let Ok(preview) = preview(
                &PlaneChoice::Existing(entity.id),
                PlaneOptions::default(),
                context,
            )
        {
            result.push((PlaneChoice::Existing(entity.id), preview.world_frame));
        }
    }
    result
}
impl PlanePicker {
    pub fn cancel(&mut self) {
        *self = Self::default();
    }
    pub fn take_cancel_requested(&mut self) -> bool {
        std::mem::take(&mut self.cancel_requested)
    }
    pub fn choose(&mut self, choice: PlaneChoice) {
        self.chosen = Some(choice);
        self.hovered = None;
        self.message = None;
    }
    pub fn options(&self) -> PlaneOptions {
        self.options
    }
    pub fn set_options(&mut self, options: PlaneOptions) -> Result<(), String> {
        options
            .apply(Datum::Xy.frame())
            .map_err(|e| e.to_string())?;
        self.options = options;
        self.message = None;
        Ok(())
    }
    pub fn update_hover(
        &mut self,
        pointer: Option<Pos2>,
        camera: Camera,
        rect: Rect,
        context: PlaneContext<'_>,
    ) {
        self.hovered = None;
        self.message = None;
        let Some(pointer) = pointer.filter(|p| rect.contains(*p)) else {
            return;
        };
        match face_choice(context, camera, rect, pointer, self.face_hover.as_ref()) {
            Ok(Some(face)) => {
                if let PlaneChoice::Face(face) = &face {
                    self.face_hover = Some(face.clone());
                }
                self.hovered = Some(face);
                return;
            }
            Err(error) => {
                self.message = Some(error);
                return;
            }
            Ok(None) => {}
        }
        let mut nearest = f32::INFINITY;
        for (choice, frame) in planes(context) {
            let Some(point) = camera.on_plane(pointer, rect, frame) else {
                continue;
            };
            if point.iter().any(|p| p.abs() > 40.) {
                continue;
            }
            let depth = camera.project(frame.point(point).map(|v| v as f32), rect)[2];
            if depth >= 0.1 && depth < nearest {
                nearest = depth;
                self.hovered = Some(choice);
            }
        }
    }
    pub fn current_preview(
        &self,
        context: PlaneContext<'_>,
    ) -> Result<Option<PlanePreview>, String> {
        self.hovered
            .as_ref()
            .or(self.chosen.as_ref())
            .map(|choice| preview(choice, self.options, context))
            .transpose()
    }
    pub fn click_hovered(&mut self, context: PlaneContext<'_>) -> Result<PlanePreview, String> {
        let choice = self.hovered.as_ref().ok_or_else(|| {
            self.message
                .clone()
                .unwrap_or_else(|| "Select a datum, construction plane or named planar face".into())
        })?;
        let result = preview(choice, self.options, context)?;
        self.chosen = Some(choice.clone());
        Ok(result)
    }
    pub fn show_inspector(
        &mut self,
        ui: &mut egui::Ui,
        context: PlaneContext<'_>,
    ) -> Option<PlanePreview> {
        ui.strong("Sketch plane");
        ui.label("Select a plane in the viewer, or choose one below.");
        ui.horizontal(|ui| {
            for datum in Datum::ALL {
                if ui
                    .selectable_label(
                        matches!(self.chosen,Some(PlaneChoice::Datum(d)) if d==datum),
                        datum.label(),
                    )
                    .on_hover_text(format!("Preview the {} origin plane", datum.label()))
                    .clicked()
                {
                    self.choose(PlaneChoice::Datum(datum));
                }
            }
        });
        egui::ComboBox::from_id_salt("new-sketch-construction-plane")
            .selected_text(match &self.chosen {
                Some(PlaneChoice::Existing(id)) => context
                    .document
                    .get(*id)
                    .map(|e| e.name.as_str())
                    .unwrap_or("Missing construction plane"),
                _ => "Construction plane…",
            })
            .show_ui(ui, |ui| {
                for entity in context.document.entities() {
                    if !matches!(entity.data, EntityData::Plane { .. }) {
                        continue;
                    }
                    let available = sketch::inspect_plane(
                        context.document,
                        entity.id,
                        context.revision,
                        context.report,
                    );
                    let response = ui.add_enabled(
                        available.is_ok(),
                        egui::Button::selectable(
                            matches!(self.chosen,Some(PlaneChoice::Existing(id)) if id==entity.id),
                            &entity.name,
                        ),
                    );
                    if response
                        .on_hover_text(
                            available
                                .err()
                                .map(|e| e.to_string())
                                .unwrap_or_else(|| "Preview this construction plane".into()),
                        )
                        .clicked()
                    {
                        self.choose(PlaneChoice::Existing(entity.id));
                    }
                }
            })
            .response
            .on_hover_text(
                "Use an existing construction plane; unresolved attachments cannot be selected",
            );
        ui.horizontal(|ui| {
            ui.label("Normal offset · mm");
            ui.add(
                egui::DragValue::new(&mut self.options.normal_offset_mm)
                    .range(-1e7..=1e7)
                    .speed(0.5),
            )
            .on_hover_text("Offset along the selected plane's positive normal");
        });
        let mut angle = self.options.angle_rad.to_degrees();
        ui.horizontal(|ui| {
            ui.label("In-plane angle");
            if ui
                .add(
                    egui::DragValue::new(&mut angle)
                        .range(-360000.0..=360000.0)
                        .suffix("°")
                        .speed(1.),
                )
                .on_hover_text("Rotate sketch U and V around the plane's positive normal")
                .changed()
            {
                self.options.angle_rad = angle.to_radians();
            }
        });
        let result = self.current_preview(context);
        if let Ok(Some(preview)) = &result {
            ui.label(label(&preview.choice, context.document));
            ui.small(format!(
                "Origin {:.3}, {:.3}, {:.3} mm",
                preview.world_frame.origin[0],
                preview.world_frame.origin[1],
                preview.world_frame.origin[2]
            ));
            for (axis, vector) in [
                ("U", preview.world_frame.u),
                ("V", preview.world_frame.v),
                ("Normal", preview.world_frame.n),
            ] {
                ui.small(format!(
                    "{axis} {:.3}, {:.3}, {:.3}",
                    vector[0], vector[1], vector[2]
                ));
            }
        }
        if let Some(error) = result.as_ref().err().or(self.message.as_ref()) {
            ui.colored_label(Color32::LIGHT_YELLOW, error);
        }
        let accept = ui
            .add_enabled(
                matches!(result, Ok(Some(_))),
                egui::Button::new("Use plane"),
            )
            .on_hover_text("Create a sketch on this preview through the shared command API")
            .clicked();
        if ui
            .button("Cancel new sketch")
            .on_hover_text("Leave plane selection without changing the document")
            .clicked()
        {
            self.cancel();
            self.cancel_requested = true;
            return None;
        }
        if accept { result.ok().flatten() } else { None }
    }
    pub fn paint_preview(
        &self,
        painter: &egui::Painter,
        camera: Camera,
        rect: Rect,
        context: PlaneContext<'_>,
    ) {
        if self.chosen.is_none() {
            for (choice, frame) in planes(context) {
                paint_plane(
                    painter,
                    camera,
                    rect,
                    frame,
                    Color32::from_rgb(75, 110, 145),
                    &label(&choice, context.document),
                    false,
                );
            }
        }
        if let Ok(Some(preview)) = self.current_preview(context) {
            paint_plane(
                painter,
                camera,
                rect,
                preview.world_frame,
                Color32::from_rgb(90, 195, 225),
                &label(&preview.choice, context.document),
                true,
            );
        }
    }
}
fn paint_plane(
    painter: &egui::Painter,
    camera: Camera,
    rect: Rect,
    frame: Frame,
    color: Color32,
    title: &str,
    axes: bool,
) {
    let corners = [[-40., -40.], [40., -40.], [40., 40.], [-40., 40.]].map(|p| frame.point(p));
    for index in 0..4 {
        if let Some(segment) = camera.segment(corners[index], corners[(index + 1) % 4], rect) {
            painter.line_segment(segment, Stroke::new(if axes { 1.8 } else { 0.8 }, color));
        }
    }
    let project = camera.project(frame.origin.map(|v| v as f32), rect);
    if project[2] < 0.1 {
        return;
    }
    let origin = Pos2::new(project[0], project[1]);
    painter.circle_filled(origin, if axes { 3.5 } else { 2. }, color);
    painter.text(
        origin + egui::vec2(7., 7.),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(11.),
        color,
    );
    if axes {
        let length = (42. / camera.scale.max(0.02)) as f64;
        for (name, vector, color) in [
            ("U", frame.u, Color32::from_rgb(225, 95, 105)),
            ("V", frame.v, Color32::from_rgb(80, 185, 125)),
            ("N", frame.n, Color32::from_rgb(85, 145, 230)),
        ] {
            let end = std::array::from_fn(|i| frame.origin[i] + vector[i] * length);
            if let Some(segment) = camera.segment(frame.origin, end, rect) {
                painter.arrow(segment[0], segment[1] - segment[0], Stroke::new(1.5, color));
                painter.text(
                    segment[1],
                    Align2::LEFT_CENTER,
                    name,
                    FontId::proportional(11.),
                    color,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        navigation::AxisView,
        viewport::{Geometry, Instance},
    };
    use multicad_core::document::{Definition, Entity, Placement};
    use multicad_kernel::{MeshOptions, Profile, ProfileEdge, Solid};
    use std::sync::Arc;
    fn producer() -> Entity {
        Entity::new(
            "Panel source",
            EntityData::Feature {
                definition: Definition {
                    type_id: "plane.picking.test".into(),
                    schema_version: 1,
                    data: serde_json::json!({}),
                },
                inputs: vec![],
                suppressed: false,
            },
        )
    }
    fn geometry(solid: &Solid, id: EntityId) -> Arc<Geometry> {
        let mesh = solid.mesh(MeshOptions::default()).unwrap();
        Arc::new(
            Geometry::new(
                id,
                mesh.vertices.as_chunks::<3>().0.to_vec(),
                mesh.indices,
                mesh.face_indices,
                solid.topology_snapshot(id, 0).unwrap().elements,
            )
            .unwrap(),
        )
    }
    fn fixture(
        solid: impl FnOnce(EntityId) -> Solid,
        placement: Placement,
    ) -> (Document, Scene, EntityId, EntityId) {
        let producer = producer();
        let part = Entity::new(
            "Panel",
            EntityData::Part {
                features: vec![producer.id],
                material: None,
                tags: Default::default(),
            },
        );
        let instance = Entity::new(
            "Placed panel",
            EntityData::Instance {
                component: part.id,
                placement,
                placement_source: None,
            },
        );
        let mut document = Document::new("Plane picking").unwrap();
        for entity in [producer.clone(), part, instance.clone()] {
            document.insert(entity).unwrap();
        }
        let asset = geometry(&solid(producer.id), producer.id);
        let scene = Scene::new(
            SceneStamp {
                document: document.id(),
                revision: 0,
                generation: 1,
            },
            vec![asset],
            vec![Instance {
                id: instance.id,
                geometry: producer.id,
                transform: Transform::from_placement(placement).unwrap(),
                color: [150, 165, 180, 255],
            }],
        )
        .unwrap();
        (document, scene, producer.id, instance.id)
    }
    fn rect() -> Rect {
        Rect::from_min_size(Pos2::ZERO, egui::vec2(900., 600.))
    }
    fn close(a: [f64; 3], b: [f64; 3]) {
        for (a, b) in a.into_iter().zip(b) {
            assert!((a - b).abs() < 1e-8, "{a} != {b}");
        }
    }
    #[test]
    fn real_rotated_face_pick_preserves_local_producer_and_world_preview() {
        let placement = Placement {
            translation_mm: [10., 20., 30.],
            rotation_xyzw: [
                0.,
                0.,
                std::f64::consts::FRAC_1_SQRT_2,
                std::f64::consts::FRAC_1_SQRT_2,
            ],
        };
        let (document, scene, producer, occurrence) = fixture(
            |id| Solid::box_named([100., 60., 18.], id, Default::default()).unwrap(),
            placement,
        );
        let before = document.clone();
        let context = PlaneContext {
            document: &document,
            revision: 0,
            scene: Some(&scene),
            report: None,
            occurrence: None,
            section: None,
        };
        let mut camera = Camera {
            target: [-15., 40., 48.],
            scale: 3.,
            ..Default::default()
        };
        camera.orient(AxisView::PositiveZ);
        let mut picker = PlanePicker::default();
        picker.update_hover(Some(rect().center()), camera, rect(), context);
        picker
            .set_options(PlaneOptions {
                normal_offset_mm: 5.,
                angle_rad: std::f64::consts::FRAC_PI_2,
            })
            .unwrap();
        let selected = picker.click_hovered(context).unwrap();
        let PlaneChoice::Face(face) = &selected.choice else {
            panic!("Real face was not picked")
        };
        assert_eq!(face.producer, producer);
        assert_eq!(face.snapshot.owner, producer);
        assert_eq!(face.occurrence, occurrence);
        assert_ne!(producer, occurrence);
        close(face.local_frame.origin, [0., 0., 18.]);
        close(face.world_frame.origin, [10., 20., 48.]);
        close(selected.local_frame.origin, [0., 0., 23.]);
        close(selected.world_frame.origin, [10., 20., 53.]);
        close(selected.local_frame.u, [0., 1., 0.]);
        close(selected.world_frame.u, [-1., 0., 0.]);
        close(selected.world_frame.n, [0., 0., 1.]);
        let local_snapshot = scene.geometry_snapshot(producer).unwrap();
        assert_eq!(local_snapshot.owner, producer);
        assert!(scene.geometry_snapshot(occurrence).is_none());
        picker.cancel();
        assert!(picker.current_preview(context).unwrap().is_none());
        assert_eq!(document, before);
    }
    #[test]
    fn a_face_click_never_uses_obsolete_generation_revision_or_another_document() {
        let (document, scene, _, _) = fixture(
            |id| Solid::box_named([100., 60., 18.], id, Default::default()).unwrap(),
            Placement::default(),
        );
        let context = PlaneContext {
            document: &document,
            revision: 0,
            scene: Some(&scene),
            report: None,
            occurrence: None,
            section: None,
        };
        let mut camera = Camera {
            target: [20., 25., 18.],
            scale: 3.,
            ..Default::default()
        };
        camera.orient(AxisView::PositiveZ);
        let mut picker = PlanePicker::default();
        picker.update_hover(Some(rect().center()), camera, rect(), context);
        let next = Scene::new(
            SceneStamp {
                generation: 2,
                ..scene.stamp()
            },
            scene.geometries().values().cloned().collect(),
            scene.instances().to_vec(),
        )
        .unwrap();
        assert!(
            picker
                .click_hovered(PlaneContext {
                    scene: Some(&next),
                    ..context
                })
                .is_err()
        );
        assert!(
            picker
                .current_preview(PlaneContext {
                    revision: 1,
                    ..context
                })
                .is_err()
        );
        let other = Document::new("Other document").unwrap();
        assert!(
            picker
                .click_hovered(PlaneContext {
                    document: &other,
                    ..context
                })
                .is_err()
        );
        assert_eq!(document.entities().count(), 3);
    }
    #[test]
    fn named_curved_and_unnamed_planar_faces_are_rejected_without_datum_fallback() {
        for named in [false, true] {
            let (document, scene, _, _) = fixture(
                |id| {
                    if named {
                        Solid::extrude_profile_named(
                            &Profile {
                                frame: Default::default(),
                                outer: vec![ProfileEdge::Circle {
                                    center: [0.; 2],
                                    radius_mm: 20.,
                                }],
                                holes: vec![],
                            },
                            18.,
                            id,
                            &[Origin {
                                source: id,
                                role: "circle".into(),
                            }],
                            Default::default(),
                        )
                        .unwrap()
                    } else {
                        Solid::box_solid([20., 20., 18.], Default::default()).unwrap()
                    }
                },
                Placement::default(),
            );
            let context = PlaneContext {
                document: &document,
                revision: 0,
                scene: Some(&scene),
                report: None,
                occurrence: None,
                section: None,
            };
            let mut camera = Camera {
                target: if named { [20., 0., 9.] } else { [20., 10., 9.] },
                scale: 3.,
                ..Default::default()
            };
            camera.orient(AxisView::PositiveX);
            let mut picker = PlanePicker::default();
            picker.update_hover(Some(rect().center()), camera, rect(), context);
            let error = picker.click_hovered(context).unwrap_err();
            assert!(
                error.contains(if named { "planar" } else { "persistent" }),
                "{error}"
            );
            assert!(picker.current_preview(context).unwrap().is_none());
        }
    }
    #[test]
    fn face_picking_uses_visible_section_geometry_and_keeps_local_source_identity() {
        let placement = Placement {
            translation_mm: [10., 20., 30.],
            rotation_xyzw: [
                0.,
                0.,
                std::f64::consts::FRAC_1_SQRT_2,
                std::f64::consts::FRAC_1_SQRT_2,
            ],
        };
        let (document, scene, producer, occurrence) = fixture(
            |id| Solid::box_named([100., 60., 18.], id, Default::default()).unwrap(),
            placement,
        );
        let before = document.clone();
        let mut camera = Camera {
            target: [-15., 40., 48.],
            scale: 3.,
            ..Default::default()
        };
        camera.orient(AxisView::PositiveZ);
        let context = PlaneContext {
            document: &document,
            revision: 0,
            scene: Some(&scene),
            report: None,
            occurrence: Some(occurrence),
            section: None,
        };
        let mut picker = PlanePicker::default();
        picker.update_hover(Some(rect().center()), camera, rect(), context);
        let top = picker.click_hovered(context).unwrap();
        let PlaneChoice::Face(top) = top.choice else {
            panic!("Top face missing")
        };
        close(top.world_frame.origin, [10., 20., 48.]);
        let clipped = PlaneContext {
            section: Some(SectionPlane::new([0., 0., -1.], -39.).unwrap()),
            ..context
        };
        picker.update_hover(Some(rect().center()), camera, rect(), clipped);
        let bottom = picker.click_hovered(clipped).unwrap();
        let PlaneChoice::Face(bottom) = bottom.choice else {
            panic!("Retained bottom face missing")
        };
        assert_ne!(bottom.key, top.key);
        assert_eq!(bottom.producer, producer);
        assert_eq!(bottom.occurrence, occurrence);
        assert_eq!(bottom.snapshot.owner, producer);
        assert!(
            bottom
                .snapshot
                .elements
                .iter()
                .find(|e| e.key == bottom.key)
                .unwrap()
                .origins
                .iter()
                .any(|o| o.role == "cap.start")
        );
        close(bottom.local_frame.origin, [0., 0., 0.]);
        close(bottom.world_frame.origin, [10., 20., 30.]);
        assert_eq!(document, before);
    }
    #[test]
    fn pointer_datum_selection_and_existing_plane_offsets_use_core_frames() {
        let mut document = Document::new("Datums").unwrap();
        for (datum, view) in [
            (Datum::Xy, AxisView::PositiveZ),
            (Datum::Xz, AxisView::PositiveY),
            (Datum::Yz, AxisView::PositiveX),
        ] {
            let frame = datum.frame();
            let mut camera = Camera {
                target: frame.point([20., 15.]).map(|v| v as f32),
                scale: 3.,
                ..Default::default()
            };
            camera.orient(view);
            let context = PlaneContext {
                document: &document,
                revision: 0,
                scene: None,
                report: None,
                occurrence: None,
                section: None,
            };
            let mut picker = PlanePicker::default();
            picker.update_hover(Some(rect().center()), camera, rect(), context);
            let selected = picker.click_hovered(context).unwrap();
            assert!(matches!(selected.choice,PlaneChoice::Datum(d) if d==datum));
            assert_eq!(selected.world_frame, frame);
        }
        let plane = Entity::new(
            "Level plane",
            EntityData::Plane {
                origin_mm: [0., 0., 42.],
                normal: [0., 0., 1.],
                anchor: None,
            },
        );
        document.insert(plane.clone()).unwrap();
        let context = PlaneContext {
            document: &document,
            revision: 0,
            scene: None,
            report: None,
            occurrence: None,
            section: None,
        };
        let mut camera = Camera {
            target: [20., 15., 42.],
            scale: 3.,
            ..Default::default()
        };
        camera.orient(AxisView::PositiveZ);
        let mut picker = PlanePicker::default();
        picker.update_hover(Some(rect().center()), camera, rect(), context);
        picker
            .set_options(PlaneOptions {
                normal_offset_mm: 6.,
                angle_rad: 0.,
            })
            .unwrap();
        let selected = picker.click_hovered(context).unwrap();
        assert!(matches!(selected.choice,PlaneChoice::Existing(id) if id==plane.id));
        close(selected.world_frame.origin, [0., 0., 48.]);
    }
    #[test]
    fn unresolved_existing_attachment_never_uses_its_saved_fallback_plane() {
        use multicad_core::{
            document::Link,
            references::{Attachment, inspect},
        };
        use std::collections::BTreeMap;
        let (mut document, scene, producer, _) = fixture(
            |id| Solid::box_named([100., 60., 18.], id, Default::default()).unwrap(),
            Placement::default(),
        );
        let snapshot = scene.geometry_snapshot(producer).unwrap();
        let face = snapshot
            .elements
            .iter()
            .find(|e| e.origins.iter().any(|o| o.role == "cap.end") && !e.frames.is_empty())
            .unwrap();
        let mut intent = GeometryIntent::capture(&snapshot, face.key).unwrap();
        intent.attachment = Some(Attachment {
            anchor: face.frames[0].origin.clone(),
            offset_mm: [5., 5., 0.],
            angle_rad: 0.,
        });
        let mut link = Link::new(producer, "plane");
        link.geometry = Some(intent);
        let plane = Entity::new(
            "Attached plane",
            EntityData::Plane {
                origin_mm: [500., 500., 500.],
                normal: [0., 0., 1.],
                anchor: Some(link),
            },
        );
        document.insert(plane.clone()).unwrap();
        let before = document.clone();
        let report = inspect(&document, 0, &BTreeMap::new());
        let context = PlaneContext {
            document: &document,
            revision: 0,
            scene: None,
            report: Some(&report),
            occurrence: None,
            section: None,
        };
        let mut picker = PlanePicker::default();
        picker.choose(PlaneChoice::Existing(plane.id));
        assert!(picker.current_preview(context).is_err());
        assert!(
            planes(context)
                .iter()
                .all(|(p, _)| !matches!(p,PlaneChoice::Existing(id) if *id==plane.id))
        );
        assert!(
            picker
                .set_options(PlaneOptions {
                    normal_offset_mm: f64::NAN,
                    angle_rad: 0.
                })
                .is_err()
        );
        assert_eq!(picker.options(), PlaneOptions::default());
        assert_eq!(document, before);
    }
    #[test]
    fn existing_face_plane_preview_uses_preferred_occurrence_without_moving_world_datums() {
        use multicad_core::{
            document::Link,
            references::{Attachment, inspect},
        };
        let (mut document, base, producer, identity) = fixture(
            |id| Solid::box_named([100., 60., 18.], id, Default::default()).unwrap(),
            Placement::default(),
        );
        let EntityData::Instance { component, .. } = document.get(identity).unwrap().data else {
            panic!("Instance missing")
        };
        let placement = Placement {
            translation_mm: [10., 20., 30.],
            rotation_xyzw: [
                0.,
                0.,
                std::f64::consts::FRAC_1_SQRT_2,
                std::f64::consts::FRAC_1_SQRT_2,
            ],
        };
        let placed = Entity::new(
            "Rotated panel",
            EntityData::Instance {
                component,
                placement,
                placement_source: None,
            },
        );
        document.insert(placed.clone()).unwrap();
        let snapshot = base.geometry_snapshot(producer).unwrap();
        let cap = snapshot
            .elements
            .iter()
            .find(|e| e.origins.iter().any(|o| o.role == "cap.end") && !e.frames.is_empty())
            .unwrap();
        let mut intent = GeometryIntent::capture(&snapshot, cap.key).unwrap();
        intent.attachment = Some(Attachment {
            anchor: cap.frames[0].origin.clone(),
            offset_mm: [0.; 3],
            angle_rad: 0.,
        });
        let mut anchor = Link::new(producer, "plane");
        anchor.geometry = Some(intent);
        let attached = Entity::new(
            "Panel face plane",
            EntityData::Plane {
                origin_mm: [0., 0., 18.],
                normal: [0., 0., 1.],
                anchor: Some(anchor),
            },
        );
        document.insert(attached.clone()).unwrap();
        let world = Entity::new(
            "World level",
            EntityData::Plane {
                origin_mm: [0., 0., 100.],
                normal: [0., 0., 1.],
                anchor: None,
            },
        );
        document.insert(world.clone()).unwrap();
        let before = document.clone();
        let mut instances = base.instances().to_vec();
        instances.push(Instance {
            id: placed.id,
            geometry: producer,
            transform: Transform::from_placement(placement).unwrap(),
            color: [150, 165, 180, 255],
        });
        let scene = Scene::new(
            SceneStamp {
                generation: 2,
                ..base.stamp()
            },
            base.geometries().values().cloned().collect(),
            instances,
        )
        .unwrap();
        let report = inspect(&document, 0, &scene.reference_snapshots());
        let context = PlaneContext {
            document: &document,
            revision: 0,
            scene: Some(&scene),
            report: Some(&report),
            occurrence: Some(placed.id),
            section: None,
        };
        let mut picker = PlanePicker::default();
        picker.choose(PlaneChoice::Existing(attached.id));
        picker
            .set_options(PlaneOptions {
                normal_offset_mm: 5.,
                angle_rad: std::f64::consts::FRAC_PI_2,
            })
            .unwrap();
        let preview = picker.current_preview(context).unwrap().unwrap();
        close(preview.local_frame.origin, [0., 0., 23.]);
        close(preview.world_frame.origin, [10., 20., 53.]);
        close(preview.world_frame.u, [-1., 0., 0.]);
        let target = planes(context)
            .into_iter()
            .find(|(p, _)| matches!(p,PlaneChoice::Existing(id) if *id==attached.id))
            .unwrap()
            .1;
        close(target.origin, [10., 20., 48.]);
        let identity_preview = picker
            .current_preview(PlaneContext {
                occurrence: Some(identity),
                ..context
            })
            .unwrap()
            .unwrap();
        close(identity_preview.world_frame.origin, [0., 0., 23.]);
        picker.set_options(PlaneOptions::default()).unwrap();
        picker.choose(PlaneChoice::Existing(world.id));
        close(
            picker
                .current_preview(context)
                .unwrap()
                .unwrap()
                .world_frame
                .origin,
            [0., 0., 100.],
        );
        picker.choose(PlaneChoice::Datum(Datum::Xy));
        assert_eq!(
            picker
                .current_preview(context)
                .unwrap()
                .unwrap()
                .world_frame,
            Datum::Xy.frame()
        );
        assert_eq!(document, before);
    }
    #[test]
    fn a_part_display_identity_cannot_replace_the_producer_feature_parent() {
        let (document, scene, producer, occurrence) = fixture(
            |id| Solid::box_named([100., 60., 18.], id, Default::default()).unwrap(),
            Placement::default(),
        );
        let part = document
            .entities()
            .find(|e| matches!(e.data, EntityData::Part { .. }))
            .unwrap()
            .id;
        let solid = Solid::box_named([100., 60., 18.], producer, Default::default()).unwrap();
        let final_part_scene = Scene::new(
            scene.stamp(),
            vec![geometry(&solid, part)],
            vec![Instance {
                id: occurrence,
                geometry: part,
                transform: Transform::default(),
                color: [150, 165, 180, 255],
            }],
        )
        .unwrap();
        let context = PlaneContext {
            document: &document,
            revision: 0,
            scene: Some(&final_part_scene),
            report: None,
            occurrence: None,
            section: None,
        };
        let mut camera = Camera {
            target: [20., 25., 18.],
            scale: 3.,
            ..Default::default()
        };
        camera.orient(AxisView::PositiveZ);
        let mut picker = PlanePicker::default();
        picker.update_hover(Some(rect().center()), camera, rect(), context);
        assert!(
            picker
                .click_hovered(context)
                .unwrap_err()
                .contains("modeling feature")
        );
    }
    #[test]
    fn attached_existing_planes_require_current_display_even_with_a_resolved_report() {
        use multicad_core::{
            document::Link,
            references::{Attachment, inspect},
        };
        let placement = Placement {
            translation_mm: [10., 20., 30.],
            rotation_xyzw: [
                0.,
                0.,
                std::f64::consts::FRAC_1_SQRT_2,
                std::f64::consts::FRAC_1_SQRT_2,
            ],
        };
        let (mut document, scene, producer, occurrence) = fixture(
            |id| Solid::box_named([100., 60., 18.], id, Default::default()).unwrap(),
            placement,
        );
        let snapshot = scene.geometry_snapshot(producer).unwrap();
        let cap = snapshot
            .elements
            .iter()
            .find(|e| e.origins.iter().any(|o| o.role == "cap.end") && !e.frames.is_empty())
            .unwrap();
        let mut intent = GeometryIntent::capture(&snapshot, cap.key).unwrap();
        intent.attachment = Some(Attachment {
            anchor: cap.frames[0].origin.clone(),
            offset_mm: [0.; 3],
            angle_rad: 0.,
        });
        let mut anchor = Link::new(producer, "plane");
        anchor.geometry = Some(intent);
        let attached = Entity::new(
            "Face plane",
            EntityData::Plane {
                origin_mm: [0., 0., 18.],
                normal: [0., 0., 1.],
                anchor: Some(anchor),
            },
        );
        let offset = Entity::new(
            "Offset plane",
            EntityData::Plane {
                origin_mm: [0., 0., 2.],
                normal: [0., 0., 1.],
                anchor: Some(Link::new(attached.id, sketch::OFFSET_PLANE_ROLE)),
            },
        );
        let world = Entity::new(
            "World datum",
            EntityData::Plane {
                origin_mm: [0., 0., 100.],
                normal: [0., 0., 1.],
                anchor: None,
            },
        );
        for entity in [attached.clone(), offset.clone(), world.clone()] {
            document.insert(entity).unwrap();
        }
        let before = document.clone();
        let report = inspect(&document, 0, &scene.reference_snapshots());
        assert!(report.all_resolved());
        let context = PlaneContext {
            document: &document,
            revision: 0,
            scene: Some(&scene),
            report: Some(&report),
            occurrence: Some(occurrence),
            section: None,
        };
        let stale = Scene::new(
            SceneStamp {
                revision: 1,
                generation: 2,
                ..scene.stamp()
            },
            scene.geometries().values().cloned().collect(),
            scene.instances().to_vec(),
        )
        .unwrap();
        let mut picker = PlanePicker::default();
        for (id, height) in [(attached.id, 48.), (offset.id, 50.)] {
            picker.choose(PlaneChoice::Existing(id));
            close(
                picker
                    .current_preview(context)
                    .unwrap()
                    .unwrap()
                    .world_frame
                    .origin,
                [10., 20., height],
            );
            assert!(
                picker
                    .current_preview(PlaneContext {
                        scene: None,
                        ..context
                    })
                    .unwrap_err()
                    .contains("current model")
            );
            assert!(
                picker
                    .current_preview(PlaneContext {
                        scene: Some(&stale),
                        ..context
                    })
                    .unwrap_err()
                    .contains("current model")
            );
        }
        picker.choose(PlaneChoice::Existing(world.id));
        close(
            picker
                .current_preview(PlaneContext {
                    scene: None,
                    ..context
                })
                .unwrap()
                .unwrap()
                .world_frame
                .origin,
            [0., 0., 100.],
        );
        assert_eq!(document, before);
    }
    fn inspector_frame(
        ctx: &egui::Context,
        picker: &mut PlanePicker,
        context: PlaneContext<'_>,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<PlanePreview>) {
        let mut accepted = None;
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900., 900.))),
                events,
                ..Default::default()
            },
            |ui| accepted = picker.show_inspector(ui, context),
        );
        (output, accepted)
    }
    fn text_position(output: &egui::FullOutput, label: &str) -> Pos2 {
        output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == label => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing plane control: {label}"))
    }
    fn click_inspector(
        ctx: &egui::Context,
        picker: &mut PlanePicker,
        context: PlaneContext<'_>,
        point: Pos2,
    ) -> (egui::FullOutput, Option<PlanePreview>) {
        inspector_frame(
            ctx,
            picker,
            context,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )
        .0
        .drop_without_applying_deltas();
        inspector_frame(
            ctx,
            picker,
            context,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )
    }
    #[test]
    fn inspector_use_and_cancel_return_data_without_creating_source_entities() {
        let document = Document::new("Inspector plane selection").unwrap();
        let before = document.clone();
        let context = PlaneContext {
            document: &document,
            revision: 0,
            scene: None,
            report: None,
            occurrence: None,
            section: None,
        };
        let ctx = egui::Context::default();
        let mut picker = PlanePicker::default();
        let (output, accepted) = inspector_frame(&ctx, &mut picker, context, vec![]);
        assert!(accepted.is_none());
        let point = text_position(&output, "XY");
        output.drop_without_applying_deltas();
        let (output, accepted) = click_inspector(&ctx, &mut picker, context, point);
        assert!(accepted.is_none());
        let point = text_position(&output, "Use plane");
        output.drop_without_applying_deltas();
        let (output, accepted) = click_inspector(&ctx, &mut picker, context, point);
        assert!(matches!(
            accepted.unwrap().choice,
            PlaneChoice::Datum(Datum::Xy)
        ));
        let point = text_position(&output, "Cancel new sketch");
        output.drop_without_applying_deltas();
        let (output, accepted) = click_inspector(&ctx, &mut picker, context, point);
        output.drop_without_applying_deltas();
        assert!(accepted.is_none());
        assert!(picker.take_cancel_requested());
        assert!(!picker.take_cancel_requested());
        assert!(picker.current_preview(context).unwrap().is_none());
        assert_eq!(document, before);
    }
}

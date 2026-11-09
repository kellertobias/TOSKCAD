use super::{
    FormStamp, ModelingTool, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui};
use multicad_core::{
    document::Document,
    model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, ResolvedPrimitive},
    sketch::{Datum, Frame},
};

mod gizmo;
mod pose;

pub(super) struct PrimitivePanel {
    request: PrimitiveRequest,
    kind: ModelingTool,
    stamp: FormStamp,
    name: String,
    origin: [DimensionField; 3],
    size: [DimensionField; 3],
    radius: DimensionField,
    height: DimensionField,
    basis: [[f64; 3]; 2],
    error: Option<String>,
    editing: bool,
    resize: gizmo::ResizeGizmo,
    pose: pose::PoseGizmo,
}
impl PrimitivePanel {
    pub fn new(request: PrimitiveRequest) -> Result<Self, String> {
        let (kind, frame, size, radius, height) = match &request.parameters {
            PrimitiveParameters::Box { size, frame } => (
                ModelingTool::Box,
                frame,
                size.clone(),
                model::length(20.),
                model::length(50.),
            ),
            PrimitiveParameters::Cylinder {
                radius,
                height,
                frame,
            } => (
                ModelingTool::Cylinder,
                frame,
                [model::length(100.), model::length(60.), model::length(20.)],
                radius.clone(),
                height.clone(),
            ),
            PrimitiveParameters::Sphere { radius, frame } => (
                ModelingTool::Sphere,
                frame,
                [model::length(100.), model::length(60.), model::length(20.)],
                radius.clone(),
                model::length(50.),
            ),
        };
        frame.validate().map_err(|error| error.to_string())?;
        let basis = [frame.u, frame.v];
        Ok(Self {
            stamp: FormStamp::new(),
            name: request.name.clone(),
            origin: [
                DimensionField::from_dimension(&frame.origin[0], "mm")?,
                DimensionField::from_dimension(&frame.origin[1], "mm")?,
                DimensionField::from_dimension(&frame.origin[2], "mm")?,
            ],
            size: [
                DimensionField::from_dimension(&size[0], "mm")?,
                DimensionField::from_dimension(&size[1], "mm")?,
                DimensionField::from_dimension(&size[2], "mm")?,
            ],
            radius: DimensionField::from_dimension(&radius, "mm")?,
            height: DimensionField::from_dimension(&height, "mm")?,
            kind,
            request,
            basis,
            error: None,
            editing: false,
            resize: Default::default(),
            pose: Default::default(),
        })
    }
    pub fn existing(request: PrimitiveRequest) -> Result<Self, String> {
        let mut panel = Self::new(request)?;
        panel.editing = true;
        Ok(panel)
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        self.kind.label()
    }
    fn plane(&self) -> Option<Datum> {
        Datum::ALL.into_iter().find(|datum| {
            let frame = datum.frame();
            self.basis == [frame.u, frame.v]
        })
    }
    fn set_plane(&mut self, plane: Datum) {
        let frame = plane.frame();
        self.basis = [frame.u, frame.v];
    }
    fn frame(&self, document: &Document) -> Result<PrimitiveFrame, String> {
        Ok(PrimitiveFrame {
            origin: [
                self.origin[0].resolve(document)?,
                self.origin[1].resolve(document)?,
                self.origin[2].resolve(document)?,
            ],
            u: self.basis[0],
            v: self.basis[1],
        })
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<PrimitiveRequest, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.name.trim().is_empty() || self.name.len() > 1000 {
            return Err("Enter a body name with 1–1000 bytes".into());
        }
        let frame = self.frame(document)?;
        let parameters = match self.kind {
            ModelingTool::Box => PrimitiveParameters::Box {
                size: [
                    self.size[0].resolve(document)?,
                    self.size[1].resolve(document)?,
                    self.size[2].resolve(document)?,
                ],
                frame,
            },
            ModelingTool::Cylinder => PrimitiveParameters::Cylinder {
                radius: self.radius.resolve(document)?,
                height: self.height.resolve(document)?,
                frame,
            },
            ModelingTool::Sphere => PrimitiveParameters::Sphere {
                radius: self.radius.resolve(document)?,
                frame,
            },
            _ => return Err("Choose a primitive shape".into()),
        };
        Ok(PrimitiveRequest {
            feature: self.request.feature,
            body: self.request.body,
            name: self.name.clone(),
            parameters,
        })
    }
    pub(super) fn remember(&mut self, request: &PrimitiveRequest) {
        let frame = match &request.parameters {
            PrimitiveParameters::Box { size, frame } => {
                for (field, value) in self.size.iter_mut().zip(size) {
                    field.remember(value);
                }
                frame
            }
            PrimitiveParameters::Cylinder {
                radius,
                height,
                frame,
            } => {
                self.radius.remember(radius);
                self.height.remember(height);
                frame
            }
            PrimitiveParameters::Sphere { radius, frame } => {
                self.radius.remember(radius);
                frame
            }
        };
        for (field, value) in self.origin.iter_mut().zip(&frame.origin) {
            field.remember(value);
        }
    }
    pub(super) fn dragging(&self) -> bool {
        self.resize.active() || self.pose.active()
    }
    fn resolved(&self, session: &multicad_core::commands::Session) -> Option<ResolvedPrimitive> {
        self.request(session.document(), true)
            .ok()?
            .parameters
            .resolve(session.parameters())
            .ok()
    }
    pub(super) fn envelope(
        &self,
        session: &multicad_core::commands::Session,
    ) -> Option<[[f64; 3]; 8]> {
        Some(gizmo::corners(&self.resolved(session)?))
    }

    pub(super) fn viewport(
        &mut self,
        ui: &mut egui::Ui,
        session: &multicad_core::commands::Session,
        camera: crate::camera::Camera,
        rect: egui::Rect,
        start_allowed: bool,
        guide: bool,
    ) -> (bool, bool, bool) {
        let document = session.document();
        let parameters = session.parameters();
        let Some(shape) = self.resolved(session) else {
            self.resize = Default::default();
            self.pose = Default::default();
            return (false, false, false);
        };
        let resize = self.resize.show(
            ui,
            &shape,
            camera,
            rect,
            start_allowed && !self.pose.active(),
            guide,
        );
        let mut changed = resize
            .changed
            .is_some_and(|next| self.resize_draft(&shape, &next));
        let view = super::PlacementViewport {
            scene: None,
            camera,
            rect,
            basis: Default::default(),
            start_allowed: start_allowed && !resize.input_owned,
        };
        let mut owned = resize.input_owned;
        let mut released = resize.released;
        match self.frame(document).and_then(|frame| {
            self.pose
                .show(ui, &view, &frame, parameters)
                .map(|gesture| (frame, gesture))
        }) {
            Ok((before, gesture)) => {
                owned |= gesture.input_owned;
                released |= gesture.released;
                if let Some(frame) = gesture.changed {
                    changed |= self.pose_draft(&before, &frame);
                }
            }
            Err(error) => {
                self.pose = Default::default();
                self.error = Some(error);
            }
        }
        if changed {
            self.stamp.changed();
            self.error = None;
        }
        (owned, changed, released)
    }

    fn pose_draft(&mut self, before: &PrimitiveFrame, frame: &PrimitiveFrame) -> bool {
        let basis = [frame.u, frame.v];
        let mut changed = self.basis != basis;
        self.basis = basis;
        for (index, field) in self.origin.iter_mut().enumerate() {
            if before.origin[index] != frame.origin[index]
                && let Ok(replacement) = DimensionField::from_dimension(&frame.origin[index], "mm")
            {
                *field = replacement;
                changed = true;
            }
        }
        changed
    }

    fn resize_draft(&mut self, before: &ResolvedPrimitive, after: &ResolvedPrimitive) -> bool {
        let mut changed = set_lengths(
            &mut self.origin,
            resolved_frame(before).origin,
            resolved_frame(after).origin,
        );
        changed |= match (before, after) {
            (
                ResolvedPrimitive::Box {
                    size_mm: before, ..
                },
                ResolvedPrimitive::Box { size_mm: after, .. },
            ) => set_lengths(&mut self.size, *before, *after),
            (
                ResolvedPrimitive::Cylinder {
                    radius_mm: old_radius,
                    height_mm: old_height,
                    ..
                },
                ResolvedPrimitive::Cylinder {
                    radius_mm: new_radius,
                    height_mm: new_height,
                    ..
                },
            ) => {
                set_length(&mut self.radius, *old_radius, *new_radius)
                    | set_length(&mut self.height, *old_height, *new_height)
            }
            (
                ResolvedPrimitive::Sphere {
                    radius_mm: before, ..
                },
                ResolvedPrimitive::Sphere {
                    radius_mm: after, ..
                },
            ) => set_length(&mut self.radius, *before, *after),
            _ => unreachable!("resize must retain the primitive kind"),
        };
        changed
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState<'_>,
    ) -> Option<ModelEvent> {
        controls::inspector_layout(ui);
        ui.strong(self.label());
        ui.small(if self.editing {"Edit this existing producer and rebuild its downstream features. Body and feature names are retained."}else{"Drag a corner to resize. Use the origin arrows or rings to move or rotate. Enter applies and Escape cancels."});
        ui.label("Body name");
        let mut changed = ui
            .add_enabled(
                !self.editing,
                egui::TextEdit::singleline(&mut self.name)
                    .id(ui.make_persistent_id("primitive-name"))
                    .desired_width(ui.available_width()),
            )
            .on_hover_text("Editable body name; intentional duplicate names are allowed")
            .changed();
        ui.separator();
        ui.label("Orientation plane");
        let current = self.plane();
        controls::inspector_combo(ui,"primitive-plane").selected_text(current.map_or("Custom orientation", Datum::label)).show_ui(ui,|ui| {
            for plane in Datum::ALL {
                if ui.selectable_label(current == Some(plane), plane.label()).clicked() {
                    if current != Some(plane) {
                        self.set_plane(plane);
                        changed = true;
                    }
                    ui.close();
                }
            }
        }).response.on_hover_text("Choose a datum orientation or rotate freely in the viewport. The origin stays in world XYZ.");
        ui.small(match self.plane() {
            Some(Datum::Xy) => "U = +X · V = +Y · height = +Z",
            Some(Datum::Xz) => "U = +X · V = +Z · height = −Y",
            Some(Datum::Yz) => "U = +Y · V = +Z · height = +X",
            None => "Width, depth and height follow the rotated primitive axes.",
        });
        for (field, label) in self
            .origin
            .iter_mut()
            .zip(["Origin X", "Origin Y", "Origin Z"])
        {
            changed |= field.show_named(
                ui,
                label,
                label,
                "World coordinates of the primitive origin. Formulas can reference $parameters.",
            );
        }
        ui.separator();
        if self.kind == ModelingTool::Box {
            for (field, label) in
                self.size
                    .iter_mut()
                    .zip(["Width along U", "Depth along V", "Height along normal"])
            {
                changed |= field.show_named(
                    ui,
                    label,
                    label,
                    "Positive size along the selected orientation axis.",
                );
            }
        } else {
            changed |= self.radius.show_named(
                ui,
                "primitive-radius",
                "Radius",
                "Positive distance from the center to the curved surface",
            );
            if self.kind == ModelingTool::Cylinder {
                changed |= self.height.show_named(
                    ui,
                    "primitive-height",
                    "Height along normal",
                    "Positive height from the cylinder base center",
                );
            }
        }
        ui.separator();
        form::preview_status(ui, self.stamp, preview);
        let validation = self.request(document, enabled);
        if let Err(error) = &validation {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        let action = form::actions(ui, validation.is_ok(), changed, self.stamp, preview);
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        self.finish_frame(changed, action, document, enabled, preview)
    }
    fn finish_frame(
        &mut self,
        changed: bool,
        action: Option<FormAction>,
        document: &Document,
        enabled: bool,
        preview: PreviewState<'_>,
    ) -> Option<ModelEvent> {
        if action == Some(FormAction::Cancel) {
            return Some(ModelEvent::Cancel);
        }
        if changed {
            self.stamp.changed();
            self.error = None;
            if action != Some(FormAction::Preview) {
                return Some(ModelEvent::Changed);
            }
        }
        match action {
            Some(FormAction::Preview) => match self.request(document, enabled) {
                Ok(request) => {
                    self.remember(&request);
                    self.error = None;
                    Some(ModelEvent::Preview {
                        stamp: self.stamp,
                        request: ModelRequest::Primitive(Box::new(request)),
                    })
                }
                Err(error) => {
                    self.error = Some(error);
                    None
                }
            },
            Some(FormAction::Apply)
                if preview.ready(self.stamp) && self.request(document, enabled).is_ok() =>
            {
                Some(ModelEvent::Apply)
            }
            _ => None,
        }
    }
}

fn resolved_frame(primitive: &ResolvedPrimitive) -> &Frame {
    match primitive {
        ResolvedPrimitive::Box { frame, .. }
        | ResolvedPrimitive::Cylinder { frame, .. }
        | ResolvedPrimitive::Sphere { frame, .. } => frame,
    }
}

fn set_length(field: &mut DimensionField, before: f64, after: f64) -> bool {
    if (after - before).abs() <= 1e-8 {
        return false;
    }
    field.set_exact_literal(after, "mm");
    true
}

fn set_lengths(fields: &mut [DimensionField; 3], before: [f64; 3], after: [f64; 3]) -> bool {
    let mut changed = false;
    for (index, field) in fields.iter_mut().enumerate() {
        changed |= set_length(field, before[index], after[index]);
    }
    changed
}

#[cfg(test)]
mod tests;

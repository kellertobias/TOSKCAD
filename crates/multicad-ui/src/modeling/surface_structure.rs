//! Editable surface relief settings. File selection only changes this draft.
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui, viewport::Scene};
use multicad_core::{
    document::{Document, EntityId},
    model::{
        BodyInput, MAX_SURFACE_IMAGE_REPEAT, MAX_SURFACE_STRUCTURE_FACES,
        SurfaceStructureDirection as Direction, SurfaceStructureParameters as Parameters,
        SurfaceStructurePattern as Pattern, SurfaceStructureRequest,
    },
    references::{self, AnalyticGeometry, GeometryIntent, Resolution, TopologyKind},
};
use std::collections::BTreeSet;

pub(super) struct SurfaceStructurePanel {
    stamp: FormStamp,
    feature: EntityId,
    input: Option<BodyInput>,
    faces: Vec<GeometryIntent>,
    pattern: Pattern,
    direction: Direction,
    amplitude: DimensionField,
    pitch: DimensionField,
    size: [DimensionField; 2],
    offset: [DimensionField; 2],
    rotation: DimensionField,
    border: DimensionField,
    picking: bool,
    picking_available: bool,
    image_requested: bool,
}

impl SurfaceStructurePanel {
    pub fn new() -> Result<Self, String> {
        Self::from_source(EntityId::new(), None, vec![], Parameters::default())
    }
    pub fn existing(request: SurfaceStructureRequest) -> Result<Self, String> {
        let mut panel = Self::from_source(
            request.feature,
            Some(request.input),
            request.faces,
            request.parameters,
        )?;
        panel.picking = false;
        Ok(panel)
    }
    fn from_source(
        feature: EntityId,
        input: Option<BodyInput>,
        faces: Vec<GeometryIntent>,
        parameters: Parameters,
    ) -> Result<Self, String> {
        let mm = |value| DimensionField::from_dimension(value, "mm");
        Ok(Self {
            stamp: FormStamp::new(),
            feature,
            input,
            faces,
            pattern: parameters.pattern,
            direction: parameters.direction,
            amplitude: mm(&parameters.amplitude)?,
            pitch: mm(&parameters.pitch)?,
            size: [mm(&parameters.size[0])?, mm(&parameters.size[1])?],
            offset: [mm(&parameters.offset[0])?, mm(&parameters.offset[1])?],
            rotation: DimensionField::from_dimension(&parameters.rotation, "deg")?,
            border: mm(&parameters.border)?,
            picking: true,
            picking_available: false,
            image_requested: false,
        })
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn input(&self) -> Option<&BodyInput> {
        self.input.as_ref()
    }
    pub fn faces(&self) -> &[GeometryIntent] {
        &self.faces
    }
    pub fn picking(&self) -> bool {
        self.picking && self.picking_available
    }
    pub fn set_picking_available(&mut self, available: bool) {
        self.picking_available = available;
    }
    pub fn stop_picking(&mut self) {
        self.picking = false;
    }
    pub fn take_image_request(&mut self) -> Option<FormStamp> {
        std::mem::take(&mut self.image_requested).then_some(self.stamp)
    }
    pub fn set_image(&mut self, pattern: Pattern) -> Result<(), String> {
        if !matches!(pattern, Pattern::HeightMap { .. }) {
            return Err("The selected file did not supply a grayscale height map".into());
        }
        self.pattern = pattern;
        self.stamp.changed();
        Ok(())
    }
    pub fn set_faces(
        &mut self,
        scene: &Scene,
        input: BodyInput,
        faces: Vec<GeometryIntent>,
    ) -> Result<bool, String> {
        if self.input.as_ref().is_some_and(|current| *current != input) {
            return Err("Choose faces on the same source Part".into());
        }
        if faces.len() > MAX_SURFACE_STRUCTURE_FACES {
            return Err(format!(
                "Choose up to {MAX_SURFACE_STRUCTURE_FACES} planar faces on one Part"
            ));
        }
        let snapshot = scene
            .geometry_snapshot(input.producer)
            .ok_or("The retained source Part is unavailable")?;
        let mut keys = BTreeSet::new();
        for face in &faces {
            match references::resolve(face, &snapshot, scene.stamp().revision) {
                Resolution::Resolved { key, .. }
                    if key.kind == TopologyKind::Face
                        && keys.insert(key)
                        && snapshot.elements.iter().any(|item| {
                            item.key == key
                                && item.frames.len() == 1
                                && matches!(item.geometry, Some(AnalyticGeometry::Plane { .. }))
                        }) => {}
                _ => return Err("Choose distinct named planar faces on the retained Part".into()),
            }
        }
        if self.input.as_ref() == Some(&input) && self.faces == faces {
            return Ok(false);
        }
        self.input = Some(input);
        self.faces = faces;
        self.stamp.changed();
        Ok(true)
    }

    pub fn request(
        &self,
        document: &Document,
        enabled: bool,
    ) -> Result<SurfaceStructureRequest, String> {
        if !enabled {
            return Err("Native modeling Preview is unavailable".into());
        }
        let input = self.input.clone().ok_or("Pick a planar face on a Part")?;
        if self.faces.is_empty() {
            return Err("Pick at least one planar face to generate the preview".into());
        }
        let pitch = match self.pitch.resolve(document) {
            Ok(pitch) => pitch,
            Err(_) if matches!(self.pattern, Pattern::HeightMap { .. }) => {
                Parameters::default().pitch
            }
            Err(error) => return Err(error),
        };
        Ok(SurfaceStructureRequest {
            feature: self.feature,
            input,
            faces: self.faces.clone(),
            parameters: Parameters {
                pattern: self.pattern.clone(),
                direction: self.direction,
                amplitude: self.amplitude.resolve(document)?,
                pitch,
                size: [
                    self.size[0].resolve(document)?,
                    self.size[1].resolve(document)?,
                ],
                offset: [
                    self.offset[0].resolve(document)?,
                    self.offset[1].resolve(document)?,
                ],
                rotation: self.rotation.resolve(document)?,
                border: self.border.resolve(document)?,
            },
        })
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState<'_>,
    ) -> Option<ModelEvent> {
        controls::inspector_layout(ui);
        ui.strong("Surface Structure");
        form::preview_status(ui, self.stamp, preview);
        if let Some(input) = &self.input {
            ui.label(
                document
                    .get(input.body)
                    .map_or("Missing Part", |part| part.name.as_str()),
            );
        }
        ui.small(format!(
            "{} of {MAX_SURFACE_STRUCTURE_FACES} planar faces selected",
            self.faces.len()
        ));
        let pick = if self.picking() {
            "Finish picking faces"
        } else {
            "Pick or repair faces"
        };
        if ui
            .add_enabled(self.picking_available, egui::Button::new(pick))
            .clicked()
        {
            self.picking = !self.picking;
        }
        if self.picking() {
            ui.small("Click a planar face. Hold Shift to add or remove a face.");
        }
        ui.separator();
        ui.label("Pattern");
        let mut changed = false;
        controls::inspector_combo(ui, "surface-structure-pattern")
            .selected_text(pattern_label(&self.pattern))
            .show_ui(ui, |ui| {
                for pattern in [Pattern::Ribs, Pattern::Waves, Pattern::Stipple] {
                    let label = pattern_label(&pattern);
                    changed |= ui
                        .selectable_value(&mut self.pattern, pattern, label)
                        .changed();
                }
                if matches!(self.pattern, Pattern::HeightMap { .. }) {
                    ui.label("Height map imported");
                }
            });
        if ui.button("Import grayscale PNG…").clicked() {
            self.image_requested = true;
        }
        if let Pattern::HeightMap {
            width,
            height,
            repeat,
            ..
        } = &mut self.pattern
        {
            ui.small(format!("Embedded {width} × {height} pixels · black is zero relief"))
                .on_hover_text("The top image row maps to +V. Repeated tiles fit the relief window and each raised or recessed cell leaves a 10% gap.");
            for (label, value) in ["Repeat U", "Repeat V"].into_iter().zip(repeat) {
                changed |= ui
                    .horizontal(|ui| {
                        ui.label(label);
                        ui.add(egui::DragValue::new(value).range(1..=MAX_SURFACE_IMAGE_REPEAT))
                            .changed()
                    })
                    .inner;
            }
        } else {
            changed |= self.pitch.show_named(
                ui,
                "surface-pitch",
                "Pattern pitch",
                "Physical spacing between repeats.",
            );
        }
        ui.horizontal(|ui| {
            changed |= ui
                .selectable_value(&mut self.direction, Direction::Raised, "Raised")
                .changed();
            changed |= ui
                .selectable_value(&mut self.direction, Direction::Recessed, "Recessed")
                .changed();
        });
        changed |= self.amplitude.show_named(
            ui,
            "surface-amplitude",
            "Relief height or depth",
            "Maximum physical relief above or below each face.",
        );
        changed |= self.size[0].show_named(
            ui,
            "surface-width",
            "Width U",
            "Width of the rotated relief window.",
        );
        changed |= self.size[1].show_named(
            ui,
            "surface-height",
            "Height V",
            "Height of the rotated relief window.",
        );
        changed |= self.offset[0].show_named(
            ui,
            "surface-offset-u",
            "Offset U",
            "Shift the window from the face extent midpoint.",
        );
        changed |= self.offset[1].show_named(
            ui,
            "surface-offset-v",
            "Offset V",
            "Shift the window from the face extent midpoint.",
        );
        changed |= self.rotation.show_named(
            ui,
            "surface-rotation",
            "Rotation",
            "Rotate the window and pattern together about the outward normal.",
        );
        changed |= self.border.show_named(
            ui,
            "surface-border",
            "Face border",
            "Keep this much of each face flat at its boundary.",
        );
        ui.small(
            "Each face has its own mapping and flat border. Patterns stop before shared edges.",
        );
        if changed {
            self.stamp.changed();
        }
        let request = self.request(document, enabled);
        if let Err(error) = &request {
            ui.small(error);
        }
        let pending = matches!(preview, PreviewState::Pending { stamp } if stamp == self.stamp);
        match form::actions(
            ui,
            request.is_ok() && !pending,
            changed,
            self.stamp,
            preview,
        ) {
            Some(FormAction::Preview) => request.ok().map(|request| ModelEvent::Preview {
                stamp: self.stamp,
                request: ModelRequest::SurfaceStructure(Box::new(request)),
            }),
            Some(FormAction::Apply) => Some(ModelEvent::Apply),
            Some(FormAction::Cancel) => Some(ModelEvent::Cancel),
            None => changed.then_some(ModelEvent::Changed),
        }
    }
}

fn pattern_label(pattern: &Pattern) -> &'static str {
    match pattern {
        Pattern::Ribs => "Ribs",
        Pattern::Waves => "Waves",
        Pattern::Stipple => "Stipple",
        Pattern::HeightMap { .. } => "Height map",
    }
}

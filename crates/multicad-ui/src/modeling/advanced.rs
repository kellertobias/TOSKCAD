//! Advanced features retain canonical intent and use the existing native receipt lane.
mod command;
mod fields;
mod inputs;
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui, viewport::Scene};
pub use command::AdvancedCommand;
use multicad_core::{
    document::{Document, EntityId},
    model::{self, BodyInput, ProfileSource, SketchPathSource},
    project_file::ContentHash,
    references::{self, GeometryIntent, Resolution, TopologyKind},
};
use std::collections::BTreeSet;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdvancedKind {
    Shell,
    Thickness,
    Sweep,
    Loft,
    Draft,
    Mirror,
    LinearPattern,
    CircularPattern,
}
impl AdvancedKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Shell | Self::Thickness => "Shell / Thickness",
            Self::Sweep => "Sweep",
            Self::Loft => "Loft",
            Self::Draft => "Draft",
            Self::Mirror => "Mirror",
            Self::LinearPattern => "Linear pattern",
            Self::CircularPattern => "Circular pattern",
        }
    }
    pub fn requires_body(self) -> bool {
        !matches!(self, Self::Sweep | Self::Loft)
    }
    fn face_tool(self) -> bool {
        matches!(self, Self::Shell | Self::Thickness | Self::Draft)
    }
    fn canonical(self) -> model::AdvancedKind {
        match self {
            Self::Shell | Self::Thickness => model::AdvancedKind::Shell,
            Self::Sweep => model::AdvancedKind::Sweep,
            Self::Loft => model::AdvancedKind::Loft,
            Self::Draft => model::AdvancedKind::Draft,
            Self::Mirror => model::AdvancedKind::Mirror,
            Self::LinearPattern => model::AdvancedKind::LinearPattern,
            Self::CircularPattern => model::AdvancedKind::CircularPattern,
        }
    }
    fn from_parameters(p: &model::AdvancedParameters) -> Self {
        match p.kind() {
            model::AdvancedKind::Shell => Self::Shell,
            model::AdvancedKind::Sweep => Self::Sweep,
            model::AdvancedKind::Loft => Self::Loft,
            model::AdvancedKind::Draft => Self::Draft,
            model::AdvancedKind::Mirror => Self::Mirror,
            model::AdvancedKind::LinearPattern => Self::LinearPattern,
            model::AdvancedKind::CircularPattern => Self::CircularPattern,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct AdvancedSelection {
    pub body: Option<EntityId>,
    pub profile: Option<ProfileSource>,
    pub captures: Vec<GeometryIntent>,
}
pub(super) struct AdvancedPanel {
    request: model::AdvancedRequest,
    kind: AdvancedKind,
    editing: bool,
    stamp: FormStamp,
    fields: fields::ParameterFields,
    inputs: inputs::Inputs,
    picking: bool,
    picking_available: bool,
}
impl AdvancedPanel {
    pub fn new(
        document: &Document,
        revision: u64,
        scene: Option<&Scene>,
        kind: AdvancedKind,
        selection: AdvancedSelection,
    ) -> Result<Self, String> {
        let parameters = model::AdvancedParameters::default_for(kind.canonical());
        let profiles = selection.profile.into_iter().collect();
        let request = if kind.requires_body() {
            let scene = scene.ok_or("Wait for the complete current native model")?;
            if !scene.is_full_model()
                || scene.model_evaluation().is_none_or(|info| {
                    ContentHash::document(document).ok() != Some(info.source_hash)
                })
            {
                return Err(
                    "Advanced Part features require source-bound complete native geometry".into(),
                );
            }
            let body = selection.body.ok_or("Select one Part definition")?;
            let input = super::body_choices::choices(document, revision, scene)?
                .into_iter()
                .find(|c| c.input.body == body)
                .ok_or("Choose a current Part definition; Instance-only features are unsupported")?
                .input;
            model::AdvancedRequest::on_body(input, parameters, selection.captures)
        } else {
            model::AdvancedRequest::new_body(kind.label(), parameters, profiles, None)
        };
        let mut panel = Self::from_request(document, request, kind, false)?;
        if let (Some(scene), Some(input)) = (scene, panel.request.input.as_ref()) {
            validate_faces(scene, input, &panel.request.faces)?;
        }
        panel.picking = kind.face_tool();
        panel.picking_available = scene.is_some();
        Ok(panel)
    }
    pub fn existing(document: &Document, feature: EntityId) -> Result<Self, String> {
        let request =
            model::AdvancedRequest::existing(document, feature).map_err(|e| e.to_string())?;
        let kind = AdvancedKind::from_parameters(&request.parameters);
        Self::from_request(document, request, kind, true)
    }
    fn from_request(
        document: &Document,
        mut request: model::AdvancedRequest,
        kind: AdvancedKind,
        editing: bool,
    ) -> Result<Self, String> {
        if !editing && let model::AdvancedParameters::Loft(parameters) = &mut request.parameters {
            parameters.seams = request
                .profiles
                .iter()
                .filter_map(|p| inputs::initial_seam(document, p))
                .collect();
        }
        Ok(Self {
            fields: fields::ParameterFields::new(&request.parameters)?,
            inputs: inputs::Inputs::new(request.profiles.clone(), request.path.clone()),
            request,
            kind,
            editing,
            stamp: FormStamp::new(),
            picking: false,
            picking_available: false,
        })
    }
    pub fn label(&self) -> &'static str {
        self.kind.label()
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn input(&self) -> Option<&BodyInput> {
        self.request.input.as_ref()
    }
    pub fn captures(&self) -> &[GeometryIntent] {
        &self.request.faces
    }
    pub fn picking(&self) -> bool {
        self.kind.face_tool() && self.picking && self.picking_available
    }
    pub fn set_picking_available(&mut self, available: bool) {
        self.picking_available = available;
    }
    pub fn stop_picking(&mut self) {
        self.picking = false;
    }
    pub fn set_captures(
        &mut self,
        scene: &Scene,
        captures: Vec<GeometryIntent>,
    ) -> Result<bool, String> {
        if !self.kind.face_tool() {
            return Err("This feature does not select source faces".into());
        }
        let input = self.input().ok_or("Select one native Part definition")?;
        validate_faces(scene, input, &captures)?;
        if captures == self.request.faces {
            return Ok(false);
        }
        self.request.faces = captures;
        self.stamp.changed();
        Ok(true)
    }
    pub fn set_sketch_context(
        &mut self,
        profiles: Vec<ProfileSource>,
        paths: Vec<SketchPathSource>,
    ) {
        self.inputs.set_context(profiles, paths);
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<AdvancedCommand, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        let mut request = self.request.clone();
        request.parameters = self.fields.resolve(document)?;
        request.profiles = self.inputs.profiles.clone();
        request.path = self.inputs.path.clone();
        request.parameters.validate().map_err(|e| e.to_string())?;
        // Input readiness is separate from geometric validity. Canonical prepare
        // and the complete native preview remain authoritative for the shape.
        match self.kind {
            AdvancedKind::Shell | AdvancedKind::Thickness | AdvancedKind::Draft
                if request.faces.is_empty() =>
            {
                return Err("Choose at least one current source face before Preview".into());
            }
            AdvancedKind::Sweep
                if request.profiles.len() != 1
                    || request.path.as_ref().is_none_or(|p| p.curves.is_empty()) =>
            {
                return Err(
                    "Choose one closed section and an ordered open straight-line path".into(),
                );
            }
            AdvancedKind::Loft if request.profiles.len() < 2 => {
                return Err(
                    "Add at least two ordered closed sections and choose their start curves".into(),
                );
            }
            _ => {}
        }
        request.source().map_err(|e| e.to_string())?;
        Ok(if self.editing {
            AdvancedCommand::Edit(request)
        } else {
            AdvancedCommand::Create(request)
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
        ui.strong(self.label());
        if let Some(input) = self.input() {
            ui.label(
                document
                    .get(input.body)
                    .map_or("Missing Part", |e| e.name.as_str()),
            );
            ui.small("Part definition · linked Instances follow this feature. A new preview rebuilds downstream geometry.");
        }
        let mut changed = false;
        if self.input().is_none() && !self.editing {
            ui.label("New Part name");
            changed |= ui
                .text_edit_singleline(&mut self.request.name)
                .on_hover_text(
                    "Name of the new native Part; output identities remain fixed across Preview",
                )
                .changed();
        }
        changed |= self.fields.show(ui);
        if matches!(self.kind, AdvancedKind::Sweep | AdvancedKind::Loft) {
            changed |= self.inputs.show(ui, document, self.kind, &mut self.fields);
        }
        if self.kind.face_tool() {
            ui.separator();
            ui.label(format!(
                "{} captured source face(s)",
                self.request.faces.len()
            ));
            if ui
                .add_enabled(
                    self.picking_available,
                    egui::Button::new(if self.picking {
                        "Selecting source faces…"
                    } else {
                        "Select / reselect faces"
                    }),
                )
                .on_hover_text("Pick faces on the frozen upstream Part; Shift adds or removes")
                .clicked()
            {
                self.picking = !self.picking;
            }
            if self.picking() {
                ui.small(
                    "Select the source faces in the viewer. Preview returns to the complete model.",
                );
            }
            let mut remove = None;
            for (index, capture) in self.request.faces.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("Face {}", index + 1))
                        .on_hover_text(format!("Persistent capture {}", capture.id));
                    if ui
                        .small_button("Remove")
                        .on_hover_text("Remove this local face capture")
                        .clicked()
                    {
                        remove = Some(index);
                    }
                });
            }
            if let Some(index) = remove {
                self.request.faces.remove(index);
                changed = true;
            }
        }
        let validation = self.request(document, enabled);
        if let Err(error) = &validation {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        form::preview_status(ui, self.stamp, preview);
        let action = form::actions(ui, validation.is_ok(), changed, self.stamp, preview);
        if action == Some(FormAction::Cancel) {
            return Some(ModelEvent::Cancel);
        }
        if changed {
            self.stamp.changed();
        }
        match action {
            Some(FormAction::Preview) => {
                let command = self.request(document, enabled).ok()?;
                self.stop_picking();
                Some(ModelEvent::Preview {
                    stamp: self.stamp,
                    request: ModelRequest::Advanced(Box::new(command)),
                })
            }
            Some(FormAction::Apply) if !changed && preview.ready(self.stamp) => {
                Some(ModelEvent::Apply)
            }
            _ if changed => Some(ModelEvent::Changed),
            _ => None,
        }
    }
}
fn validate_faces(
    scene: &Scene,
    input: &BodyInput,
    captures: &[GeometryIntent],
) -> Result<(), String> {
    let snapshot = scene
        .geometry_snapshot(input.producer)
        .ok_or("The frozen source Part topology is unavailable")?;
    let mut ids = BTreeSet::new();
    let mut keys = BTreeSet::new();
    for capture in captures {
        if capture.kind != TopologyKind::Face || !ids.insert(capture.id) {
            return Err("Choose distinct persistent source faces".into());
        }
        match references::resolve(capture,&snapshot,scene.stamp().revision){Resolution::Resolved{key,..}if key.kind==TopologyKind::Face&&keys.insert(key)=>{},_=>return Err("A selected source face is missing, ambiguous, duplicated or belongs to another Part".into())}
    }
    Ok(())
}

#[cfg(test)]
mod tests;

//! Explicit definition transforms and independent placed copies share the
//! modeling candidate lane; these controls never alter display geometry.
mod command;
#[cfg(test)]
mod native_bounds_tests;
#[cfg(test)]
mod tests;
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui, viewport::Scene};
pub use command::PlacementCommand;
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{self, BodyInput, NumericTransform, PlacementBatchRequest},
    project_file::ContentHash,
};
use std::collections::BTreeSet;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacementMode {
    Move,
    Copy,
}
pub(super) struct PlacementPanel {
    command: PlacementCommand,
    alternate: Option<PlacementCommand>,
    create_copy: bool,
    copy_modifier: bool,
    selected: BTreeSet<EntityId>,
    stamp: FormStamp,
    translation: [DimensionField; 3],
    pivot: [DimensionField; 3],
    angle: DimensionField,
    axis: [f64; 3],
    targets: Vec<String>,
}
fn fields(
    values: &[multicad_core::constraint_sketch::Dimension; 3],
) -> Result<[DimensionField; 3], String> {
    Ok([
        DimensionField::from_dimension(&values[0], "mm")?,
        DimensionField::from_dimension(&values[1], "mm")?,
        DimensionField::from_dimension(&values[2], "mm")?,
    ])
}
pub(super) fn available(
    document: &Document,
    revision: u64,
    scene: &Scene,
    selected: &BTreeSet<EntityId>,
) -> bool {
    scene.is_full_model()
        && scene.stamp().document == document.id()
        && scene.stamp().revision == revision
        && scene.model_evaluation().is_some()
        && !selected.is_empty()
        && selected.len() <= model::MAX_TRANSFORM_TARGETS
        && selected.iter().all(|id| {
            scene.reference_instances().iter().any(|i| i.id == *id)
                && matches!(
                    document.get(*id).map(|e| &e.data),
                    Some(EntityData::Part { .. } | EntityData::Instance { .. })
                )
        })
}
impl PlacementPanel {
    pub fn new(
        document: &Document,
        revision: u64,
        scene: &Scene,
        selected: &BTreeSet<EntityId>,
        mode: PlacementMode,
    ) -> Result<Self, String> {
        if !available(document, revision, scene, selected) {
            return Err(
                "Select 1–128 current native Parts or Instances before placing them".into(),
            );
        }
        if scene
            .model_evaluation()
            .is_none_or(|info| ContentHash::document(document).ok() != Some(info.source_hash))
        {
            return Err("Placement requires a current source-bound native model".into());
        }
        let mut parameters = NumericTransform::default();
        if let Some(bounds) = selected
            .iter()
            .map(|id| scene.native_instance_bounds(*id))
            .collect::<Option<Vec<_>>>()
        {
            let center = std::array::from_fn(|i| {
                let min = bounds.iter().map(|b| b[0][i]).fold(f64::INFINITY, f64::min);
                let max = bounds
                    .iter()
                    .map(|b| b[1][i])
                    .fold(f64::NEG_INFINITY, f64::max);
                model::length(min + (max - min) * 0.5)
            });
            parameters.pivot = center;
        }
        let (command, targets) = Self::commands(document, scene, selected, mode, &parameters)?;
        let other = if mode == PlacementMode::Move {
            PlacementMode::Copy
        } else {
            PlacementMode::Move
        };
        let (alternate, _) = Self::commands(document, scene, selected, other, &parameters)?;
        let mut panel = Self::from_command(command, parameters, targets)?;
        panel.alternate = Some(alternate);
        panel.selected = selected.clone();
        Ok(panel)
    }
    fn commands(
        document: &Document,
        scene: &Scene,
        selected: &BTreeSet<EntityId>,
        mode: PlacementMode,
        parameters: &NumericTransform,
    ) -> Result<(PlacementCommand, Vec<String>), String> {
        let mut batch = PlacementBatchRequest::default();
        let mut targets = vec![];
        for id in selected {
            let entity = document.get(*id).ok_or("Selected object is missing")?;
            let instance = scene
                .reference_instances()
                .iter()
                .find(|i| i.id == *id)
                .ok_or("Selected native object is missing")?;
            match (&entity.data, mode) {
                (EntityData::Part { .. }, PlacementMode::Move) => {
                    batch.transforms.push(model::TransformRequest::new(
                        BodyInput {
                            body: *id,
                            producer: instance.geometry,
                        },
                        parameters.clone(),
                    ))
                }
                (EntityData::Instance { .. }, PlacementMode::Move) => batch
                    .instances
                    .push(model::InstanceMoveRequest::new(*id, parameters.clone())),
                (EntityData::Part { .. }, PlacementMode::Copy) => {
                    batch.copies.push(model::InstanceCopyRequest::new(
                        model::InstanceCopySource::Part {
                            source: BodyInput {
                                body: *id,
                                producer: instance.geometry,
                            },
                        },
                        parameters.clone(),
                    ))
                }
                (EntityData::Instance { .. }, PlacementMode::Copy) => {
                    batch.copies.push(model::InstanceCopyRequest::new(
                        model::InstanceCopySource::Instance { instance: *id },
                        parameters.clone(),
                    ))
                }
                _ => return Err("Only explicit Parts and Instances can be placed".into()),
            }
            targets.push(format!(
                "{} · {}",
                entity.name,
                if matches!(entity.data, EntityData::Part { .. }) {
                    "Part definition"
                } else {
                    "Instance pose"
                }
            ));
        }
        Ok((PlacementCommand::Batch(batch), targets))
    }
    pub fn existing(document: &Document, id: EntityId) -> Result<Self, String> {
        if let Ok(source) = model::read_transform(document, id) {
            let mut panel = Self::from_command(
                PlacementCommand::EditPart {
                    feature: id,
                    parameters: source.parameters.clone(),
                },
                source.parameters,
                vec![format!(
                    "{} · Part definition",
                    document
                        .get(source.output.body)
                        .map_or("Missing Part", |e| e.name.as_str())
                )],
            )?;
            panel.selected.insert(source.output.body);
            return Ok(panel);
        }
        let source = model::read_instance_placement(document, id).map_err(|e| e.to_string())?;
        let mut panel = Self::from_command(
            PlacementCommand::EditInstance {
                driver: id,
                instance: source.owner,
                parameters: source.parameters.clone(),
            },
            source.parameters,
            vec![format!(
                "{} · Instance pose",
                document
                    .get(source.owner)
                    .map_or("Missing Instance", |e| e.name.as_str())
            )],
        )?;
        panel.selected.insert(source.owner);
        Ok(panel)
    }
    fn from_command(
        command: PlacementCommand,
        parameters: NumericTransform,
        targets: Vec<String>,
    ) -> Result<Self, String> {
        Ok(Self {
            create_copy: command.is_copy(),
            command,
            alternate: None,
            copy_modifier: false,
            selected: BTreeSet::new(),
            stamp: FormStamp::new(),
            translation: fields(&parameters.translation)?,
            pivot: fields(&parameters.pivot)?,
            angle: DimensionField::from_dimension(&parameters.angle, "deg")?,
            axis: parameters.axis,
            targets,
        })
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        "Move / Copy"
    }
    pub fn selected(&self) -> &BTreeSet<EntityId> {
        &self.selected
    }
    pub fn retarget(
        &mut self,
        document: &Document,
        revision: u64,
        scene: &Scene,
        selected: &BTreeSet<EntityId>,
    ) -> Result<(), String> {
        if self.alternate.is_none() {
            return Err("Finish the saved feature edit before choosing different targets".into());
        }
        if selected == &self.selected {
            return Ok(());
        }
        if !selected.is_empty() && !available(document, revision, scene, selected) {
            return Err("Choose current native Parts or Instances".into());
        }
        if scene
            .model_evaluation()
            .is_none_or(|info| ContentHash::document(document).ok() != Some(info.source_hash))
        {
            return Err("Target selection requires the current native source".into());
        }
        let parameters = self.parameters(document)?;
        let (command, targets) =
            Self::commands(document, scene, selected, PlacementMode::Move, &parameters)?;
        let (alternate, _) =
            Self::commands(document, scene, selected, PlacementMode::Copy, &parameters)?;
        self.command = command;
        self.alternate = Some(alternate);
        self.targets = targets;
        self.selected = selected.clone();
        self.stamp.changed();
        Ok(())
    }
    fn active_command(&self) -> &PlacementCommand {
        let copy = self.create_copy || self.copy_modifier;
        if copy != self.command.is_copy() {
            self.alternate.as_ref().unwrap_or(&self.command)
        } else {
            &self.command
        }
    }
    pub fn set_copy_modifier(&mut self, active: bool) -> bool {
        let active = active && self.alternate.is_some();
        if self.copy_modifier == active {
            return false;
        }
        self.copy_modifier = active;
        self.stamp.changed();
        true
    }
    pub(super) fn finish_copy_gesture(&mut self) {
        self.create_copy |= self.copy_modifier;
        self.copy_modifier = false;
    }

    pub fn request(&self, document: &Document, enabled: bool) -> Result<PlacementCommand, String> {
        if !enabled {
            return Err("Native placement preview is unavailable".into());
        }
        if self.alternate.is_some() && self.selected.is_empty() {
            return Err(
                "Select one or more Parts or Instances; Shift-click adds or removes targets".into(),
            );
        }
        if !self.active_command().is_copy() && self.selected.iter().any(|id| matches!(document.get(*id).map(|e| &e.data), Some(EntityData::Instance { component, .. }) if self.selected.contains(component))) {
            return Err("Choose a Part definition or its linked Instance for moving; Create copy can copy both independently".into());
        }
        Ok(self
            .active_command()
            .with_parameters(self.parameters(document)?))
    }
    pub(super) fn parameters(&self, document: &Document) -> Result<NumericTransform, String> {
        let parameters = NumericTransform {
            translation: [
                self.translation[0].resolve(document)?,
                self.translation[1].resolve(document)?,
                self.translation[2].resolve(document)?,
            ],
            pivot: [
                self.pivot[0].resolve(document)?,
                self.pivot[1].resolve(document)?,
                self.pivot[2].resolve(document)?,
            ],
            axis: self.axis,
            angle: self.angle.resolve(document)?,
        };
        parameters.validate().map_err(|e| e.to_string())?;
        Ok(parameters)
    }
    pub(super) fn set_drag_parameters(
        &mut self,
        parameters: &NumericTransform,
    ) -> Result<(), String> {
        parameters.validate().map_err(|e| e.to_string())?;
        self.translation = fields(&parameters.translation)?;
        self.pivot = fields(&parameters.pivot)?;
        self.angle = DimensionField::from_dimension(&parameters.angle, "deg")?;
        self.axis = parameters.axis;
        self.stamp.changed();
        Ok(())
    }
    fn preview_request(
        &mut self,
        document: &Document,
        enabled: bool,
    ) -> Result<PlacementCommand, String> {
        if !enabled {
            return Err("Native placement preview is unavailable".into());
        }
        let parameters = self.parameters(document)?;
        for (field, value) in self.translation.iter_mut().zip(&parameters.translation) {
            field.remember(value);
        }
        for (field, value) in self.pivot.iter_mut().zip(&parameters.pivot) {
            field.remember(value);
        }
        self.angle.remember(&parameters.angle);
        self.request(document, enabled)?;
        Ok(self.active_command().with_parameters(parameters))
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
        ui.small("Release the handle to stage. Press Enter to apply or Escape to cancel.");
        ui.small(format!("{} selected object(s)", self.targets.len()));
        egui::ScrollArea::vertical()
            .id_salt("placement-targets")
            .max_height(110.)
            .show(ui, |ui| {
                for target in &self.targets {
                    ui.label(target).on_hover_text(target);
                }
            });
        let mut changed = false;
        if self.alternate.is_some() {
            changed |= ui.checkbox(&mut self.create_copy, "Create copy")
                .on_hover_text("Create independent linked Instances. Hold Option on macOS or Alt on Windows/Linux while dragging to copy temporarily; Shift changes selection")
                .changed();
            if self.copy_modifier {
                ui.small("Option / Alt: copying this gesture");
            }
        }
        ui.small(if self.active_command().is_copy(){"Create independent Instances sharing the original Part geometry. Existing geometry and poses stay unchanged."}else{"Part definition edits propagate to its linked Instances. An Instance selection changes only that occurrence's pose."});
        ui.separator();
        ui.strong("World translation");
        for (field, label) in self.translation.iter_mut().zip(["X", "Y", "Z"]) {
            changed |= field.show(
                ui,
                label,
                "Translation after rotation; value or a stable parameter formula",
            );
        }
        ui.separator();
        ui.strong("Rotation pivot · world coordinates");
        for (field, label) in self.pivot.iter_mut().zip(["Pivot X", "Pivot Y", "Pivot Z"]) {
            changed |= field.show(
                ui,
                label,
                "Explicit shared world pivot used by every selected object",
            );
        }
        ui.label("World rotation axis");
        ui.horizontal_wrapped(|ui| {
            for (label, axis) in [
                ("X", [1., 0., 0.]),
                ("Y", [0., 1., 0.]),
                ("Z", [0., 0., 1.]),
            ] {
                if ui
                    .selectable_label(self.axis == axis, label)
                    .on_hover_text("Use this world axis through the explicit pivot")
                    .clicked()
                {
                    self.axis = axis;
                    changed = true;
                }
            }
        });
        for (axis, label) in self.axis.iter_mut().zip(["Axis X", "Axis Y", "Axis Z"]) {
            ui.horizontal(|ui| {
                ui.label(label);
                changed |= ui
                    .add(egui::DragValue::new(axis).speed(0.1))
                    .on_hover_text(
                        "Custom finite nonzero axis; normalized by the typed geometric operation",
                    )
                    .changed();
            });
        }
        changed|=self.angle.show(ui,"Angle","Signed angle: rotate about the pivot first, then translate; formulas retain parameter identities");
        let validation = self.request(document, enabled);
        if let Err(error) = &validation {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        ui.separator();
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
                self.preview_request(document, enabled)
                    .ok()
                    .map(|request| ModelEvent::Preview {
                        stamp: self.stamp,
                        request: ModelRequest::Placement(Box::new(request)),
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

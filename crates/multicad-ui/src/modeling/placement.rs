//! Explicit definition transforms and independent placed copies share the
//! modeling candidate lane; these controls never alter display geometry.
mod command;
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
    scene.stamp().document == document.id()
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
        if mode==PlacementMode::Move && selected.iter().any(|id| matches!(document.get(*id).map(|e|&e.data),Some(EntityData::Instance{component,..}) if selected.contains(component))) { return Err("Choose a Part definition or its linked Instance, not both; moving the definition already moves the linked geometry".into()); }
        let parameters = NumericTransform::default();
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
        Self::from_command(PlacementCommand::Batch(batch), parameters, targets)
    }
    pub fn existing(document: &Document, id: EntityId) -> Result<Self, String> {
        if let Ok(source) = model::read_transform(document, id) {
            return Self::from_command(
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
            );
        }
        let source = model::read_instance_placement(document, id).map_err(|e| e.to_string())?;
        Self::from_command(
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
        )
    }
    fn from_command(
        command: PlacementCommand,
        parameters: NumericTransform,
        targets: Vec<String>,
    ) -> Result<Self, String> {
        Ok(Self {
            command,
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
        if self.command.is_copy() {
            "Placed copy"
        } else {
            "Move / rotate"
        }
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<PlacementCommand, String> {
        if !enabled {
            return Err("Native placement preview is unavailable".into());
        }
        Ok(self.command.with_parameters(self.parameters(document)?))
    }
    fn parameters(&self, document: &Document) -> Result<NumericTransform, String> {
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
        Ok(self.command.with_parameters(parameters))
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
        ui.small(format!("{} selected object(s)", self.targets.len()));
        egui::ScrollArea::vertical()
            .id_salt("placement-targets")
            .max_height(110.)
            .show(ui, |ui| {
                for target in &self.targets {
                    ui.label(target).on_hover_text(target);
                }
            });
        ui.small(if self.command.is_copy(){"Create independent Instances sharing the original Part geometry. Existing geometry and poses stay unchanged."}else{"Part definition edits propagate to its linked Instances. An Instance selection changes only that occurrence's pose."});
        let mut changed = false;
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

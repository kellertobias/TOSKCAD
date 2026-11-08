//! Exact scale controls share the native modeling candidate lane. Authored
//! formulas and output identities survive previews and copy-modifier changes.
mod command;
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui, viewport::Scene};
pub use command::ScaleCommand;
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{self, BodyInput, ScaleBatchRequest, ScaleInput, ScaleParameters, ScaleTarget},
    project_file::ContentHash,
};
use std::collections::BTreeSet;

pub(super) struct ScalePanel {
    command: ScaleCommand,
    alternate: Option<ScaleCommand>,
    create_copy: bool,
    copy_modifier: bool,
    selected: BTreeSet<EntityId>,
    stamp: FormStamp,
    factors: [DimensionField; 3],
    pivot: [DimensionField; 3],
    axes: [[f64; 3]; 3],
}
fn fields(
    values: &[multicad_core::constraint_sketch::Dimension; 3],
    unit: &'static str,
) -> Result<[DimensionField; 3], String> {
    Ok([
        DimensionField::from_dimension(&values[0], unit)?,
        DimensionField::from_dimension(&values[1], unit)?,
        DimensionField::from_dimension(&values[2], unit)?,
    ])
}
pub(super) fn available(
    document: &Document,
    revision: u64,
    scene: &Scene,
    selected: &BTreeSet<EntityId>,
) -> bool {
    super::placement::available(document, revision, scene, selected)
        && scene
            .model_evaluation()
            .is_some_and(|info| ContentHash::document(document).ok() == Some(info.source_hash))
}
impl ScalePanel {
    pub fn new(
        document: &Document,
        revision: u64,
        scene: &Scene,
        selected: &BTreeSet<EntityId>,
        copy: bool,
    ) -> Result<Self, String> {
        if !available(document, revision, scene, selected) {
            return Err("Select current native Parts or Instances before scaling".into());
        }
        let bounds = selected
            .iter()
            .map(|id| scene.native_instance_bounds(*id))
            .collect::<Option<Vec<_>>>()
            .ok_or("Scale pivot requires exact native target bounds")?;
        let parameters = ScaleParameters {
            pivot: std::array::from_fn(|i| {
                let low = bounds.iter().map(|b| b[0][i]).fold(f64::INFINITY, f64::min);
                let high = bounds
                    .iter()
                    .map(|b| b[1][i])
                    .fold(f64::NEG_INFINITY, f64::max);
                model::length(low + (high - low) * 0.5)
            }),
            ..Default::default()
        };
        let command = Self::batch(document, scene, selected, &parameters, copy)?;
        let alternate = Self::batch(document, scene, selected, &parameters, !copy)?;
        let mut result = Self::from_command(command, parameters, selected.clone())?;
        result.alternate = Some(alternate);
        Ok(result)
    }
    fn batch(
        document: &Document,
        scene: &Scene,
        selected: &BTreeSet<EntityId>,
        parameters: &ScaleParameters,
        copy: bool,
    ) -> Result<ScaleCommand, String> {
        let targets = selected
            .iter()
            .map(|id| match document.get(*id).map(|e| &e.data) {
                Some(EntityData::Part { .. }) => scene
                    .reference_instances()
                    .iter()
                    .find(|i| i.id == *id)
                    .map(|i| ScaleTarget::Part {
                        input: BodyInput {
                            body: *id,
                            producer: i.geometry,
                        },
                    })
                    .ok_or_else(|| "Scale target has no current native producer".into()),
                Some(EntityData::Instance { .. }) => Ok(ScaleTarget::Instance { instance: *id }),
                _ => Err("Only explicit Parts and Instances can be scaled".into()),
            })
            .collect::<Result<Vec<_>, String>>()?;
        ScaleBatchRequest::new(document, targets, parameters.clone(), copy)
            .map(ScaleCommand::Batch)
            .map_err(|e| e.to_string())
    }
    pub fn existing(document: &Document, feature: EntityId) -> Result<Self, String> {
        let source = model::read_scale(document, feature).map_err(|e| e.to_string())?;
        let instance = match source.input {
            ScaleInput::Component { owner, .. } => Some(owner),
            _ => None,
        };
        let selected = BTreeSet::from([instance.unwrap_or(source.output.body)]);
        Self::from_command(
            ScaleCommand::Edit {
                feature,
                instance,
                parameters: source.parameters.clone(),
            },
            source.parameters,
            selected,
        )
    }
    fn from_command(
        command: ScaleCommand,
        parameters: ScaleParameters,
        selected: BTreeSet<EntityId>,
    ) -> Result<Self, String> {
        Ok(Self {
            create_copy: command.is_copy(),
            command,
            alternate: None,
            copy_modifier: false,
            selected,
            stamp: FormStamp::new(),
            factors: fields(&parameters.factors, "ratio")?,
            pivot: fields(&parameters.pivot, "mm")?,
            axes: parameters.axes,
        })
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn selected(&self) -> &BTreeSet<EntityId> {
        &self.selected
    }
    pub fn label(&self) -> &'static str {
        "Scale"
    }
    fn active_command(&self) -> &ScaleCommand {
        if (self.create_copy || self.copy_modifier) != self.command.is_copy() {
            self.alternate.as_ref().unwrap_or(&self.command)
        } else {
            &self.command
        }
    }
    pub fn set_copy_modifier(&mut self, active: bool) -> bool {
        let active = active && self.alternate.is_some();
        if active == self.copy_modifier {
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

    pub fn retarget(
        &mut self,
        document: &Document,
        revision: u64,
        scene: &Scene,
        selected: &BTreeSet<EntityId>,
    ) -> Result<(), String> {
        if self.alternate.is_none() {
            return Err("Finish the saved scale edit before choosing other targets".into());
        }
        if selected == &self.selected {
            return Ok(());
        }
        if !selected.is_empty() && !available(document, revision, scene, selected) {
            return Err("Choose current native scale targets".into());
        }
        if !selected.is_empty() {
            let parameters = self.parameters(document)?;
            self.command = Self::batch(document, scene, selected, &parameters, false)?;
            self.alternate = Some(Self::batch(document, scene, selected, &parameters, true)?);
        }
        self.selected = selected.clone();
        self.stamp.changed();
        Ok(())
    }
    pub(super) fn parameters(&self, document: &Document) -> Result<ScaleParameters, String> {
        let parameters = ScaleParameters {
            factors: [
                self.factors[0].resolve(document)?,
                self.factors[1].resolve(document)?,
                self.factors[2].resolve(document)?,
            ],
            pivot: [
                self.pivot[0].resolve(document)?,
                self.pivot[1].resolve(document)?,
                self.pivot[2].resolve(document)?,
            ],
            axes: self.axes,
        };
        parameters
            .resolve(&multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        Ok(parameters)
    }
    pub(super) fn set_axes(&mut self, axes: [[f64; 3]; 3]) -> Result<bool, String> {
        if self.axes == axes {
            return Ok(false);
        }
        ScaleParameters {
            axes,
            ..Default::default()
        }
        .validate()
        .map_err(|e| e.to_string())?;
        self.axes = axes;
        self.stamp.changed();
        Ok(true)
    }
    pub(super) fn set_drag_parameters(
        &mut self,
        parameters: &ScaleParameters,
    ) -> Result<(), String> {
        parameters.validate().map_err(|e| e.to_string())?;
        self.factors = fields(&parameters.factors, "ratio")?;
        self.pivot = fields(&parameters.pivot, "mm")?;
        self.axes = parameters.axes;
        self.stamp.changed();
        Ok(())
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<ScaleCommand, String> {
        if !enabled {
            return Err("Native scale preview is unavailable".into());
        }
        if self.selected.is_empty() {
            return Err("Select Parts or Instances; Shift adds or removes targets".into());
        }
        if !self.active_command().is_copy()&&self.selected.iter().any(|id|matches!(document.get(*id).map(|e|&e.data),Some(EntityData::Instance{component,..})if self.selected.contains(component))){return Err("Choose a Part definition or its linked Instance; scaling both applies twice".into())}
        Ok(self
            .active_command()
            .with_parameters(self.parameters(document)?))
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState<'_>,
    ) -> Option<ModelEvent> {
        controls::inspector_layout(ui);
        ui.strong("Scale");
        ui.small("Release the handle to stage. Press Enter to apply or Escape to cancel.");
        ui.small(format!("{} selected object(s)", self.selected.len()));
        ui.small("Part edits affect linked Instances. An Instance selection scales only its own geometry.");
        let mut changed = false;
        if self.alternate.is_some() {
            changed|=ui.checkbox(&mut self.create_copy,"Create copy").on_hover_text("Keep originals and create independent scale identities; Option / Alt temporarily copies; Shift changes targets").changed();
            if self.copy_modifier {
                ui.small("Option / Alt: copying this gesture");
            }
        }
        ui.separator();
        ui.strong("Factors");
        for (field, label) in self.factors.iter_mut().zip(["X", "Y", "Z"]) {
            changed|=field.show(ui,label,"Positive ratio or =$parameter; zero, negative and nonfinite factors are unsupported");
        }
        ui.strong("Pivot · world coordinates");
        for (field, label) in self.pivot.iter_mut().zip(["Pivot X", "Pivot Y", "Pivot Z"]) {
            changed |= field.show(
                ui,
                label,
                "Shared explicit world pivot; initial value is the selected native bounds center",
            );
        }
        ui.small("Drag the white center square for uniform scaling; colored squares scale one axis. Choose World, Object or datum axes in the viewport coordinate control.");
        for (i, axis) in self.axes.iter().enumerate() {
            ui.small(format!(
                "{} direction: {:.3}, {:.3}, {:.3}",
                ["X", "Y", "Z"][i],
                axis[0],
                axis[1],
                axis[2]
            ));
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
                let request = self.request(document, enabled).ok()?;
                let parameters = self.parameters(document).ok()?;
                for (field, value) in self.factors.iter_mut().zip(&parameters.factors) {
                    field.remember(value);
                }
                for (field, value) in self.pivot.iter_mut().zip(&parameters.pivot) {
                    field.remember(value);
                }
                Some(ModelEvent::Preview {
                    stamp: self.stamp,
                    request: ModelRequest::Scale(Box::new(request)),
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
#[cfg(test)]
mod tests;

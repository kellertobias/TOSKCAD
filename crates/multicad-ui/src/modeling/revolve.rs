use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui};
use multicad_core::{
    document::Document,
    model::{RevolveParameters, RevolveRequest},
};

pub(super) struct RevolvePanel {
    request: RevolveRequest,
    stamp: FormStamp,
    origin: [DimensionField; 2],
    bearing: DimensionField,
    sweep: DimensionField,
    error: Option<String>,
}
impl RevolvePanel {
    pub fn new(request: RevolveRequest) -> Result<Self, String> {
        if request.outputs.is_empty() {
            return Err("Select at least one exact closed profile".into());
        }
        let parameters = &request.parameters;
        Ok(Self {
            origin: [
                DimensionField::from_dimension(&parameters.axis_origin[0], "mm")?,
                DimensionField::from_dimension(&parameters.axis_origin[1], "mm")?,
            ],
            bearing: DimensionField::from_dimension(&parameters.axis_angle, "deg")?,
            sweep: DimensionField::from_dimension(&parameters.angle, "deg")?,
            stamp: FormStamp::new(),
            request,
            error: None,
        })
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<RevolveRequest, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        Ok(RevolveRequest {
            sketch: self.request.sketch,
            outputs: self.request.outputs.clone(),
            parameters: RevolveParameters {
                axis_origin: [
                    self.origin[0].resolve(document)?,
                    self.origin[1].resolve(document)?,
                ],
                axis_angle: self.bearing.resolve(document)?,
                angle: self.sweep.resolve(document)?,
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
        ui.strong("Revolve");
        ui.label(format!(
            "{} selected profiles · independent output bodies",
            self.request.outputs.len()
        ));
        ui.small("The axis uses the selected sketch's local U/V plane. Native Preview checks exact profiles and volumetric results before Apply.");
        ui.separator();
        let mut changed = false;
        for (field, label) in self
            .origin
            .iter_mut()
            .zip(["Axis origin U", "Axis origin V"])
        {
            changed|=field.show(ui,label,"A point on the revolution axis in local sketch coordinates; parameter formulas stay associative");
        }
        changed |= self.bearing.show(
            ui,
            "Axis bearing",
            "Angle from sketch U towards V. A 90° bearing follows the sketch V axis.",
        );
        changed|=self.sweep.show(ui,"Signed sweep angle","Positive follows the right-hand rule about the axis; a negative angle reverses it. The native preview accepts a nonzero sweep up to one full turn (360°). ");
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
                    self.origin[0].remember(&request.parameters.axis_origin[0]);
                    self.origin[1].remember(&request.parameters.axis_origin[1]);
                    self.bearing.remember(&request.parameters.axis_angle);
                    self.sweep.remember(&request.parameters.angle);
                    self.error = None;
                    Some(ModelEvent::Preview {
                        stamp: self.stamp,
                        request: ModelRequest::Revolve(Box::new(request)),
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

#[cfg(test)]
mod tests;

use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui};
use multicad_core::{
    document::Document,
    model::{RevolveParameters, RevolveRequest},
    parameters::Unit,
};

pub(super) struct RevolvePanel {
    request: RevolveRequest,
    stamp: FormStamp,
    origin: [DimensionField; 2],
    bearing: DimensionField,
    sweep: DimensionField,
    error: Option<String>,
    /// A new revolve must not guess its axis. Reopened features and requests
    /// with an explicitly chosen axis keep it.
    axis_confirmed: bool,
    axis_lines: Vec<super::operation_guide::GuideLine>,
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
            axis_confirmed: true,
            axis_lines: vec![],
        })
    }
    /// New revolves start with an explicit axis-acquisition step.
    pub fn acquire_axis(mut self) -> Self {
        self.axis_confirmed = false;
        self
    }
    #[cfg(test)]
    pub fn axis_confirmed(&self) -> bool {
        self.axis_confirmed
    }
    pub(crate) fn set_axis_lines(&mut self, lines: Vec<super::operation_guide::GuideLine>) {
        self.axis_lines = lines;
    }
    /// Use a straight sketch segment (or a sketch datum axis) as the axis.
    pub(crate) fn set_axis(&mut self, origin: [f64; 2], bearing_deg: f64) {
        self.origin[0].set_literal(origin[0], "mm");
        self.origin[1].set_literal(origin[1], "mm");
        self.bearing.set_literal(bearing_deg, "deg");
        self.axis_confirmed = true;
        self.stamp.changed();
        self.error = None;
    }
    pub(crate) fn set_axis_line(&mut self, a: [f64; 2], b: [f64; 2]) -> Result<(), String> {
        let d = [b[0] - a[0], b[1] - a[1]];
        if !(d[0].is_finite() && d[1].is_finite()) || d[0].hypot(d[1]) < 1e-9 {
            return Err("A revolution axis needs a straight line with two distinct ends".into());
        }
        self.set_axis(a, d[1].atan2(d[0]).to_degrees());
        Ok(())
    }
    /// Axis origin (mm, sketch U/V) and bearing (rad) once confirmed.
    pub(crate) fn axis(&self, document: &Document) -> Option<Result<([f64; 2], f64), String>> {
        if !self.axis_confirmed {
            return None;
        }
        Some((|| {
            let parameters =
                multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?;
            let value = |field: &DimensionField, unit| {
                field
                    .resolve(document)?
                    .value_in(unit, Some(&parameters))
                    .map_err(|e| e.to_string())
            };
            Ok((
                [
                    value(&self.origin[0], Unit::Millimeter)?,
                    value(&self.origin[1], Unit::Millimeter)?,
                ],
                value(&self.bearing, Unit::Radian)?,
            ))
        })())
    }
    pub(crate) fn sweep_degrees(&self, document: &Document) -> Result<f64, String> {
        let parameters =
            multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?;
        Ok(self
            .sweep
            .resolve(document)?
            .value_in(Unit::Radian, Some(&parameters))
            .map_err(|e| e.to_string())?
            .to_degrees())
    }
    pub(crate) fn set_sweep_degrees(&mut self, degrees: f64) {
        self.sweep.set_literal(degrees, "deg");
        self.stamp.changed();
        self.error = None;
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<RevolveRequest, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if !self.axis_confirmed {
            return Err("Choose the revolution axis: click a straight sketch line in the viewer, or pick a sketch axis or line in the Inspector".into());
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
        if !self.axis_confirmed {
            ui.colored_label(
                egui::Color32::LIGHT_YELLOW,
                "Axis not chosen · click a straight sketch line in the viewer, or choose one here",
            );
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Sketch U axis")
                .on_hover_text("Revolve about the sketch's local U axis through its origin")
                .clicked()
            {
                self.set_axis([0., 0.], 0.);
                changed = true;
            }
            if ui
                .button("Sketch V axis")
                .on_hover_text("Revolve about the sketch's local V axis through its origin")
                .clicked()
            {
                self.set_axis([0., 0.], 90.);
                changed = true;
            }
            if !self.axis_confirmed
                && ui
                    .button("Use typed axis")
                    .on_hover_text("Confirm the axis origin and bearing typed below")
                    .clicked()
            {
                self.axis_confirmed = true;
                changed = true;
            }
        });
        if !self.axis_lines.is_empty() {
            let mut chosen = None;
            controls::inspector_combo(ui, "revolve-axis-line")
                .selected_text("Straight sketch line…")
                .show_ui(ui, |ui| {
                    for (index, line) in self.axis_lines.iter().enumerate() {
                        if ui
                            .selectable_label(
                                false,
                                format!(
                                    "Line {} · ({:.1}, {:.1}) → ({:.1}, {:.1})",
                                    index + 1,
                                    line.a[0],
                                    line.a[1],
                                    line.b[0],
                                    line.b[1]
                                ),
                            )
                            .clicked()
                        {
                            chosen = Some((line.a, line.b));
                        }
                    }
                });
            if let Some((a, b)) = chosen {
                match self.set_axis_line(a, b) {
                    Ok(()) => changed = true,
                    Err(error) => self.error = Some(error),
                }
            }
        }
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

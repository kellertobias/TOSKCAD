use super::{
    FormStamp, ModelingTool, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui};
use multicad_core::{
    document::Document,
    model::{self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest},
    sketch::Datum,
};

pub(super) struct PrimitivePanel {
    request: PrimitiveRequest,
    kind: ModelingTool,
    stamp: FormStamp,
    name: String,
    origin: [DimensionField; 3],
    size: [DimensionField; 3],
    radius: DimensionField,
    height: DimensionField,
    plane: Datum,
    error: Option<String>,
    editing: bool,
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
        };
        let plane = Datum::ALL
            .into_iter()
            .find(|datum| datum.frame().u == frame.u && datum.frame().v == frame.v)
            .ok_or(
                "This form supports XY, XZ and YZ primitive frames; the stored frame was preserved",
            )?;
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
            plane,
            error: None,
            editing: false,
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
    pub fn request(&self, document: &Document, enabled: bool) -> Result<PrimitiveRequest, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.name.trim().is_empty() || self.name.len() > 1000 {
            return Err("Enter a body name with 1–1000 bytes".into());
        }
        let basis = self.plane.frame();
        let frame = PrimitiveFrame {
            origin: [
                self.origin[0].resolve(document)?,
                self.origin[1].resolve(document)?,
                self.origin[2].resolve(document)?,
            ],
            u: basis.u,
            v: basis.v,
        };
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
            _ => return Err("Choose a box or cylinder primitive".into()),
        };
        Ok(PrimitiveRequest {
            feature: self.request.feature,
            body: self.request.body,
            name: self.name.clone(),
            parameters,
        })
    }
    fn remember(&mut self, request: &PrimitiveRequest) {
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
        };
        for (field, value) in self.origin.iter_mut().zip(&frame.origin) {
            field.remember(value);
        }
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
        ui.small(if self.editing {"Edit this existing producer and rebuild its downstream features. Body and feature names are retained."}else{"Create an independent native body. Preview validates geometry before Apply changes the document."});
        ui.label("Body name");
        let mut changed = ui
            .add_enabled(
                !self.editing,
                egui::TextEdit::singleline(&mut self.name).desired_width(ui.available_width()),
            )
            .on_hover_text("Editable body name; intentional duplicate names are allowed")
            .changed();
        ui.separator();
        ui.label("Orientation plane");
        controls::inspector_combo(ui,"primitive-plane").selected_text(self.plane.label()).show_ui(ui,|ui| {
            for plane in Datum::ALL {changed|=ui.selectable_value(&mut self.plane,plane,plane.label()).changed();}
        }).response.on_hover_text("Size follows the chosen plane's U, V and normal axes; origin coordinates stay in world XYZ");
        ui.small(match self.plane {
            Datum::Xy => "U = +X · V = +Y · height = +Z",
            Datum::Xz => "U = +X · V = +Z · height = −Y",
            Datum::Yz => "U = +Y · V = +Z · height = +X",
        });
        for (field, label) in self
            .origin
            .iter_mut()
            .zip(["Origin X", "Origin Y", "Origin Z"])
        {
            changed|=field.show(ui,label,"World coordinates of the box corner or cylinder base center; formulas can reference $parameters");
        }
        ui.separator();
        if self.kind == ModelingTool::Box {
            for (field, label) in
                self.size
                    .iter_mut()
                    .zip(["Width along U", "Depth along V", "Height along normal"])
            {
                changed|=field.show(ui,label,"Positive size along the selected orientation frame; values or =expressions are retained as canonical source");
            }
        } else {
            changed |= self.radius.show(
                ui,
                "Radius",
                "Positive radius about the selected plane's normal",
            );
            changed |= self.height.show(
                ui,
                "Height along normal",
                "Positive height from the cylinder base center",
            );
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

#[cfg(test)]
mod tests;

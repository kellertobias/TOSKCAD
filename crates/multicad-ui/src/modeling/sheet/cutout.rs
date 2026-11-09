//! Through-cutouts authored in the selected folded sheet face's coordinates.
use super::{
    super::{
        FormStamp, PreviewState,
        dimension::DimensionField,
        form::{self, FormAction, ModelEvent, ModelRequest},
    },
    SheetCommand, field,
};
use crate::{controls, egui};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{self, Cutout, CutoutFace, CutoutShape, SheetDefinition},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Circle,
    Rectangle,
}

pub(in crate::modeling) struct CutoutPanel {
    stamp: FormStamp,
    target: EntityId,
    feature: EntityId,
    name: String,
    source: SheetDefinition,
    face: Option<CutoutFace>,
    shape: Shape,
    center: [DimensionField; 2],
    diameter: DimensionField,
    size: [DimensionField; 2],
}

impl CutoutPanel {
    pub fn source(
        document: &Document,
        target: EntityId,
    ) -> Result<(EntityId, SheetDefinition), String> {
        let part = match document.get(target).map(|entity| &entity.data) {
            Some(EntityData::Instance { component, .. }) => *component,
            _ => target,
        };
        let feature = model::sheet_feature(document, part).unwrap_or(part);
        let source = model::read_sheet(document, feature)
            .map_err(|_| "Select a sheet-metal part to add a cutout".to_owned())?;
        if matches!(
            document.get(feature).map(|entity| &entity.data),
            Some(EntityData::Feature {
                suppressed: true,
                ..
            })
        ) {
            return Err("Unsuppress the sheet feature before adding a cutout".into());
        }
        let Some(EntityData::Part { features, .. }) =
            document.get(source.output.body).map(|entity| &entity.data)
        else {
            return Err("The sheet output Part is missing".into());
        };
        if features.iter().rev().find(|&&id| {
            matches!(
                document.get(id).map(|entity| &entity.data),
                Some(EntityData::Feature {
                    suppressed: false,
                    ..
                })
            )
        }) != Some(&feature)
        {
            return Err("The selected sheet has later features that cannot be developed".into());
        }
        if source.cutouts.len() >= model::MAX_CUTOUTS {
            return Err(format!(
                "This sheet already has the maximum of {} cutouts",
                model::MAX_CUTOUTS,
            ));
        }
        Ok((feature, source))
    }

    pub fn new(document: &Document, target: EntityId) -> Result<(EntityId, Self), String> {
        let (feature, source) = Self::source(document, target)?;
        let name = document
            .get(source.output.body)
            .map_or("Sheet part", |entity| &entity.name)
            .to_owned();
        Ok((
            feature,
            Self {
                stamp: FormStamp::new(),
                target,
                feature,
                name,
                source,
                face: None,
                shape: Shape::Circle,
                center: [field(0., "mm"), field(0., "mm")],
                diameter: field(6., "mm"),
                size: [field(12., "mm"), field(6., "mm")],
            },
        ))
    }

    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }

    pub fn picking_target(&self) -> Option<(EntityId, EntityId)> {
        self.face
            .is_none()
            .then_some((self.source.output.body, self.target))
    }

    pub fn pick(&mut self, face: CutoutFace, uv: [f64; 2]) -> bool {
        if self.face.is_some()
            || matches!(face, CutoutFace::Flange { index } if index as usize >= self.source.flanges.len())
            || !uv
                .iter()
                .all(|value| value.is_finite() && value.abs() <= 1e9)
        {
            return false;
        }
        self.face = Some(face);
        for (coordinate, value) in self.center.iter_mut().zip(uv) {
            coordinate.set_exact_literal(value, "mm");
        }
        self.stamp.changed();
        true
    }

    pub fn request(&self, document: &Document, enabled: bool) -> Result<SheetCommand, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        let face = self
            .face
            .ok_or("Pick a base face or flange leg on the folded sheet")?;
        let center = [
            self.center[0].resolve(document)?,
            self.center[1].resolve(document)?,
        ];
        let shape = match self.shape {
            Shape::Circle => CutoutShape::Circle {
                center,
                diameter: self.diameter.resolve(document)?,
            },
            Shape::Rectangle => CutoutShape::Rectangle {
                center,
                size: [
                    self.size[0].resolve(document)?,
                    self.size[1].resolve(document)?,
                ],
            },
        };
        let mut sheet = self.source.clone();
        sheet.cutouts.push(Cutout { face, shape });
        Ok(SheetCommand {
            editing: true,
            feature: self.feature,
            name: self.name.clone(),
            sheet,
        })
    }

    pub fn remember(&mut self, command: &SheetCommand) {
        if let Some(cutout) = command.sheet.cutouts.last() {
            let center = match &cutout.shape {
                CutoutShape::Circle { center, diameter } => {
                    self.diameter.remember(diameter);
                    center
                }
                CutoutShape::Rectangle { center, size } => {
                    for (field, value) in self.size.iter_mut().zip(size) {
                        field.remember(value);
                    }
                    center
                }
            };
            for (field, value) in self.center.iter_mut().zip(center) {
                field.remember(value);
            }
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
        ui.strong("Add cutout");
        ui.small(&self.name);
        ui.small("The preview checks both the folded sheet and its developed pattern. Tooling and press setup require process data.");

        let mut changed = false;
        if let Some(face) = self.face {
            ui.label(match face {
                CutoutFace::Base => "Base sheet face".to_owned(),
                CutoutFace::Flange { index } => format!("Flange {} leg", index + 1),
            });
            if ui.button("Pick another location").clicked() {
                self.face = None;
                changed = true;
            }
        } else {
            ui.label("Click the cutout center on a planar base face or flange leg.");
            ui.small("Choose a face away from the curved bend and thin edge faces.");
        }

        ui.horizontal(|ui| {
            changed |= ui
                .radio_value(&mut self.shape, Shape::Circle, "Circle")
                .changed();
            changed |= ui
                .radio_value(&mut self.shape, Shape::Rectangle, "Rectangle")
                .changed();
        });
        if let Some(face) = self.face {
            let [u_help, v_help] = match face {
                CutoutFace::Base => [
                    "Along the base sheet's U axis",
                    "Along the base sheet's V axis",
                ],
                CutoutFace::Flange { .. } => [
                    "Along the edge from the flange start",
                    "Along the leg from the end of its last bend",
                ],
            };
            changed |= self.center[0].show_named(ui, "cutout-u", "Center U", u_help);
            changed |= self.center[1].show_named(ui, "cutout-v", "Center V", v_help);
        }
        match self.shape {
            Shape::Circle => {
                changed |= self.diameter.show_named(
                    ui,
                    "cutout-diameter",
                    "Diameter",
                    "Through-hole diameter",
                );
            }
            Shape::Rectangle => {
                changed |=
                    self.size[0].show_named(ui, "cutout-width", "Width", "Opening width along U");
                changed |= self.size[1].show_named(
                    ui,
                    "cutout-height",
                    "Height",
                    "Opening height along V",
                );
            }
        }

        if changed {
            self.stamp.changed();
        }
        form::preview_status(ui, self.stamp, preview);
        let request = self.request(document, enabled);
        if let Err(error) = &request
            && self.face.is_some()
        {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        match form::actions(ui, request.is_ok(), changed, self.stamp, preview) {
            Some(FormAction::Cancel) => Some(ModelEvent::Cancel),
            Some(FormAction::Apply) => Some(ModelEvent::Apply),
            Some(FormAction::Preview) => request.ok().map(|request| {
                self.remember(&request);
                ModelEvent::Preview {
                    stamp: self.stamp,
                    request: ModelRequest::Sheet(Box::new(request)),
                }
            }),
            None if changed => Some(ModelEvent::Changed),
            None => None,
        }
    }
}

//! Generic hardware forms share canonical geometry and thread recipe controls.
mod screw_shape;
mod wood;
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
    thread::ThreadPanel,
};
use crate::{controls, egui};
use multicad_core::{
    document::Document,
    model::{
        self, MechanicalPartParameters, MechanicalPartRequest, PrimitiveFrame, ThreadMode,
        ThreadParameters, ThreadRepresentation, ThreadRequest, length,
    },
    parameters::EvaluatedParameters,
    sketch::Datum,
};
use screw_shape::ScrewShapeForm;
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Washer,
    HexNut,
    MachineScrew,
    WoodScrew,
    BearingEnvelope,
}
impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Washer => "Plain washer",
            Self::HexNut => "Hex nut",
            Self::MachineScrew => "Machine screw",
            Self::WoodScrew => "Wood screw envelope",
            Self::BearingEnvelope => "Bearing mounting envelope",
        }
    }
}
pub(super) struct MechanicalPartPanel {
    request: MechanicalPartRequest,
    parameters: EvaluatedParameters,
    kind: Kind,
    fields: [DimensionField; 3],
    thread: ThreadPanel,
    screw_fields: [DimensionField; 2],
    screw_shape: ScrewShapeForm,
    wood: wood::WoodForm,
    screw_length_convention: model::ScrewLengthConvention,
    screw_length_typed: bool,
    origin: [DimensionField; 3],
    plane: Option<Datum>,
    stamp: FormStamp,
    editing: bool,
}
impl MechanicalPartPanel {
    pub fn new(
        request: MechanicalPartRequest,
        parameters: EvaluatedParameters,
        editing: bool,
    ) -> Result<Self, String> {
        let wood_defaults = model::default_wood_screw();
        let MechanicalPartParameters::WoodScrew {
            parameters: wood_source,
        } = (if matches!(
            request.parameters,
            MechanicalPartParameters::WoodScrew { .. }
        ) {
            &request.parameters
        } else {
            &wood_defaults
        })
        else {
            unreachable!("canonical wood default")
        };
        let wood = wood::WoodForm::new(wood_source)?;
        let defaults = model::default_machine_screw();
        let screw_source = if matches!(
            request.parameters,
            MechanicalPartParameters::MachineScrew { .. }
        ) {
            &request.parameters
        } else {
            &defaults
        };
        let MechanicalPartParameters::MachineScrew {
            underhead_length,
            length_input,
            shank_diameter,
            head,
            drive,
            ..
        } = screw_source
        else {
            unreachable!("canonical screw default")
        };
        let (screw_length_convention, screw_length) =
            model::screw_length_source(underhead_length, length_input)
                .map_err(|e| e.to_string())?;
        let screw_length_typed = length_input.is_some();
        let screw_dimensions = [screw_length.clone(), shank_diameter.clone()];
        let screw_shape = ScrewShapeForm::new(head, drive)?;
        let (kind, dimensions, thread) = match &request.parameters {
            MechanicalPartParameters::Washer {
                bore_diameter,
                outside_diameter,
                thickness,
                ..
            } => (
                Kind::Washer,
                [
                    bore_diameter.clone(),
                    outside_diameter.clone(),
                    thickness.clone(),
                ],
                Self::default_thread(),
            ),
            MechanicalPartParameters::BearingEnvelope {
                bore_diameter,
                outside_diameter,
                width,
                ..
            } => (
                Kind::BearingEnvelope,
                [
                    bore_diameter.clone(),
                    outside_diameter.clone(),
                    width.clone(),
                ],
                Self::default_thread(),
            ),
            MechanicalPartParameters::CustomConnection { .. } => {
                return Err("Edit custom connection hardware through its Screw connection editor so the complete receiving stack is validated".into());
            }
            MechanicalPartParameters::WoodScrew { .. } => (
                Kind::WoodScrew,
                [length(0.), length(0.), length(0.)],
                Self::default_thread(),
            ),
            MechanicalPartParameters::MachineScrew { thread, .. } => (
                Kind::MachineScrew,
                [length(0.), length(0.), length(0.)],
                *thread.clone(),
            ),
            MechanicalPartParameters::HexNut {
                across_flats,
                thread,
            } => (
                Kind::HexNut,
                [across_flats.clone(), length(0.), length(0.)],
                *thread.clone(),
            ),
        };
        let frame = request.parameters.frame();
        let field = |d| DimensionField::from_dimension(d, "mm");
        Ok(Self {
            kind,
            screw_shape,
            wood,
            screw_length_convention,
            screw_length_typed,
            screw_fields: [field(&screw_dimensions[0])?, field(&screw_dimensions[1])?],
            fields: [
                field(&dimensions[0])?,
                field(&dimensions[1])?,
                field(&dimensions[2])?,
            ],
            origin: [
                field(&frame.origin[0])?,
                field(&frame.origin[1])?,
                field(&frame.origin[2])?,
            ],
            plane: Datum::ALL
                .into_iter()
                .find(|d| d.frame().u == frame.u && d.frame().v == frame.v),
            thread: ThreadPanel::new(
                ThreadRequest::new("Hardware thread recipe", thread),
                parameters.clone(),
                vec![],
                &Default::default(),
                editing,
            )?,
            parameters,
            request,
            stamp: FormStamp::new(),
            editing,
        })
    }
    fn default_thread() -> ThreadParameters {
        ThreadParameters {
            major_diameter: length(6.),
            pitch: length(1.),
            length: length(5.2),
            representation: ThreadRepresentation::CosmeticEnvelope,
            ..Default::default()
        }
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        "Hardware"
    }
    pub fn request(
        &self,
        document: &Document,
        enabled: bool,
    ) -> Result<MechanicalPartRequest, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.request.name.trim().is_empty() || self.request.name.len() > 1000 {
            return Err("Enter a name with 1–1000 bytes".into());
        }
        let basis = self.plane.map(Datum::frame);
        let original = self.request.parameters.frame();
        let frame = PrimitiveFrame {
            origin: [
                self.origin[0].resolve(document)?,
                self.origin[1].resolve(document)?,
                self.origin[2].resolve(document)?,
            ],
            u: basis.map_or(original.u, |b| b.u),
            v: basis.map_or(original.v, |b| b.v),
        };
        let parameters = match self.kind {
            Kind::Washer => MechanicalPartParameters::Washer {
                bore_diameter: self.fields[0].resolve(document)?,
                outside_diameter: self.fields[1].resolve(document)?,
                thickness: self.fields[2].resolve(document)?,
                frame,
            },
            Kind::BearingEnvelope => MechanicalPartParameters::BearingEnvelope {
                bore_diameter: self.fields[0].resolve(document)?,
                outside_diameter: self.fields[1].resolve(document)?,
                width: self.fields[2].resolve(document)?,
                frame,
            },
            Kind::WoodScrew => self.wood.resolve(document, frame)?,
            Kind::MachineScrew => {
                let mut thread = self.thread.recipe(document, ThreadMode::NewExternal)?;
                thread.frame = frame;
                let (head, drive) = self.screw_shape.resolve(document)?;
                MechanicalPartParameters::MachineScrew {
                    underhead_length: if self.screw_length_typed {
                        None
                    } else {
                        Some(self.screw_fields[0].resolve(document)?)
                    },
                    length_input: if self.screw_length_typed {
                        let value = self.screw_fields[0].resolve(document)?;
                        Some(match self.screw_length_convention {
                            model::ScrewLengthConvention::Underhead => {
                                model::ScrewLengthInput::Underhead { value }
                            }
                            model::ScrewLengthConvention::Overall => {
                                model::ScrewLengthInput::Overall { value }
                            }
                        })
                    } else {
                        None
                    },
                    shank_diameter: self.screw_fields[1].resolve(document)?,
                    head,
                    drive,
                    thread: Box::new(thread),
                }
            }
            Kind::HexNut => {
                let mut thread = self.thread.recipe(document, ThreadMode::CutInternal)?;
                thread.frame = frame;
                MechanicalPartParameters::HexNut {
                    across_flats: self.fields[0].resolve(document)?,
                    thread: Box::new(thread),
                }
            }
        };
        parameters
            .resolve(&self.parameters)
            .map_err(|e| e.to_string())?;
        Ok(MechanicalPartRequest {
            parameters,
            ..self.request.clone()
        })
    }
    fn change_kind(&mut self, kind: Kind) {
        self.kind = kind;
        let values = match kind {
            Kind::Washer => [3.2, 7., 0.5],
            Kind::BearingEnvelope => [10., 30., 9.],
            Kind::HexNut => [10., 0., 0.],
            Kind::MachineScrew | Kind::WoodScrew => [0., 0., 0.],
        };
        if kind == Kind::MachineScrew {
            // A new selection starts from the canonical default. Reopening an
            // authored screw never enters this path or rewrites its lengths.
            if let MechanicalPartParameters::MachineScrew { thread, .. } =
                model::default_machine_screw()
                && let Ok(panel) = ThreadPanel::new(
                    ThreadRequest::new("Screw thread recipe", *thread),
                    self.parameters.clone(),
                    vec![],
                    &Default::default(),
                    false,
                )
            {
                self.thread = panel;
            }
        } else if kind == Kind::HexNut
            && let Ok(panel) = ThreadPanel::new(
                ThreadRequest::new("Nut thread recipe", Self::default_thread()),
                self.parameters.clone(),
                vec![],
                &Default::default(),
                false,
            )
        {
            self.thread = panel;
        }
        for (field, value) in self.fields.iter_mut().zip(values) {
            field.set_literal(value, "mm");
        }
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState,
    ) -> Option<ModelEvent> {
        controls::inspector_layout(ui);
        ui.strong(if self.editing {
            "Edit hardware"
        } else {
            "Create hardware"
        });
        form::preview_status(ui, self.stamp, preview);
        ui.small("Custom parametric hardware. Sourced presets are available through Library.");
        ui.label("Part name");
        let mut changed = ui
            .add_enabled(
                !self.editing,
                egui::TextEdit::singleline(&mut self.request.name)
                    .id(ui.make_persistent_id(("hardware_name", self.request.feature)))
                    .desired_width(ui.available_width()),
            )
            .changed();
        let mut selected = self.kind;
        ui.add_enabled_ui(!self.editing, |ui| {
            controls::inspector_combo(ui, "mechanical-kind")
                .selected_text(self.kind.label())
                .show_ui(ui, |ui| {
                    for kind in [
                        Kind::Washer,
                        Kind::HexNut,
                        Kind::MachineScrew,
                        Kind::WoodScrew,
                        Kind::BearingEnvelope,
                    ] {
                        ui.selectable_value(&mut selected, kind, kind.label());
                    }
                });
        });
        if selected != self.kind {
            self.change_kind(selected);
            changed = true;
        }
        if self.kind == Kind::WoodScrew {
            changed |= self.wood.show(ui, self.request.feature);
        } else if self.kind == Kind::MachineScrew {
            egui::ComboBox::from_id_salt("screw_length_convention")
                .selected_text(match self.screw_length_convention {
                    model::ScrewLengthConvention::Underhead => "Underhead length",
                    model::ScrewLengthConvention::Overall => "Overall length",
                })
                .show_ui(ui, |ui| {
                    for (value, label) in [
                        (model::ScrewLengthConvention::Underhead, "Underhead length"),
                        (model::ScrewLengthConvention::Overall, "Overall length"),
                    ] {
                        if ui
                            .selectable_value(&mut self.screw_length_convention, value, label)
                            .changed()
                        {
                            self.screw_length_typed = true;
                            changed = true;
                        }
                    }
                });
            for (index, label) in ["Length", "Shank diameter"].into_iter().enumerate() {
                changed |= self.screw_fields[index].show_named(ui,(self.request.feature,"screw",index),label,"Retained millimeters or a bound expression; the selected length convention is explicit.");
            }
            changed |= self.screw_shape.show(ui, self.request.feature);
            ui.small("The head extends behind the neck datum. Overall length includes its actual height; underhead length runs from the neck datum to the tip. Thread length is independent and ends at the tip. Changing convention retains the authored value or expression.");
            changed |=
                self.thread
                    .recipe_controls(ui, ThreadMode::NewExternal, "Thread length", false);
        } else if self.kind == Kind::HexNut {
            changed |= self.fields[0].show(
                ui,
                "Across flats",
                "Nut height and axis are retained once in the thread recipe",
            );
            changed |= self.thread.recipe_controls(
                ui,
                ThreadMode::CutInternal,
                "Nut height / thread length",
                false,
            );
        } else {
            for (field, label) in self.fields.iter_mut().zip([
                "Bore diameter",
                "Outside diameter",
                if self.kind == Kind::Washer {
                    "Thickness"
                } else {
                    "Width"
                },
            ]) {
                changed |= field.show(
                    ui,
                    label,
                    "Positive dimensions; outside diameter must exceed the bore",
                );
            }
            if self.kind == Kind::BearingEnvelope {
                ui.small(
                    "Mounting envelope only. Races, rolling elements and seals are not modeled.",
                );
            }
        }
        ui.separator();
        ui.strong("Part axis");
        controls::inspector_combo(ui, "mechanical-plane")
            .selected_text(self.plane.map_or("Stored orientation", Datum::label))
            .show_ui(ui, |ui| {
                for datum in Datum::ALL {
                    changed |= ui
                        .selectable_value(&mut self.plane, Some(datum), datum.label())
                        .changed();
                }
            });
        for (field, label) in
            self.origin
                .iter_mut()
                .zip(["Base center X", "Base center Y", "Base center Z"])
        {
            changed |= field.show(
                ui,
                label,
                if matches!(self.kind, Kind::MachineScrew | Kind::WoodScrew) {
                    "Underhead center; shaft extends along the normal and head behind it"
                } else {
                    "The part extends along the plane normal from this base center"
                },
            );
        }
        let validation = self.request(document, enabled);
        match &validation {
            Ok(request) => {
                let source = model::MechanicalPartDefinition {
                    output: model::BodyOutput { body: request.body },
                    parameters: request.parameters.clone(),
                    provenance: request.provenance.clone(),
                };
                if let Some(reference) = &source.provenance {
                    ui.small(format!(
                        "Reference: {} · {}",
                        reference.designation,
                        if source
                            .matches_reference_dimensions(&self.parameters)
                            .unwrap_or(false)
                        {
                            "dimensions match"
                        } else {
                            "custom / derived dimensions"
                        }
                    ));
                } else {
                    ui.small("Custom geometry · no verified catalog designation");
                }
            }
            Err(error) => {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
        }
        let action = form::actions(ui, validation.is_ok(), changed, self.stamp, preview);
        if action == Some(FormAction::Cancel) {
            return Some(ModelEvent::Cancel);
        }
        if changed {
            self.stamp.changed();
            if action != Some(FormAction::Preview) {
                return Some(ModelEvent::Changed);
            }
        }
        match action {
            Some(FormAction::Preview) => {
                self.request(document, enabled)
                    .ok()
                    .map(|r| ModelEvent::Preview {
                        stamp: self.stamp,
                        request: ModelRequest::MechanicalPart(Box::new(r)),
                    })
            }
            Some(FormAction::Apply) if preview.ready(self.stamp) && validation.is_ok() => {
                Some(ModelEvent::Apply)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod length_contract_tests {
    use super::*;
    use multicad_core::{
        commands::Session, constraint_sketch::Dimension, parameters::BoundExpression,
    };
    #[test]
    fn wood_parent_form_reopens_retained_source_and_keeps_machine_controls_separate() {
        let session = Session::new(Document::new("Wood parent form").unwrap()).unwrap();
        let mut parameters = model::default_wood_screw();
        let MechanicalPartParameters::WoodScrew { parameters: wood } = &mut parameters else {
            unreachable!()
        };
        wood.length_input = model::ScrewLengthInput::Overall {
            value: Dimension::Expression {
                expression: BoundExpression::bind(session.document(), "=44", "mm", None).unwrap(),
            },
        };
        wood.frame.origin[0] = length(15.);
        let request = MechanicalPartRequest::new("Wood parent", parameters);
        let mut panel =
            MechanicalPartPanel::new(request.clone(), session.parameters().clone(), true).unwrap();
        assert!(panel.kind == Kind::WoodScrew);
        assert_eq!(panel.request(session.document(), true).unwrap(), request);
        // Machine recipe drafts must not leak into a reopened wood source.
        panel.screw_fields[0].text = "=".into();
        assert_eq!(panel.request(session.document(), true).unwrap(), request);
        panel.origin[1].text = "21".into();
        let edited = panel.request(session.document(), true).unwrap();
        assert_eq!(edited.feature, request.feature);
        assert_eq!(edited.body, request.body);
        assert_ne!(edited.parameters.frame(), request.parameters.frame());
        assert!(panel.request(session.document(), false).is_err());
    }
    #[test]
    fn one_length_control_retains_expression_and_request_ids() {
        let s = Session::new(Document::new("Length form").unwrap()).unwrap();
        // A constant expression still has authored syntax which must survive loading/editing.
        let expression = Dimension::Expression {
            expression: BoundExpression::bind(s.document(), "=13", "mm", None).unwrap(),
        };
        let mut p = model::default_machine_screw();
        let MechanicalPartParameters::MachineScrew {
            underhead_length,
            length_input,
            ..
        } = &mut p
        else {
            unreachable!()
        };
        *underhead_length = None;
        *length_input = Some(model::ScrewLengthInput::Overall {
            value: expression.clone(),
        });
        let request = MechanicalPartRequest::new("Length form", p);
        let mut panel =
            MechanicalPartPanel::new(request.clone(), s.parameters().clone(), true).unwrap();
        assert_eq!(panel.request(s.document(), true).unwrap(), request);
        panel.screw_shape.head = screw_shape::HeadKind::ExternalHex;
        panel.screw_shape.drive = screw_shape::DriveKind::ExternalHex;
        let changed = panel.request(s.document(), true).unwrap();
        assert_eq!(changed.feature, request.feature);
        let MechanicalPartParameters::MachineScrew {
            underhead_length,
            length_input,
            ..
        } = changed.parameters
        else {
            unreachable!()
        };
        assert_eq!(underhead_length, None);
        assert_eq!(
            length_input,
            Some(model::ScrewLengthInput::Overall { value: expression })
        );
    }
}

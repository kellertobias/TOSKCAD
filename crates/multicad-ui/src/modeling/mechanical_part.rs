//! Generic hardware forms share canonical geometry and thread recipe controls.
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
        self, MechanicalPartParameters, MechanicalPartRequest, PrimitiveFrame, ScrewDrive,
        ScrewHead, ThreadMode, ThreadParameters, ThreadRepresentation, ThreadRequest, length,
    },
    parameters::EvaluatedParameters,
    sketch::Datum,
};
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Washer,
    HexNut,
    MachineScrew,
    BearingEnvelope,
}
impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Washer => "Plain washer",
            Self::HexNut => "Hex nut",
            Self::MachineScrew => "Machine screw",
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
    screw_fields: [DimensionField; 6],
    screw_hex: bool,
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
            shank_diameter,
            head,
            drive,
            ..
        } = screw_source
        else {
            unreachable!("canonical screw default")
        };
        let (head_size, head_height, screw_hex) = match head {
            ScrewHead::SocketCap { diameter, height } => (diameter, height, false),
            ScrewHead::ExternalHex {
                across_flats,
                height,
            } => (across_flats, height, true),
        };
        let (socket_size, socket_depth) = match drive {
            ScrewDrive::HexSocket {
                across_flats,
                depth,
            } => (across_flats.clone(), depth.clone()),
            ScrewDrive::ExternalHex => (length(2.55), length(1.3)),
        };
        let screw_dimensions = [
            underhead_length.clone(),
            shank_diameter.clone(),
            head_size.clone(),
            head_height.clone(),
            socket_size,
            socket_depth,
        ];
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
            screw_hex,
            screw_fields: [
                field(&screw_dimensions[0])?,
                field(&screw_dimensions[1])?,
                field(&screw_dimensions[2])?,
                field(&screw_dimensions[3])?,
                field(&screw_dimensions[4])?,
                field(&screw_dimensions[5])?,
            ],
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
            Kind::MachineScrew => {
                let mut thread = self.thread.recipe(document, ThreadMode::NewExternal)?;
                thread.frame = frame;
                let head_size = self.screw_fields[2].resolve(document)?;
                let height = self.screw_fields[3].resolve(document)?;
                MechanicalPartParameters::MachineScrew {
                    underhead_length: self.screw_fields[0].resolve(document)?,
                    shank_diameter: self.screw_fields[1].resolve(document)?,
                    head: if self.screw_hex {
                        ScrewHead::ExternalHex {
                            across_flats: head_size,
                            height,
                        }
                    } else {
                        ScrewHead::SocketCap {
                            diameter: head_size,
                            height,
                        }
                    },
                    drive: if self.screw_hex {
                        ScrewDrive::ExternalHex
                    } else {
                        ScrewDrive::HexSocket {
                            across_flats: self.screw_fields[4].resolve(document)?,
                            depth: self.screw_fields[5].resolve(document)?,
                        }
                    },
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
            Kind::MachineScrew => [0., 0., 0.],
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
        if self.kind == Kind::MachineScrew {
            controls::inspector_combo(ui, "screw-head-drive")
                .selected_text(if self.screw_hex {
                    "External hex head"
                } else {
                    "Socket cap · hex socket"
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut self.screw_hex, false, "Socket cap · hex socket")
                        .changed();
                    changed |= ui
                        .selectable_value(&mut self.screw_hex, true, "External hex head")
                        .changed();
                });
            for (index, label) in [
                "Underhead length",
                "Shank diameter",
                if self.screw_hex {
                    "Head across flats"
                } else {
                    "Head diameter"
                },
                "Head height",
                "Socket across flats",
                "Socket depth",
            ]
            .into_iter()
            .enumerate()
            {
                if index < 4 || !self.screw_hex {
                    changed |= self.screw_fields[index].show(ui, label, "Authored millimeters or a bound project parameter; validation precedes native Preview");
                }
            }
            ui.small("Underhead length runs toward the tip. The head lies behind the underhead plane. Thread length is independent: shorter threads leave a plain shank beside the head.");
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
                if self.kind == Kind::MachineScrew {
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

//! Thread controls keep one canonical request; native candidate receipts gate Apply.
use super::{
    FormStamp, PreviewState,
    body_choices::BodyChoice,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui};
use multicad_core::{
    document::{Document, EntityId},
    model::{
        self, CustomThreadProfile, PrimitiveFrame, ThreadFamily, ThreadFit, ThreadHand, ThreadMode,
        ThreadParameters, ThreadRepresentation, ThreadRequest,
    },
    parameters::EvaluatedParameters,
    sketch::Datum,
};

pub(super) struct ThreadPanel {
    request: ThreadRequest,
    parameters: EvaluatedParameters,
    stamp: FormStamp,
    choices: Vec<BodyChoice>,
    target: Option<EntityId>,
    mode: ThreadMode,
    family: ThreadFamily,
    fit: ThreadFit,
    engagement_basis: model::MetricFitEngagementBasis,
    engagement_length: DimensionField,
    end_treatments_enabled: bool,
    start_chamfer: bool,
    end_relief: bool,
    start_depth: DimensionField,
    end_length: DimensionField,
    end_depth: DimensionField,
    pipe: Option<model::PipeThreadIntent>,
    npt: Option<model::NptThreadIntent>,
    buttress: Option<model::ButtressIntent>,
    hand: ThreadHand,
    starts: u32,
    representation: ThreadRepresentation,
    representation_parameter: Option<EntityId>,
    name: String,
    diameter: DimensionField,
    pitch: DimensionField,
    length: DimensionField,
    preset_length_default: bool,
    radial: DimensionField,
    axial: DimensionField,
    wall: DimensionField,
    custom: [DimensionField; 5],
    origin: [DimensionField; 3],
    plane: Option<Datum>,
    editing: bool,
    error: Option<String>,
}
impl ThreadPanel {
    pub fn new(
        request: ThreadRequest,
        parameters: EvaluatedParameters,
        choices: Vec<BodyChoice>,
        selected: &std::collections::BTreeSet<EntityId>,
        editing: bool,
    ) -> Result<Self, String> {
        let p = &request.parameters;
        let custom = p.custom.clone().unwrap_or_default();
        let target = request.target.as_ref().map(|t| t.body).or_else(|| {
            choices
                .iter()
                .find(|c| selected.contains(&c.input.body))
                .map(|c| c.input.body)
        });
        let field = |dimension, unit| DimensionField::from_dimension(dimension, unit);
        let start = p.end_treatments.as_ref().and_then(|t| match &t.start {
            model::ThreadStartTreatment::Chamfer45 { radial_depth } => Some(radial_depth),
            _ => None,
        });
        let end = p.end_treatments.as_ref().and_then(|t| match &t.end {
            model::ThreadEndTreatment::CylindricalRelief {
                length,
                radial_depth,
            } => Some((length, radial_depth)),
            _ => None,
        });
        Ok(Self {
            parameters,
            stamp: FormStamp::new(),
            choices,
            target,
            mode: request.mode,
            family: p.family,
            fit: p.fit,
            engagement_basis: match &p.metric_fit_engagement {
                None => model::MetricFitEngagementBasis::LegacyCompleteProfile,
                Some(model::MetricFitEngagement::IntendedLength { .. }) => {
                    model::MetricFitEngagementBasis::IntendedLength
                }
                Some(model::MetricFitEngagement::NormalGroupAssumed {}) => {
                    model::MetricFitEngagementBasis::NormalGroupAssumed
                }
            },
            engagement_length: field(
                match &p.metric_fit_engagement {
                    Some(model::MetricFitEngagement::IntendedLength { length }) => length,
                    _ => &p.length,
                },
                "mm",
            )?,
            end_treatments_enabled: p.end_treatments.is_some(),
            start_chamfer: start.is_some(),
            end_relief: end.is_some(),
            start_depth: field(start.unwrap_or(&model::length(0.2)), "mm")?,
            end_length: field(end.map_or(&model::length(0.5), |e| e.0), "mm")?,
            end_depth: field(end.map_or(&model::length(0.1), |e| e.1), "mm")?,
            pipe: p.pipe,
            npt: p.npt,
            buttress: p.buttress,
            hand: p.hand,
            starts: p.starts,
            representation: p.representation,
            representation_parameter: p.representation_parameter,
            name: request.name.clone(),
            diameter: field(&p.major_diameter, "mm")?,
            pitch: field(&p.pitch, "mm")?,
            length: field(&p.length, "mm")?,
            preset_length_default: !editing && *p == ThreadParameters::default(),
            radial: field(&p.radial_allowance, "mm")?,
            axial: field(&p.axial_allowance, "mm")?,
            wall: field(&p.minimum_wall, "mm")?,
            custom: [
                field(&custom.depth, "mm")?,
                field(&custom.crest_width, "mm")?,
                field(&custom.lower_flank, "deg")?,
                field(&custom.upper_flank, "deg")?,
                field(&custom.radial_taper, "ratio")?,
            ],
            origin: [
                field(&p.frame.origin[0], "mm")?,
                field(&p.frame.origin[1], "mm")?,
                field(&p.frame.origin[2], "mm")?,
            ],
            plane: Datum::ALL
                .into_iter()
                .find(|d| d.frame().u == p.frame.u && d.frame().v == p.frame.v),
            editing,
            error: None,
            request,
        })
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        "Thread"
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<ThreadRequest, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.name.trim().is_empty() || self.name.len() > 1000 {
            return Err("Enter a name with 1–1000 bytes".into());
        }
        let mut request = self.request.clone();
        request.name = self.name.clone();
        request.mode = self.mode;
        if !self.editing {
            request.target = if self.mode == ThreadMode::NewExternal {
                None
            } else {
                Some(
                    self.choices
                        .iter()
                        .find(|choice| Some(choice.input.body) == self.target)
                        .ok_or("Choose a current receiving Part")?
                        .input
                        .clone(),
                )
            };
            request.body = request
                .target
                .as_ref()
                .map_or(self.request.body, |target| target.body);
        }
        request.parameters = self.recipe(document, self.mode)?;
        Ok(request)
    }
    pub(super) fn recipe(
        &self,
        document: &Document,
        mode: ThreadMode,
    ) -> Result<ThreadParameters, String> {
        let basis = self.plane.map(Datum::frame);
        let parameters = ThreadParameters {
            family: self.family,
            buttress: if self.family == ThreadFamily::Buttress {
                self.buttress
            } else {
                None
            },
            npt: if self.family == ThreadFamily::Npt {
                self.npt
            } else {
                None
            },
            pipe: if self.family == ThreadFamily::Bspt {
                self.pipe
            } else {
                None
            },
            custom: if self.family == ThreadFamily::Custom {
                Some(CustomThreadProfile {
                    depth: self.custom[0].resolve(document)?,
                    crest_width: self.custom[1].resolve(document)?,
                    lower_flank: self.custom[2].resolve(document)?,
                    upper_flank: self.custom[3].resolve(document)?,
                    radial_taper: self.custom[4].resolve(document)?,
                })
            } else {
                None
            },
            major_diameter: self.diameter.resolve(document)?,
            pitch: self.pitch.resolve(document)?,
            length: self.length.resolve(document)?,
            hand: self.hand,
            starts: self.starts,
            radial_allowance: self.radial.resolve(document)?,
            axial_allowance: self.axial.resolve(document)?,
            minimum_wall: self.wall.resolve(document)?,
            representation: self.representation,
            representation_parameter: self.representation_parameter,
            frame: PrimitiveFrame {
                origin: [
                    self.origin[0].resolve(document)?,
                    self.origin[1].resolve(document)?,
                    self.origin[2].resolve(document)?,
                ],
                u: basis.map_or(self.request.parameters.frame.u, |b| b.u),
                v: basis.map_or(self.request.parameters.frame.v, |b| b.v),
            },
            fit: self.fit,
            metric_fit_engagement: match self.engagement_basis {
                model::MetricFitEngagementBasis::LegacyCompleteProfile => None,
                model::MetricFitEngagementBasis::IntendedLength => {
                    Some(model::MetricFitEngagement::IntendedLength {
                        length: self.engagement_length.resolve(document)?,
                    })
                }
                model::MetricFitEngagementBasis::NormalGroupAssumed => {
                    Some(model::MetricFitEngagement::NormalGroupAssumed {})
                }
            },
            runout: self.request.parameters.runout,
            end_treatments: if self.end_treatments_enabled {
                Some(model::ThreadEndTreatments {
                    start: if self.start_chamfer {
                        model::ThreadStartTreatment::Chamfer45 {
                            radial_depth: self.start_depth.resolve(document)?,
                        }
                    } else {
                        model::ThreadStartTreatment::FlatCut
                    },
                    end: if self.end_relief {
                        model::ThreadEndTreatment::CylindricalRelief {
                            length: self.end_length.resolve(document)?,
                            radial_depth: self.end_depth.resolve(document)?,
                        }
                    } else {
                        model::ThreadEndTreatment::FlatCut
                    },
                })
            } else {
                None
            },
        };
        parameters
            .resolve(&self.parameters, mode)
            .map_err(|e| e.to_string())?;
        Ok(parameters)
    }
    pub(super) fn recipe_controls(
        &mut self,
        ui: &mut egui::Ui,
        mode: ThreadMode,
        length_label: &str,
        show_frame: bool,
    ) -> bool {
        let mut changed = false;
        ui.separator();
        ui.label("Profile family");
        controls::inspector_combo(ui, "thread-family")
            .selected_text(self.family.label())
            .show_ui(ui, |ui| {
                for family in ThreadFamily::ALL {
                    let response = ui
                        .add_enabled(
                            family.supported(),
                            egui::Button::selectable(self.family == family, family.label()),
                        )
                        .on_hover_text(family.source())
                        .on_disabled_hover_text(
                            "Not implemented; no standard-family geometry is claimed",
                        );
                    if response.clicked() && self.family != family {
                        changed = true;
                        self.family = family;
                        self.fit = Self::fit_for(family, mode);
                        if family != ThreadFamily::IsoMetric {
                            self.end_treatments_enabled = false;
                        }
                        if family == ThreadFamily::Buttress {
                            self.starts = 1;
                            if self.buttress.is_some() {
                                let row = model::buttress_thread_presets().remove(0);
                                self.diameter.set_exact_literal(row.diameter_mm, "mm");
                                self.pitch.set_exact_literal(row.pitch_mm, "mm");
                            }
                        }
                        if family == ThreadFamily::Npt
                            && let Some(mut npt) = self.npt
                        {
                            npt.form = if mode == ThreadMode::CutInternal {
                                model::NptThreadForm::Internal
                            } else {
                                model::NptThreadForm::External
                            };
                            self.npt = Some(npt);
                            self.diameter.set_exact_literal(npt.base_major_mm(), "mm");
                            self.pitch.set_exact_literal(npt.preset().pitch_mm, "mm");
                            self.starts = 1;
                            self.hand = ThreadHand::Right;
                        }
                        if family == ThreadFamily::Bspt
                            && let Some(mut pipe) = self.pipe
                        {
                            pipe.form = if mode == ThreadMode::CutInternal {
                                model::PipeThreadForm::Rc
                            } else {
                                model::PipeThreadForm::R
                            };
                            self.pipe = Some(pipe);
                            self.diameter.set_exact_literal(pipe.base_major_mm(), "mm");
                            self.pitch.set_exact_literal(pipe.preset().pitch_mm, "mm");
                            self.starts = 1;
                            self.hand = ThreadHand::Right;
                        }
                        if family.is_unified()
                            || family.is_rounded()
                            || family == ThreadFamily::Round
                        {
                            self.starts = 1;
                        }
                    }
                }
            });
        ui.small(self.family.source());
        if self.family == ThreadFamily::Buttress {
            changed |= self.buttress_controls(ui);
        }
        if self.family == ThreadFamily::Round {
            changed |= self.round_controls(ui);
        }
        if self.family.is_lead_screw() {
            changed |= self.lead_screw_controls(ui);
        }
        if self.family == ThreadFamily::Npt {
            changed |= self.npt_controls(ui, mode);
        }
        if self.family == ThreadFamily::Bspt {
            changed |= self.pipe_controls(ui, mode);
        }
        if self.family.is_rounded() {
            changed |= self.rounded_controls(ui, mode);
        }
        if self.family == ThreadFamily::IsoMetric {
            let mut preset = None;
            controls::inspector_combo(ui, "thread-preset")
                .selected_text("Choose a sourced metric preset…")
                .show_ui(ui, |ui| {
                    for p in model::metric_thread_presets() {
                        if ui.selectable_label(false, &p.designation).clicked() {
                            preset = Some(p);
                        }
                    }
                });
            if let Some(p) = preset {
                self.diameter = DimensionField::from_dimension(&model::length(p.diameter_mm), "mm")
                    .expect("finite preset");
                self.pitch = DimensionField::from_dimension(&model::length(p.pitch_mm), "mm")
                    .expect("finite preset");
                changed = true;
            }
            changed |= self.metric_fit_controls(ui, mode);
        }
        if self.family.is_unified() {
            let fit = Self::fit_for(self.family, mode);
            ui.label("Fit class");
            controls::inspector_combo(ui, "thread-fit")
                .selected_text(self.fit.label())
                .show_ui(ui, |ui| {
                    for class in [ThreadFit::Unified2A, ThreadFit::Unified2B] {
                        if ui
                            .add_enabled(
                                class == fit,
                                egui::Button::selectable(self.fit == class, class.label()),
                            )
                            .on_disabled_hover_text(
                                "Choose an external or internal thread mode for this class",
                            )
                            .clicked()
                        {
                            changed |= self.fit != class;
                            self.fit = class;
                        }
                    }
                });
            let mut selected = None;
            let presets: Vec<_> = model::unified_thread_presets()
                .into_iter()
                .filter(|p| p.family == self.family)
                .collect();
            controls::inspector_combo(ui, "thread-unified-preset")
                .selected_text(format!("Choose a sourced {} preset…", self.family.label()))
                .show_ui(ui, |ui| {
                    for p in &presets {
                        if ui
                            .selectable_label(false, &p.designation)
                            .on_hover_text(format!(
                                "{} in nominal · {} TPI · {:.9} mm pitch",
                                p.nominal_diameter_inch, p.threads_per_inch, p.pitch_mm
                            ))
                            .clicked()
                        {
                            selected = Some(p.clone());
                        }
                    }
                });
            if let Some(p) = selected {
                self.diameter
                    .set_exact_literal(p.nominal_diameter_inch, "in");
                self.pitch.set_exact_literal(p.pitch_mm, "mm");
                if self.preset_length_default {
                    self.length.set_exact_literal(3. * p.pitch_mm, "mm");
                }
                self.starts = 1;
                self.fit = fit;
                changed = true;
            }
            ui.small("Only listed 1/4 and 5/16 inch sizes, single-start uncoated UN flat roots. UNR and other classes are unavailable.");
            ui.small("Thread length must not exceed 1.5 times the nominal diameter. Physical fit and current ASME compliance are unverified.");
            ui.collapsing("Published size and fit limits", |ui| {
                for p in &presets {
                    ui.strong(format!("{} · {} TPI", p.designation, p.threads_per_inch));
                    let limits = if mode == ThreadMode::CutInternal {
                        &p.internal
                    } else {
                        &p.external
                    };
                    Self::show_limits(ui, limits);
                }
            });
        }
        changed |= self.diameter.show(
            ui,
            "Nominal major diameter",
            "Basic crest diameter at the base plane, before fabrication allowance",
        );
        changed |= self.pitch.show(
            ui,
            "Pitch",
            "Axial spacing of adjacent threads; lead = pitch × starts",
        );
        let length_changed = self.length.show(
            ui,
            length_label,
            "Complete receiving length from the base plane; at least one pitch",
        );
        self.preset_length_default &= !length_changed;
        changed |= length_changed;
        ui.horizontal(|ui| {
            ui.label("Starts");
            changed |= ui
                .add_enabled(
                    !(self.family == ThreadFamily::IsoMetric
                        && (self.fit != ThreadFit::NominalBasic || self.end_treatments_enabled))
                        && !self.family.is_unified()
                        && !self.family.is_rounded()
                        && !matches!(
                            self.family,
                            ThreadFamily::Bspt
                                | ThreadFamily::Npt
                                | ThreadFamily::Buttress
                                | ThreadFamily::Round
                        ),
                    egui::DragValue::new(&mut self.starts).range(1..=8),
                )
                .changed();
        });
        ui.horizontal(|ui| {
            for (hand, label) in [
                (ThreadHand::Right, "Right hand"),
                (ThreadHand::Left, "Left hand"),
            ] {
                ui.add_enabled_ui(
                    !matches!(self.family, ThreadFamily::Bspt | ThreadFamily::Npt)
                        || hand == ThreadHand::Right,
                    |ui| {
                        changed |= ui.radio_value(&mut self.hand, hand, label).changed();
                    },
                );
            }
        });
        if self.family == ThreadFamily::Custom {
            for (field, label) in self.custom.iter_mut().zip([
                "Radial thread depth",
                "Axial crest width",
                "Lower flank from radial (deg)",
                "Upper flank from radial (deg)",
                "Radius taper (mm/mm)",
            ]) {
                changed |= field.show(
                    ui,
                    label,
                    "Explicit custom axial profile; diameter taper is twice radius taper",
                );
            }
        }
        if matches!(
            self.fit,
            ThreadFit::Metric6gExternal | ThreadFit::Metric6HInternal
        ) || self.engagement_basis != model::MetricFitEngagementBasis::LegacyCompleteProfile
        {
            ui.strong("Metric fit engagement");
            for (basis, label) in [
                (
                    model::MetricFitEngagementBasis::LegacyCompleteProfile,
                    "Use complete physical thread",
                ),
                (
                    model::MetricFitEngagementBasis::IntendedLength,
                    "Declare intended engagement",
                ),
                (
                    model::MetricFitEngagementBasis::NormalGroupAssumed,
                    "Assume normal group; mate unknown",
                ),
            ] {
                changed |= ui
                    .radio_value(&mut self.engagement_basis, basis, label)
                    .changed();
            }
            if self.engagement_basis == model::MetricFitEngagementBasis::IntendedLength {
                changed |= self.engagement_length.show(
                    ui,
                    "Intended engagement",
                    "Mating length inside the complete thread profile; does not shorten the screw",
                );
            }
            ui.small("Fit intent does not verify assembled engagement, strength or blind-hole clearance.");
        }
        ui.separator();
        ui.strong("Fabrication allowance");
        ui.small("Separate from the nominal basic profile. Positive clearance shrinks external material or enlarges an internal cut.");
        changed |= self.radial.show(
            ui,
            "Radial clearance",
            "Millimetres on radius; not an ISO tolerance class",
        );
        changed |= self.axial.show(
            ui,
            "Axial clearance",
            "Total tooth-width allowance in millimetres",
        );
        if self.family.is_rounded() || self.family == ThreadFamily::Bspt {
            ui.small("Rounded profiles currently require zero axial clearance; radial clearance must remain within the selected fit limits.");
        }
        if self.family == ThreadFamily::Npt {
            ui.small(
                "NPT requires zero axial clearance and radial clearance from zero to pitch / 32.",
            );
        }
        if mode == ThreadMode::CutInternal {
            changed |= self.wall.show(
                ui,
                "Minimum receiving wall",
                "Required material around the complete thread, beyond the major radius",
            );
        }
        changed |= self.representation_controls(ui);
        if show_frame && self.family == ThreadFamily::IsoMetric {
            changed |= ui
                .checkbox(
                    &mut self.end_treatments_enabled,
                    "Custom entry and end relief",
                )
                .changed();
            if self.end_treatments_enabled {
                self.starts = 1;
                changed |= ui
                    .checkbox(&mut self.start_chamfer, "45° entry chamfer at z = 0")
                    .changed();
                if self.start_chamfer {
                    changed |= self.start_depth.show(
                        ui,
                        "Entry radial depth",
                        "Authored millimetres on radius; axial extent equals radial depth",
                    );
                }
                changed |= ui
                    .checkbox(&mut self.end_relief, "Cylindrical relief at z = length")
                    .changed();
                if self.end_relief {
                    changed |= self.end_length.show(
                        ui,
                        "Relief axial length",
                        "Consumes the end of the authored length",
                    );
                    changed |= self.end_depth.show(
                        ui,
                        "Relief radial depth",
                        "Beyond the external root or internal major radius",
                    );
                }
                ui.small("Custom geometry inside unchanged length. Retains at least one full pitch. Not DIN 76 shoulder relief or natural runout. Modeled geometry required.");
            }
        }
        if show_frame {
            ui.separator();
            ui.label("Axis plane");
            controls::inspector_combo(ui, "thread-plane")
                .selected_text(self.plane.map_or("Stored orientation", Datum::label))
                .show_ui(ui, |ui| {
                    for datum in Datum::ALL {
                        changed |= ui
                            .selectable_value(&mut self.plane, Some(datum), datum.label())
                            .changed();
                    }
                });
            ui.small("The thread axis follows the plane normal. Origin is the base center in project coordinates.");
            for (field, label) in
                self.origin
                    .iter_mut()
                    .zip(["Base center X", "Base center Y", "Base center Z"])
            {
                changed |= field.show(
                    ui,
                    label,
                    "Explicit axis placement; formulas may reference $parameters",
                );
            }
        }
        changed
    }
    fn representation_controls(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.separator();
        ui.strong("Representation");
        let effective = ThreadParameters {
            representation: self.representation,
            representation_parameter: self.representation_parameter,
            ..Default::default()
        }
        .resolved_representation(&self.parameters);
        if self.representation_parameter.is_some() {
            ui.add(egui::Label::new("Thread representation follows a parameter; edit its value in Project or the Library instance").wrap());
            match &effective {
                Ok(value) => {
                    ui.label(format!("Effective representation: {}", value.label()));
                }
                Err(error) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, error.to_string());
                }
            }
            if ui.button("Use fixed thread representation").clicked() {
                if let Ok(value) = &effective {
                    self.representation = *value;
                }
                self.representation_parameter = None;
                changed = true;
            }
        }
        if self.representation_parameter.is_none() {
            for representation in [
                ThreadRepresentation::Modeled,
                ThreadRepresentation::CosmeticEnvelope,
            ] {
                changed |= ui
                    .radio_value(
                        &mut self.representation,
                        representation,
                        representation.label(),
                    )
                    .changed();
            }
        }
        let cosmetic = if self.representation_parameter.is_some() {
            matches!(effective, Ok(ThreadRepresentation::CosmeticEnvelope))
        } else {
            self.representation == ThreadRepresentation::CosmeticEnvelope
        };
        if cosmetic {
            ui.colored_label(
                egui::Color32::YELLOW,
                "Display only. Switch to Modeled before STEP, mesh or print export.",
            );
        }
        changed
    }
    fn buttress_controls(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        let mut selected = None;
        controls::inspector_combo(ui, "thread-buttress-preset")
            .selected_text("Choose sourced ST size…")
            .show_ui(ui, |ui| {
                for row in model::buttress_thread_presets() {
                    if ui.selectable_label(false, row.designation).clicked() {
                        selected = Some(row);
                    }
                }
            });
        if let Some(row) = selected {
            let side = self
                .buttress
                .map_or(model::ButtressLoadFlankSide::NegativeAxisSide, |b| {
                    b.load_flank_side
                });
            self.buttress = Some(model::ButtressIntent {
                series: row.series,
                load_flank_side: side,
            });
            self.diameter.set_exact_literal(row.diameter_mm, "mm");
            self.pitch.set_exact_literal(row.pitch_mm, "mm");
            if self.preset_length_default {
                self.length.set_exact_literal(3. * row.pitch_mm, "mm");
            }
            self.starts = 1;
            self.fit = ThreadFit::NominalBasic;
            changed = true;
        }
        if let Some(intent) = &mut self.buttress {
            ui.label("3° load flank placement");
            for (side, label) in [
                (
                    model::ButtressLoadFlankSide::NegativeAxisSide,
                    "Negative local axis side",
                ),
                (
                    model::ButtressLoadFlankSide::PositiveAxisSide,
                    "Positive local axis side",
                ),
            ] {
                changed |= ui
                    .radio_value(&mut intent.load_flank_side, side, label)
                    .changed();
            }
        }
        ui.small("The side names the external tooth's 3° face, independently of RH/LH. It does not name applied force; the mating material normal reverses.");
        ui.small("BIS 2004 nominal ST20×4 only, one start. External roots are circular; the internal cutter is flat. Axial fabrication allowance is zero. No default 7e/7H fit class is claimed.");
        changed
    }
    fn round_controls(&mut self, ui: &mut egui::Ui) -> bool {
        let mut selected = None;
        controls::inspector_combo(ui, "thread-round-preset")
            .selected_text("Choose a sourced Rd preset…")
            .show_ui(ui, |ui| {
                for row in model::round_thread_presets() {
                    if ui.selectable_label(false, row.designation).clicked() {
                        selected = Some(row);
                    }
                }
            });
        if let Some(row) = &selected {
            self.diameter.set_exact_literal(row.diameter_mm, "mm");
            self.pitch
                .set_exact_literal(1. / f64::from(row.threads_per_inch), "in");
            if self.preset_length_default {
                self.length.set_exact_literal(3. * row.pitch_mm, "mm");
            }
            self.fit = ThreadFit::NominalBasic;
            self.starts = 1;
        }
        ui.small("Metric nominal diameter with reciprocal-inch pitch. True 30° circular crest/root profiles; external and internal threads use different radii.");
        ui.small("Nominal source contour and radial crest clearance; RH/LH, one start, zero axial fabrication allowance. No fit-class or physical-fit certification.");
        selected.is_some()
    }
    fn lead_screw_controls(&mut self, ui: &mut egui::Ui) -> bool {
        let mut selected = None;
        controls::inspector_combo(ui, "thread-lead-screw-preset")
            .selected_text(format!("Choose a sourced {} preset…", self.family.label()))
            .show_ui(ui, |ui| {
                for row in model::lead_screw_presets()
                    .into_iter()
                    .filter(|p| p.family == self.family)
                {
                    if ui.selectable_label(false, row.designation).clicked() {
                        selected = Some(row);
                    }
                }
            });
        if let Some(row) = &selected {
            if let Some(inches) = row.nominal_diameter_inch {
                self.diameter.set_exact_literal(inches, "in");
            } else {
                self.diameter.set_exact_literal(row.diameter_mm, "mm");
            }
            if let Some(tpi) = row.threads_per_inch {
                self.pitch.set_exact_literal(1. / f64::from(tpi), "in");
            } else {
                self.pitch.set_exact_literal(row.pitch_mm, "mm");
            }
            if self.preset_length_default {
                self.length.set_exact_literal(3. * row.pitch_mm, "mm");
            }
            self.fit = ThreadFit::NominalBasic;
        }
        ui.small("Nominal sharp limiting profiles with separate source crest/root clearance. Equal basic pitch diameters have zero theoretical flank allowance; fabrication clearance is separate.");
        ui.small("RH/LH, 1–8 starts are bounded engineering geometry. Pitch is adjacent thread spacing; lead is pitch × starts. No fit-class or physical-fit certification.");
        selected.is_some()
    }
    fn rounded_controls(&mut self, ui: &mut egui::Ui, mode: ThreadMode) -> bool {
        let mut changed = false;
        let fit = Self::fit_for(self.family, mode);
        let classes = if self.family == ThreadFamily::Bspp {
            [ThreadFit::BsppClassBExternal, ThreadFit::BsppInternal]
        } else {
            [
                ThreadFit::WhitworthMediumExternal,
                ThreadFit::WhitworthMediumInternal,
            ]
        };
        ui.label("Fit class");
        controls::inspector_combo(ui, "thread-rounded-fit")
            .selected_text(self.fit.label())
            .show_ui(ui, |ui| {
                for class in classes {
                    if ui
                        .add_enabled(
                            class == fit,
                            egui::Button::selectable(self.fit == class, class.label()),
                        )
                        .on_disabled_hover_text(
                            "Choose the corresponding external or internal thread mode",
                        )
                        .clicked()
                    {
                        changed |= self.fit != class;
                        self.fit = class;
                    }
                }
            });
        let presets: Vec<_> = model::whitworth_thread_presets()
            .into_iter()
            .filter(|p| p.family == self.family)
            .collect();
        let mut selected = None;
        controls::inspector_combo(ui, "thread-rounded-preset")
            .selected_text(format!("Choose a sourced {} preset…", self.family.label()))
            .show_ui(ui, |ui| {
                for p in &presets {
                    if ui
                        .selectable_label(false, &p.designation)
                        .on_hover_text(format!(
                            "Physical major {:.6} mm · {} TPI · {:.9} mm pitch",
                            p.diameter_mm, p.threads_per_inch, p.pitch_mm
                        ))
                        .clicked()
                    {
                        selected = Some(p.clone());
                    }
                }
            });
        if let Some(p) = selected {
            if let Some(inches) = p.nominal_diameter_inch {
                self.diameter.set_exact_literal(inches, "in");
            } else {
                self.diameter.set_exact_literal(p.diameter_mm, "mm");
            }
            self.pitch.set_exact_literal(p.pitch_mm, "mm");
            if self.preset_length_default {
                self.length.set_exact_literal(3. * p.pitch_mm, "mm");
            }
            self.starts = 1;
            self.fit = fit;
            changed = true;
        }
        ui.small("55° profile with circular crests and roots. Listed sizes support one start and zero axial relief.");
        if self.family == ThreadFamily::Bspp {
            ui.small("G pipe-size labels are nominal; use the listed physical major diameter. A BSPP joint seals outside the thread.");
        } else {
            ui.small("Uncoated Medium limits from BS 84:2007; supported engagement is at most 15 pitches.");
        }
        ui.collapsing("Published size and fit limits", |ui| {
            for p in &presets {
                ui.strong(format!("{} · {} TPI", p.designation, p.threads_per_inch));
                Self::show_limits(
                    ui,
                    if mode == ThreadMode::CutInternal {
                        &p.internal
                    } else {
                        &p.external
                    },
                );
            }
        });
        changed
    }
    fn fit_for(family: ThreadFamily, mode: ThreadMode) -> ThreadFit {
        if family == ThreadFamily::Npt {
            ThreadFit::NptHistoricalMidpoint
        } else if family == ThreadFamily::Bspt {
            ThreadFit::BsptNominal
        } else if matches!(family, ThreadFamily::Bsw | ThreadFamily::Bsf) {
            if mode == ThreadMode::CutInternal {
                ThreadFit::WhitworthMediumInternal
            } else {
                ThreadFit::WhitworthMediumExternal
            }
        } else if family == ThreadFamily::Bspp {
            if mode == ThreadMode::CutInternal {
                ThreadFit::BsppInternal
            } else {
                ThreadFit::BsppClassBExternal
            }
        } else if !family.is_unified() {
            ThreadFit::NominalBasic
        } else if mode == ThreadMode::CutInternal {
            ThreadFit::Unified2B
        } else {
            ThreadFit::Unified2A
        }
    }
    fn npt_controls(&mut self, ui: &mut egui::Ui, mode: ThreadMode) -> bool {
        let form = if mode == ThreadMode::CutInternal {
            model::NptThreadForm::Internal
        } else {
            model::NptThreadForm::External
        };
        let mut selected = None;
        controls::inspector_combo(ui, "thread-npt-preset")
            .selected_text(self.npt.map_or("Choose sourced NPT size…".into(), |p| {
                format!("NPT {}", p.preset().size_label)
            }))
            .show_ui(ui, |ui| {
                for row in model::npt_thread_presets() {
                    if ui
                        .selectable_label(
                            self.npt
                                .is_some_and(|p| p.size == row.size && p.form == form),
                            format!("NPT {} · {} TPI", row.size_label, row.threads_per_inch),
                        )
                        .clicked()
                    {
                        selected = Some(model::NptThreadIntent {
                            form,
                            size: row.size,
                            gauge: model::NptGaugeConvention::Nbs1959PlainFace,
                            truncation: model::NptTruncationTarget::MidpointConeParallel,
                        });
                    }
                }
            });
        if let Some(npt) = selected {
            self.npt = Some(npt);
            self.diameter.set_exact_literal(npt.base_major_mm(), "mm");
            self.pitch.set_exact_literal(npt.preset().pitch_mm, "mm");
            if self.preset_length_default {
                self.length.set_exact_literal(npt.default_length_mm(), "mm");
            }
            self.fit = ThreadFit::NptHistoricalMidpoint;
            self.starts = 1;
            self.hand = ThreadHand::Right;
        }
        if let Some(npt) = self.npt {
            ui.small(format!("Gauge {:.6} mm inward · radius slope {:+.5} · nominal gauge pitch diameter {:.6} mm", npt.gauge_offset_mm(),npt.radial_taper(),npt.preset().published.e1_inch*25.4));
            ui.small(if form==model::NptThreadForm::External {"External length is sourced L2; crests are clipped to nominal pipe stock OD."}else{"Internal depth includes three wrench turns plus positive gauge travel; the default adds one pitch of gauge travel."});
        }
        ui.small("Historical NBS 1959 dimensions; midpoint cone-parallel truncations. RH, one start, 1/8 and 1/4 only. NPTF and vanish/runout are unavailable.");
        selected.is_some()
    }
    fn pipe_controls(&mut self, ui: &mut egui::Ui, mode: ThreadMode) -> bool {
        let form = if mode == ThreadMode::CutInternal {
            model::PipeThreadForm::Rc
        } else {
            model::PipeThreadForm::R
        };
        let mut selected = None;
        controls::inspector_combo(ui, "thread-pipe-preset")
            .selected_text(self.pipe.map_or("Choose sourced R/Rc size…".into(), |p| {
                format!(
                    "{} {}",
                    if p.form == model::PipeThreadForm::R {
                        "R"
                    } else {
                        "Rc"
                    },
                    p.preset().size_label
                )
            }))
            .show_ui(ui, |ui| {
                for row in model::pipe_thread_presets() {
                    if ui
                        .selectable_label(
                            self.pipe
                                .is_some_and(|p| p.size == row.size && p.form == form),
                            format!(
                                "{} {} · {} TPI",
                                if form == model::PipeThreadForm::R {
                                    "R"
                                } else {
                                    "Rc"
                                },
                                row.size_label,
                                row.threads_per_inch
                            ),
                        )
                        .clicked()
                    {
                        selected = Some(model::PipeThreadIntent {
                            form,
                            size: row.size,
                            gauge: model::PipeGaugeConvention::Iso7Nominal1994,
                        });
                    }
                }
            });
        if let Some(pipe) = selected {
            self.pipe = Some(pipe);
            self.diameter.set_exact_literal(pipe.base_major_mm(), "mm");
            self.pitch.set_exact_literal(pipe.preset().pitch_mm, "mm");
            if self.preset_length_default {
                self.length
                    .set_exact_literal(pipe.default_length_mm(), "mm");
            }
            self.fit = ThreadFit::BsptNominal;
            self.starts = 1;
            self.hand = ThreadHand::Right;
        }
        if let Some(pipe) = self.pipe {
            ui.small(format!(
                "{} reference · gauge {:.6} mm inward · radius slope {:+.5} · gauge major {:.3} mm",
                if pipe.form == model::PipeThreadForm::R {
                    "Small end"
                } else {
                    "Fitting face"
                },
                pipe.gauge_offset_mm(),
                pipe.radial_taper(),
                pipe.preset().gauge_major_mm
            ));
            ui.small(format!("Minimum complete/useful length {:.6} mm. Nominal gauge; position tolerance ±{} turns. Flat ends, no washout.", pipe.default_length_mm(), if pipe.form == model::PipeThreadForm::R { 1. } else { 1.25 }));
        }
        ui.small("Right-hand R/Rc 1/8 and 1/4 only; Rp unavailable. Suitable jointing medium is required; geometry does not certify sealing or pressure rating.");
        selected.is_some()
    }
    fn metric_fit_controls(&mut self, ui: &mut egui::Ui, mode: ThreadMode) -> bool {
        let mut changed = false;
        let mode_fit = if mode == ThreadMode::CutInternal {
            ThreadFit::Metric6HInternal
        } else {
            ThreadFit::Metric6gExternal
        };
        ui.label("Fit class");
        controls::inspector_combo(ui, "thread-metric-fit")
            .selected_text(self.fit.label())
            .show_ui(ui, |ui| {
                for class in [
                    ThreadFit::NominalBasic,
                    ThreadFit::Metric6gExternal,
                    ThreadFit::Metric6HInternal,
                ] {
                    if ui
                        .add_enabled(
                            class == ThreadFit::NominalBasic || class == mode_fit,
                            egui::Button::selectable(self.fit == class, class.label()),
                        )
                        .on_disabled_hover_text("Choose the matching external/internal mode")
                        .clicked()
                    {
                        changed |= self.fit != class;
                        self.fit = class;
                        if class != ThreadFit::NominalBasic {
                            self.starts = 1;
                        }
                    }
                }
            });
        let mut selected = None;
        controls::inspector_combo(ui, "thread-metric-fit-preset")
            .selected_text("Choose a sourced M3/M6 fit preset…")
            .show_ui(ui, |ui| {
                for p in model::metric_fit_presets() {
                    if ui
                        .selectable_label(
                            false,
                            format!("{} · {}", p.designation, mode_fit.label()),
                        )
                        .clicked()
                    {
                        selected = Some(p);
                    }
                }
            });
        if let Some(p) = selected {
            self.diameter.set_exact_literal(p.diameter_mm, "mm");
            self.pitch.set_exact_literal(p.pitch_mm, "mm");
            self.fit = mode_fit;
            self.starts = 1;
            if self.preset_length_default {
                self.length.set_exact_literal(p.diameter_mm, "mm");
            }
            changed = true;
        }
        ui.small("6g/6H: historical 2001/1998 source, M3×0.5 and M6×1 only, one start. Other metric sizes retain Nominal basic. Fabrication allowance cannot change the selected class or its limits.");
        ui.collapsing("Published metric fit limits", |ui| {
            for p in model::metric_fit_presets() {
                ui.strong(p.designation);
                ui.small(format!(
                    "Complete profile engagement > {} and ≤ {} mm",
                    p.engagement_over_mm, p.engagement_up_to_mm
                ));
                Self::show_limits(
                    ui,
                    &if mode == ThreadMode::CutInternal {
                        p.internal
                    } else {
                        p.external
                    },
                );
            }
        });
        changed
    }
    fn show_limits(ui: &mut egui::Ui, limits: &model::ThreadFitLimits) {
        for (label, range) in [
            ("Major", &limits.major),
            ("Pitch", &limits.pitch),
            ("Minor", &limits.minor),
        ] {
            let text = match (range.min_mm, range.max_mm) {
                (Some(min), Some(max)) => format!("{min:.6} to {max:.6} mm"),
                (Some(min), None) => format!("at least {min:.6} mm"),
                (None, Some(max)) => format!("at most {max:.6} mm"),
                (None, None) => "No published bound in this table".into(),
            };
            ui.small(format!("{label} diameter: {text}"));
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
        ui.strong("Thread");
        ui.small("Preview checks the complete native solid before one undoable Apply. End geometry is explicit; no automatic runout.");
        ui.label(if self.mode == ThreadMode::NewExternal {
            "Part name"
        } else {
            "Feature name"
        });
        let mut changed = ui
            .add_enabled(
                !self.editing,
                egui::TextEdit::singleline(&mut self.name).desired_width(ui.available_width()),
            )
            .changed();
        let previous_mode = self.mode;
        ui.add_enabled_ui(!self.editing, |ui| {
            controls::inspector_combo(ui, "thread-mode")
                .selected_text(match self.mode {
                    ThreadMode::NewExternal => "New threaded shaft",
                    ThreadMode::CutExternal => "Cut external thread",
                    ThreadMode::CutInternal => "Cut internal thread",
                })
                .show_ui(ui, |ui| {
                    for (mode, label) in [
                        (ThreadMode::NewExternal, "New threaded shaft"),
                        (ThreadMode::CutExternal, "Cut external thread"),
                        (ThreadMode::CutInternal, "Cut internal thread"),
                    ] {
                        changed |= ui.selectable_value(&mut self.mode, mode, label).changed();
                    }
                });
            if self.mode != ThreadMode::NewExternal {
                ui.label("Receiving Part");
                let selected = self
                    .choices
                    .iter()
                    .find(|c| Some(c.input.body) == self.target)
                    .map_or("Choose…", |c| c.label.as_str());
                controls::inspector_combo(ui, "thread-target")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for choice in &self.choices {
                            changed |= ui
                                .selectable_value(
                                    &mut self.target,
                                    Some(choice.input.body),
                                    &choice.label,
                                )
                                .on_hover_text(&choice.path)
                                .changed();
                        }
                    });
                if self.choices.is_empty() {
                    ui.small(
                        "Wait for current native bodies, then reopen Thread to choose a receiver.",
                    );
                }
            }
        });
        if previous_mode != self.mode && (self.family.is_unified() || self.family.is_rounded()) {
            self.fit = Self::fit_for(self.family, self.mode);
        }
        if previous_mode != self.mode
            && self.family == ThreadFamily::IsoMetric
            && self.fit != ThreadFit::NominalBasic
        {
            self.fit = if self.mode == ThreadMode::CutInternal {
                ThreadFit::Metric6HInternal
            } else {
                ThreadFit::Metric6gExternal
            };
        }
        if previous_mode != self.mode
            && self.family == ThreadFamily::Bspt
            && let Some(mut pipe) = self.pipe
        {
            pipe.form = if self.mode == ThreadMode::CutInternal {
                model::PipeThreadForm::Rc
            } else {
                model::PipeThreadForm::R
            };
            self.pipe = Some(pipe);
            self.diameter.set_exact_literal(pipe.base_major_mm(), "mm");
            self.fit = ThreadFit::BsptNominal;
        }
        if previous_mode != self.mode
            && self.family == ThreadFamily::Npt
            && let Some(mut npt) = self.npt
        {
            npt.form = if self.mode == ThreadMode::CutInternal {
                model::NptThreadForm::Internal
            } else {
                model::NptThreadForm::External
            };
            self.npt = Some(npt);
            self.diameter.set_exact_literal(npt.base_major_mm(), "mm");
            if self.preset_length_default {
                self.length.set_exact_literal(npt.default_length_mm(), "mm");
            }
            self.fit = ThreadFit::NptHistoricalMidpoint;
        }
        if self.editing
            && let Some(target) = &self.request.target
        {
            ui.small(format!(
                "Receiving Part: {} · original predecessor retained",
                document
                    .get(target.body)
                    .map_or("Missing Part", |e| e.name.as_str())
            ));
        }
        changed |= self.recipe_controls(ui, self.mode, "Thread length", true);
        let validation = self.request(document, enabled);
        match &validation {
            Ok(request) => {
                if let Ok(resolved) = request.parameters.resolve(&self.parameters, request.mode) {
                    ui.small(format!(
                        "Pitch {:.4} mm · lead {:.4} mm · {} starts",
                        resolved.pitch_mm, resolved.lead_mm, resolved.starts
                    ));
                    if let Some(round) = &resolved.round {
                        ui.strong(&round.designation);
                        ui.small(format!("Thread crest R {:.6} mm · root R {:.6} mm · source radial crest clearance {:.6} mm",round.material_crest_radius_mm,round.material_root_radius_mm,round.source_radial_crest_clearance_mm));
                        ui.small(round.scope);
                    }
                    if let Some(buttress) = &resolved.buttress {
                        ui.strong(&buttress.designation);
                        ui.small(format!(
                            "Engagement height {:.6} mm · actual profile depth {:.6} mm",
                            buttress.nominal_engagement_height_mm, buttress.depth_mm
                        ));
                        if let Some(circle) = &buttress.root_circle {
                            ui.small(format!(
                                "Tangent external root circle · radius {:.6} mm",
                                circle.radius_mm
                            ));
                        } else {
                            ui.small(format!("Flat internal cutter · source relief-side axial clearance {:.6} mm · crest phase {:+.6} mm",buttress.axial_relief_clearance_mm,buttress.crest_phase_axial_mm));
                        }
                    }
                    if let Some(lead) = &resolved.lead_screw {
                        ui.strong(&lead.designation);
                        ui.small(format!(
                            "{}° axial form · source radial crest/root clearance {:.6} mm",
                            lead.preset.included_angle_deg, lead.preset.basic_radial_clearance_mm
                        ));
                        ui.small(format!("Basic pitch diameter {:.6} mm · construction major {:.6} mm · minor {:.6} mm",lead.nominal_pitch_diameter_mm,lead.nominal_construction_major_diameter_mm,lead.nominal_construction_minor_diameter_mm));
                        if let Some(tpi) = lead.preset.threads_per_inch {
                            ui.small(format!("{tpi} adjacent threads per inch"));
                        }
                    }
                    if let Some(npt) = &resolved.npt {
                        ui.strong(&npt.designation);
                        ui.small(format!("Pitch diameter at nominal gauge plane {:.6} mm · gauge shift {:.6} turns",npt.actual_pitch_at_nominal_gauge_mm,npt.gauge_displacement_turns));
                        if let Some(z) = npt.actual_stock_intersection_mm {
                            ui.small(format!(
                                "Actual stock intersection {:.6} mm · source basic L5 {:.6} mm",
                                z, npt.source_basic_complete_length_mm
                            ));
                        }
                    }
                    if let Some(pipe) = &resolved.pipe {
                        ui.strong(&pipe.designation);
                        ui.small(format!(
                            "At nominal gauge plane: major {:.6} mm · pitch {:.6} mm · minor {:.6} mm",
                            pipe.actual_gauge_major_mm,
                            pipe.actual_gauge_pitch_mm,
                            pipe.actual_gauge_minor_mm
                        ));
                        ui.small(format!("Allowance shifts nominal gauge position {:.6} turns; this is construction intent, not a measured gage result.", pipe.gauge_displacement_turns));
                    }
                    if let Some(fit) = &resolved.fit {
                        ui.strong(format!("{} · {}", fit.designation, fit.class.label()));
                        if self.family == ThreadFamily::IsoMetric {
                            ui.small(format!(
                                "Nominal {:.6} mm · pitch {:.6} mm",
                                2. * resolved.nominal_major_radius_mm,
                                resolved.pitch_mm
                            ));
                        } else if self.family == ThreadFamily::Bspp {
                            ui.small(format!(
                                "Basic physical major {:.6} mm · {:.0} TPI",
                                2. * resolved.nominal_major_radius_mm,
                                25.4 / resolved.pitch_mm
                            ));
                        } else {
                            ui.small(format!(
                                "Nominal {:.4} in · {:.0} TPI",
                                2. * resolved.nominal_major_radius_mm / 25.4,
                                25.4 / resolved.pitch_mm
                            ));
                        }
                        ui.small(format!(
                            "Resolved major {:.6} mm · pitch {:.6} mm · minor {:.6} mm",
                            fit.major_diameter_mm, fit.pitch_diameter_mm, fit.minor_diameter_mm
                        ));
                        if let Some(limit) = fit.engagement_limit_mm {
                            ui.small(format!("Maximum sourced engagement length {limit:.6} mm"));
                        } else {
                            ui.small("The source table specifies no engagement-length bound; native construction limits still apply.");
                        }
                        if let Some(engagement) = &resolved.metric_fit_engagement {
                            ui.small(format!(
                                "Physical thread {:.4} mm · complete profile {:.4} to {:.4} mm",
                                resolved.length_mm,
                                engagement.full_profile_start_mm,
                                engagement.full_profile_end_mm
                            ));
                            match engagement.basis {
                                model::MetricFitEngagementBasis::NormalGroupAssumed => {
                                    ui.small("Normal group assumed; actual mating engagement is unknown.");
                                }
                                model::MetricFitEngagementBasis::IntendedLength => {
                                    ui.small(format!("Intended engagement {:.4} mm; assembled overlap not yet verified.", engagement.intended_length_mm.expect("intended receipt")));
                                }
                                model::MetricFitEngagementBasis::LegacyCompleteProfile => {
                                    ui.small("Fit group uses the complete physical profile (legacy source).");
                                }
                            }
                        }
                        if let Some(radius) = resolved.profile.rounding_radius_mm {
                            ui.small(format!("Rounded crest and root · radius {radius:.6} mm · 55° included angle"));
                        }
                        if let Some(metric) = &resolved.metric_fit
                            && let Some(radius) = metric.external_root_radius_mm
                        {
                            ui.small(format!(
                                "Tangent external root radius {radius:.6} mm · 60° included angle"
                            ));
                        }
                        Self::show_limits(ui, &fit.limits);
                        if let Some(reference) = fit.external_minor_reference_mm {
                            ui.small(format!("Published external minor reference {reference:.6} mm; not a fit limit"));
                        }
                    }
                }
            }
            Err(error) => {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
        }
        form::preview_status(ui, self.stamp, preview);
        let action = form::actions(ui, validation.is_ok(), changed, self.stamp, preview);
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
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
                Ok(request) => Some(ModelEvent::Preview {
                    stamp: self.stamp,
                    request: ModelRequest::Thread(Box::new(request)),
                }),
                Err(error) => {
                    self.error = Some(error);
                    None
                }
            },
            Some(FormAction::Apply) if preview.ready(self.stamp) && validation.is_ok() => {
                Some(ModelEvent::Apply)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod representation_binding_tests {
    use super::*;
    use multicad_core::{
        document::{Entity, EntityData},
        parameters,
    };

    #[test]
    fn painted_engagement_choice_changes_the_canonical_recipe_without_applying() {
        let document = Document::new("Painted engagement").unwrap();
        let before = document.clone();
        let mut panel = ThreadPanel::new(
            ThreadRequest::new(
                "Long metric",
                ThreadParameters {
                    major_diameter: model::length(3.),
                    pitch: model::length(0.5),
                    length: model::length(8.),
                    fit: ThreadFit::Metric6gExternal,
                    metric_fit_engagement: Some(model::MetricFitEngagement::NormalGroupAssumed {}),
                    ..Default::default()
                },
            ),
            parameters::evaluate(&document).unwrap(),
            vec![],
            &Default::default(),
            true,
        )
        .unwrap();
        panel.engagement_length.set_literal(3., "mm");
        let context = egui::Context::default();
        let mut frame = |events| {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600., 2400.),
                    )),
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ui| {
                    panel.recipe_controls(
                        ui,
                        ThreadMode::NewExternal,
                        "Physical thread length",
                        true,
                    );
                },
            );
            output.textures_delta.clear();
            output
        };
        let output = frame(vec![]);
        let position = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "Declare intended engagement" => {
                    Some(t.pos + t.galley.size() / 2.)
                }
                _ => None,
            })
            .expect("visible engagement choice");
        for pressed in [true, false] {
            frame(vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]);
        }
        assert_eq!(
            panel
                .request(&document, true)
                .unwrap()
                .parameters
                .metric_fit_engagement,
            Some(model::MetricFitEngagement::IntendedLength {
                length: model::length(3.)
            })
        );
        assert_eq!(document, before);
    }

    #[test]
    fn engagement_edit_retains_intent_and_can_restore_legacy_validation() {
        let document = Document::new("Engagement form").unwrap();
        for intent in [
            model::MetricFitEngagement::IntendedLength {
                length: model::length(3.),
            },
            model::MetricFitEngagement::NormalGroupAssumed {},
        ] {
            let source = ThreadParameters {
                major_diameter: model::length(3.),
                pitch: model::length(0.5),
                length: model::length(8.),
                fit: ThreadFit::Metric6gExternal,
                metric_fit_engagement: Some(intent.clone()),
                ..Default::default()
            };
            let mut panel = ThreadPanel::new(
                ThreadRequest::new("Long screw", source),
                parameters::evaluate(&document).unwrap(),
                vec![],
                &Default::default(),
                true,
            )
            .unwrap();
            panel.length.set_literal(9., "mm");
            let edited = panel.request(&document, true).unwrap();
            assert_eq!(edited.parameters.metric_fit_engagement, Some(intent));
            panel.engagement_basis = model::MetricFitEngagementBasis::LegacyCompleteProfile;
            assert!(panel.request(&document, true).is_err());
            panel.length.set_literal(4., "mm");
            assert_eq!(
                panel
                    .request(&document, true)
                    .unwrap()
                    .parameters
                    .metric_fit_engagement,
                None
            );
        }
    }

    #[test]
    fn bound_representation_is_retained_and_fixed_action_adopts_effective_mode() {
        let mut document = Document::new("Thread choice form").unwrap();
        let parameter = Entity::new(
            "thread_mode",
            EntityData::Parameter {
                expression: "0".into(),
                unit: "count".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        let binding = parameter.id;
        document.insert(parameter).unwrap();
        let before = document.clone();
        let recipe = ThreadParameters {
            representation: ThreadRepresentation::Modeled,
            representation_parameter: Some(binding),
            ..Default::default()
        };
        let request = ThreadRequest::new("Bound thread", recipe.clone());
        let feature = request.feature;
        let mut panel = ThreadPanel::new(
            request,
            parameters::evaluate(&document).unwrap(),
            vec![],
            &Default::default(),
            true,
        )
        .unwrap();
        panel.length.set_literal(8., "mm");
        let edited = panel.request(&document, true).unwrap();
        assert_eq!(edited.feature, feature);
        assert_eq!(edited.parameters.representation_parameter, Some(binding));
        assert_eq!(edited.parameters.representation, recipe.representation);
        assert_eq!(
            edited
                .parameters
                .resolved_representation(&panel.parameters)
                .unwrap(),
            ThreadRepresentation::CosmeticEnvelope
        );
        let context = egui::Context::default();
        fn frame(
            context: &egui::Context,
            panel: &mut ThreadPanel,
            events: Vec<egui::Event>,
        ) -> egui::FullOutput {
            context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(500., 500.),
                    )),
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ui| {
                    panel.representation_controls(ui);
                },
            )
        }
        let output = frame(&context, &mut panel, vec![]);
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        assert!(
            texts
                .iter()
                .any(|t| t.contains("Thread representation follows a parameter"))
        );
        assert!(texts.iter().any(|t| t
            == &format!(
                "Effective representation: {}",
                ThreadRepresentation::CosmeticEnvelope.label()
            )));
        assert!(
            !texts
                .iter()
                .any(|t| t == ThreadRepresentation::Modeled.label())
        );
        assert!(
            !texts
                .iter()
                .any(|t| t == ThreadRepresentation::CosmeticEnvelope.label())
        );
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text)
                    if text.galley.text() == "Use fixed thread representation" =>
                {
                    Some(text.pos + text.galley.size() / 2.)
                }
                _ => None,
            })
            .expect("explicit unbinding action");
        output.drop_without_applying_deltas();
        for pressed in [true, false] {
            frame(
                &context,
                &mut panel,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            )
            .drop_without_applying_deltas();
        }
        let fixed = panel.request(&document, true).unwrap();
        assert_eq!(fixed.feature, feature);
        assert_eq!(fixed.parameters.representation_parameter, None);
        assert_eq!(
            fixed.parameters.representation,
            ThreadRepresentation::CosmeticEnvelope
        );
        assert_eq!(fixed.parameters.length, model::length(8.));
        assert_eq!(document, before);
    }
}

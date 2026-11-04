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
    pipe: Option<model::PipeThreadIntent>,
    npt: Option<model::NptThreadIntent>,
    buttress: Option<model::ButtressIntent>,
    hand: ThreadHand,
    starts: u32,
    representation: ThreadRepresentation,
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
        Ok(Self {
            parameters,
            stamp: FormStamp::new(),
            choices,
            target,
            mode: request.mode,
            family: p.family,
            fit: p.fit,
            pipe: p.pipe,
            npt: p.npt,
            buttress: p.buttress,
            hand: p.hand,
            starts: p.starts,
            representation: p.representation,
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
            runout: self.request.parameters.runout,
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
                    if response.clicked() {
                        changed |= self.family != family;
                        self.family = family;
                        self.fit = Self::fit_for(family, mode);
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
            ui.small("Basic profile only; no ISO 965 tolerance class. Multiple starts use this profile as custom geometry, not a standard fastening designation.");
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
                    !self.family.is_unified()
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
        ui.separator();
        ui.strong("Representation");
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
        if self.representation == ThreadRepresentation::CosmeticEnvelope {
            ui.colored_label(
                egui::Color32::YELLOW,
                "Display only. Switch to Modeled before STEP, mesh or print export.",
            );
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
        ui.small("Preview checks the complete native solid before one undoable Apply. Flat cut ends; no automatic runout or relief.");
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
                        if self.family == ThreadFamily::Bspp {
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
                        if let Some(radius) = resolved.profile.rounding_radius_mm {
                            ui.small(format!("Rounded crest and root · radius {radius:.6} mm · 55° included angle"));
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

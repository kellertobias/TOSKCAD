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
                        if family.is_unified() {
                            self.starts = 1;
                        }
                    }
                }
            });
        ui.small(self.family.source());
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
                    !self.family.is_unified(),
                    egui::DragValue::new(&mut self.starts).range(1..=8),
                )
                .changed();
        });
        ui.horizontal(|ui| {
            for (hand, label) in [
                (ThreadHand::Right, "Right hand"),
                (ThreadHand::Left, "Left hand"),
            ] {
                changed |= ui.radio_value(&mut self.hand, hand, label).changed();
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
    fn fit_for(family: ThreadFamily, mode: ThreadMode) -> ThreadFit {
        if !family.is_unified() {
            ThreadFit::NominalBasic
        } else if mode == ThreadMode::CutInternal {
            ThreadFit::Unified2B
        } else {
            ThreadFit::Unified2A
        }
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
        if previous_mode != self.mode && self.family.is_unified() {
            self.fit = Self::fit_for(self.family, self.mode);
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
                    if let Some(fit) = &resolved.fit {
                        ui.strong(format!("{} · {}", fit.designation, fit.class.label()));
                        ui.small(format!(
                            "Nominal {:.4} in · {:.0} TPI",
                            2. * resolved.nominal_major_radius_mm / 25.4,
                            25.4 / resolved.pitch_mm
                        ));
                        ui.small(format!(
                            "Resolved major {:.6} mm · pitch {:.6} mm · minor {:.6} mm",
                            fit.major_diameter_mm, fit.pitch_diameter_mm, fit.minor_diameter_mm
                        ));
                        ui.small(format!(
                            "Maximum supported thread length {:.6} mm",
                            fit.engagement_limit_mm
                        ));
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

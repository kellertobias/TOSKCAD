//! Gear controls share canonical intent and native candidate receipts with MCP.
mod pair;
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui};
use multicad_core::{
    document::Document,
    model::{GearFamily, GearHand, GearModuleSystem, GearRequest, PrimitiveFrame, angle},
    parameters::EvaluatedParameters,
    sketch::Datum,
};
pub use pair::GearPairCommand;

#[derive(Clone)]
pub(super) struct GearPanel {
    request: GearRequest,
    parameters: EvaluatedParameters,
    fields: [DimensionField; 11],
    origin: [DimensionField; 3],
    family: GearFamily,
    system: GearModuleSystem,
    hand: GearHand,
    plane: Option<Datum>,
    stamp: FormStamp,
    editing: bool,
    pair: Option<Box<pair::GearPairPanel>>,
}
impl GearPanel {
    pub fn new(
        request: GearRequest,
        parameters: EvaluatedParameters,
        editing: bool,
    ) -> Result<Self, String> {
        let p = &request.parameters;
        let field = DimensionField::from_dimension;
        Ok(Self {
            fields: [
                field(&p.module, "mm")?,
                field(&p.teeth, "count")?,
                field(&p.pressure_angle, "deg")?,
                field(&p.face_width, "mm")?,
                field(&p.helix_angle, "deg")?,
                field(&p.center_gap, "mm")?,
                field(&p.bore_diameter, "mm")?,
                field(&p.backlash, "mm")?,
                field(&p.fabrication_allowance, "mm")?,
                field(&p.root_fillet_ratio, "ratio")?,
                field(&p.minimum_wall, "mm")?,
            ],
            origin: [
                field(&p.frame.origin[0], "mm")?,
                field(&p.frame.origin[1], "mm")?,
                field(&p.frame.origin[2], "mm")?,
            ],
            family: p.family,
            system: p.module_system,
            hand: p.hand,
            plane: Datum::ALL
                .into_iter()
                .find(|d| d.frame().u == p.frame.u && d.frame().v == p.frame.v),
            parameters,
            stamp: FormStamp::new(),
            editing,
            pair: None,
            request,
        })
    }
    pub(super) fn enable_pair(&mut self) {
        if !self.editing && self.pair.is_none() {
            self.pair = Some(Box::new(pair::GearPairPanel::from_single(self.clone())));
        }
    }
    pub fn stamp(&self) -> FormStamp {
        self.pair.as_ref().map_or(self.stamp, |p| p.stamp())
    }
    pub fn label(&self) -> &'static str {
        if self.pair.is_some() {
            "Gear pair"
        } else {
            "Gear"
        }
    }
    pub fn existing_pair(
        document: &Document,
        pair: multicad_core::document::EntityId,
        parameters: EvaluatedParameters,
    ) -> Result<Self, String> {
        let panel = pair::GearPairPanel::existing(document, pair, parameters)?;
        let mut outer = panel.primary_clone();
        outer.pair = Some(Box::new(panel));
        Ok(outer)
    }
    pub fn model_request(
        &self,
        document: &Document,
        enabled: bool,
    ) -> Result<ModelRequest, String> {
        if let Some(panel) = &self.pair {
            panel
                .request(document, enabled)
                .map(|r| ModelRequest::GearPair(Box::new(r)))
        } else {
            self.request(document, enabled)
                .map(|r| ModelRequest::Gear(Box::new(r)))
        }
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<GearRequest, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.request.name.trim().is_empty() || self.request.name.len() > 1000 {
            return Err("Enter a name with 1–1000 bytes".into());
        }
        let mut request = self.request.clone();
        let p = &mut request.parameters;
        p.family = self.family;
        p.module_system = self.system;
        p.hand = self.hand;
        p.module = self.fields[0].resolve(document)?;
        p.teeth = self.fields[1].resolve(document)?;
        p.pressure_angle = self.fields[2].resolve(document)?;
        p.face_width = self.fields[3].resolve(document)?;
        p.helix_angle = self.fields[4].resolve(document)?;
        p.center_gap = self.fields[5].resolve(document)?;
        p.bore_diameter = self.fields[6].resolve(document)?;
        p.backlash = self.fields[7].resolve(document)?;
        p.fabrication_allowance = self.fields[8].resolve(document)?;
        p.root_fillet_ratio = self.fields[9].resolve(document)?;
        p.minimum_wall = self.fields[10].resolve(document)?;
        let basis = self.plane.map(Datum::frame);
        p.frame = PrimitiveFrame {
            origin: [
                self.origin[0].resolve(document)?,
                self.origin[1].resolve(document)?,
                self.origin[2].resolve(document)?,
            ],
            u: basis.map_or(p.frame.u, |f| f.u),
            v: basis.map_or(p.frame.v, |f| f.v),
        };
        p.resolve(&self.parameters).map_err(|e| e.to_string())?;
        Ok(request)
    }
    fn geometry_controls(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.label("Gear family");
        controls::inspector_combo(ui, "gear-family")
            .selected_text(self.family.label())
            .show_ui(ui, |ui| {
                for family in GearFamily::ALL {
                    if ui
                        .add_enabled(
                            family.supported(),
                            egui::Button::selectable(self.family == family, family.label()),
                        )
                        .on_disabled_hover_text("Family geometry is not implemented")
                        .clicked()
                        && self.family != family
                    {
                        self.family = family;
                        changed = true;
                        if family == GearFamily::Spur {
                            self.hand = GearHand::None;
                            self.fields[4] = DimensionField::from_dimension(&angle(0.), "deg")
                                .expect("zero angle");
                        } else if self.hand == GearHand::None {
                            self.hand = GearHand::Right;
                            self.fields[4] = DimensionField::from_dimension(&angle(15.), "deg")
                                .expect("preset angle");
                        }
                        self.fields[5] = DimensionField::from_dimension(
                            &multicad_core::model::length(if family == GearFamily::DoubleHelical {
                                2.
                            } else {
                                0.
                            }),
                            "mm",
                        )
                        .expect("preset gap");
                    }
                }
            });
        ui.small("Unshifted involute profile with circular root fillets. No accuracy, strength or cutter-generation certification.");
        controls::inspector_combo(ui, "gear-module-system")
            .selected_text(match self.system {
                GearModuleSystem::Normal => "Normal module and pressure angle",
                GearModuleSystem::Transverse => "Transverse module and pressure angle",
            })
            .show_ui(ui, |ui| {
                changed |= ui
                    .selectable_value(&mut self.system, GearModuleSystem::Normal, "Normal")
                    .changed();
                changed |= ui
                    .selectable_value(&mut self.system, GearModuleSystem::Transverse, "Transverse")
                    .changed();
            });
        for (i, label, help) in [
            (
                0,
                "Module",
                "Basic module in the selected normal/transverse system",
            ),
            (
                1,
                "Teeth",
                "Whole tooth count; conservative unshifted undercut policy applies",
            ),
            (
                2,
                "Pressure angle",
                "Angle in the selected normal/transverse system",
            ),
            (
                3,
                "Face width",
                "Total axial width including any center gap",
            ),
        ] {
            changed |= self.fields[i].show(ui, label, help);
        }
        if self.family != GearFamily::Spur {
            changed |= self.fields[4].show(
                ui,
                "Helix angle",
                "Positive reference-cylinder angle; hand is separate",
            );
            ui.horizontal(|ui| {
                changed |= ui
                    .radio_value(&mut self.hand, GearHand::Right, "Right hand")
                    .changed();
                changed |= ui
                    .radio_value(&mut self.hand, GearHand::Left, "Left hand")
                    .changed();
            });
            if matches!(
                self.family,
                GearFamily::Herringbone | GearFamily::DoubleHelical
            ) {
                ui.small(
                    "Hand describes the lower tooth band; the upper band has the opposite hand.",
                );
            }
        }
        if self.family == GearFamily::DoubleHelical {
            changed |= self.fields[5].show(
                ui,
                "Center gap",
                "Root-cylinder band between opposite-handed tooth bands",
            );
        }
        for (i, label, help) in [
            (
                6,
                "Bore diameter",
                "Through bore; zero makes a solid center",
            ),
            (
                7,
                "Design backlash per gear",
                "Tangential tooth-thickness reduction at the reference circle",
            ),
            (
                8,
                "Additional fabrication clearance",
                "Additional tangential thinning, separate from design backlash",
            ),
            (
                9,
                "Root fillet / module",
                "Custom circular fillet radius divided by the selected module",
            ),
            (
                10,
                "Minimum root wall",
                "Minimum radial material between bore and tooth roots",
            ),
        ] {
            changed |= self.fields[i].show(ui, label, help);
        }
        changed
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState,
    ) -> Option<ModelEvent> {
        if let Some(panel) = &mut self.pair {
            let event = panel.show(ui, document, enabled, preview);
            if panel.single_requested() {
                *self = panel.as_single();
                return Some(ModelEvent::Changed);
            }
            return event;
        }
        if !self.editing && ui.button("Create mating pair").clicked() {
            self.enable_pair();
            return Some(ModelEvent::Changed);
        }
        ui.label(if self.editing {
            "Feature name"
        } else {
            "Part name"
        });
        let mut changed = ui
            .add_enabled(
                !self.editing,
                egui::TextEdit::singleline(&mut self.request.name)
                    .desired_width(ui.available_width()),
            )
            .changed();
        changed |= self.geometry_controls(ui);
        ui.label("Axis plane");
        controls::inspector_combo(ui, "gear-plane")
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
            changed |= field.show(ui, label, "Axis follows the plane normal");
        }
        let validation = self.request(document, enabled);
        match &validation {
            Ok(request) => {
                if let Ok(r) = request.parameters.resolve(&self.parameters) {
                    ui.small(format!(
                        "Reference Ø {:.4} mm · base Ø {:.4} mm · tip Ø {:.4} mm",
                        2. * r.pitch_radius_mm,
                        2. * r.base_radius_mm,
                        2. * r.tip_radius_mm
                    ));
                }
            }
            Err(error) => {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
        }
        form::preview_status(ui, self.stamp, preview);
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
                    .map(|request| ModelEvent::Preview {
                        stamp: self.stamp,
                        request: ModelRequest::Gear(Box::new(request)),
                    })
            }
            Some(FormAction::Apply) if preview.ready(self.stamp) && validation.is_ok() => {
                Some(ModelEvent::Apply)
            }
            _ => None,
        }
    }
}

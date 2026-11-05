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
    outside: DimensionField,
    rack_backing: DimensionField,
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
            rack_backing: field(
                p.rack
                    .as_ref()
                    .map(|rack| &rack.backing_thickness)
                    .unwrap_or(&multicad_core::model::length(4.)),
                "mm",
            )?,
            outside: field(
                p.outer_diameter
                    .as_ref()
                    .unwrap_or(&multicad_core::model::length(112.)),
                "mm",
            )?,
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
        p.outer_diameter = if self.family == GearFamily::InternalRing {
            Some(self.outside.resolve(document)?)
        } else {
            None
        };
        p.backlash = self.fields[7].resolve(document)?;
        p.fabrication_allowance = self.fields[8].resolve(document)?;
        p.root_fillet_ratio = self.fields[9].resolve(document)?;
        p.minimum_wall = self.fields[10].resolve(document)?;
        p.rack = if self.family == GearFamily::Rack {
            Some(multicad_core::model::RackDimensions {
                backing_thickness: self.rack_backing.resolve(document)?,
            })
        } else {
            None
        };
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
        p.resolve_geometry(&self.parameters)
            .map_err(|e| e.to_string())?;
        Ok(request)
    }
    fn select_family(&mut self, family: GearFamily) {
        if self.family == family {
            return;
        }
        self.family = family;
        if matches!(
            family,
            GearFamily::Spur | GearFamily::InternalRing | GearFamily::Rack
        ) {
            self.hand = GearHand::None;
            self.fields[4] = DimensionField::from_dimension(&angle(0.), "deg").expect("zero angle");
        } else if self.hand == GearHand::None {
            self.hand = GearHand::Right;
            self.fields[4] =
                DimensionField::from_dimension(&angle(15.), "deg").expect("preset angle");
        }
        if family == GearFamily::InternalRing {
            self.fields[1] = DimensionField::from_dimension(
                &multicad_core::constraint_sketch::Dimension::Literal {
                    value: 48.,
                    unit: "count".into(),
                },
                "count",
            )
            .expect("ring teeth");
            self.fields[2] =
                DimensionField::from_dimension(&angle(20.), "deg").expect("ring pressure");
            self.fields[6] =
                DimensionField::from_dimension(&multicad_core::model::length(0.), "mm")
                    .expect("derived opening");
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
        if family == GearFamily::Rack {
            self.fields[6] =
                DimensionField::from_dimension(&multicad_core::model::length(0.), "mm")
                    .expect("rack has no bore");
        }
    }
    fn geometry_controls(&mut self, ui: &mut egui::Ui) -> bool {
        self.geometry_controls_with_family(ui, true)
    }
    fn geometry_controls_with_family(&mut self, ui: &mut egui::Ui, choose_family: bool) -> bool {
        let mut changed = false;
        if choose_family {
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
                            self.select_family(family);
                            changed = true;
                        }
                    }
                });
        }
        if self.family == GearFamily::Rack {
            ui.small("Straight basic rack · custom circular root fillets, not generated cutter runout. 4–64 complete pitch cells; 14.5–35° pressure; root radius/module 0.01–0.2. No accuracy or strength certification.");
            ui.small("Centered local X length = cells × π × module. Pitch line Y = 0; teeth point toward +Y; face width follows +Z. Zero helix, bore, center gap and profile shift.");
        } else {
            ui.small("Unshifted involute profile with circular root fillets. No accuracy, strength or cutter-generation certification.");
        }
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
                if self.family == GearFamily::Rack {
                    "Rack pitch cells"
                } else {
                    "Teeth"
                },
                if self.family == GearFamily::Rack {
                    "Whole complete pitch-cell count; finite rack length is count × π × module"
                } else {
                    "Whole tooth count; conservative unshifted undercut policy applies"
                },
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
            changed |= self.fields[i].show_named(
                ui,
                (self.request.feature, "gear-dimension", i),
                label,
                help,
            );
        }
        if !matches!(
            self.family,
            GearFamily::Spur | GearFamily::InternalRing | GearFamily::Rack
        ) {
            changed |= self.fields[4].show_named(
                ui,
                (self.request.feature, "gear-dimension", 4_usize),
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
            changed |= self.fields[5].show_named(
                ui,
                (self.request.feature, "gear-dimension", 5_usize),
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
            if i == 6 && matches!(self.family, GearFamily::InternalRing | GearFamily::Rack) {
                continue;
            }
            if i == 10 && self.family == GearFamily::Rack {
                changed |= self.fields[i].show_named(ui, (self.request.feature, "gear-dimension", i), "Minimum backing wall", "Minimum material below the rack tooth roots; backing must be at least this thick");
                continue;
            }
            if i == 10 && self.family == GearFamily::InternalRing {
                changed |= self.fields[i].show_named(
                    ui,
                    (self.request.feature, "gear-dimension", i),
                    "Minimum rim wall",
                    "Minimum radial material beyond the outward tooth roots",
                );
                continue;
            }
            changed |= self.fields[i].show_named(
                ui,
                (self.request.feature, "gear-dimension", i),
                label,
                help,
            );
        }
        if self.family == GearFamily::Rack {
            changed |= self.rack_backing.show_named(ui, (self.request.feature, "gear-backing"), "Rack backing thickness", "Material below the tooth root line, excluding the 1.25-module dedendum and one-module addendum");
        }
        if self.family == GearFamily::InternalRing {
            changed |= self.outside.show_named(ui, (self.request.feature, "gear-outside"), "Ring outside diameter", "Explicit outside diameter; minimum radial wall is measured beyond the outward tooth roots");
            ui.small("48 or 60 teeth · 20° spur · derived inward opening · compatible 24-tooth pinion. Custom root circles are not a generated cutter trochoid.");
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
                    .id(ui.make_persistent_id((self.request.feature, "gear-name")))
                    .desired_width(ui.available_width()),
            )
            .changed();
        changed |= self.geometry_controls(ui);
        ui.label(if self.family == GearFamily::Rack {
            "Rack plane"
        } else {
            "Axis plane"
        });
        controls::inspector_combo(ui, "gear-plane")
            .selected_text(self.plane.map_or("Stored orientation", Datum::label))
            .show_ui(ui, |ui| {
                for datum in Datum::ALL {
                    changed |= ui
                        .selectable_value(&mut self.plane, Some(datum), datum.label())
                        .changed();
                }
            });
        for (index, (field, label)) in self
            .origin
            .iter_mut()
            .zip(["Base center X", "Base center Y", "Base center Z"])
            .enumerate()
        {
            changed |= field.show_named(
                ui,
                (self.request.feature, "gear-origin", index),
                label,
                "Axis follows the plane normal",
            );
        }
        let validation = self.request(document, enabled);
        match &validation {
            Ok(request) => match request.parameters.resolve_geometry(&self.parameters) {
                Ok(multicad_core::model::ResolvedGearGeometry::Rotary(r)) => {
                    ui.small(format!(
                        "Reference Ø {:.4} mm · base Ø {:.4} mm · tip Ø {:.4} mm",
                        2. * r.pitch_radius_mm,
                        2. * r.base_radius_mm,
                        2. * r.tip_radius_mm
                    ));
                }
                Ok(multicad_core::model::ResolvedGearGeometry::Rack(r)) => {
                    ui.small(format!(
                        "Rack pitch {:.4} mm · finite length {:.4} mm · {} complete cells",
                        r.pitch_mm, r.length_mm, r.teeth
                    ));
                }
                Err(_) => {}
            },
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

#[cfg(test)]
mod rack_fields_tests {
    use super::*;
    use multicad_core::{
        commands::Session,
        document::{Entity, EntityData, Mutation},
        model::{self, GearParameters, RackDimensions},
        parameters::BoundExpression,
    };
    #[test]
    fn rack_fields_preserve_authored_expressions_and_reject_unsupported_hidden_rotary_values() {
        let entities = [
            Entity::new(
                "rack_module",
                EntityData::Parameter {
                    expression: "1 mm".into(),
                    unit: "mm".into(),
                    exposed: false,
                    inputs: vec![],
                },
            ),
            Entity::new(
                "rack_cells",
                EntityData::Parameter {
                    expression: "24".into(),
                    unit: "count".into(),
                    exposed: false,
                    inputs: vec![],
                },
            ),
            Entity::new(
                "rack_backing",
                EntityData::Parameter {
                    expression: "3 mm".into(),
                    unit: "mm".into(),
                    exposed: false,
                    inputs: vec![],
                },
            ),
        ];
        let document = Document::new("Rack fields")
            .unwrap()
            .edited(
                &entities
                    .into_iter()
                    .map(Mutation::Insert)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let session = Session::new(document.clone()).unwrap();
        let expression = |text, unit| multicad_core::constraint_sketch::Dimension::Expression {
            expression: BoundExpression::bind(&document, text, unit, None).unwrap(),
        };
        let parameters = GearParameters {
            family: GearFamily::Rack,
            module: expression("=$rack_module", "mm"),
            teeth: expression("=$rack_cells", "count"),
            bore_diameter: model::length(0.),
            face_width: model::length(2.),
            rack: Some(RackDimensions {
                backing_thickness: expression("=$rack_backing", "mm"),
            }),
            ..Default::default()
        };
        let request = GearRequest::new("Expression rack", parameters.clone());
        let mut panel =
            GearPanel::new(request.clone(), session.parameters().clone(), true).unwrap();
        let unchanged = panel.request(&document, true).unwrap();
        assert_eq!(unchanged, request);
        let resolved = unchanged
            .parameters
            .resolve_geometry(session.parameters())
            .unwrap();
        let rack = resolved.rack().unwrap();
        assert!((rack.pitch_mm - std::f64::consts::PI).abs() < 1e-12);
        assert!((rack.length_mm - 24. * std::f64::consts::PI).abs() < 1e-12);
        assert_eq!(rack.backing_thickness_mm, 3.);
        panel.fields[1].set_literal(3.5, "count");
        assert!(
            panel
                .request(&document, true)
                .unwrap_err()
                .contains("whole pitch cells")
        );
        panel.fields[1].set_literal(65., "count");
        assert!(panel.request(&document, true).is_err());
        for cells in [4., 64.] {
            panel.fields[1].set_literal(cells, "count");
            assert!(panel.request(&document, true).is_ok());
        }
        assert_eq!(session.document(), &document);
        let mut invalid = parameters;
        invalid.helix_angle = angle(15.);
        let panel = GearPanel::new(
            GearRequest::new("Invalid rack", invalid),
            session.parameters().clone(),
            true,
        )
        .unwrap();
        assert!(
            panel
                .request(&document, true)
                .unwrap_err()
                .contains("zero helix")
        );
        assert_eq!(session.revision(), 0);
    }
}

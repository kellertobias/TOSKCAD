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
    model::{
        BevelDimensions, BevelToothSystem, GearFamily, GearHand, GearModuleSystem, GearPitchInput,
        GearRequest, PrimitiveFrame, WormDimensions, WormMember, WormToothSystem, angle,
    },
    parameters::EvaluatedParameters,
    sketch::Datum,
};
pub use pair::GearPairCommand;

fn bevel_system_label(system: BevelToothSystem) -> &'static str {
    match system {
        BevelToothSystem::SphericalInvoluteStraight => "Spherical-involute straight",
        BevelToothSystem::GleasonOctoid => "Gleason / octoid (unavailable)",
        BevelToothSystem::Spiral => "Spiral bevel (unavailable)",
        BevelToothSystem::Zerol => "Zerol (unavailable)",
    }
}

fn show_bevel_summary(ui: &mut egui::Ui, r: &multicad_core::model::ResolvedBevel) {
    ui.small(format!(
        "Pitch cone {:.4}° · base cone {:.4}° · outer cone distance R {:.4} mm",
        r.pitch_cone_angle_rad.to_degrees(),
        r.base_cone_angle_rad.to_degrees(),
        r.outer_cone_distance_mm
    ));
    ui.small(format!(
        "Back-cone heel / toe tip Ø {:.4} / {:.4} mm · root Ø {:.4} / {:.4} mm",
        2. * r.heel_tip_radius_mm,
        2. * r.toe_tip_radius_mm,
        2. * r.heel_root_radius_mm,
        2. * r.toe_root_radius_mm
    ));
    ui.small(format!(
        "Pitch-cone face width {:.4} mm · toe bore wall {:.4} mm (minimum {:.4} mm)",
        r.face_width_mm,
        r.toe_root_radius_mm - r.bore_radius_mm,
        r.minimum_wall_mm
    ));
}

#[derive(Clone)]
struct HubFields {
    enabled: bool,
    dimensions: [DimensionField; 3],
}
impl HubFields {
    fn new(source: Option<&multicad_core::model::GearHub>) -> Result<Self, String> {
        let defaults = [
            multicad_core::model::length(20.),
            multicad_core::model::length(-5.),
            multicad_core::model::length(15.),
        ];
        let values = source
            .map(|h| [&h.diameter, &h.axial_start, &h.axial_end])
            .unwrap_or(defaults.each_ref());
        Ok(Self {
            enabled: source.is_some(),
            dimensions: [
                DimensionField::from_dimension(values[0], "mm")?,
                DimensionField::from_dimension(values[1], "mm")?,
                DimensionField::from_dimension(values[2], "mm")?,
            ],
        })
    }
    fn resolve(
        &self,
        document: &Document,
    ) -> Result<Option<multicad_core::model::GearHub>, String> {
        Ok(if self.enabled {
            Some(multicad_core::model::GearHub {
                diameter: self.dimensions[0].resolve(document)?,
                axial_start: self.dimensions[1].resolve(document)?,
                axial_end: self.dimensions[2].resolve(document)?,
            })
        } else {
            None
        })
    }
    fn show(
        &mut self,
        ui: &mut egui::Ui,
        owner: multicad_core::document::EntityId,
        supported: bool,
        label: &str,
    ) -> bool {
        let mut changed = ui
            .add_enabled(
                supported || self.enabled,
                egui::Checkbox::new(&mut self.enabled, label),
            )
            .changed();
        if !supported {
            ui.small("Hubs require an external cylindrical gear; other families are unavailable.");
        }
        if self.enabled {
            ui.small("One connected hub along local Z; start/end measured from the gear base at Z=0. Through bore spans the combined body. Radius stays below the root; overlap ≥0.01 mm and bore wall must remain.");
            for (i, (field, label)) in self
                .dimensions
                .iter_mut()
                .zip(["Hub diameter", "Hub axial start", "Hub axial end"])
                .enumerate()
            {
                changed |= field.show_named(
                    ui,
                    (owner, "gear-hub", i),
                    label,
                    "Authored intrinsic geometry; no standard stock proportions are inferred",
                );
            }
        }
        changed
    }
}
fn external_cylindrical(family: GearFamily) -> bool {
    matches!(
        family,
        GearFamily::Spur
            | GearFamily::Helical
            | GearFamily::Herringbone
            | GearFamily::DoubleHelical
    )
}

#[derive(Clone)]
pub(super) struct GearPanel {
    request: GearRequest,
    parameters: EvaluatedParameters,
    fields: [DimensionField; 11],
    pitch_input: GearPitchInput,
    alternate_pitch: DimensionField,
    hub: HubFields,
    shift: DimensionField,
    shift_mate_enabled: bool,
    shift_mate_teeth: DimensionField,
    shift_mate_shift: DimensionField,
    outside: DimensionField,
    rack_backing: DimensionField,
    bevel_mate: DimensionField,
    bevel_shaft: DimensionField,
    bevel_system: BevelToothSystem,
    worm_role: WormMember,
    worm_system: WormToothSystem,
    worm_mate: DimensionField,
    worm_quotient: DimensionField,
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
            pitch_input: p.pitch_input,
            alternate_pitch: field(
                &if p.pitch_input == GearPitchInput::Module {
                    multicad_core::constraint_sketch::Dimension::Literal {
                        value: 12.7,
                        unit: "teeth_per_in".into(),
                    }
                } else {
                    multicad_core::model::length(2.)
                },
                if p.pitch_input == GearPitchInput::Module {
                    "teeth_per_in"
                } else {
                    "mm"
                },
            )?,
            hub: HubFields::new(p.hub.as_ref())?,
            shift: field(&p.profile_shift, "ratio")?,
            shift_mate_enabled: p.shift_mate.is_some(),
            shift_mate_teeth: field(
                p.shift_mate.as_ref().map(|m| &m.teeth).unwrap_or(&p.teeth),
                "count",
            )?,
            shift_mate_shift: field(
                p.shift_mate
                    .as_ref()
                    .map(|m| &m.profile_shift)
                    .unwrap_or(&multicad_core::model::ratio(0.)),
                "ratio",
            )?,
            worm_role: p.worm.as_ref().map_or(WormMember::Worm, |w| w.role),
            worm_system: p
                .worm
                .as_ref()
                .map_or(WormToothSystem::ZaAxialStraight, |w| w.tooth_system),
            worm_mate: field(
                p.worm.as_ref().map(|w| &w.mate_teeth).unwrap_or(
                    &multicad_core::constraint_sketch::Dimension::Literal {
                        value: 32.,
                        unit: "count".into(),
                    },
                ),
                "count",
            )?,
            worm_quotient: field(
                p.worm
                    .as_ref()
                    .map(|w| &w.diameter_quotient)
                    .unwrap_or(&multicad_core::model::ratio(10.)),
                "ratio",
            )?,
            bevel_mate: field(
                p.bevel.as_ref().map(|b| &b.mate_teeth).unwrap_or(&p.teeth),
                "count",
            )?,
            bevel_shaft: field(
                p.bevel
                    .as_ref()
                    .map(|b| &b.shaft_angle)
                    .unwrap_or(&angle(90.)),
                "deg",
            )?,
            bevel_system: p
                .bevel
                .as_ref()
                .map_or(BevelToothSystem::SphericalInvoluteStraight, |b| {
                    b.tooth_system
                }),
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
                field(
                    &p.module,
                    if p.pitch_input == GearPitchInput::Module {
                        "mm"
                    } else {
                        "teeth_per_in"
                    },
                )?,
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
        let request = self.authored_request(document, enabled)?;
        request
            .parameters
            .resolve_geometry(&self.parameters)
            .map_err(|e| e.to_string())?;
        Ok(request)
    }
    // Pairs couple typed mate dimensions before resolving either member.
    fn authored_request(&self, document: &Document, enabled: bool) -> Result<GearRequest, String> {
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
        p.pitch_input = self.pitch_input;
        p.module = self.fields[0].resolve(document)?;
        p.hub = self.hub.resolve(document)?;
        p.profile_shift = self.shift.resolve(document)?;
        p.shift_mate = if self.shift_mate_enabled {
            Some(multicad_core::model::ShiftedGearMate {
                teeth: self.shift_mate_teeth.resolve(document)?,
                profile_shift: self.shift_mate_shift.resolve(document)?,
            })
        } else {
            None
        };
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
        p.bevel = if self.family == GearFamily::Bevel {
            Some(BevelDimensions {
                tooth_system: self.bevel_system,
                mate_teeth: self.bevel_mate.resolve(document)?,
                shaft_angle: self.bevel_shaft.resolve(document)?,
            })
        } else {
            None
        };
        p.worm = if self.family == GearFamily::Worm {
            Some(WormDimensions {
                role: self.worm_role,
                tooth_system: self.worm_system,
                mate_teeth: self.worm_mate.resolve(document)?,
                diameter_quotient: self.worm_quotient.resolve(document)?,
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
        Ok(request)
    }
    fn select_pitch_input(&mut self, input: GearPitchInput) {
        if input != self.pitch_input {
            std::mem::swap(&mut self.fields[0], &mut self.alternate_pitch);
            self.pitch_input = input;
        }
    }
    fn select_family(&mut self, family: GearFamily) {
        if self.family == family {
            return;
        }
        if !external_cylindrical(family) {
            self.select_pitch_input(GearPitchInput::Module);
        }
        if self.family == GearFamily::Worm && family != GearFamily::Worm {
            self.system = GearModuleSystem::Normal;
            self.fields[9].set_literal(0.2, "ratio");
            if self.worm_role == WormMember::Worm {
                self.fields[1].set_literal(24., "count");
            }
        }
        self.family = family;
        if family == GearFamily::Worm {
            self.system = GearModuleSystem::Axial;
            self.hand = GearHand::Right;
            self.worm_role = WormMember::Worm;
            self.worm_system = WormToothSystem::ZaAxialStraight;
            self.fields[0].set_literal(1., "mm");
            self.fields[1].set_literal(1., "count");
            self.fields[2].set_literal(20., "deg");
            self.fields[3].set_literal(24., "mm");
            self.fields[4].set_literal(0., "deg");
            self.fields[5].set_literal(0., "mm");
            self.fields[6].set_literal(0., "mm");
            self.fields[7].set_literal(0.05, "mm");
            self.fields[8].set_literal(0., "mm");
            self.fields[9].set_literal(0., "ratio");
            self.worm_mate.set_literal(32., "count");
            self.worm_quotient.set_literal(10., "ratio");
            return;
        }
        if matches!(
            family,
            GearFamily::Spur | GearFamily::InternalRing | GearFamily::Rack | GearFamily::Bevel
        ) {
            self.hand = GearHand::None;
            self.fields[4] = DimensionField::from_dimension(&angle(0.), "deg").expect("zero angle");
        } else if self.hand == GearHand::None {
            self.hand = GearHand::Right;
            self.fields[4] =
                DimensionField::from_dimension(&angle(15.), "deg").expect("preset angle");
        }
        if family == GearFamily::Bevel {
            self.fields[2] =
                DimensionField::from_dimension(&angle(20.), "deg").expect("bevel pressure");
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
        self.geometry_controls_with_family(ui, true, false)
    }
    fn geometry_controls_with_family(
        &mut self,
        ui: &mut egui::Ui,
        choose_family: bool,
        paired: bool,
    ) -> bool {
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
        if self.family == GearFamily::Worm {
            ui.small("ZA axial-straight worm / generated wheel · axial module 0.5–2 mm · NORMAL pressure 20° · one right-hand start · diameter quotient 10.");
            ui.small("Worm length 16–32 × module; wheel face width 0.5–2 × module; wheel 32–64 teeth. Total thinning ≤ 0.1 × module. No load, fit or accuracy certification.");
            if !paired {
                let old = self.worm_role;
                controls::inspector_combo(ui, (self.request.feature, "worm-role"))
                    .selected_text(if old == WormMember::Worm {
                        "Worm"
                    } else {
                        "Worm wheel"
                    })
                    .show_ui(ui, |ui| {
                        changed |= ui
                            .selectable_value(&mut self.worm_role, WormMember::Worm, "Worm")
                            .changed();
                        changed |= ui
                            .selectable_value(&mut self.worm_role, WormMember::Wheel, "Worm wheel")
                            .changed();
                    });
                if old != self.worm_role {
                    self.fields[1].set_literal(
                        if self.worm_role == WormMember::Worm {
                            1.
                        } else {
                            32.
                        },
                        "count",
                    );
                    self.fields[3].set_literal(
                        if self.worm_role == WormMember::Worm {
                            24.
                        } else {
                            1.
                        },
                        "mm",
                    );
                    self.worm_mate.set_literal(
                        if self.worm_role == WormMember::Worm {
                            32.
                        } else {
                            1.
                        },
                        "count",
                    );
                }
            }
            controls::inspector_combo(ui, (self.request.feature, "worm-system"))
                .selected_text(self.worm_system.label())
                .show_ui(ui, |ui| {
                    for system in WormToothSystem::ALL {
                        let old_system = self.worm_system;
                        changed |= ui
                            .add_enabled(
                                system.supported(),
                                egui::Button::selectable(
                                    self.worm_system == system,
                                    system.label(),
                                ),
                            )
                            .clicked()
                            && old_system != system
                            && {
                                self.worm_system = system;
                                true
                            };
                    }
                });
            if !paired {
                changed|=self.worm_mate.show_named(ui,(self.request.feature,"worm-mate"),if self.worm_role==WormMember::Worm {"Mating wheel teeth"} else {"Mating worm starts"},"Original typed mate count; the supported worm has one start and its wheel has 32–64 teeth");
            }
            changed |= self.worm_quotient.show_named(
                ui,
                (self.request.feature, "worm-quotient"),
                "Diameter quotient",
                "Only 10 is supported; pitch diameter = quotient × axial module",
            );
        } else if self.family == GearFamily::Bevel {
            ui.small("Spherical-involute straight bevel · KG/KHK back-cone boundaries · custom tangent spherical-circle roots. No Gleason, octoid, spiral, Zerol, cutter or strength certification.");
            ui.small("20° pressure · 90° intersecting shafts · 24–64 teeth per member · face width along the pitch cone, at most min(R/3, 10 × module).");
            controls::inspector_combo(ui, (self.request.feature, "bevel-tooth-system"))
                .selected_text(bevel_system_label(self.bevel_system))
                .show_ui(ui, |ui| {
                    changed |= ui.selectable_value(&mut self.bevel_system, BevelToothSystem::SphericalInvoluteStraight, "Spherical-involute straight").changed();
                    for label in ["Gleason / octoid", "Spiral bevel", "Zerol"] {
                        ui.add_enabled(false, egui::Button::new(label)).on_disabled_hover_text("This tooth system has no supported native geometry or mating verification");
                    }
                });
            if !paired {
                changed |= self.bevel_mate.show_named(ui, (self.request.feature, "bevel-mate-teeth"), "Mating gear teeth", "The explicit mate tooth count determines this gear's pitch-cone angle; preserved as a typed project expression");
            }
            changed |= self.bevel_shaft.show_named(ui, (self.request.feature, "bevel-shaft-angle"), "Shared shaft angle", "Only 90° intersecting shafts are supported. A pair shares this same authored dimension.");
        } else if self.family == GearFamily::Rack {
            ui.small("Straight basic rack · custom circular root fillets, not generated cutter runout. 4–64 complete pitch cells; 14.5–35° pressure; root radius/module 0.01–0.2. No accuracy or strength certification.");
            ui.small("Centered local X length = cells × π × module. Pitch line Y = 0; teeth point toward +Y; face width follows +Z. Zero helix, bore, center gap and profile shift.");
        } else {
            ui.small("Involute profile with custom circular root fillets. No accuracy, strength or cutter-generation certification.");
        }
        if external_cylindrical(self.family) {
            let mut input = self.pitch_input;
            controls::inspector_combo(ui, (self.request.feature, "gear-pitch-input"))
                .selected_text(if input == GearPitchInput::Module {
                    "Module"
                } else {
                    "Diametral pitch"
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut input, GearPitchInput::Module, "Module (mm)")
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut input,
                            GearPitchInput::DiametralPitch,
                            "Diametral pitch (teeth/in)",
                        )
                        .changed();
                });
            self.select_pitch_input(input);
            ui.small("One authored pitch input. Diametral pitch counts teeth per inch of reference diameter; module mm =25.4/DP. Normal/transverse convention is independent.");
        }
        controls::inspector_combo(ui, "gear-module-system")
            .selected_text(if self.family == GearFamily::Worm {
                "Axial module; normal pressure angle"
            } else if self.family == GearFamily::Bevel {
                "Outer module (normal = transverse)"
            } else {
                match self.system {
                    GearModuleSystem::Normal => "Normal pitch and pressure angle",
                    GearModuleSystem::Transverse => "Transverse pitch and pressure angle",
                    GearModuleSystem::Axial => "Axial module; normal pressure angle",
                }
            })
            .show_ui(ui, |ui| {
                if self.family == GearFamily::Worm {
                    ui.label("Axial module with normal pressure angle");
                    return;
                }
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
                if self.pitch_input == GearPitchInput::DiametralPitch {
                    "Diametral pitch"
                } else if self.family == GearFamily::Worm {
                    "Axial module"
                } else if self.family == GearFamily::Bevel {
                    "Outer module"
                } else {
                    "Module"
                },
                "Active authored pitch scalar in the selected normal/transverse convention; module in mm or DP in teeth_per_in",
            ),
            (
                1,
                if self.family == GearFamily::Worm {
                    if self.worm_role == WormMember::Worm {
                        "Worm starts"
                    } else {
                        "Wheel teeth"
                    }
                } else if self.family == GearFamily::Rack {
                    "Rack pitch cells"
                } else {
                    "Teeth"
                },
                if self.family == GearFamily::Rack {
                    "Whole complete pitch-cell count; finite rack length is count × π × module"
                } else {
                    "Whole tooth count; unshifted undercut bound or sourced shifted Spur envelope applies"
                },
            ),
            (
                2,
                if self.family == GearFamily::Worm {
                    "Normal pressure angle"
                } else {
                    "Pressure angle"
                },
                if self.family == GearFamily::Worm {
                    "Only normal 20°; axial pressure is derived from lead angle"
                } else {
                    "Angle in the selected normal/transverse system"
                },
            ),
            (
                3,
                if self.family == GearFamily::Worm {
                    if self.worm_role == WormMember::Worm {
                        "Threaded worm length"
                    } else {
                        "Wheel face width"
                    }
                } else if self.family == GearFamily::Bevel {
                    "Pitch-cone face width"
                } else {
                    "Face width"
                },
                if self.family == GearFamily::Bevel {
                    "Distance along the pitch-cone generator from heel to toe, not axial Z width"
                } else {
                    "Total axial width including any center gap"
                },
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
            GearFamily::Spur
                | GearFamily::InternalRing
                | GearFamily::Rack
                | GearFamily::Bevel
                | GearFamily::Worm
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
                if self.family == GearFamily::Bevel {
                    "Custom tangent small-circle radius on the outer reference sphere divided by outer module; not a cutter-generated root"
                } else {
                    "Custom circular fillet radius divided by the selected module"
                },
            ),
            (
                10,
                "Minimum root wall",
                if self.family == GearFamily::Bevel {
                    "Minimum bore wall checked against the toe root envelope"
                } else {
                    "Minimum radial material between bore and tooth roots"
                },
            ),
        ] {
            if i == 9 && self.family == GearFamily::Worm {
                continue;
            }
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
        changed |= self.hub.show(
            ui,
            self.request.feature,
            external_cylindrical(self.family),
            "Intrinsic hub",
        );
        if external_cylindrical(self.family)
            && (self.shift_mate_enabled || self.shift.text.trim().parse::<f64>() != Ok(0.))
        {
            ui.small(multicad_core::model::SHIFTED_GEAR_SOURCE);
            ui.hyperlink_to(
                "Shifted external gear reference",
                multicad_core::model::SHIFTED_GEAR_SOURCE_URL,
            );
            ui.hyperlink_to(
                "Shifted undercut reference",
                multicad_core::model::SHIFTED_GEAR_UNDERCUT_SOURCE_URL,
            );
        }
        let shifted_draft =
            self.shift_mate_enabled || self.shift.text.trim().parse::<f64>() != Ok(0.);
        if !paired && !external_cylindrical(self.family) && shifted_draft {
            ui.colored_label(egui::Color32::LIGHT_RED,"This family does not support the retained profile shift or matched context. Clear them explicitly to use an unshifted recipe.");
            if ui.button("Clear profile shift and mate context").clicked() {
                self.shift.set_literal(0., "ratio");
                self.shift_mate_enabled = false;
                changed = true;
            }
        }
        if external_cylindrical(self.family) || shifted_draft {
            changed|=self.shift.show_named(ui,(self.request.feature,"gear-shift"),"Profile shift coefficient","External Spur, Helical, Herringbone and Double-helical support source-defined shifts in either selected pitch plane. Authored coefficient -0.5..1; selected-plane source undercut, crest, root and bore limits still apply. Other families require zero.");
            ui.small("External shifts: selected pressure14.5–35°, coefficient -0.5 to1 in the selected module plane. The selected-plane source undercut equation and actual profile/contact limits remain. Thinning stays separate.");
        }
        if !paired && (external_cylindrical(self.family) || self.shift_mate_enabled) {
            changed |= ui
                .checkbox(
                    &mut self.shift_mate_enabled,
                    "Clearance-matched external mate",
                )
                .changed();
            if self.shift_mate_enabled {
                ui.small("Retained mate count/shift selects KHK paired tip shortening in this pitch plane. Without context the standalone tip is unshortened; shifted helical pairs require matching selected conventions.");
                changed |= self.shift_mate_teeth.show_named(
                    ui,
                    (self.request.feature, "shift-mate-count"),
                    "Shift mate teeth",
                    "Original typed mate tooth-count source",
                );
                changed |= self.shift_mate_shift.show_named(
                    ui,
                    (self.request.feature, "shift-mate-shift"),
                    "Shift mate coefficient",
                    "Original typed mate shift; -0.5..1",
                );
            }
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
            .zip(if self.family == GearFamily::Bevel {
                ["Pitch apex X", "Pitch apex Y", "Pitch apex Z"]
            } else {
                if self.family == GearFamily::Worm {
                    ["Axial center X", "Axial center Y", "Axial center Z"]
                } else {
                    ["Base center X", "Base center Y", "Base center Z"]
                }
            })
            .enumerate()
        {
            changed |= field.show_named(
                ui,
                (self.request.feature, "gear-origin", index),
                label,
                if self.family == GearFamily::Bevel {
                    "The pitch-cone apex is the origin; shaft follows the plane normal"
                } else {
                    "Axis follows the plane normal"
                },
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
                    ui.small(format!(
                        "Shift x={:.6} · tip shortening coefficient {:.6}",
                        r.profile_shift_coefficient, r.tip_shortening_coefficient
                    ));
                    if let Some(h) = &r.hub {
                        ui.small(format!(
                            "Intrinsic hub Ø {:.4} mm · local Z {:.4} to {:.4} mm",
                            2. * h.radius_mm,
                            h.axial_start_mm,
                            h.axial_end_mm
                        ));
                    }
                }
                Ok(multicad_core::model::ResolvedGearGeometry::Rack(r)) => {
                    ui.small(format!(
                        "Rack pitch {:.4} mm · finite length {:.4} mm · {} complete cells",
                        r.pitch_mm, r.length_mm, r.teeth
                    ));
                }
                Ok(multicad_core::model::ResolvedGearGeometry::Worm(r)) => {
                    ui.small(format!("{:?} · axial pitch {:.4} mm · lead {:.4} mm · derived lead angle {:.4}° / axial pressure {:.4}°",r.role,r.pitch_mm,r.lead_mm,r.lead_angle_rad.to_degrees(),r.axial_pressure_angle_rad.to_degrees()));
                    ui.small(format!("Reference Ø {:.4} mm · tip Ø {:.4} mm · minimum root radius {:.4} mm · centered axial extent {:.4} mm",2.*r.pitch_radius_mm,2.*r.tip_radius_mm,r.minimum_root_radius_mm,r.face_width_mm));
                }
                Ok(multicad_core::model::ResolvedGearGeometry::Bevel(r)) => {
                    show_bevel_summary(ui, &r)
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

#[cfg(test)]
mod bevel_fields_tests {
    use super::*;
    use multicad_core::{
        commands::Session,
        constraint_sketch::Dimension,
        document::{Entity, EntityData, Mutation},
        model::{self, GearParameters},
        parameters::BoundExpression,
    };
    #[test]
    fn bevel_fields_keep_typed_mate_shaft_and_outer_module_without_rotary_radius_claims() {
        let entities = [
            ("module", "1 mm", "mm"),
            ("teeth", "24", "count"),
            ("mate", "36", "count"),
            ("shaft", "90 deg", "deg"),
        ]
        .map(|(name, expression, unit)| {
            Entity::new(
                name,
                EntityData::Parameter {
                    expression: expression.into(),
                    unit: unit.into(),
                    exposed: false,
                    inputs: vec![],
                },
            )
        });
        let document = Document::new("Bevel typed fields")
            .unwrap()
            .edited(
                &entities
                    .into_iter()
                    .map(Mutation::Insert)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let session = Session::new(document.clone()).unwrap();
        let expression = |text, unit| Dimension::Expression {
            expression: BoundExpression::bind(&document, text, unit, None).unwrap(),
        };
        let recipe = GearParameters {
            family: GearFamily::Bevel,
            module: expression("=$module", "mm"),
            teeth: expression("=$teeth", "count"),
            face_width: model::length(4.),
            bore_diameter: model::length(2.),
            bevel: Some(BevelDimensions {
                tooth_system: BevelToothSystem::SphericalInvoluteStraight,
                mate_teeth: expression("=$mate", "count"),
                shaft_angle: expression("=$shaft", "deg"),
            }),
            ..Default::default()
        };
        let original = GearRequest::new("Typed bevel", recipe);
        let mut panel =
            GearPanel::new(original.clone(), session.parameters().clone(), true).unwrap();
        assert_eq!(panel.request(&document, true).unwrap(), original);
        let normal = panel
            .request(&document, true)
            .unwrap()
            .parameters
            .resolve_geometry(session.parameters())
            .unwrap();
        panel.system = GearModuleSystem::Transverse;
        let transverse = panel
            .request(&document, true)
            .unwrap()
            .parameters
            .resolve_geometry(session.parameters())
            .unwrap();
        assert_eq!(normal, transverse);
        let ctx = egui::Context::default();
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(650., 2400.),
                )),
                ..Default::default()
            },
            |ui| {
                panel.show(ui, &document, true, PreviewState::Idle);
            },
        );
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|s| {
                if let egui::Shape::Text(t) = &s.shape {
                    Some(t.galley.text())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            texts
                .iter()
                .any(|t| t.contains("Pitch cone") && t.contains("base cone"))
        );
        assert!(texts.iter().any(|t| t.contains("Back-cone heel / toe tip")));
        assert!(texts.iter().any(|t| t.contains("Pitch apex X")));
        assert!(
            !texts
                .iter()
                .any(|t| t.contains("base Ø") || t.contains("Reference Ø"))
        );
        output.drop_without_applying_deltas();
        for unsupported in [
            BevelToothSystem::GleasonOctoid,
            BevelToothSystem::Spiral,
            BevelToothSystem::Zerol,
        ] {
            panel.bevel_system = unsupported;
            assert!(panel.request(&document, true).is_err());
        }
        panel.bevel_system = BevelToothSystem::SphericalInvoluteStraight;
        panel.bevel_shaft.set_literal(80., "deg");
        assert!(panel.request(&document, true).is_err());
        panel.bevel_shaft.set_literal(90., "deg");
        panel.bevel_mate.set_literal(23., "count");
        assert!(panel.request(&document, true).is_err());
        assert_eq!(session.document(), &document);
        assert_eq!(session.revision(), 0);
    }
}

#[cfg(test)]
mod coverage_fields_tests {
    use super::*;
    use multicad_core::{
        commands::Session,
        constraint_sketch::Dimension,
        document::{Entity, EntityData, Mutation},
        model,
        parameters::BoundExpression,
    };
    #[test]
    fn active_dp_and_hub_sources_retain_bindings_and_pitch_drafts_do_not_numeric_copy() {
        let d = Document::new("Retained DP")
            .unwrap()
            .edited(&[
                Mutation::Insert(Entity::new(
                    "pitch",
                    EntityData::Parameter {
                        expression: "12.7 teeth_per_in".into(),
                        unit: "teeth_per_in".into(),
                        exposed: false,
                        inputs: vec![],
                    },
                )),
                Mutation::Insert(Entity::new(
                    "hub_d",
                    EntityData::Parameter {
                        expression: "20 mm".into(),
                        unit: "mm".into(),
                        exposed: false,
                        inputs: vec![],
                    },
                )),
            ])
            .unwrap();
        let s = Session::new(d.clone()).unwrap();
        let expression = |text, unit| Dimension::Expression {
            expression: BoundExpression::bind(&d, text, unit, None).unwrap(),
        };
        let dp = expression("=$pitch", "teeth_per_in");
        let diameter = expression("=$hub_d", "mm");
        let request = GearRequest::new(
            "Authored DP",
            model::GearParameters {
                pitch_input: GearPitchInput::DiametralPitch,
                module: dp.clone(),
                hub: Some(model::GearHub {
                    diameter: diameter.clone(),
                    axial_start: model::length(-3.),
                    axial_end: model::length(13.),
                }),
                ..Default::default()
            },
        );
        let feature = request.feature;
        let body = request.body;
        let mut panel = GearPanel::new(request, s.parameters().clone(), false).unwrap();
        let a = panel.request(&d, true).unwrap();
        assert_eq!(a.parameters.module, dp);
        assert_eq!(a.parameters.hub.as_ref().unwrap().diameter, diameter);
        panel.select_pitch_input(GearPitchInput::Module);
        panel.fields[0].set_literal(3., "mm");
        let module = panel.request(&d, true).unwrap();
        assert_eq!(module.parameters.module, model::length(3.));
        panel.select_pitch_input(GearPitchInput::DiametralPitch);
        let restored = panel.request(&d, true).unwrap();
        assert_eq!(restored.parameters.module, dp);
        assert_eq!(restored.feature, feature);
        assert_eq!(restored.body, body);
        assert_eq!(restored.parameters.hub, a.parameters.hub);
        panel.hub.dimensions[2].set_literal(-4., "mm");
        assert!(panel.request(&d, true).is_err());
        assert_eq!(s.document(), &d);
        assert_eq!(s.revision(), 0);
        assert!(!s.can_undo());
    }
    #[test]
    fn external_family_switches_retain_shift_context_in_both_planes_and_unsupported_clear_is_explicit()
     {
        let s = Session::new(Document::new("Retained external shifts").unwrap()).unwrap();
        for system in [GearModuleSystem::Normal, GearModuleSystem::Transverse] {
            let mut panel = GearPanel::new(
                GearRequest::new(
                    "Matched external",
                    model::GearParameters {
                        module_system: system,
                        pressure_angle: model::angle(25.),
                        profile_shift: model::ratio(0.2),
                        shift_mate: Some(model::ShiftedGearMate {
                            teeth: Dimension::Literal {
                                value: 36.,
                                unit: "count".into(),
                            },
                            profile_shift: model::ratio(0.1),
                        }),
                        ..Default::default()
                    },
                ),
                s.parameters().clone(),
                false,
            )
            .unwrap();
            for family in [
                GearFamily::Helical,
                GearFamily::Herringbone,
                GearFamily::DoubleHelical,
                GearFamily::Spur,
            ] {
                panel.select_family(family);
                let request = panel.request(s.document(), true).unwrap();
                assert_eq!(request.parameters.profile_shift, model::ratio(0.2));
                assert_eq!(
                    request
                        .parameters
                        .shift_mate
                        .as_ref()
                        .unwrap()
                        .profile_shift,
                    model::ratio(0.1)
                );
                assert_eq!(request.parameters.module_system, system);
                assert!(panel.shift_mate_enabled);
            }
            panel.select_family(GearFamily::Rack);
            assert!(panel.shift_mate_enabled);
            assert_eq!(
                panel.shift.resolve(s.document()).unwrap(),
                model::ratio(0.2)
            );
            assert!(panel.request(s.document(), true).is_err());
            let ctx = egui::Context::default();
            fn frame(
                ctx: &egui::Context,
                panel: &mut GearPanel,
                s: &Session,
                events: Vec<egui::Event>,
            ) -> egui::FullOutput {
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(650., 3600.),
                        )),
                        events,
                        focused: true,
                        ..Default::default()
                    },
                    |ui| {
                        panel.show(ui, s.document(), true, PreviewState::Idle);
                    },
                )
            }
            let out = frame(&ctx, &mut panel, &s, vec![]);
            let pos = out
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(t)
                        if t.galley.text() == "Clear profile shift and mate context" =>
                    {
                        Some(t.pos + t.galley.size() / 2.)
                    }
                    _ => None,
                })
                .expect("explicit unsupported-family clear action");
            out.drop_without_applying_deltas();
            for pressed in [true, false] {
                frame(
                    &ctx,
                    &mut panel,
                    &s,
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
            let request = panel.request(s.document(), true).unwrap();
            assert_eq!(request.parameters.profile_shift, model::ratio(0.));
            assert!(request.parameters.shift_mate.is_none());
        }
        assert_eq!(s.revision(), 0);
        assert!(!s.can_undo());
    }
}

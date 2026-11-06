use super::*;
use multicad_core::{
    commands::{Actor, Session},
    document::{EntityData, EntityId},
    model::{
        self, GearPairParameters, GearPairRequest, GearParameters, ModelPrepared, ResolvedGearPair,
    },
};

#[derive(Clone, Debug, PartialEq)]
pub enum GearPairCommand {
    Create(Box<GearPairRequest>),
    Edit {
        pair: EntityId,
        parameters: Box<GearPairParameters>,
        gears: Option<Box<[GearParameters; 2]>>,
    },
}
impl GearPairCommand {
    pub(crate) fn pair(&self) -> EntityId {
        match self {
            Self::Create(r) => r.pair,
            Self::Edit { pair, .. } => *pair,
        }
    }
    pub(crate) fn occurrence(&self) -> Option<EntityId> {
        match self {
            Self::Create(r) => Some(r.placement.instance),
            Self::Edit { .. } => None,
        }
    }
    pub(crate) fn instances(&self, document: &Document) -> Result<Vec<EntityId>, String> {
        if let Some(root) = self.occurrence() {
            return Ok(vec![root]);
        }
        let source = model::read_gear_pair(document, self.pair()).map_err(|e| e.to_string())?;
        Ok(document.entities().filter_map(|e|matches!(&e.data,EntityData::Instance{component,..} if *component==source.outputs.assembly).then_some(e.id)).collect())
    }
    pub(crate) fn resolved(
        &self,
        document: &Document,
        evaluated: &EvaluatedParameters,
    ) -> Result<ResolvedGearPair, String> {
        match self {
            Self::Create(r) => r
                .parameters
                .resolve_recipes([&r.gears[0].parameters, &r.gears[1].parameters], evaluated),
            Self::Edit {
                parameters,
                gears: Some(gears),
                ..
            } => parameters.resolve_recipes([&gears[0], &gears[1]], evaluated),
            Self::Edit { parameters, .. } => {
                model::resolve_gear_pair_parameters(document, evaluated, parameters)
            }
        }
        .map_err(|e| e.to_string())
    }
    pub(crate) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        let prepared = match self {
            Self::Create(r) => model::prepare_gear_pair(session, session.revision(), actor, r),
            Self::Edit {
                pair,
                parameters,
                gears: Some(gears),
            } => model::prepare_gear_pair_geometry_edit(
                session,
                session.revision(),
                actor,
                *pair,
                parameters,
                gears,
            ),
            Self::Edit {
                pair, parameters, ..
            } => {
                model::prepare_gear_pair_edit(session, session.revision(), actor, *pair, parameters)
            }
        }
        .map_err(|e| e.to_string())?;
        Ok(ModelPrepared {
            prepared: prepared.prepared,
            outputs: prepared.outputs,
        })
    }
}

#[derive(Clone)]
pub(super) struct GearPairPanel {
    primary: GearPanel,
    template: GearPairRequest,
    name: String,
    secondary: [DimensionField; 4],
    secondary_shift: DimensionField,
    secondary_hub: HubFields,
    secondary_ring: bool,
    rack_member: Option<usize>,
    worm_member: Option<usize>,
    secondary_width: DimensionField,
    secondary_backing: DimensionField,
    secondary_outside: DimensionField,
    origin: [DimensionField; 3],
    plane: Option<Datum>,
    drive: DimensionField,
    editing: bool,
    update_geometry: bool,
    single_requested: bool,
    stamp: FormStamp,
}
impl GearPairPanel {
    pub fn from_single(mut primary: GearPanel) -> Self {
        let origin = primary.origin.clone();
        let plane = primary.plane;
        let frame = primary.request.parameters.frame.clone();
        primary.request.parameters.frame = Default::default();
        primary.plane = Some(Datum::Xy);
        primary.origin = std::array::from_fn(|_| {
            DimensionField::from_dimension(&model::length(0.), "mm").expect("zero origin")
        });
        let mut second = primary.request.clone();
        second.feature = EntityId::new();
        second.body = EntityId::new();
        second.parameters.teeth = multicad_core::constraint_sketch::Dimension::Literal {
            value: if primary.family == GearFamily::InternalRing {
                24.
            } else {
                36.
            },
            unit: "count".into(),
        };
        let mut template =
            GearPairRequest::new(&primary.request.name, [primary.request.clone(), second]);
        template.parameters.frame = frame;
        let rack_member = (primary.family == GearFamily::Rack).then_some(0);
        let worm_member = (primary.family == GearFamily::Worm).then_some(
            if primary.worm_role == WormMember::Worm {
                0
            } else {
                1
            },
        );
        let secondary_width = DimensionField::from_dimension(
            &model::length(if worm_member == Some(1) { 24. } else { 1. }),
            "mm",
        )
        .expect("worm pair width");
        let secondary_backing = primary.rack_backing.clone();
        let secondary_shift = if primary.shift_mate_enabled {
            primary.shift_mate_shift.clone()
        } else {
            DimensionField::from_dimension(&model::ratio(0.), "ratio").expect("mate shift")
        };
        let secondary_hub = HubFields::new(None).expect("hub defaults");
        let secondary = [
            if primary.family == GearFamily::Worm {
                primary.worm_mate.clone()
            } else if primary.family == GearFamily::Spur && primary.shift_mate_enabled {
                primary.shift_mate_teeth.clone()
            } else if primary.family == GearFamily::Bevel {
                primary.bevel_mate.clone()
            } else {
                DimensionField::from_dimension(&template.gears[1].parameters.teeth, "count")
                    .expect("preset count")
            },
            primary.fields[6].clone(),
            primary.fields[7].clone(),
            primary.fields[8].clone(),
        ];
        Self {
            name: primary.request.name.clone(),
            primary,
            template,
            secondary,
            secondary_shift,
            secondary_hub,
            secondary_ring: false,
            rack_member,
            worm_member,
            secondary_width,
            secondary_backing,
            secondary_outside: DimensionField::from_dimension(&model::length(112.), "mm")
                .expect("ring outside"),
            origin,
            plane,
            drive: DimensionField::from_dimension(&angle(0.), "deg").expect("zero drive"),
            editing: false,
            update_geometry: true,
            single_requested: false,
            stamp: FormStamp::new(),
        }
    }
    pub fn existing(
        document: &Document,
        pair: EntityId,
        evaluated: EvaluatedParameters,
    ) -> Result<Self, String> {
        let source = model::read_gear_pair(document, pair).map_err(|e| e.to_string())?;
        let mut requests = Vec::new();
        for feature in source.parameters.gears {
            let gear = model::read_gear(document, feature).map_err(|e| e.to_string())?;
            requests.push(GearRequest {
                feature,
                body: gear.output.body,
                name: document
                    .get(gear.output.body)
                    .ok_or("Gear Part is missing")?
                    .name
                    .clone(),
                parameters: gear.parameters,
            });
        }
        let requests: [GearRequest; 2] =
            requests.try_into().map_err(|_| "Pair needs two recipes")?;
        let primary = GearPanel::new(requests[0].clone(), evaluated, true)?;
        let mut template = GearPairRequest::new("Pair", requests);
        template.pair = pair;
        template.parameters = source.parameters;
        template.outputs = source.outputs;
        // The outer placement is not part of a definition edit.
        let field = DimensionField::from_dimension;
        let b = &template.gears[1].parameters;
        Ok(Self {
            secondary_shift: field(&b.profile_shift, "ratio")?,
            secondary_hub: HubFields::new(b.hub.as_ref())?,
            secondary_ring: b.family == GearFamily::InternalRing,
            worm_member: template.gears[0]
                .parameters
                .worm
                .as_ref()
                .map(|w| if w.role == WormMember::Worm { 0 } else { 1 }),
            secondary_width: field(&b.face_width, "mm")?,
            rack_member: if template.gears[0].parameters.family == GearFamily::Rack {
                Some(0)
            } else if b.family == GearFamily::Rack {
                Some(1)
            } else {
                None
            },
            secondary_backing: field(
                b.rack
                    .as_ref()
                    .map(|rack| &rack.backing_thickness)
                    .unwrap_or(&model::length(4.)),
                "mm",
            )?,
            secondary_outside: field(
                b.outer_diameter.as_ref().unwrap_or(&model::length(112.)),
                "mm",
            )?,
            name: document
                .get(pair)
                .ok_or("Gear pair is missing")?
                .name
                .clone(),
            secondary: [
                field(&b.teeth, "count")?,
                field(&b.bore_diameter, "mm")?,
                field(&b.backlash, "mm")?,
                field(&b.fabrication_allowance, "mm")?,
            ],
            origin: [
                field(&template.parameters.frame.origin[0], "mm")?,
                field(&template.parameters.frame.origin[1], "mm")?,
                field(&template.parameters.frame.origin[2], "mm")?,
            ],
            plane: Datum::ALL.into_iter().find(|d| {
                d.frame().u == template.parameters.frame.u
                    && d.frame().v == template.parameters.frame.v
            }),
            drive: field(&template.parameters.drive_angle, "deg")?,
            primary,
            template,
            editing: true,
            update_geometry: false,
            single_requested: false,
            stamp: FormStamp::new(),
        })
    }
    pub fn primary_clone(&self) -> GearPanel {
        self.primary.clone()
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn single_requested(&self) -> bool {
        self.single_requested
    }
    pub fn as_single(&self) -> GearPanel {
        let mut p = self.primary.clone();
        p.pair = None;
        p.request.name = self.name.clone();
        p.request.parameters.frame = self.template.parameters.frame.clone();
        p.origin = self.origin.clone();
        p.plane = self.plane;
        if p.family == GearFamily::Spur {
            p.shift_mate_enabled |= p.shift.text.trim().parse::<f64>() != Ok(0.)
                || self.secondary_shift.text.trim().parse::<f64>() != Ok(0.);
            if p.shift_mate_enabled {
                p.shift_mate_teeth = self.secondary[0].clone();
                p.shift_mate_shift = self.secondary_shift.clone();
            }
        }
        if p.family == GearFamily::Worm {
            p.worm_mate = self.secondary[0].clone();
        }
        if p.family == GearFamily::Bevel {
            p.bevel_mate = self.secondary[0].clone();
        }
        p.stamp = FormStamp::new();
        p
    }
    fn set_worm_member(&mut self, member: Option<usize>) {
        if self.worm_member == member {
            return;
        }
        if self.worm_member.is_some() && member.is_some() {
            for (a, b) in [(1, 0), (6, 1), (7, 2), (8, 3)] {
                std::mem::swap(&mut self.primary.fields[a], &mut self.secondary[b]);
            }
            std::mem::swap(&mut self.primary.fields[3], &mut self.secondary_width);
        } else if let Some(index) = member {
            self.rack_member = None;
            self.secondary_ring = false;
            self.primary.select_family(GearFamily::Worm);
            self.primary.fields[1].set_literal(if index == 0 { 1. } else { 32. }, "count");
            self.primary.fields[3].set_literal(if index == 0 { 24. } else { 1. }, "mm");
            self.secondary[0].set_literal(if index == 0 { 32. } else { 1. }, "count");
            self.secondary[1].set_literal(0., "mm");
            self.secondary[2].set_literal(0.05, "mm");
            self.secondary[3].set_literal(0., "mm");
            self.secondary_width
                .set_literal(if index == 0 { 1. } else { 24. }, "mm");
        }
        self.worm_member = member;
        if let Some(index) = member {
            self.secondary_shift.set_literal(0., "ratio");
            self.primary.worm_role = if index == 0 {
                WormMember::Worm
            } else {
                WormMember::Wheel
            };
        } else if self.primary.family == GearFamily::Worm {
            self.primary.select_family(GearFamily::Spur);
        }
    }
    fn set_rack_member(&mut self, member: Option<usize>) {
        if self.rack_member == member {
            return;
        }
        if self.rack_member.is_some() && member.is_some() {
            // Move the role-specific authored fields with the rack/pinion roles,
            // while retaining the pair's member and output identities.
            for (primary, secondary) in [(1, 0), (6, 1), (7, 2), (8, 3)] {
                std::mem::swap(
                    &mut self.primary.fields[primary],
                    &mut self.secondary[secondary],
                );
            }
            std::mem::swap(&mut self.primary.rack_backing, &mut self.secondary_backing);
        }
        self.rack_member = member;
        if let Some(index) = member {
            self.primary.shift.set_literal(0., "ratio");
            self.primary.shift_mate_enabled = false;
            self.secondary_shift.set_literal(0., "ratio");
            self.secondary_ring = false;
            self.primary.select_family(if index == 0 {
                GearFamily::Rack
            } else {
                GearFamily::Spur
            });
        } else if self.primary.family == GearFamily::Rack {
            self.primary.select_family(GearFamily::Spur);
        }
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<GearPairCommand, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.name.trim().is_empty() || self.name.len() > 1000 {
            return Err("Enter a pair name with 1–1000 bytes".into());
        }
        let mut parameters = self.template.parameters.clone();
        parameters.drive_angle = self.drive.resolve(document)?;
        parameters.frame.origin = [
            self.origin[0].resolve(document)?,
            self.origin[1].resolve(document)?,
            self.origin[2].resolve(document)?,
        ];
        if let Some(datum) = self.plane {
            let frame = datum.frame();
            parameters.frame.u = frame.u;
            parameters.frame.v = frame.v;
        }
        let command = if self.editing && !self.update_geometry {
            GearPairCommand::Edit {
                pair: self.template.pair,
                parameters: Box::new(parameters),
                gears: None,
            }
        } else {
            let first = self.primary.authored_request(document, enabled)?.parameters;
            let mut second = GearParameters {
                hub: self.secondary_hub.resolve(document)?,
                profile_shift: self.secondary_shift.resolve(document)?,
                shift_mate: None,
                worm: if first.family == GearFamily::Worm {
                    Some(WormDimensions {
                        role: if self.worm_member == Some(0) {
                            WormMember::Wheel
                        } else {
                            WormMember::Worm
                        },
                        tooth_system: self.primary.worm_system,
                        mate_teeth: first.teeth.clone(),
                        diameter_quotient: first
                            .worm
                            .as_ref()
                            .ok_or("Worm dimensions are missing")?
                            .diameter_quotient
                            .clone(),
                    })
                } else {
                    None
                },
                face_width: if first.family == GearFamily::Worm {
                    self.secondary_width.resolve(document)?
                } else {
                    first.face_width.clone()
                },
                family: if let Some(index) = self.rack_member {
                    if index == 1 {
                        GearFamily::Rack
                    } else {
                        GearFamily::Spur
                    }
                } else if first.family == GearFamily::InternalRing {
                    GearFamily::Spur
                } else if self.secondary_ring && first.family == GearFamily::Spur {
                    GearFamily::InternalRing
                } else {
                    first.family
                },
                rack: if self.rack_member == Some(1) {
                    Some(model::RackDimensions {
                        backing_thickness: self.secondary_backing.resolve(document)?,
                    })
                } else {
                    None
                },
                outer_diameter: if self.rack_member.is_none()
                    && self.secondary_ring
                    && first.family == GearFamily::Spur
                {
                    Some(self.secondary_outside.resolve(document)?)
                } else {
                    None
                },
                teeth: self.secondary[0].resolve(document)?,
                bore_diameter: if self.rack_member == Some(1) {
                    model::length(0.)
                } else {
                    self.secondary[1].resolve(document)?
                },
                backlash: self.secondary[2].resolve(document)?,
                fabrication_allowance: self.secondary[3].resolve(document)?,
                hand: if first.family == GearFamily::Worm {
                    GearHand::Right
                } else {
                    match first.hand {
                        GearHand::None => GearHand::None,
                        GearHand::Left => GearHand::Right,
                        GearHand::Right => GearHand::Left,
                    }
                },
                frame: self.template.gears[1].parameters.frame.clone(),
                ..first.clone()
            };
            if self.editing {
                let old = &self.template.gears[0].parameters;
                let other = &self.template.gears[1].parameters;
                if first.family == old.family && second.family == other.family {
                    if first.pitch_input == old.pitch_input
                        && first.module == old.module
                        && first.module_system == old.module_system
                    {
                        second.pitch_input = other.pitch_input;
                        second.module = other.module.clone();
                        second.module_system = other.module_system;
                    }
                    if first.pressure_angle == old.pressure_angle {
                        second.pressure_angle = other.pressure_angle.clone();
                    }
                    if first.helix_angle == old.helix_angle {
                        second.helix_angle = other.helix_angle.clone();
                    }
                    if first.family != GearFamily::Worm && first.face_width == old.face_width {
                        second.face_width = other.face_width.clone();
                    }
                    if first.center_gap == old.center_gap {
                        second.center_gap = other.center_gap.clone();
                    }
                    if first.root_fillet_ratio == old.root_fillet_ratio {
                        second.root_fillet_ratio = other.root_fillet_ratio.clone();
                    }
                    if first.minimum_wall == old.minimum_wall {
                        second.minimum_wall = other.minimum_wall.clone();
                    }
                }
            }
            let [first, second] = if first.family == GearFamily::Worm {
                model::couple_worm_pair_recipes(&[first, second]).map_err(|e| e.to_string())?
            } else if first.family == GearFamily::Bevel {
                model::couple_bevel_pair_recipes(
                    &[first, second],
                    &self.primary.bevel_shaft.resolve(document)?,
                )
                .map_err(|e| e.to_string())?
            } else {
                [first, second]
            };
            let mut recipes = [first, second];
            if recipes[0].family == GearFamily::Spur
                && (self.primary.shift_mate_enabled
                    || self
                        .template
                        .gears
                        .iter()
                        .any(|g| g.parameters.shift_mate.is_some()))
            {
                let original = recipes.clone();
                for i in 0..2 {
                    recipes[i].shift_mate = Some(model::ShiftedSpurMate {
                        teeth: original[1 - i].teeth.clone(),
                        profile_shift: original[1 - i].profile_shift.clone(),
                    });
                }
            }
            model::couple_shifted_spur_pair_recipes(&mut recipes).map_err(|e| e.to_string())?;
            let [first, second] = recipes;
            if self.editing {
                GearPairCommand::Edit {
                    pair: self.template.pair,
                    parameters: Box::new(parameters),
                    gears: Some(Box::new([first, second])),
                }
            } else {
                let mut request = self.template.clone();
                request.name = self.name.clone();
                request.placement.name = self.name.clone();
                request.parameters = parameters;
                request.gears[0].parameters = first;
                request.gears[1].parameters = second;
                request.gears[0].name = format!("{} A", self.name);
                request.gears[1].name = format!("{} B", self.name);
                GearPairCommand::Create(Box::new(request))
            }
        };
        command.resolved(document, &self.primary.parameters)?;
        Ok(command)
    }
    fn show_travel_limits(ui: &mut egui::Ui, travel: &model::ResolvedRackTravel) {
        ui.small(format!(
            "Conservative finite travel {:.4} to {:.4} mm · pinion drive {:.4} to {:.4}°",
            travel.min_translation_mm,
            travel.max_translation_mm,
            travel.min_drive_angle_rad.to_degrees(),
            travel.max_drive_angle_rad.to_degrees()
        ));
        ui.small("The full pinion tip-circle projection must stay within the finite rack ends. Out-of-range drive is refused.");
    }
    fn show_resolved(ui: &mut egui::Ui, resolved: &ResolvedGearPair) {
        if let (Some(a), Some(b)) = (resolved.gears[0].rotary(), resolved.gears[1].rotary())
            && a.family == GearFamily::Spur
            && b.family == GearFamily::Spur
            && (a.profile_shift_coefficient != 0. || b.profile_shift_coefficient != 0.)
            && let Ok(shift) = model::shifted_spur_pair_geometry(a, b)
        {
            ui.small(format!("Shifted operating pressure {:.4}° · center {:.4} mm · contact ratio {:.4} · tip shortening {:.6}",shift.working_pressure_angle_rad.to_degrees(),shift.center_distance_mm,shift.contact_ratio,shift.tip_shortening_coefficient));
        }
        if let Some(worm) = &resolved.worm {
            ui.small(format!("ZA worm {:?} / wheel {:?} · crossed axes 90° · center distance {:.4} mm · one start : {} wheel teeth",worm.worm_member,worm.wheel_member,worm.center_distance_mm,worm.wheel_teeth));
            ui.small("Drive is the worm angle for either member order. Native proximity is a geometry witness, not a load or manufactured-contact certificate.");
        } else if let Some(travel) = &resolved.rack_travel {
            ui.small(format!(
                "Rack {:?} / pinion {:?} · translation {:.4} mm",
                travel.rack_member, travel.pinion_member, travel.translation_mm
            ));
            Self::show_travel_limits(ui, travel);
        } else if let Some(bevel) = &resolved.bevel {
            ui.small(format!(
                "Shared pitch apex · shaft {:.4}° · pitch cones {:.4}° / {:.4}° · ratio {} : {}",
                bevel.shaft_angle_rad.to_degrees(),
                bevel.pitch_cone_angles_rad[0].to_degrees(),
                bevel.pitch_cone_angles_rad[1].to_degrees(),
                resolved.gears[0].teeth(),
                resolved.gears[1].teeth()
            ));
            ui.small(format!(
                "Outer cone distance {:.4} mm · pitch-cone face width {:.4} mm",
                bevel.cone_distance_mm, bevel.face_width_mm
            ));
        } else {
            ui.small(format!(
                "Center distance {:.4} mm · {} : {} teeth",
                resolved.center_distance_mm,
                resolved.gears[0].teeth(),
                resolved.gears[1].teeth()
            ));
        }
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState,
    ) -> Option<ModelEvent> {
        ui.strong(if self.editing {
            "Edit gear pair"
        } else {
            "Mating gear pair"
        });
        form::preview_status(ui, self.stamp, preview);
        if !self.editing && ui.button("Single gear").clicked() {
            self.single_requested = true;
            return Some(ModelEvent::Changed);
        }
        ui.label("Pair name");
        let mut changed = ui
            .add_enabled(
                !self.editing,
                egui::TextEdit::singleline(&mut self.name)
                    .id(ui.make_persistent_id((self.template.pair, "pair-name")))
                    .desired_width(ui.available_width()),
            )
            .changed();
        if self.editing {
            changed |= ui
                .checkbox(&mut self.update_geometry, "Update both gear recipes")
                .changed();
            ui.small("Leave this off to change only the drive or pair layout. Enabling it applies the shared geometry controls to both gears.");
        }
        if self.update_geometry {
            ui.label("Pair type");
            let old_kind = if self.worm_member.is_some() {
                2
            } else if self.rack_member.is_some() {
                1
            } else {
                0
            };
            let mut kind = old_kind;
            controls::inspector_combo(ui, "gear-pair-type")
                .selected_text(match kind {
                    2 => "ZA worm and wheel",
                    1 => "Rack and pinion",
                    _ => "Rotary gears",
                })
                .show_ui(ui, |ui| {
                    changed |= ui.selectable_value(&mut kind, 0, "Rotary gears").changed();
                    changed |= ui
                        .selectable_value(&mut kind, 1, "Rack and pinion")
                        .changed();
                    if ui
                        .add_enabled(
                            GearFamily::Worm.supported(),
                            egui::Button::selectable(kind == 2, "ZA worm and wheel"),
                        )
                        .clicked()
                    {
                        kind = 2;
                        changed = true;
                    }
                });
            if kind != old_kind {
                self.set_worm_member(None);
                self.set_rack_member(None);
                if kind == 2 {
                    self.set_worm_member(Some(0));
                } else if kind == 1 {
                    self.set_rack_member(Some(0));
                }
            }
            if let Some(index) = self.worm_member {
                let mut member = index;
                ui.label("Worm member / wheel member");
                changed |= ui.radio_value(&mut member, 0, "Worm A / Wheel B").changed();
                changed |= ui.radio_value(&mut member, 1, "Wheel A / Worm B").changed();
                if member != index {
                    self.set_worm_member(Some(member));
                }
            }
            if let Some(index) = self.rack_member {
                let mut member = index;
                ui.label("Rack member / pinion member");
                changed |= ui
                    .radio_value(&mut member, 0, "Rack A / Pinion B")
                    .changed();
                changed |= ui
                    .radio_value(&mut member, 1, "Pinion A / Rack B")
                    .changed();
                if member != index {
                    self.set_rack_member(Some(member));
                }
            }
            ui.strong("Shared geometry and first member");
            if self.rack_member.is_some() {
                ui.small(if self.rack_member == Some(0) {
                    "Member A: straight rack"
                } else {
                    "Member A: spur pinion"
                });
            }
            let previous_family = self.primary.family;
            changed |=
                self.primary
                    .geometry_controls_with_family(ui, self.rack_member.is_none(), true);
            if self.primary.family == GearFamily::Worm && self.worm_member.is_none() {
                self.set_worm_member(Some(0));
                changed = true;
            }
            if self.primary.family != GearFamily::Worm {
                self.worm_member = None;
            }
            if previous_family != self.primary.family && !external_cylindrical(self.primary.family)
            {
                self.secondary_shift.set_literal(0., "ratio");
            }
            if self.primary.family == GearFamily::Rack && self.rack_member.is_none() {
                self.set_rack_member(Some(0));
                changed = true;
            }
            if self.primary.family == GearFamily::InternalRing
                && previous_family != self.primary.family
            {
                self.secondary_ring = false;
                self.secondary[0] = DimensionField::from_dimension(
                    &multicad_core::constraint_sketch::Dimension::Literal {
                        value: 24.,
                        unit: "count".into(),
                    },
                    "count",
                )
                .expect("pinion teeth");
            }
            ui.separator();
            if self.rack_member.is_none()
                && self.primary.family == GearFamily::Spur
                && ui
                    .checkbox(&mut self.secondary_ring, "Internal ring as second member")
                    .changed()
            {
                changed = true;
                self.secondary_shift.set_literal(0., "ratio");
                self.secondary[0] = DimensionField::from_dimension(
                    &multicad_core::constraint_sketch::Dimension::Literal {
                        value: if self.secondary_ring { 48. } else { 36. },
                        unit: "count".into(),
                    },
                    "count",
                )
                .expect("mating teeth");
                self.secondary[1] = DimensionField::from_dimension(
                    &model::length(if self.secondary_ring { 0. } else { 8. }),
                    "mm",
                )
                .expect("mating bore");
            }
            let rack_second = self.rack_member == Some(1);
            let ring = self.rack_member.is_none()
                && self.primary.family == GearFamily::Spur
                && self.secondary_ring;
            ui.strong(if self.worm_member == Some(1) {
                "Mating ZA worm"
            } else if self.worm_member == Some(0) {
                "Mating generated worm wheel"
            } else if rack_second {
                "Mating straight rack"
            } else if self.rack_member == Some(0) {
                "Mating spur pinion"
            } else if ring {
                "Mating internal ring"
            } else if self.primary.family == GearFamily::InternalRing {
                "Mating pinion"
            } else {
                "Mating wheel"
            });
            for (index, (field, label)) in self
                .secondary
                .iter_mut()
                .zip([
                    if self.worm_member == Some(1) {
                        "Worm starts"
                    } else if rack_second {
                        "Rack pitch cells"
                    } else if self.rack_member.is_some() {
                        "Pinion teeth"
                    } else {
                        "Wheel teeth"
                    },
                    if self.worm_member == Some(1) {
                        "Worm bore diameter"
                    } else {
                        "Wheel bore diameter"
                    },
                    if self.worm_member == Some(1) {
                        "Worm design backlash"
                    } else {
                        "Wheel design backlash"
                    },
                    if self.worm_member == Some(1) {
                        "Worm fabrication clearance"
                    } else {
                        "Wheel fabrication clearance"
                    },
                ])
                .enumerate()
            {
                if (ring || rack_second) && label == "Wheel bore diameter" {
                    continue;
                }
                changed |= field.show_named(
                    ui,
                    (self.template.gears[1].feature, "gear-secondary", index),
                    label,
                    "Independent wheel value; formulas may reference project parameters",
                );
            }
            if external_cylindrical(self.primary.family) && !ring {
                changed|=self.secondary_shift.show_named(ui,(self.template.gears[1].feature,"gear-shift"),"Mating profile shift coefficient","Independent sourced Spur coefficient; matched contexts clone original opposite count/shift sources");
                ui.small("Shifted pairs derive the KHK operating center and paired tip shortening from both authored shifts. Nonzero shifts require external20° Spur; contact ratio and native interference gates remain.");
            }
            changed |= self.secondary_hub.show(
                ui,
                self.template.gears[1].feature,
                external_cylindrical(self.primary.family) && !ring,
                "Mating intrinsic hub",
            );
            if self.worm_member.is_some() {
                changed |= self.secondary_width.show_named(
                    ui,
                    (self.template.gears[1].feature, "worm-width"),
                    if self.worm_member == Some(0) {
                        "Wheel face width"
                    } else {
                        "Threaded worm length"
                    },
                    "Independent axial dimension: wheel 0.5–2 × module; worm 16–32 × module",
                );
            }
            if rack_second {
                changed |= self.secondary_backing.show_named(
                    ui,
                    (self.template.gears[1].feature, "gear-backing"),
                    "Rack backing thickness",
                    "Material below the root line; backing must meet the shared minimum wall",
                );
            }
            if ring {
                changed |= self.secondary_outside.show_named(
                    ui,
                    (self.template.gears[1].feature, "gear-outside"),
                    "Mating ring outside diameter",
                    "Outside rim must meet the shared minimum-wall requirement",
                );
            }
            ui.small(if self.primary.family==GearFamily::Worm {"ZA axial-straight pair: shared axial module, normal20° pressure angle, q10 and right hand. Worm length and wheel face width remain independent. Mate counts follow original opposite typed counts."}else if self.primary.family == GearFamily::Bevel { "Both spherical-involute members share the pitch apex and typed shaft angle. Each mate count follows the opposite authored tooth-count dimension; source geometry updates atomically." } else if self.rack_member.is_some() { "Straight rack and spur pinion share module, pressure angle, face width and custom root ratio. The pinion stays at +Y × pitch radius; the rack translates on local X." } else if ring || self.primary.family == GearFamily::InternalRing {"Ring/pinion share module, 20° pressure angle and face width. Centers use the radius difference; both rotate in the same direction."} else {"The wheel shares the module convention, pressure angle, face width, root profile and center gap. Helix hand is opposite."});
        } else {
            for gear in &self.template.gears {
                ui.small(&gear.name);
            }
        }
        ui.separator();
        ui.strong("Pair layout and drive");
        changed|=self.drive.show_named(ui, (self.template.pair, "pair-drive"),if self.primary.family==GearFamily::Worm {"Worm drive angle"}else if self.primary.family == GearFamily::InternalRing {"Ring drive angle"}else{"Pinion drive angle"},if self.primary.family==GearFamily::Worm {"Worm rotation regardless of member order; wheel follows the signed one-start tooth-count ratio"}else if self.rack_member.is_some() { "Pinion angle regardless of member role: rack translation s = pinion pitch radius × angle in radians. Finite-end travel is bounded." } else { "The second member follows the tooth-count ratio; copies share this definition's drive coordinate" });
        controls::inspector_combo(ui, "gear-pair-plane")
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
            .zip(if self.primary.family == GearFamily::Bevel {
                [
                    "Shared pitch apex X",
                    "Shared pitch apex Y",
                    "Shared pitch apex Z",
                ]
            } else {
                ["Pair origin X", "Pair origin Y", "Pair origin Z"]
            })
            .enumerate()
        {
            changed |= field.show_named(
                ui,
                (self.template.pair, "pair-origin", index),
                label,
                "Layout within the reusable pair; use Move/Copy for an independent outer placement",
            );
        }
        let validation = self.request(document, enabled);
        if let Ok(command) = &validation {
            if let Ok(r) = command.resolved(document, &self.primary.parameters) {
                Self::show_resolved(ui, &r);
            }
        } else if let Err(error) = &validation {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
            if self.rack_member.is_some() {
                let mut neutral = self.clone();
                neutral.drive =
                    DimensionField::from_dimension(&angle(0.), "deg").expect("zero drive");
                if let Ok(command) = neutral.request(document, enabled)
                    && let Ok(resolved) = command.resolved(document, &self.primary.parameters)
                    && let Some(travel) = &resolved.rack_travel
                {
                    Self::show_travel_limits(ui, travel);
                }
            }
        }
        ui.small("Preview checks the current mating pose and sampled phases. It does not certify continuous motion, strength or physical fit.");
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
                    .map(|command| ModelEvent::Preview {
                        stamp: self.stamp,
                        request: ModelRequest::GearPair(Box::new(command)),
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
mod rack_roles_tests {
    use super::*;
    /// Widget-only status injection exercises layout changes, not native proof.
    fn typing_frame(
        ctx: &egui::Context,
        panel: &mut GearPairPanel,
        document: &Document,
        events: Vec<egui::Event>,
        status: usize,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                focused: true,
                events,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(650., 2400.),
                )),
                ..Default::default()
            },
            |ui| {
                let preview = match status {
                    1 => PreviewState::Pending {
                        stamp: panel.stamp(),
                    },
                    2 => PreviewState::Ready {
                        stamp: panel.stamp(),
                    },
                    3 => PreviewState::Failed {
                        stamp: panel.stamp(),
                        message: "Test-only preview-status layout change",
                    },
                    _ => PreviewState::Idle,
                };
                panel.show(ui, document, true, preview);
            },
        )
    }
    #[test]
    fn rack_cell_typing_keeps_focus_through_partial_invalid_text_and_preview_status_changes() {
        for member in [0, 1] {
            let session = Session::new(Document::new("Typing is source neutral").unwrap()).unwrap();
            let before = session.document().clone();
            let mut primary = GearPanel::new(
                GearRequest::new("Pair", GearParameters::default()),
                session.parameters().clone(),
                false,
            )
            .unwrap();
            primary.select_family(GearFamily::Rack);
            let mut panel = GearPairPanel::from_single(primary);
            panel.set_rack_member(Some(member));
            let GearPairCommand::Create(original) =
                panel.request(session.document(), true).unwrap()
            else {
                panic!("create")
            };
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            typing_frame(&ctx, &mut panel, session.document(), vec![], 0)
                .drop_without_applying_deltas();
            let output = typing_frame(&ctx, &mut panel, session.document(), vec![], 0);
            let label_y = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Rack pitch cells (count)" => {
                        Some(text.pos.y)
                    }
                    _ => None,
                })
                .unwrap();
            let bounds = output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .filter_map(|(_, node)| {
                    (node.role() == egui::accesskit::Role::TextInput)
                        .then(|| node.bounds())
                        .flatten()
                })
                .filter(|bounds| bounds.y0 > f64::from(label_y))
                .min_by(|a, b| a.y0.total_cmp(&b.y0))
                .unwrap();
            let pos = egui::pos2(
                ((bounds.x0 + bounds.x1) / 2.) as f32,
                ((bounds.y0 + bounds.y1) / 2.) as f32,
            );
            output.drop_without_applying_deltas();
            for pressed in [true, false] {
                typing_frame(
                    &ctx,
                    &mut panel,
                    session.document(),
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                    0,
                )
                .drop_without_applying_deltas();
            }
            let focused = ctx
                .memory(|memory| memory.focused())
                .expect("cell input focused");
            let key = |key, modifiers| egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            };
            for blank_first in [false, true] {
                typing_frame(
                    &ctx,
                    &mut panel,
                    session.document(),
                    vec![key(egui::Key::A, egui::Modifiers::COMMAND)],
                    0,
                )
                .drop_without_applying_deltas();
                if blank_first {
                    typing_frame(
                        &ctx,
                        &mut panel,
                        session.document(),
                        vec![key(egui::Key::Backspace, Default::default())],
                        0,
                    )
                    .drop_without_applying_deltas();
                    assert!(panel.request(session.document(), true).is_err());
                    typing_frame(&ctx, &mut panel, session.document(), vec![], 3)
                        .drop_without_applying_deltas();
                    assert_eq!(ctx.memory(|memory| memory.focused()), Some(focused));
                }
                typing_frame(
                    &ctx,
                    &mut panel,
                    session.document(),
                    vec![egui::Event::Text("2".into())],
                    0,
                )
                .drop_without_applying_deltas();
                assert!(panel.request(session.document(), true).is_err());
                typing_frame(&ctx, &mut panel, session.document(), vec![], 1)
                    .drop_without_applying_deltas();
                assert_eq!(ctx.memory(|memory| memory.focused()), Some(focused));
                typing_frame(
                    &ctx,
                    &mut panel,
                    session.document(),
                    vec![egui::Event::Text("1".into())],
                    2,
                )
                .drop_without_applying_deltas();
                assert_eq!(ctx.memory(|memory| memory.focused()), Some(focused));
                let GearPairCommand::Create(edited) =
                    panel.request(session.document(), true).unwrap()
                else {
                    panic!("create")
                };
                assert_eq!(
                    edited.gears[member].parameters.teeth,
                    multicad_core::constraint_sketch::Dimension::Literal {
                        value: 21.,
                        unit: "count".into()
                    }
                );
                assert_eq!(edited.pair, original.pair);
                assert_eq!(edited.outputs, original.outputs);
                assert_eq!(edited.placement, original.placement);
                assert_eq!(
                    [edited.gears[0].feature, edited.gears[1].feature],
                    [original.gears[0].feature, original.gears[1].feature]
                );
                assert_eq!(session.document(), &before);
                assert_eq!(session.revision(), 0);
            }
        }
    }
    #[test]
    fn rack_role_switch_keeps_role_dimensions_member_ids_and_reports_finite_drive_refusal() {
        let session = Session::new(Document::new("Rack roles").unwrap()).unwrap();
        let mut primary = GearPanel::new(
            GearRequest::new("Pair", GearParameters::default()),
            session.parameters().clone(),
            false,
        )
        .unwrap();
        primary.select_family(GearFamily::Rack);
        primary.fields[1].set_literal(32., "count");
        primary.rack_backing.set_literal(5., "mm");
        let mut panel = GearPairPanel::from_single(primary);
        let GearPairCommand::Create(a) = panel.request(session.document(), true).unwrap() else {
            panic!("create")
        };
        assert_eq!(a.gears[0].parameters.family, GearFamily::Rack);
        assert_eq!(a.gears[1].parameters.family, GearFamily::Spur);
        let resolved_a = a
            .parameters
            .resolve_recipes(
                [&a.gears[0].parameters, &a.gears[1].parameters],
                session.parameters(),
            )
            .unwrap();
        let rack_a = resolved_a.gears[0].rack().unwrap();
        panel.set_rack_member(Some(1));
        let GearPairCommand::Create(b) = panel.request(session.document(), true).unwrap() else {
            panic!("create")
        };
        assert_eq!(a.pair, b.pair);
        assert_eq!(a.outputs, b.outputs);
        assert_eq!(a.placement.instance, b.placement.instance);
        assert_eq!(
            [a.gears[0].feature, a.gears[1].feature],
            [b.gears[0].feature, b.gears[1].feature]
        );
        assert_eq!(b.gears[0].parameters.family, GearFamily::Spur);
        assert_eq!(b.gears[1].parameters.family, GearFamily::Rack);
        let resolved_b = b
            .parameters
            .resolve_recipes(
                [&b.gears[0].parameters, &b.gears[1].parameters],
                session.parameters(),
            )
            .unwrap();
        let rack_b = resolved_b.gears[1].rack().unwrap();
        assert_eq!(rack_a.teeth, rack_b.teeth);
        assert_eq!(rack_a.length_mm, rack_b.length_mm);
        assert_eq!(rack_a.backing_thickness_mm, rack_b.backing_thickness_mm);
        let travel = resolved_b.rack_travel.unwrap();
        assert_eq!(travel.rack_member, model::GearPairMember::B);
        assert!(travel.min_translation_mm < 0. && travel.max_translation_mm > 0.);
        panel.drive.set_literal(1000., "deg");
        assert!(
            panel
                .request(session.document(), true)
                .unwrap_err()
                .contains("finite end travel")
        );
        assert_eq!(session.revision(), 0);
        assert!(!session.can_undo());
    }
}

#[cfg(test)]
mod bevel_pair_fields_tests {
    use super::*;
    use multicad_core::{
        constraint_sketch::Dimension,
        document::{Entity, Mutation},
        parameters::BoundExpression,
    };
    #[test]
    fn bevel_pair_clones_opposite_authored_counts_and_shared_shaft_before_resolution() {
        let entities = [
            ("teeth_a", "24", "count"),
            ("teeth_b", "36", "count"),
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
        let document = Document::new("Typed bevel pair")
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
        let a = expression("=$teeth_a", "count");
        let b = expression("=$teeth_b", "count");
        let shaft = expression("=$shaft", "deg");
        let primary = GearPanel::new(
            GearRequest::new(
                "Pair",
                GearParameters {
                    family: GearFamily::Bevel,
                    module: model::length(1.),
                    face_width: model::length(2.),
                    bore_diameter: model::length(2.),
                    teeth: a.clone(),
                    bevel: Some(model::BevelDimensions {
                        tooth_system: model::BevelToothSystem::SphericalInvoluteStraight,
                        mate_teeth: b.clone(),
                        shaft_angle: shaft.clone(),
                    }),
                    ..Default::default()
                },
            ),
            session.parameters().clone(),
            false,
        )
        .unwrap();
        let mut panel = GearPairPanel::from_single(primary);
        let GearPairCommand::Create(first) = panel.request(&document, true).unwrap() else {
            panic!("create")
        };
        assert_eq!(first.gears[0].parameters.teeth, a);
        assert_eq!(first.gears[1].parameters.teeth, b);
        for (i, mate) in [b.clone(), a.clone()].iter().enumerate() {
            let bevel = first.gears[i].parameters.bevel.as_ref().unwrap();
            assert_eq!(&bevel.mate_teeth, mate);
            assert_eq!(bevel.shaft_angle, shaft);
        }
        panel.secondary[0].set_literal(48., "count");
        let GearPairCommand::Create(edited) = panel.request(&document, true).unwrap() else {
            panic!("create")
        };
        assert_eq!(
            edited.gears[0]
                .parameters
                .bevel
                .as_ref()
                .unwrap()
                .mate_teeth,
            edited.gears[1].parameters.teeth
        );
        assert_eq!(
            edited.gears[1]
                .parameters
                .bevel
                .as_ref()
                .unwrap()
                .mate_teeth,
            a
        );
        assert_eq!(edited.pair, first.pair);
        assert_eq!(edited.outputs, first.outputs);
        assert_eq!(edited.placement, first.placement);
        assert_eq!(edited.gears[0].feature, first.gears[0].feature);
        assert_eq!(edited.gears[1].feature, first.gears[1].feature);
        let single = panel.as_single();
        assert_eq!(
            single
                .request(&document, true)
                .unwrap()
                .parameters
                .bevel
                .as_ref()
                .unwrap()
                .mate_teeth,
            edited.gears[1].parameters.teeth
        );
        panel.primary.bevel_shaft.set_literal(80., "deg");
        assert!(panel.request(&document, true).is_err());
        assert_eq!(session.document(), &document);
        assert_eq!(session.revision(), 0);
        assert!(!session.can_undo());
    }
}

#[cfg(test)]
mod worm_pair_fields_tests {
    use super::*;
    #[test]
    fn worm_roles_keep_independent_lengths_and_original_ids_without_source_edits() {
        let s = Session::new(Document::new("Worm roles").unwrap()).unwrap();
        let mut primary = GearPanel::new(
            GearRequest::new("ZA", Default::default()),
            s.parameters().clone(),
            false,
        )
        .unwrap();
        primary.select_family(GearFamily::Worm);
        let mut p = GearPairPanel::from_single(primary);
        p.secondary[0].set_literal(40., "count");
        p.primary.fields[3].set_literal(20., "mm");
        p.secondary_width.set_literal(1.5, "mm");
        let GearPairCommand::Create(a) = p.request(s.document(), true).unwrap() else {
            panic!("create")
        };
        let resolved = a
            .parameters
            .resolve_recipes(
                [&a.gears[0].parameters, &a.gears[1].parameters],
                s.parameters(),
            )
            .unwrap();
        assert_eq!(
            resolved.worm.as_ref().unwrap().worm_member,
            model::GearPairMember::A
        );
        assert_eq!(resolved.gears[0].worm().unwrap().face_width_mm, 20.);
        assert_eq!(resolved.gears[1].worm().unwrap().face_width_mm, 1.5);
        p.set_worm_member(Some(1));
        let GearPairCommand::Create(b) = p.request(s.document(), true).unwrap() else {
            panic!("create")
        };
        assert_eq!(a.pair, b.pair);
        assert_eq!(a.outputs, b.outputs);
        assert_eq!(a.placement, b.placement);
        assert_eq!(
            a.gears.each_ref().map(|g| g.feature),
            b.gears.each_ref().map(|g| g.feature)
        );
        let resolved = b
            .parameters
            .resolve_recipes(
                [&b.gears[0].parameters, &b.gears[1].parameters],
                s.parameters(),
            )
            .unwrap();
        assert_eq!(
            resolved.worm.as_ref().unwrap().worm_member,
            model::GearPairMember::B
        );
        assert_eq!(resolved.gears[1].worm().unwrap().face_width_mm, 20.);
        assert_eq!(resolved.gears[0].worm().unwrap().face_width_mm, 1.5);
        assert_eq!(resolved.gears[0].teeth(), 40);
        assert_eq!(resolved.gears[1].teeth(), 1);
        assert_eq!(
            b.gears[0].parameters.worm.as_ref().unwrap().mate_teeth,
            b.gears[1].parameters.teeth
        );
        assert_eq!(
            b.gears[1].parameters.worm.as_ref().unwrap().mate_teeth,
            b.gears[0].parameters.teeth
        );
        let single = p.as_single().request(s.document(), true).unwrap();
        assert_eq!(
            single.parameters.worm.as_ref().unwrap().mate_teeth,
            b.gears[1].parameters.teeth
        );
        p.primary.worm_system = WormToothSystem::Zi;
        assert!(p.request(s.document(), true).is_err());
        p.primary.worm_system = WormToothSystem::ZaAxialStraight;
        p.secondary_width.set_literal(15., "mm");
        assert!(p.request(s.document(), true).is_err());
        assert_eq!(s.revision(), 0);
        assert!(!s.can_undo());
    }
}

#[cfg(test)]
mod coverage_pair_fields_tests {
    use super::*;
    use multicad_core::commands::Channel;
    use multicad_core::{
        constraint_sketch::Dimension,
        document::{Entity, Mutation},
        parameters::BoundExpression,
    };
    #[test]
    fn hub_only_pair_edit_preserves_independent_dp_conventions_and_original_sources() {
        let d = Document::new("Independent pitch sources")
            .unwrap()
            .edited(&[
                Mutation::Insert(Entity::new(
                    "module_a",
                    EntityData::Parameter {
                        expression: "2 mm".into(),
                        unit: "mm".into(),
                        exposed: false,
                        inputs: vec![],
                    },
                )),
                Mutation::Insert(Entity::new(
                    "dp_b",
                    EntityData::Parameter {
                        expression: "12.7 teeth_per_in".into(),
                        unit: "teeth_per_in".into(),
                        exposed: false,
                        inputs: vec![],
                    },
                )),
            ])
            .unwrap();
        let d = d
            .edited(
                &[
                    ("alpha_a", "20 deg"),
                    ("alpha_b", "20 deg"),
                    ("beta_a", "0 deg"),
                    ("beta_b", "0 deg"),
                ]
                .map(|(name, expression)| {
                    Mutation::Insert(Entity::new(
                        name,
                        EntityData::Parameter {
                            expression: expression.into(),
                            unit: "deg".into(),
                            exposed: false,
                            inputs: vec![],
                        },
                    ))
                }),
            )
            .unwrap();
        let mut s = Session::new(d.clone()).unwrap();
        let expression = |text, unit| Dimension::Expression {
            expression: BoundExpression::bind(&d, text, unit, None).unwrap(),
        };
        let alpha_a = Dimension::Expression {
            expression: BoundExpression::bind(&d, "=$alpha_a", "deg", None).unwrap(),
        };
        let alpha_b = Dimension::Expression {
            expression: BoundExpression::bind(&d, "=$alpha_b", "deg", None).unwrap(),
        };
        let beta_a = Dimension::Expression {
            expression: BoundExpression::bind(&d, "=$beta_a", "deg", None).unwrap(),
        };
        let beta_b = Dimension::Expression {
            expression: BoundExpression::bind(&d, "=$beta_b", "deg", None).unwrap(),
        };
        let a = GearParameters {
            pressure_angle: alpha_a,
            helix_angle: beta_a,
            module: expression("=$module_a", "mm"),
            hub: Some(model::GearHub {
                diameter: model::length(20.),
                axial_start: model::length(-3.),
                axial_end: model::length(13.),
            }),
            ..Default::default()
        };
        let b = GearParameters {
            pressure_angle: alpha_b,
            helix_angle: beta_b,
            module: expression("=$dp_b", "teeth_per_in"),
            pitch_input: GearPitchInput::DiametralPitch,
            module_system: GearModuleSystem::Transverse,
            teeth: Dimension::Literal {
                value: 36.,
                unit: "count".into(),
            },
            hub: Some(model::GearHub {
                diameter: model::length(24.),
                axial_start: model::length(0.),
                axial_end: model::length(15.),
            }),
            ..Default::default()
        };
        let request = GearPairRequest::new(
            "Mixed authored sources",
            [
                GearRequest::new("A", a.clone()),
                GearRequest::new("B", b.clone()),
            ],
        );
        let prepared =
            model::prepare_gear_pair(&s, 0, Actor::local(Channel::Ui), &request).unwrap();
        s.commit(prepared.prepared).unwrap();
        let before = s.document().clone();
        let mut panel =
            GearPairPanel::existing(s.document(), request.pair, s.parameters().clone()).unwrap();
        panel.update_geometry = true;
        panel.secondary_hub.dimensions[2].set_literal(17., "mm");
        let GearPairCommand::Edit {
            pair,
            gears: Some(gears),
            ..
        } = panel.request(s.document(), true).unwrap()
        else {
            panic!("grouped geometry edit")
        };
        assert_eq!(pair, request.pair);
        assert_eq!(gears[0].module, a.module);
        assert_eq!(gears[1].module, b.module);
        assert_eq!(gears[1].pitch_input, b.pitch_input);
        assert_eq!(gears[1].module_system, b.module_system);
        assert_eq!(gears[0].hub, a.hub);
        let hub = gears[1].hub.as_ref().unwrap();
        assert_eq!(hub.diameter, b.hub.as_ref().unwrap().diameter);
        assert_eq!(hub.axial_start, b.hub.as_ref().unwrap().axial_start);
        assert_eq!(hub.axial_end, model::length(17.));
        assert!(gears.iter().all(|g| g.shift_mate.is_none()));
        panel.primary.fields[0].set_literal(3., "mm");
        let GearPairCommand::Edit {
            gears: Some(module_edit),
            ..
        } = panel.request(s.document(), true).unwrap()
        else {
            panic!("module edit")
        };
        assert_eq!(
            module_edit[1].pressure_angle, b.pressure_angle,
            "module-only edit must retain the independent second pressure binding"
        );
        assert_eq!(
            module_edit[1].helix_angle, b.helix_angle,
            "module-only edit must retain the independent second helix source"
        );
        assert_eq!(module_edit[1].module, model::length(3.));
        assert_eq!(s.document(), &before);
        assert_eq!(s.revision(), 1);
    }
    #[test]
    fn shifted_pair_contexts_follow_own_sources_without_literalizing_or_changing_outputs() {
        let s = Session::new(Document::new("Shifted pair form").unwrap()).unwrap();
        let mut primary = GearPanel::new(
            GearRequest::new(
                "KHK pair",
                GearParameters {
                    module: model::length(3.),
                    teeth: multicad_core::constraint_sketch::Dimension::Literal {
                        value: 12.,
                        unit: "count".into(),
                    },
                    profile_shift: model::ratio(0.6),
                    shift_mate: Some(model::ShiftedSpurMate {
                        teeth: multicad_core::constraint_sketch::Dimension::Literal {
                            value: 24.,
                            unit: "count".into(),
                        },
                        profile_shift: model::ratio(0.36),
                    }),
                    ..Default::default()
                },
            ),
            s.parameters().clone(),
            false,
        )
        .unwrap();
        primary.hub.enabled = true;
        let mut pair = GearPairPanel::from_single(primary);
        let GearPairCommand::Create(first) = pair.request(s.document(), true).unwrap() else {
            panic!("create")
        };
        assert_eq!(first.gears[1].parameters.profile_shift, model::ratio(0.36));
        assert!(first.gears[1].parameters.hub.is_none());
        let resolved = first
            .parameters
            .resolve_recipes(
                [&first.gears[0].parameters, &first.gears[1].parameters],
                s.parameters(),
            )
            .unwrap();
        assert!((resolved.center_distance_mm - 56.4999).abs() < 0.001);
        pair.secondary_shift.set_literal(0.4, "ratio");
        let GearPairCommand::Create(changed) = pair.request(s.document(), true).unwrap() else {
            panic!("create")
        };
        assert_eq!(first.pair, changed.pair);
        assert_eq!(first.outputs, changed.outputs);
        assert_eq!(first.placement, changed.placement);
        assert_eq!(
            changed.gears[0]
                .parameters
                .shift_mate
                .as_ref()
                .unwrap()
                .profile_shift,
            changed.gears[1].parameters.profile_shift
        );
        pair.primary.shift.set_literal(-0.5, "ratio");
        assert!(pair.request(s.document(), true).is_err());
        assert_eq!(s.revision(), 0);
        assert!(!s.can_undo());
    }
    #[test]
    fn paired_spur_to_helical_keeps_unsupported_secondary_shift_visible_until_cleared() {
        let s = Session::new(Document::new("Visible shift correction").unwrap()).unwrap();
        let primary = GearPanel::new(
            GearRequest::new(
                "Matched Spur",
                GearParameters {
                    profile_shift: model::ratio(0.2),
                    shift_mate: Some(model::ShiftedSpurMate {
                        teeth: multicad_core::constraint_sketch::Dimension::Literal {
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
        let mut pair = GearPairPanel::from_single(primary);
        pair.primary.select_family(GearFamily::Helical);
        pair.primary.hand = GearHand::Right;
        pair.primary.fields[4].set_literal(15., "deg");
        pair.primary.shift.set_literal(0., "ratio");
        assert!(!pair.primary.shift_mate_enabled);
        assert!(pair.request(s.document(), true).is_err());
        let ctx = egui::Context::default();
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(650., 3600.),
                )),
                ..Default::default()
            },
            |ui| {
                pair.show(ui, s.document(), true, PreviewState::Idle);
            },
        );
        assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(t) if t.galley.text()=="Mating profile shift coefficient (ratio)")));
        output.drop_without_applying_deltas();
        pair.secondary_shift.set_literal(0., "ratio");
        let GearPairCommand::Create(request) = pair.request(s.document(), true).unwrap() else {
            panic!("helical pair")
        };
        assert!(
            request
                .gears
                .iter()
                .all(|g| g.parameters.shift_mate.is_none())
        );
        assert_eq!(s.revision(), 0);
        assert!(!s.can_undo());
    }
}

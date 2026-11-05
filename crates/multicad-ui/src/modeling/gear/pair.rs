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
    secondary_ring: bool,
    rack_member: Option<usize>,
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
        let secondary_backing = primary.rack_backing.clone();
        let secondary = [
            DimensionField::from_dimension(&template.gears[1].parameters.teeth, "count")
                .expect("preset count"),
            primary.fields[6].clone(),
            primary.fields[7].clone(),
            primary.fields[8].clone(),
        ];
        Self {
            name: primary.request.name.clone(),
            primary,
            template,
            secondary,
            secondary_ring: false,
            rack_member,
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
            secondary_ring: b.family == GearFamily::InternalRing,
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
        p.stamp = FormStamp::new();
        p
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
            let first = self.primary.request(document, enabled)?.parameters;
            let second = GearParameters {
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
                hand: match first.hand {
                    GearHand::None => GearHand::None,
                    GearHand::Left => GearHand::Right,
                    GearHand::Right => GearHand::Left,
                },
                frame: self.template.gears[1].parameters.frame.clone(),
                ..first.clone()
            };
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
        if let Some(travel) = &resolved.rack_travel {
            ui.small(format!(
                "Rack {:?} / pinion {:?} · translation {:.4} mm",
                travel.rack_member, travel.pinion_member, travel.translation_mm
            ));
            Self::show_travel_limits(ui, travel);
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
            let mut rack = self.rack_member.is_some();
            controls::inspector_combo(ui, "gear-pair-type")
                .selected_text(if rack {
                    "Rack and pinion"
                } else {
                    "Rotary gears"
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut rack, false, "Rotary gears")
                        .changed();
                    changed |= ui
                        .selectable_value(&mut rack, true, "Rack and pinion")
                        .changed();
                });
            if rack != self.rack_member.is_some() {
                self.set_rack_member(rack.then_some(0));
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
            changed |= self
                .primary
                .geometry_controls_with_family(ui, self.rack_member.is_none());
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
            ui.strong(if rack_second {
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
                    if rack_second {
                        "Rack pitch cells"
                    } else if self.rack_member.is_some() {
                        "Pinion teeth"
                    } else {
                        "Wheel teeth"
                    },
                    "Wheel bore diameter",
                    "Wheel design backlash",
                    "Wheel fabrication clearance",
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
            ui.small(if self.rack_member.is_some() { "Straight rack and spur pinion share module, pressure angle, face width and custom root ratio. The pinion stays at +Y × pitch radius; the rack translates on local X." } else if ring || self.primary.family == GearFamily::InternalRing {"Ring/pinion share module, 20° pressure angle and face width. Centers use the radius difference; both rotate in the same direction."} else {"The wheel shares the module convention, pressure angle, face width, root profile and center gap. Helix hand is opposite."});
        } else {
            for gear in &self.template.gears {
                ui.small(&gear.name);
            }
        }
        ui.separator();
        ui.strong("Pair layout and drive");
        changed|=self.drive.show_named(ui, (self.template.pair, "pair-drive"),if self.primary.family == GearFamily::InternalRing {"Ring drive angle"}else{"Pinion drive angle"},if self.rack_member.is_some() { "Pinion angle regardless of member role: rack translation s = pinion pitch radius × angle in radians. Finite-end travel is bounded." } else { "The second member follows the tooth-count ratio; copies share this definition's drive coordinate" });
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
            .zip(["Pair origin X", "Pair origin Y", "Pair origin Z"])
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

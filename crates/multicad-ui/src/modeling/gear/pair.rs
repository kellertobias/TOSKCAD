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
                family: if first.family == GearFamily::InternalRing {
                    GearFamily::Spur
                } else if self.secondary_ring && first.family == GearFamily::Spur {
                    GearFamily::InternalRing
                } else {
                    first.family
                },
                outer_diameter: if self.secondary_ring && first.family == GearFamily::Spur {
                    Some(self.secondary_outside.resolve(document)?)
                } else {
                    None
                },
                teeth: self.secondary[0].resolve(document)?,
                bore_diameter: self.secondary[1].resolve(document)?,
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
        if let Err(error) = self.request(document, enabled) {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if !self.editing && ui.button("Single gear").clicked() {
            self.single_requested = true;
            return Some(ModelEvent::Changed);
        }
        ui.label("Pair name");
        let mut changed = ui
            .add_enabled(
                !self.editing,
                egui::TextEdit::singleline(&mut self.name).desired_width(ui.available_width()),
            )
            .changed();
        if self.editing {
            changed |= ui
                .checkbox(&mut self.update_geometry, "Update both gear recipes")
                .changed();
            ui.small("Leave this off to change only the drive or pair layout. Enabling it applies the shared geometry controls to both gears.");
        }
        if self.update_geometry {
            ui.strong("Shared geometry and first member");
            let previous_family = self.primary.family;
            changed |= self.primary.geometry_controls(ui);
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
            if self.primary.family == GearFamily::Spur
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
            let ring = self.primary.family == GearFamily::Spur && self.secondary_ring;
            ui.strong(if ring {
                "Mating internal ring"
            } else if self.primary.family == GearFamily::InternalRing {
                "Mating pinion"
            } else {
                "Mating wheel"
            });
            for (field, label) in self.secondary.iter_mut().zip([
                "Wheel teeth",
                "Wheel bore diameter",
                "Wheel design backlash",
                "Wheel fabrication clearance",
            ]) {
                if ring && label == "Wheel bore diameter" {
                    continue;
                }
                changed |= field.show(
                    ui,
                    label,
                    "Independent wheel value; formulas may reference project parameters",
                );
            }
            if ring {
                changed |= self.secondary_outside.show(
                    ui,
                    "Mating ring outside diameter",
                    "Outside rim must meet the shared minimum-wall requirement",
                );
            }
            ui.small(if ring || self.primary.family == GearFamily::InternalRing {"Ring/pinion share module, 20° pressure angle and face width. Centers use the radius difference; both rotate in the same direction."} else {"The wheel shares the module convention, pressure angle, face width, root profile and center gap. Helix hand is opposite."});
        } else {
            for gear in &self.template.gears {
                ui.small(&gear.name);
            }
        }
        ui.separator();
        ui.strong("Pair layout and drive");
        changed|=self.drive.show(ui,if self.primary.family == GearFamily::InternalRing {"Ring drive angle"}else{"Pinion drive angle"},"The second member follows the tooth-count ratio; copies share this definition's drive coordinate");
        controls::inspector_combo(ui, "gear-pair-plane")
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
                .zip(["Pair origin X", "Pair origin Y", "Pair origin Z"])
        {
            changed |= field.show(
                ui,
                label,
                "Layout within the reusable pair; use Move/Copy for an independent outer placement",
            );
        }
        let validation = self.request(document, enabled);
        if let Ok(command) = &validation {
            if let Ok(r) = command.resolved(document, &self.primary.parameters) {
                ui.small(format!(
                    "Center distance {:.4} mm · {} : {} teeth",
                    r.center_distance_mm, r.gears[0].teeth, r.gears[1].teeth
                ));
            }
        } else if let Err(error) = &validation {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
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

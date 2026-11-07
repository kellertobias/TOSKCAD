use super::*;

impl ScrewConnectionPanel {
    pub(super) fn show_axis(&mut self, ui: &mut egui::Ui, document: &Document, changed: &mut bool) {
        ui.separator();
        ui.strong("Receiving face and axis");
        let selected = self.choices.get(self.choice);
        controls::inspector_combo(ui, (self.connection, "receiver-choice"))
            .selected_text(selected.map_or("Choose a receiving occurrence", |choice| {
                document
                    .get(choice.component)
                    .map_or("Receiving occurrence", |entity| entity.name.as_str())
            }))
            .show_ui(ui, |ui| {
                for (index, choice) in self.choices.iter().enumerate() {
                    let name = document
                        .get(choice.component)
                        .map_or("Receiving Part", |entity| entity.name.as_str());
                    ui.selectable_value(
                        &mut self.choice,
                        index,
                        format!("{name} · {}", choice.path.root),
                    );
                }
            });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.choices.is_empty(),
                    egui::Button::new(if self.axis.is_some() {
                        "Replace entry face"
                    } else {
                        "Pick entry face / axis"
                    }),
                )
                .clicked()
                && let Some(choice) = self.choices.get(self.choice)
            {
                self.pick = Some(ScrewConnectionPick {
                    receiver: BodyInput {
                        body: choice.component,
                        producer: choice.producer,
                    },
                    occurrence: choice.path.clone(),
                    axis: true,
                });
                *changed = true;
            }
            if self.pick.is_some() && ui.button("Cancel face pick").clicked() {
                self.pick = None;
                *changed = true;
            }
        });
        if let Some(pick) = &self.pick {
            ui.label(if pick.axis {
                "Click a planar receiving face in the viewer."
            } else {
                "Click a planar face on the additional receiver."
            });
        }
        if let Some(axis) = &mut self.axis {
            ui.small(format!(
                "Captured predecessor {} · face {}",
                axis.source.receiver.producer, axis.face.intent.id
            ));
            for (index, key, label) in [
                (0, "axis-u", "Axis U offset"),
                (1, "axis-v", "Axis V offset"),
                (2, "axis-n", "Axis axial offset"),
            ] {
                self.fields.dimension(
                    ui,
                    document,
                    self.axis_id,
                    (key, label, "mm"),
                    &mut axis.source.offset[index],
                );
            }
            self.fields.dimension(
                ui,
                document,
                self.axis_id,
                ("axis-roll", "Axis roll", "deg"),
                &mut axis.source.roll,
            );
            *changed |= ui
                .checkbox(&mut axis.source.reverse, "Reverse connection direction")
                .changed();
        }
    }

    pub(super) fn show_hardware(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        changed: &mut bool,
    ) {
        ui.separator();
        ui.strong("Hardware stack");
        ui.small("Stations follow each role. Up and Down reorder washers within the same role.");
        let mut action = None;
        let roles = self
            .hardware
            .iter()
            .map(|hardware| hardware.role)
            .collect::<Vec<_>>();
        for (index, hardware) in self.hardware.iter_mut().enumerate() {
            ui.push_id(hardware.role_id, |ui| {
                ui.group(|ui| {
                    ui.strong(format!("{} · {}", index + 1, role_name(hardware.role)));
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Choose from Library").clicked() {
                            self.library.begin(hardware.role_id);
                            *changed = true;
                        }
                        if ui
                            .add_enabled(
                                roles[..index].contains(&hardware.role),
                                egui::Button::new("Up"),
                            )
                            .clicked()
                        {
                            action = Some((index, 0));
                        }
                        if ui
                            .add_enabled(
                                roles[index + 1..].contains(&hardware.role),
                                egui::Button::new("Down"),
                            )
                            .clicked()
                        {
                            action = Some((index, 1));
                        }
                        if ui.button("Remove").clicked() {
                            action = Some((index, 2));
                        }
                        if ui
                            .add_enabled(
                                matches!(
                                    hardware.source,
                                    ScrewConnectionHardwareSource::Library { .. }
                                ),
                                egui::Button::new("Use custom engineering geometry"),
                            )
                            .clicked()
                        {
                            action = Some((index, 3));
                        }
                    });
                    match &mut hardware.source {
                        ScrewConnectionHardwareSource::Generated { part } => {
                            ui.small("Custom parametric hardware; no stock or fit certification");
                            show_custom(
                                &mut self.fields,
                                ui,
                                document,
                                hardware.role_id,
                                &mut part.parameters,
                            );
                        }
                        ScrewConnectionHardwareSource::Library {
                            definition,
                            placement,
                        } => {
                            ui.label(&definition.name);
                            ui.small(format!("Pinned revision {}", definition.revision));
                            if let std::collections::btree_map::Entry::Vacant(entry) =
                                self.overrides.entry(hardware.role_id)
                            {
                                match crate::symbols::SymbolOverrides::new(
                                    definition,
                                    &placement.overrides,
                                ) {
                                    Ok(fields) => {
                                        entry.insert(fields);
                                    }
                                    Err(error) => {
                                        self.error = Some(error);
                                    }
                                }
                            }
                            if let Some(fields) = self.overrides.get_mut(&hardware.role_id) {
                                *changed |= fields.show(ui, definition);
                                match fields.resolve(definition) {
                                    Ok(values) => placement.overrides = values,
                                    Err(error) => {
                                        self.fields.error.get_or_insert(error);
                                    }
                                }
                            }
                        }
                    }
                })
            });
        }
        if let Some((index, kind)) = action {
            match kind {
                0 => {
                    if let Some(previous) =
                        (0..index).rev().find(|&other| roles[other] == roles[index])
                    {
                        self.hardware.swap(index, previous);
                    }
                }
                1 => {
                    if let Some(next) =
                        (index + 1..roles.len()).find(|&other| roles[other] == roles[index])
                    {
                        self.hardware.swap(index, next);
                    }
                }
                2 => {
                    let removed = self.hardware.remove(index);
                    self.overrides.remove(&removed.role_id);
                    self.fields.remove_owner(removed.role_id);
                }
                3 => {
                    let role = &mut self.hardware[index];
                    if let Some(mut custom) = generated(role.role) {
                        if self.mode == ScrewConnectionMode::DirectWood
                            && role.role == ScrewConnectionHardwareRole::Screw
                            && let ScrewConnectionHardwareSource::Generated { part } =
                                &mut custom.source
                        {
                            part.parameters = default_custom_connection_wood_screw();
                        }
                        role.source = custom.source;
                        self.overrides.remove(&role.role_id);
                        self.fields.remove_owner(role.role_id);
                    }
                }
                _ => {}
            }
            *changed = true;
        }
        ui.horizontal_wrapped(|ui| {
            for role in [
                ScrewConnectionHardwareRole::HeadWasher,
                ScrewConnectionHardwareRole::NutWasher,
                ScrewConnectionHardwareRole::Nut,
            ] {
                if ui
                    .add_enabled(
                        self.hardware.len() < 64
                            && self.role_available(role)
                            && (role != ScrewConnectionHardwareRole::Nut
                                || !self.hardware.iter().any(|hardware| hardware.role == role)),
                        egui::Button::new(format!("Add {}", role_name(role))),
                    )
                    .clicked()
                {
                    self.hardware.extend(generated(role));
                    *changed = true;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            for role in [
                ScrewConnectionHardwareRole::Screw,
                ScrewConnectionHardwareRole::Insert,
                ScrewConnectionHardwareRole::HangerBolt,
            ] {
                if ui
                    .add_enabled(
                        self.role_available(role)
                            && (self.hardware.iter().any(|hardware| hardware.role == role)
                                || self.hardware.len() < 64),
                        egui::Button::new(format!("Choose {}", role_name(role))),
                    )
                    .clicked()
                {
                    match self.choose_hardware(role) {
                        Ok(()) => *changed = true,
                        Err(error) => self.error = Some(error),
                    }
                }
            }
        });
        if matches!(
            self.mode,
            ScrewConnectionMode::DirectWood
                | ScrewConnectionMode::WoodInsert
                | ScrewConnectionMode::HangerBolt
        ) {
            ui.small("Purchased usable thread and installation spans must be explicit. Unknown point or proprietary thread geometry is not recovered from an envelope.");
        }
    }

    pub(super) fn use_definition(
        &mut self,
        document: &Document,
        role: EntityId,
        definition: SymbolDefinition,
    ) -> Result<(), String> {
        let pin = definition.pin().map_err(|error| error.to_string())?;
        let existing = symbols::library_placement_definition(document, &definition)
            .map_err(|error| error.to_string())?;
        let definition_id = existing
            .or_else(|| {
                self.hardware
                    .iter()
                    .find_map(|hardware| match &hardware.source {
                        ScrewConnectionHardwareSource::Library {
                            definition: other,
                            placement,
                        } if other.pin().ok() == Some(pin) => Some(placement.definition),
                        _ => None,
                    })
            })
            .unwrap_or_else(EntityId::new);
        let mut placement = SymbolPlaceRequest::from_definition(
            definition_id,
            &definition,
            &definition.name,
            BTreeMap::new(),
            Default::default(),
        )
        .map_err(|error| error.to_string())?;
        if let Some(hardware) = self
            .hardware
            .iter_mut()
            .find(|hardware| hardware.role_id == role)
        {
            placement.instance = hardware.instance;
            hardware.source = ScrewConnectionHardwareSource::Library {
                definition: Box::new(definition),
                placement: Box::new(placement),
            };
            self.overrides.remove(&role);
            self.fields.remove_owner(role);
        } else {
            let kind = self
                .pending_roles
                .remove(&role)
                .ok_or("The hardware stack role is no longer selected")?;
            self.hardware.push(ScrewConnectionHardwareRequest {
                role_id: role,
                role: kind,
                name: role_name(kind).into(),
                instance: placement.instance,
                driver: EntityId::new(),
                source: ScrewConnectionHardwareSource::Library {
                    definition: Box::new(definition),
                    placement: Box::new(placement),
                },
            });
        }
        self.error = None;
        Ok(())
    }

    pub(super) fn show_receivers(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        changed: &mut bool,
    ) {
        ui.separator();
        ui.strong("Receiving cuts");
        if ui
            .add_enabled(
                self.axis.is_some() && !self.choices.is_empty() && self.receivers.len() < 16,
                egui::Button::new("Pick additional receiver face"),
            )
            .clicked()
            && let Some(choice) = self.choices.get(self.choice)
        {
            self.pick = Some(ScrewConnectionPick {
                receiver: BodyInput {
                    body: choice.component,
                    producer: choice.producer,
                },
                occurrence: choice.path.clone(),
                axis: false,
            });
            *changed = true;
        }
        let mut remove = None;
        for (index, receiver) in self.receivers.iter_mut().enumerate() {
            ui.push_id(receiver.feature, |ui| {
                ui.group(|ui| {
                    ui.strong(format!(
                        "{} · {}",
                        index + 1,
                        document
                            .get(receiver.source.input.body)
                            .map_or("Receiving Part", |entity| entity.name.as_str())
                    ));
                    ui.small(format!(
                        "Exact predecessor {}",
                        receiver.source.input.producer
                    ));
                    if ui.button("Remove receiver").clicked() {
                        remove = Some(index);
                    }
                    self.fields.dimension(
                        ui,
                        document,
                        receiver.feature,
                        ("station", "Cut start along axis", "mm"),
                        &mut receiver.source.station,
                    );
                    let mut tapped =
                        matches!(receiver.source.cut, ConnectionReceiverCut::Tapped { .. });
                    if ui
                        .checkbox(&mut tapped, "Modeled tapped receiver")
                        .changed()
                    {
                        receiver.source.cut = if tapped {
                            ConnectionReceiverCut::Tapped {
                                thread: Box::new(ThreadParameters {
                                    major_diameter: length(3.),
                                    pitch: length(0.5),
                                    length: length(4.),
                                    fit: ThreadFit::Metric6HInternal,
                                    metric_fit_engagement: Some(
                                        MetricFitEngagement::NormalGroupAssumed {},
                                    ),
                                    minimum_wall: length(0.5),
                                    ..Default::default()
                                }),
                            }
                        } else {
                            ConnectionReceiverCut::Round {
                                diameter: length(3.4),
                            }
                        };
                        *changed = true;
                    }
                    if matches!(receiver.source.cut, ConnectionReceiverCut::Round { .. }) {
                        ui.small("Engineering diameter presets are separate from stock installation guidance. Choose each clamp clearance or wood/insert pilot independently.");
                        ui.horizontal_wrapped(|ui| {
                            for (label, diameter) in [("Metric clearance 3.4", 3.4), ("Wood pilot 4", 4.), ("Insert pilot 8", 8.)] {
                                if ui.button(label).clicked() {
                                    receiver.source.cut = ConnectionReceiverCut::Round { diameter: length(diameter) };
                                    self.fields.remove_field(receiver.feature, "hole-diameter");
                                    *changed = true;
                                }
                            }
                        });
                    }
                    match &mut receiver.source.cut {
                        ConnectionReceiverCut::Round { diameter } => self.fields.dimension(
                            ui,
                            document,
                            receiver.feature,
                            ("hole-diameter", "Custom receiving hole diameter", "mm"),
                            diameter,
                        ),
                        ConnectionReceiverCut::Tapped { thread } => {
                            show_thread(&mut self.fields, ui, document, receiver.feature, thread)
                        }
                    }
                    let mut blind = matches!(
                        receiver.source.extent,
                        ConnectionReceiverExtent::Blind { .. }
                    );
                    if ui.checkbox(&mut blind, "Blind receiving cut").changed() {
                        receiver.source.extent = if blind {
                            ConnectionReceiverExtent::Blind {
                                depth: length(4.),
                                floor: length(1.),
                            }
                        } else {
                            ConnectionReceiverExtent::Through {}
                        };
                        *changed = true;
                    }
                    if let ConnectionReceiverExtent::Blind { depth, floor } =
                        &mut receiver.source.extent
                    {
                        self.fields.dimension(
                            ui,
                            document,
                            receiver.feature,
                            ("depth", "Blind depth", "mm"),
                            depth,
                        );
                        self.fields.dimension(
                            ui,
                            document,
                            receiver.feature,
                            ("floor", "Minimum remaining floor", "mm"),
                            floor,
                        );
                    }
                    self.fields.dimension(
                        ui,
                        document,
                        receiver.feature,
                        ("wall", "Minimum receiving wall", "mm"),
                        &mut receiver.source.minimum_wall,
                    );
                    let mut seat = match &receiver.source.seat {
                        None => 0,
                        Some(ConnectionReceiverSeat::Counterbore { .. }) => 1,
                        Some(ConnectionReceiverSeat::Countersink { .. }) => 2,
                    };
                    let previous = seat;
                    controls::inspector_combo(ui, (receiver.feature, "seat"))
                        .selected_text(["No head seat", "Counterbore", "Countersink"][seat])
                        .show_ui(ui, |ui| {
                            for (value, label) in ["No head seat", "Counterbore", "Countersink"]
                                .into_iter()
                                .enumerate()
                            {
                                ui.selectable_value(&mut seat, value, label);
                            }
                        });
                    if previous != seat {
                        receiver.source.seat = match seat {
                            1 => Some(ConnectionReceiverSeat::Counterbore {
                                diameter: length(6.),
                                depth: length(3.4),
                            }),
                            2 => Some(ConnectionReceiverSeat::Countersink {
                                diameter: length(6.6),
                                depth: length(1.6),
                            }),
                            _ => None,
                        };
                        *changed = true;
                    }
                    if let Some(
                        ConnectionReceiverSeat::Counterbore { diameter, depth }
                        | ConnectionReceiverSeat::Countersink { diameter, depth },
                    ) = &mut receiver.source.seat
                    {
                        self.fields.dimension(
                            ui,
                            document,
                            receiver.feature,
                            ("seat-diameter", "Seat diameter", "mm"),
                            diameter,
                        );
                        self.fields.dimension(
                            ui,
                            document,
                            receiver.feature,
                            ("seat-depth", "Seat depth", "mm"),
                            depth,
                        );
                    }
                })
            });
        }
        if let Some(index) = remove {
            let receiver = self.receivers.remove(index);
            self.fields.remove_owner(receiver.feature);
            *changed = true;
        }
    }
}

fn show_thread(
    fields: &mut fields::Fields,
    ui: &mut egui::Ui,
    document: &Document,
    owner: EntityId,
    thread: &mut ThreadParameters,
) {
    fields.dimension(
        ui,
        document,
        owner,
        ("thread-diameter", "Thread major diameter", "mm"),
        &mut thread.major_diameter,
    );
    fields.dimension(
        ui,
        document,
        owner,
        ("pitch", "Pitch", "mm"),
        &mut thread.pitch,
    );
    fields.dimension(
        ui,
        document,
        owner,
        ("thread-length", "Thread span / nut height", "mm"),
        &mut thread.length,
    );
    fields.changed |= controls::inspector_combo(ui, (owner, "thread-hand"))
        .selected_text(if thread.hand == ThreadHand::Right {
            "Right hand"
        } else {
            "Left hand"
        })
        .show_ui(ui, |ui| {
            let mut changed = false;
            changed |= ui
                .selectable_value(&mut thread.hand, ThreadHand::Right, "Right hand")
                .changed();
            changed |= ui
                .selectable_value(&mut thread.hand, ThreadHand::Left, "Left hand")
                .changed();
            changed
        })
        .inner
        .unwrap_or(false);
    ui.small(format!("Retained fit: {}", thread.fit.label()));
    if matches!(
        thread.metric_fit_engagement,
        Some(MetricFitEngagement::NormalGroupAssumed {})
    ) {
        ui.small("Metric fit uses an explicitly assumed normal engagement group; actual mating engagement is checked separately.");
    }
    if thread.representation_parameter.is_some() {
        ui.small("Thread representation follows the selected source parameter.");
    } else {
        ui.small(format!(
            "Thread representation: {}",
            thread.representation.label()
        ));
    }
    ui.small("Native Preview validates the complete pairing and its usable spans.");
}
fn show_custom(
    fields: &mut fields::Fields,
    ui: &mut egui::Ui,
    document: &Document,
    owner: EntityId,
    parameters: &mut MechanicalPartParameters,
) {
    match parameters {
        MechanicalPartParameters::MachineScrew {
            underhead_length,
            length_input,
            shank_diameter,
            thread,
            ..
        } => {
            if let Some(value) = underhead_length {
                fields.dimension(
                    ui,
                    document,
                    owner,
                    ("length", "Underhead length", "mm"),
                    value,
                );
            }
            if let Some(value) = length_input {
                let label = if matches!(value, ScrewLengthInput::Overall { .. }) {
                    "Overall length"
                } else {
                    "Underhead length"
                };
                match value {
                    ScrewLengthInput::Overall { value } | ScrewLengthInput::Underhead { value } => {
                        fields.dimension(ui, document, owner, ("length", label, "mm"), value)
                    }
                }
            }
            fields.dimension(
                ui,
                document,
                owner,
                ("shank", "Plain shank diameter", "mm"),
                shank_diameter,
            );
            show_thread(fields, ui, document, owner, thread);
        }
        MechanicalPartParameters::Washer {
            bore_diameter,
            outside_diameter,
            thickness,
            ..
        } => {
            fields.dimension(
                ui,
                document,
                owner,
                ("bore", "Bore diameter", "mm"),
                bore_diameter,
            );
            fields.dimension(
                ui,
                document,
                owner,
                ("outside", "Outside diameter", "mm"),
                outside_diameter,
            );
            fields.dimension(
                ui,
                document,
                owner,
                ("thickness", "Thickness", "mm"),
                thickness,
            );
        }
        MechanicalPartParameters::HexNut {
            across_flats,
            thread,
        } => {
            fields.dimension(
                ui,
                document,
                owner,
                ("flats", "Across flats", "mm"),
                across_flats,
            );
            show_thread(fields, ui, document, owner, thread);
        }
        MechanicalPartParameters::WoodScrew { parameters } => {
            let value = match &mut parameters.length_input {
                ScrewLengthInput::Underhead { value } | ScrewLengthInput::Overall { value } => {
                    value
                }
            };
            fields.dimension(
                ui,
                document,
                owner,
                ("length", "Authored wood screw length", "mm"),
                value,
            );
            fields.dimension(
                ui,
                document,
                owner,
                ("diameter", "Nominal stock diameter", "mm"),
                &mut parameters.nominal_diameter,
            );
            ui.small("Simplified stock envelope. Manufacturer point and wood helix remain unknown; hardware manufacturing is unavailable.");
        }
        MechanicalPartParameters::CustomConnection { parameters } => {
            ui.small("Custom engineering geometry. No manufacturer thread, point, installation or strength certification.");
            match parameters.as_mut() {
                CustomConnectionHardwareParameters::WoodScrew {
                    wood_thread,
                    plain_length,
                    plain_diameter,
                    head,
                    ..
                } => {
                    ui.strong("Explicit custom wood thread");
                    show_thread_named(fields, ui, document, owner, wood_thread, false);
                    fields.dimension(
                        ui,
                        document,
                        owner,
                        ("plain-length", "Plain transition length", "mm"),
                        plain_length,
                    );
                    fields.dimension(
                        ui,
                        document,
                        owner,
                        ("plain-diameter", "Plain transition diameter", "mm"),
                        plain_diameter,
                    );
                    show_head(fields, ui, document, owner, head);
                }
                CustomConnectionHardwareParameters::WoodInsert {
                    outer_thread,
                    inner_thread,
                    ..
                } => {
                    ui.push_id("insert-outer", |ui| {
                        ui.strong("Custom outside thread");
                        show_thread_named(fields, ui, document, owner, outer_thread, true);
                    });
                    ui.push_id("insert-inner", |ui| {
                        ui.strong("Inside metric receiving thread");
                        show_thread(fields, ui, document, owner, inner_thread);
                    });
                }
                CustomConnectionHardwareParameters::HangerBolt {
                    wood_thread,
                    plain_length,
                    plain_diameter,
                    metric_thread,
                    ..
                } => {
                    ui.push_id("hanger-wood", |ui| {
                        ui.strong("Custom wood end");
                        show_thread_named(fields, ui, document, owner, wood_thread, true);
                    });
                    fields.dimension(
                        ui,
                        document,
                        owner,
                        ("plain-length", "Plain transition length", "mm"),
                        plain_length,
                    );
                    fields.dimension(
                        ui,
                        document,
                        owner,
                        ("plain-diameter", "Plain transition diameter", "mm"),
                        plain_diameter,
                    );
                    ui.push_id("hanger-metric", |ui| {
                        ui.strong("Metric nut end");
                        show_thread(fields, ui, document, owner, metric_thread);
                    });
                }
            }
        }
        MechanicalPartParameters::BearingEnvelope { .. } => {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                "A bearing is not a supported connection stack role",
            );
        }
    }
}

fn show_head(
    fields: &mut fields::Fields,
    ui: &mut egui::Ui,
    document: &Document,
    owner: EntityId,
    head: &mut ScrewHead,
) {
    match head {
        ScrewHead::ExternalHex {
            across_flats,
            height,
        } => {
            fields.dimension(
                ui,
                document,
                owner,
                ("head-flats", "Head across flats", "mm"),
                across_flats,
            );
            fields.dimension(
                ui,
                document,
                owner,
                ("head-height", "Head height", "mm"),
                height,
            );
        }
        ScrewHead::SocketCap { diameter, height } => {
            fields.dimension(
                ui,
                document,
                owner,
                ("head-diameter", "Head diameter", "mm"),
                diameter,
            );
            fields.dimension(
                ui,
                document,
                owner,
                ("head-height", "Head height", "mm"),
                height,
            );
        }
        _ => {
            ui.small("Retained head profile; use its sourced Library inputs to alter its physical shape.");
        }
    }
}
fn show_thread_named(
    fields: &mut fields::Fields,
    ui: &mut egui::Ui,
    document: &Document,
    owner: EntityId,
    thread: &mut ThreadParameters,
    outer: bool,
) {
    if outer {
        fields.dimension(
            ui,
            document,
            owner,
            (
                "outer-thread-diameter",
                "Outside thread major diameter",
                "mm",
            ),
            &mut thread.major_diameter,
        );
        fields.dimension(
            ui,
            document,
            owner,
            ("outer-pitch", "Outside thread pitch", "mm"),
            &mut thread.pitch,
        );
        fields.dimension(
            ui,
            document,
            owner,
            ("outer-thread-length", "Outside thread complete span", "mm"),
            &mut thread.length,
        );
    } else {
        show_thread(fields, ui, document, owner, thread);
    }
    if let Some(profile) = &mut thread.custom {
        fields.dimension(
            ui,
            document,
            owner,
            ("custom-depth", "Custom thread radial depth", "mm"),
            &mut profile.depth,
        );
        fields.dimension(
            ui,
            document,
            owner,
            ("custom-crest", "Custom crest width", "mm"),
            &mut profile.crest_width,
        );
        fields.dimension(
            ui,
            document,
            owner,
            ("custom-lower", "Lower flank angle", "deg"),
            &mut profile.lower_flank,
        );
        fields.dimension(
            ui,
            document,
            owner,
            ("custom-upper", "Upper flank angle", "deg"),
            &mut profile.upper_flank,
        );
    }
}

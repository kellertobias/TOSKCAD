//! One source-neutral editor for complete associative screw connections.
mod fields;
mod library;
mod panel_fields;
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui};
pub(crate) use library::HardwareLibraryRequest;
use multicad_core::{
    commands::{Actor, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    model::{self, *},
    symbols::{self, SymbolDefinition, SymbolPlaceRequest},
};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug)]
pub enum ScrewConnectionCommand {
    Set(Box<ScrewConnectionRequest>),
    Remove { connection: EntityId },
}
impl ScrewConnectionCommand {
    pub(crate) fn connection(&self) -> EntityId {
        match self {
            Self::Set(request) => request.connection,
            Self::Remove { connection } => *connection,
        }
    }
    pub(crate) fn instances(&self) -> Vec<EntityId> {
        match self {
            Self::Set(request) => request.hardware.iter().map(|role| role.instance).collect(),
            Self::Remove { .. } => vec![],
        }
    }
    pub(crate) fn removed_outputs(&self, document: &Document) -> Result<Vec<EntityId>, String> {
        let Self::Remove { connection } = self else {
            return Ok(vec![]);
        };
        let source = model::read_screw_connection_request(document, *connection)
            .map_err(|error| error.to_string())?;
        let mut ids = vec![*connection, source.axis.id];
        ids.extend(source.receivers.iter().map(|receiver| receiver.feature));
        for role in source.hardware {
            ids.extend([role.instance, role.driver]);
            match role.source {
                ScrewConnectionHardwareSource::Generated { part } => {
                    ids.extend([part.feature, part.body])
                }
                ScrewConnectionHardwareSource::Library { placement, .. } => {
                    ids.extend([
                        placement.record,
                        placement.wrapper,
                        placement.wrapper_metadata,
                        placement.member,
                    ]);
                    ids.extend(placement.ids.into_values());
                }
            }
        }
        ids.sort();
        ids.dedup();
        Ok(ids)
    }
    pub(crate) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        match self {
            Self::Set(request) => {
                model::prepare_screw_connection(session, session.revision(), actor, request)
                    .map_err(|error| error.to_string())
            }
            Self::Remove { connection } => model::prepare_remove_screw_connection(
                session,
                session.revision(),
                actor,
                &ScrewConnectionRemoveRequest {
                    connection: *connection,
                },
            )
            .map(|prepared| ModelPrepared {
                prepared,
                outputs: vec![],
            })
            .map_err(|error| error.to_string()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScrewConnectionPickKind {
    EntryAxis,
    ReceiverEntry,
    GripEnd,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ScrewConnectionPick {
    pub receiver: BodyInput,
    pub occurrence: OccurrencePath,
    pub kind: ScrewConnectionPickKind,
}

#[derive(Clone)]
struct ModeDraft {
    axis: Option<ScrewConnectionAxisRequest>,
    parameters: ScrewConnectionParameters,
    manual_grip: Option<Dimension>,
    pub(super) hardware: Vec<ScrewConnectionHardwareRequest>,
    receivers: Vec<ConnectionReceiverRequest>,
    fields: fields::Fields,
    overrides: BTreeMap<EntityId, crate::symbols::SymbolOverrides>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConnectionPanelFocus {
    Hardware(EntityId),
    Receiver(EntityId),
}
pub(super) struct ScrewConnectionPanel {
    stamp: FormStamp,
    connection: EntityId,
    name: String,
    mode: ScrewConnectionMode,
    axis_id: EntityId,
    pub(super) axis: Option<ScrewConnectionAxisRequest>,
    pub(super) hardware: Vec<ScrewConnectionHardwareRequest>,
    receivers: Vec<ConnectionReceiverRequest>,
    parameters: ScrewConnectionParameters,
    manual_grip: Option<Dimension>,
    drafts: BTreeMap<&'static str, ModeDraft>,
    fields: fields::Fields,
    library: library::HardwareLibrary,
    choices: Vec<TransferOccurrence>,
    choice: usize,
    pick: Option<ScrewConnectionPick>,
    error: Option<String>,
    overrides: BTreeMap<EntityId, crate::symbols::SymbolOverrides>,
    pending_roles: BTreeMap<EntityId, ScrewConnectionHardwareRole>,
    editing: bool,
    remove: bool,
    pub section: bool,
    pub explode: bool,
    review: Option<ScrewConnectionCheck>,
    entry_review: BTreeMap<EntityId, f64>,
    focus: Option<ConnectionPanelFocus>,
    scroll_focus: bool,
}

fn modes() -> [(ScrewConnectionMode, &'static str); 5] {
    [
        (ScrewConnectionMode::ThroughNut, "Through screw and nut"),
        (ScrewConnectionMode::TappedReceiver, "Tapped receiver"),
        (ScrewConnectionMode::DirectWood, "Direct wood screw"),
        (ScrewConnectionMode::WoodInsert, "Wood threaded insert"),
        (ScrewConnectionMode::HangerBolt, "Hanger bolt"),
    ]
}
fn mode_name(mode: ScrewConnectionMode) -> &'static str {
    modes()
        .into_iter()
        .find(|(value, _)| *value == mode)
        .unwrap()
        .1
}
fn role_name(role: ScrewConnectionHardwareRole) -> &'static str {
    match role {
        ScrewConnectionHardwareRole::Screw => "Screw",
        ScrewConnectionHardwareRole::HangerBolt => "Hanger bolt",
        ScrewConnectionHardwareRole::HeadWasher => "Head-side washer",
        ScrewConnectionHardwareRole::NutWasher => "Nut-side washer",
        ScrewConnectionHardwareRole::Nut => "Nut",
        ScrewConnectionHardwareRole::Insert => "Insert",
    }
}
fn generated(role: ScrewConnectionHardwareRole) -> Option<ScrewConnectionHardwareRequest> {
    let parameters = match role {
        ScrewConnectionHardwareRole::Screw => {
            let mut part = default_machine_screw();
            if let MechanicalPartParameters::MachineScrew { thread, .. } = &mut part {
                thread.fit = ThreadFit::Metric6gExternal;
                thread.metric_fit_engagement = Some(MetricFitEngagement::NormalGroupAssumed {});
                thread.length = length(8.);
            }
            part
        }
        ScrewConnectionHardwareRole::HeadWasher | ScrewConnectionHardwareRole::NutWasher => {
            MechanicalPartParameters::Washer {
                bore_diameter: length(3.2),
                outside_diameter: length(7.),
                thickness: length(0.5),
                frame: Default::default(),
            }
        }
        ScrewConnectionHardwareRole::Nut => MechanicalPartParameters::HexNut {
            across_flats: length(5.5),
            thread: Box::new(ThreadParameters {
                major_diameter: length(3.),
                pitch: length(0.5),
                length: length(2.5),
                fit: ThreadFit::Metric6HInternal,
                metric_fit_engagement: Some(MetricFitEngagement::NormalGroupAssumed {}),
                minimum_wall: length(0.5),
                ..Default::default()
            }),
        },
        ScrewConnectionHardwareRole::Insert => default_custom_connection_wood_insert(),
        ScrewConnectionHardwareRole::HangerBolt => default_custom_connection_hanger_bolt(),
    };
    Some(ScrewConnectionHardwareRequest {
        role_id: EntityId::new(),
        role,
        name: role_name(role).into(),
        source: ScrewConnectionHardwareSource::Generated {
            part: Box::new(MechanicalPartRequest::new(role_name(role), parameters)),
        },
        instance: EntityId::new(),
        driver: EntityId::new(),
    })
}

fn default_receiver_cut(mode: ScrewConnectionMode) -> ConnectionReceiverCut {
    match mode {
        ScrewConnectionMode::TappedReceiver => ConnectionReceiverCut::Tapped {
            thread: Box::new(ThreadParameters {
                major_diameter: length(3.),
                pitch: length(0.5),
                length: length(4.),
                fit: ThreadFit::Metric6HInternal,
                metric_fit_engagement: Some(MetricFitEngagement::NormalGroupAssumed {}),
                minimum_wall: length(0.5),
                ..Default::default()
            }),
        },
        ScrewConnectionMode::ThroughNut => ConnectionReceiverCut::Round {
            diameter: length(3.4),
        },
        ScrewConnectionMode::DirectWood | ScrewConnectionMode::HangerBolt => {
            ConnectionReceiverCut::Round {
                diameter: length(4.),
            }
        }
        ScrewConnectionMode::WoodInsert => ConnectionReceiverCut::Round {
            diameter: length(8.),
        },
    }
}

impl ScrewConnectionPanel {
    pub fn new(document: &Document) -> Result<Self, String> {
        Ok(Self {
            stamp: FormStamp::new(),
            connection: EntityId::new(),
            name: "Screw connection".into(),
            mode: ScrewConnectionMode::ThroughNut,
            axis_id: EntityId::new(),
            axis: None,
            hardware: [
                ScrewConnectionHardwareRole::Screw,
                ScrewConnectionHardwareRole::HeadWasher,
                ScrewConnectionHardwareRole::NutWasher,
                ScrewConnectionHardwareRole::Nut,
            ]
            .into_iter()
            .filter_map(generated)
            .collect(),
            receivers: vec![],
            parameters: ScrewConnectionParameters {
                end_reference: None,
                grip: length(4.),
                minimum_engagement: length(1.),
                blind_tip_clearance: length(1.),
            },
            manual_grip: None,
            drafts: BTreeMap::new(),
            fields: Default::default(),
            library: Default::default(),
            choices: model::transfer_occurrence_catalog(document)
                .map_err(|error| error.to_string())?,
            choice: 0,
            pick: None,
            error: None,
            overrides: BTreeMap::new(),
            pending_roles: BTreeMap::new(),
            editing: false,
            remove: false,
            section: false,
            explode: false,
            review: None,
            entry_review: BTreeMap::new(),
            focus: None,
            scroll_focus: false,
        })
    }
    pub fn existing(document: &Document, connection: EntityId) -> Result<Self, String> {
        let request = model::read_screw_connection_request(document, connection)
            .map_err(|error| error.to_string())?;
        Self::from_request(document, request, true)
    }
    pub(super) fn from_request(
        document: &Document,
        request: ScrewConnectionRequest,
        editing: bool,
    ) -> Result<Self, String> {
        let mut panel = Self::new(document)?;
        panel.connection = request.connection;
        panel.name = request.name;
        panel.mode = request.mode;
        panel.axis_id = request.axis.id;
        panel.axis = Some(request.axis);
        panel.hardware = request.hardware;
        panel.receivers = request.receivers;
        panel.parameters = request.parameters;
        let owned_bodies = panel
            .hardware
            .iter()
            .flat_map(|role| match &role.source {
                ScrewConnectionHardwareSource::Generated { part } => vec![part.body],
                ScrewConnectionHardwareSource::Library {
                    definition,
                    placement,
                } => definition
                    .template
                    .entities()
                    .filter(|entity| {
                        matches!(
                            entity.data,
                            multicad_core::document::EntityData::Part { .. }
                        )
                    })
                    .filter_map(|entity| placement.ids.get(&entity.id).copied())
                    .collect(),
            })
            .collect::<std::collections::BTreeSet<_>>();
        panel
            .choices
            .retain(|choice| !owned_bodies.contains(&choice.component));
        for choice in &mut panel.choices {
            if let Some(receiver) = panel.receivers.iter().find(|receiver| {
                receiver.source.input.body == choice.component
                    && receiver.source.occurrence == choice.path
            }) {
                choice.producer = receiver.source.input.producer;
            }
        }
        panel.editing = editing;
        Ok(panel)
    }
    pub(super) fn role_available(&self, role: ScrewConnectionHardwareRole) -> bool {
        match role {
            ScrewConnectionHardwareRole::Screw => self.mode != ScrewConnectionMode::HangerBolt,
            ScrewConnectionHardwareRole::HangerBolt => self.mode == ScrewConnectionMode::HangerBolt,
            ScrewConnectionHardwareRole::Insert => self.mode == ScrewConnectionMode::WoodInsert,
            ScrewConnectionHardwareRole::HeadWasher => self.mode != ScrewConnectionMode::HangerBolt,
            ScrewConnectionHardwareRole::Nut | ScrewConnectionHardwareRole::NutWasher => matches!(
                self.mode,
                ScrewConnectionMode::ThroughNut | ScrewConnectionMode::HangerBolt
            ),
        }
    }
    pub(super) fn choose_hardware(
        &mut self,
        role: ScrewConnectionHardwareRole,
    ) -> Result<(), String> {
        if !self.role_available(role) {
            return Err("This hardware role does not apply to the selected connection mode".into());
        }
        let id = if let Some(existing) = self.hardware.iter().find(|hardware| hardware.role == role)
        {
            existing.role_id
        } else {
            if self.hardware.len() >= 64 {
                return Err("A connection supports at most 64 hardware roles".into());
            }
            let id = EntityId::new();
            self.pending_roles.insert(id, role);
            id
        };
        self.library.begin(id);
        Ok(())
    }
    pub fn set_review(
        &mut self,
        review: Option<ScrewConnectionCheck>,
        entries: BTreeMap<EntityId, f64>,
    ) {
        self.review = review;
        self.entry_review = entries;
    }
    fn use_explicit_grip(&mut self) {
        self.parameters.end_reference = None;
        self.parameters.grip = self.manual_grip.take().unwrap_or_else(|| length(0.));
    }
    pub fn connection(&self) -> EntityId {
        self.connection
    }
    fn target_for_path(&self, path: &OccurrencePath) -> Option<ConnectionPanelFocus> {
        self.hardware
            .iter()
            .find(|row| row.instance == path.root)
            .map(|row| ConnectionPanelFocus::Hardware(row.role_id))
            .or_else(|| {
                self.receivers
                    .iter()
                    .find(|row| &row.source.occurrence == path)
                    .map(|row| ConnectionPanelFocus::Receiver(row.feature))
            })
    }
    pub(super) fn owns_display_path(&self, path: &OccurrencePath) -> bool {
        self.target_for_path(path).is_some()
    }
    pub(super) fn focus_path(&mut self, path: &OccurrencePath) -> bool {
        if self.pick.is_some() {
            return false;
        }
        let Some(target) = self.target_for_path(path) else {
            return false;
        };
        self.focus = Some(target);
        self.scroll_focus = true;
        true
    }
    pub(super) fn receiving_paths(&self) -> Vec<OccurrencePath> {
        self.receivers
            .iter()
            .map(|row| row.source.occurrence.clone())
            .collect()
    }
    pub(super) fn focused_path(&self, path: &OccurrencePath) -> bool {
        self.focus.is_some() && self.target_for_path(path) == self.focus
    }
    fn focus_card(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        target: ConnectionPanelFocus,
    ) {
        if self.focus == Some(target) {
            ui.painter().rect_stroke(
                response.rect,
                4.,
                ui.visuals().selection.stroke,
                egui::StrokeKind::Inside,
            );
            if self.scroll_focus {
                ui.ctx().memory_mut(|memory| {
                    if let Some(focused) = memory.focused() {
                        memory.surrender_focus(focused);
                    }
                });
                response.scroll_to_me(Some(egui::Align::TOP));
                self.scroll_focus = false;
            }
        }
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn label(&self) -> &'static str {
        "Screw connection"
    }
    pub fn picker(&self) -> Option<&ScrewConnectionPick> {
        self.pick.as_ref()
    }
    pub fn set_entries(&mut self, entries: Arc<Vec<crate::library::LibraryEntry>>) {
        self.library.set_entries(entries);
    }
    pub fn library_request(&self) -> Option<&HardwareLibraryRequest> {
        self.library.pending_request()
    }
    pub fn accept_library(
        &mut self,
        request: &HardwareLibraryRequest,
        result: Result<Vec<SymbolDefinition>, String>,
    ) -> Result<(), String> {
        self.library.accept(request, result)
    }
    pub fn capture(&mut self, face: FaceInput) -> Result<(), String> {
        let pick = self
            .pick
            .clone()
            .ok_or("Choose a connection capture first")?;
        if face.source != pick.receiver.producer {
            return Err("Choose a face on the explicit receiving predecessor".into());
        }
        match pick.kind {
            ScrewConnectionPickKind::EntryAxis => {
                let mut source = ScrewConnectionAxisDefinition::new(
                    pick.receiver.clone(),
                    pick.occurrence.clone(),
                    face.intent.id,
                );
                source.reverse = true;
                if let Some(previous) = &self.axis {
                    source.offset = previous.source.offset.clone();
                    source.roll = previous.source.roll.clone();
                    source.reverse = previous.source.reverse;
                }
                self.axis = Some(ScrewConnectionAxisRequest {
                    id: self.axis_id,
                    source,
                    face: face.clone(),
                });
                self.add_current_receiver(pick.receiver, pick.occurrence, face);
            }
            ScrewConnectionPickKind::ReceiverEntry => {
                self.add_current_receiver(pick.receiver, pick.occurrence, face)
            }
            ScrewConnectionPickKind::GripEnd => {
                if !self.receivers.iter().any(|receiver| {
                    receiver.source.input == pick.receiver
                        && receiver.source.occurrence == pick.occurrence
                }) {
                    return Err("Add this receiving predecessor before picking its grip end".into());
                }
                if self.parameters.end_reference.is_none() {
                    self.manual_grip = Some(self.parameters.grip.clone());
                }
                self.parameters.grip = length(0.);
                self.parameters.end_reference = Some(ScrewConnectionEndReference {
                    receiver: pick.receiver,
                    occurrence: pick.occurrence,
                    face,
                });
            }
        }
        self.pick = None;
        self.stamp = FormStamp::new();
        Ok(())
    }
    fn add_current_receiver(
        &mut self,
        input: BodyInput,
        occurrence: OccurrencePath,
        face: FaceInput,
    ) {
        self.add_receiver(input.clone(), occurrence.clone());
        if let Some(receiver) = self.receivers.iter_mut().find(|receiver| {
            receiver.source.input == input && receiver.source.occurrence == occurrence
        }) {
            receiver.source.entry_reference = Some(face);
            receiver.source.station = length(0.);
            self.fields.remove_field(receiver.feature, "station");
        }
    }
    fn add_receiver(&mut self, input: BodyInput, occurrence: OccurrencePath) {
        if self.receivers.iter().any(|receiver| {
            receiver.source.input == input && receiver.source.occurrence == occurrence
        }) {
            return;
        }
        self.receivers.push(ConnectionReceiverRequest {
            feature: EntityId::new(),
            name: format!("Connection receiver {}", self.receivers.len() + 1),
            source: ConnectionReceiverDefinition {
                entry_reference: None,
                output: BodyOutput { body: input.body },
                input,
                occurrence,
                axis: self.axis_id,
                station: length(0.),
                cut: default_receiver_cut(self.mode),
                extent: ConnectionReceiverExtent::Through {},
                minimum_wall: length(0.5),
                seat: None,
            },
        });
    }
    fn select_mode(&mut self, mode: ScrewConnectionMode) {
        if mode == self.mode {
            return;
        }
        self.drafts.insert(
            mode_name(self.mode),
            ModeDraft {
                axis: self.axis.clone(),
                parameters: self.parameters.clone(),
                manual_grip: self.manual_grip.clone(),
                hardware: self.hardware.clone(),
                receivers: self.receivers.clone(),
                fields: self.fields.clone(),
                overrides: self.overrides.clone(),
            },
        );
        self.mode = mode;
        if let Some(draft) = self.drafts.remove(mode_name(mode)) {
            self.axis = draft.axis;
            self.parameters = draft.parameters;
            self.manual_grip = draft.manual_grip;
            self.hardware = draft.hardware;
            self.receivers = draft.receivers;
            self.fields = draft.fields;
            self.overrides = draft.overrides;
        } else {
            self.fields = Default::default();
            self.overrides.clear();
            self.hardware.retain(|role| {
                (role.role == ScrewConnectionHardwareRole::HeadWasher
                    && mode != ScrewConnectionMode::HangerBolt)
                    || (role.role == ScrewConnectionHardwareRole::NutWasher
                        && matches!(
                            mode,
                            ScrewConnectionMode::ThroughNut | ScrewConnectionMode::HangerBolt
                        ))
            });
            let role = if mode == ScrewConnectionMode::HangerBolt {
                ScrewConnectionHardwareRole::HangerBolt
            } else {
                ScrewConnectionHardwareRole::Screw
            };
            if let Some(mut screw) = generated(role) {
                if mode == ScrewConnectionMode::DirectWood
                    && let ScrewConnectionHardwareSource::Generated { part } = &mut screw.source
                {
                    part.parameters = default_custom_connection_wood_screw();
                }
                self.hardware.insert(0, screw);
            }
            if matches!(
                mode,
                ScrewConnectionMode::ThroughNut | ScrewConnectionMode::HangerBolt
            ) {
                self.hardware
                    .extend(generated(ScrewConnectionHardwareRole::Nut));
            }
            if mode == ScrewConnectionMode::WoodInsert {
                self.hardware
                    .extend(generated(ScrewConnectionHardwareRole::Insert));
            }
            for receiver in &mut self.receivers {
                receiver.source.cut = default_receiver_cut(mode);
            }
        }
        self.pick = None;
        self.library.cancel();
        self.pending_roles.clear();
        self.stamp = FormStamp::new();
    }
    pub fn request(
        &self,
        _document: &Document,
        enabled: bool,
    ) -> Result<ScrewConnectionCommand, String> {
        if !enabled {
            return Err("The native connection evaluator is unavailable".into());
        }
        if let Some(error) = self.fields.error.as_ref().or(self.error.as_ref()) {
            return Err(error.clone());
        }
        if self.remove {
            return Ok(ScrewConnectionCommand::Remove {
                connection: self.connection,
            });
        }
        if self.library.active() {
            return Err("Finish or cancel the hardware choice before Preview".into());
        }
        if self.pick.is_some() {
            return Err("Finish or cancel the receiving-face choice".into());
        }
        let axis = self
            .axis
            .clone()
            .ok_or("Choose the entry face and connection axis in the viewer")?;
        let required = match self.mode {
            ScrewConnectionMode::HangerBolt => ScrewConnectionHardwareRole::HangerBolt,
            _ => ScrewConnectionHardwareRole::Screw,
        };
        if !self.hardware.iter().any(|role| role.role == required) {
            return Err(format!(
                "Choose a sourced {} before Preview",
                role_name(required)
            ));
        }
        if self.receivers.is_empty() {
            return Err("Choose at least one actual receiving predecessor before Preview".into());
        }
        if matches!(
            self.mode,
            ScrewConnectionMode::ThroughNut | ScrewConnectionMode::HangerBolt
        ) && !self
            .hardware
            .iter()
            .any(|role| role.role == ScrewConnectionHardwareRole::Nut)
        {
            return Err(
                "Choose a nut with a complete known engagement interval before Preview".into(),
            );
        }
        if self.mode == ScrewConnectionMode::WoodInsert
            && !self
                .hardware
                .iter()
                .any(|role| role.role == ScrewConnectionHardwareRole::Insert)
        {
            return Err("Choose a sourced insert with complete installation geometry".into());
        }
        Ok(ScrewConnectionCommand::Set(Box::new(
            ScrewConnectionRequest {
                connection: self.connection,
                name: self.name.clone(),
                mode: self.mode,
                axis,
                hardware: self.hardware.clone(),
                receivers: self.receivers.clone(),
                parameters: self.parameters.clone(),
            },
        )))
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState<'_>,
    ) -> Option<ModelEvent> {
        controls::inspector_layout(ui);
        self.fields.start();
        ui.strong(if self.editing {
            "Edit screw connection"
        } else {
            "Attach screw connection"
        });
        form::preview_status(ui, self.stamp, preview);
        ui.label("Connection name");
        let mut changed = ui
            .add(
                egui::TextEdit::singleline(&mut self.name)
                    .id(ui.make_persistent_id((self.connection, "name")))
                    .desired_width(controls::inspector_width(ui)),
            )
            .changed();
        let mut selected_mode = self.mode;
        controls::inspector_combo(ui, (self.connection, "mode"))
            .selected_text(mode_name(self.mode))
            .show_ui(ui, |ui| {
                for (mode, label) in modes() {
                    ui.selectable_value(&mut selected_mode, mode, label);
                }
            });
        if selected_mode != self.mode {
            self.select_mode(selected_mode);
            changed = true;
        }
        ui.small("One connection owns its hardware and receiving cuts. Preview changes no source; Apply commits them together.");
        if self.library.active() {
            if let Some((role, definition)) = self.library.show(ui) {
                match self.use_definition(document, role, definition) {
                    Ok(()) => changed = true,
                    Err(error) => self.error = Some(error),
                }
            }
            changed |= !self.library.active();
            if !self.library.active() {
                self.pending_roles.clear();
            }
            if changed {
                self.stamp = FormStamp::new();
            }
            return match form::actions(ui, false, changed, self.stamp, preview) {
                Some(FormAction::Cancel) => Some(ModelEvent::Cancel),
                _ => changed.then_some(ModelEvent::Changed),
            };
        }
        self.show_axis(ui, document, &mut changed);
        self.show_grip(ui, document, &mut changed);
        self.show_hardware(ui, document, &mut changed);
        self.show_receivers(ui, document, &mut changed);

        self.fields.dimension(
            ui,
            document,
            self.connection,
            ("engagement", "Minimum engagement", "mm"),
            &mut self.parameters.minimum_engagement,
        );
        self.fields.dimension(
            ui,
            document,
            self.connection,
            ("bottom", "Blind tip clearance", "mm"),
            &mut self.parameters.blind_tip_clearance,
        );
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.section, "Section through axis");
            ui.checkbox(&mut self.explode, "Exploded stack");
        });
        ui.small("Section and exploded display are inspection views. Native checks use the complete unshifted connection.");
        if let Some(review) = &self.review {
            ui.separator();
            ui.strong("Native connection checks");
            ui.label(format!("Checked grip: {:.3} mm", review.grip_mm));
            if let Some(engagement) = review.actual_engagement_mm {
                ui.label(format!(
                    "Actual engagement: {:.3} mm; required: {:.3} mm",
                    engagement, review.minimum_engagement_mm
                ));
            }
            if let Some(engagement) = review.actual_wood_engagement_mm {
                ui.label(format!(
                    "Actual wood engagement: {:.3} mm; required: {:.3} mm",
                    engagement, review.minimum_engagement_mm
                ));
            }
            ui.label(format!(
                "Blind tip clearance requirement: {:.3} mm",
                review.blind_tip_clearance_mm
            ));
            ui.small(&review.scope);
        }

        if self.editing {
            changed |= ui
                .checkbox(
                    &mut self.remove,
                    "Remove this connection and its owned outputs",
                )
                .changed();
        }
        changed |= self.fields.changed;
        if changed {
            self.stamp = FormStamp::new();
        }
        let request = self.request(document, enabled);
        if let Err(error) = &request {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        match form::actions(ui, request.is_ok(), changed, self.stamp, preview) {
            Some(FormAction::Preview) => request.ok().map(|request| ModelEvent::Preview {
                stamp: self.stamp,
                request: ModelRequest::ScrewConnection(Box::new(request)),
            }),
            Some(FormAction::Apply) => Some(ModelEvent::Apply),
            Some(FormAction::Cancel) => Some(ModelEvent::Cancel),
            None => changed.then_some(ModelEvent::Changed),
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod selection_tests;

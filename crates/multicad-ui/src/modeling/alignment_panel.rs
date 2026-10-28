//! Persistent alignment controls. Anchors are explicit canonical captures;
//! hover geometry and candidate display transforms never become authoring data.
mod picker;
use super::{
    FormStamp, PreviewState,
    dimension::DimensionField,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{
    controls, egui,
    viewport::{Scene, Selection},
};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{self, AlignmentAnchor, AlignmentInput, AlignmentMode, AlignmentParameters, BodyInput},
    project_file::ContentHash,
    references::{Attachment, Origin},
    sketch::Datum,
};
use std::collections::BTreeSet;
#[derive(Clone, Debug)]
pub enum AlignmentCommand {
    Part(model::AlignmentRequest),
    Instance(model::InstanceAlignmentRequest),
    EditPart {
        feature: EntityId,
        parameters: AlignmentParameters,
        references: Vec<AlignmentInput>,
    },
    EditInstance {
        driver: EntityId,
        instance: EntityId,
        parameters: AlignmentParameters,
        references: Vec<AlignmentInput>,
    },
}
impl AlignmentCommand {
    pub(crate) fn edit_id(&self) -> Option<EntityId> {
        match self {
            Self::EditPart { feature, .. } => Some(*feature),
            Self::EditInstance { driver, .. } => Some(*driver),
            _ => None,
        }
    }

    pub(crate) fn prepare(
        &self,
        session: &multicad_core::commands::Session,
        actor: multicad_core::commands::Actor,
    ) -> Result<model::ModelPrepared, String> {
        let rev = session.revision();
        match self {
            Self::Part(request) => model::prepare_alignment(session, rev, actor, request),
            Self::Instance(request) => model::prepare_instance_alignments(
                session,
                rev,
                actor,
                std::slice::from_ref(request),
            )
            .map(|result| model::ModelPrepared {
                prepared: result.prepared,
                outputs: vec![],
            }),
            Self::EditPart {
                feature,
                parameters,
                references,
            } => {
                model::prepare_alignment_edit(session, rev, actor, *feature, parameters, references)
            }
            Self::EditInstance {
                driver,
                instance,
                parameters,
                references,
            } => {
                let (source, _) = model::read_instance_alignment(session.document(), *driver)
                    .map_err(|e| e.to_string())?;
                if source.owner != *instance {
                    return Err("Alignment driver belongs to another Instance".into());
                }
                model::prepare_instance_alignment_edit(
                    session, rev, actor, *driver, parameters, references,
                )
                .map(|result| model::ModelPrepared {
                    prepared: result.prepared,
                    outputs: vec![],
                })
            }
        }
        .map_err(|e| e.to_string())
    }
    pub(crate) fn instances(&self) -> Vec<EntityId> {
        match self {
            Self::Instance(request) => vec![request.instance],
            Self::EditInstance { instance, .. } => vec![*instance],
            _ => vec![],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Source,
    Target,
}
#[derive(Clone)]
struct AnchorChoice {
    anchor: AlignmentAnchor,
    input: Option<AlignmentInput>,
    label: String,
}
struct PendingFrame {
    side: Side,
    input: AlignmentInput,
    anchors: Vec<Origin>,
    label: String,
}
pub(super) struct AlignmentPanel {
    command: AlignmentCommand,
    moving: EntityId,
    component: EntityId,
    producer: EntityId,
    name: String,
    stamp: FormStamp,
    source: Option<AnchorChoice>,
    target: Option<AnchorChoice>,
    mode: AlignmentMode,
    axis_faces: bool,
    offset: [DimensionField; 3],
    roll: DimensionField,
    opposed: bool,
    up: [f64; 3],
    picking: Option<Side>,
    highlight: Option<Selection>,
    pending: Option<PendingFrame>,
    error: Option<String>,
}
impl AlignmentPanel {
    pub fn new(
        document: &Document,
        revision: u64,
        scene: &Scene,
        selected: &BTreeSet<EntityId>,
    ) -> Result<Self, String> {
        if selected.len() != 1 || !super::placement::available(document, revision, scene, selected)
        {
            return Err(
                "Select one current Part or Instance to align; the target is chosen separately"
                    .into(),
            );
        }
        if scene
            .model_evaluation()
            .is_none_or(|info| ContentHash::document(document).ok() != Some(info.source_hash))
        {
            return Err("Alignment requires a current source-bound native model".into());
        }
        let moving = *selected.first().expect("one target");
        let entity = document.get(moving).ok_or("Moving object is missing")?;
        let occurrence = scene
            .reference_instances()
            .iter()
            .find(|i| i.id == moving)
            .ok_or("Moving object is unavailable")?;
        let parameters = AlignmentParameters::default();
        let (component, command) = match entity.data {
            EntityData::Part { .. } => (
                moving,
                AlignmentCommand::Part(model::AlignmentRequest::new(
                    BodyInput {
                        body: moving,
                        producer: occurrence.geometry,
                    },
                    parameters.clone(),
                    vec![],
                )),
            ),
            EntityData::Instance { component, .. } => (
                component,
                AlignmentCommand::Instance(model::InstanceAlignmentRequest::new(
                    moving,
                    parameters.clone(),
                    vec![],
                )),
            ),
            _ => return Err("Choose a Part or Instance".into()),
        };
        Self::from_command(
            command,
            moving,
            component,
            occurrence.geometry,
            entity.name.clone(),
            parameters,
            vec![],
            false,
        )
    }
    pub fn existing(document: &Document, id: EntityId) -> Result<Self, String> {
        let (command, moving, component, producer, parameters, references) =
            if let Ok((source, references)) = model::read_alignment(document, id) {
                (
                    AlignmentCommand::EditPart {
                        feature: id,
                        parameters: source.parameters.clone(),
                        references: references.clone(),
                    },
                    source.input.body,
                    source.input.body,
                    source.input.producer,
                    source.parameters,
                    references,
                )
            } else {
                let (source, references) =
                    model::read_instance_alignment(document, id).map_err(|e| e.to_string())?;
                let Some(EntityData::Instance { component, .. }) =
                    document.get(source.owner).map(|e| &e.data)
                else {
                    return Err("Alignment owner is missing".into());
                };
                let Some(EntityData::Part { features, .. }) =
                    document.get(*component).map(|e| &e.data)
                else {
                    return Err("Alignment component is missing".into());
                };
                let producer = *features
                    .iter()
                    .rev()
                    .find(|id| {
                        matches!(
                            document.get(**id).map(|e| &e.data),
                            Some(EntityData::Feature {
                                suppressed: false,
                                ..
                            })
                        )
                    })
                    .ok_or("Component has no active producer")?;
                (
                    AlignmentCommand::EditInstance {
                        driver: id,
                        instance: source.owner,
                        parameters: source.parameters.clone(),
                        references: references.clone(),
                    },
                    source.owner,
                    *component,
                    producer,
                    source.parameters,
                    references,
                )
            };
        let name = document
            .get(moving)
            .ok_or("Moving object is missing")?
            .name
            .clone();
        Self::from_command(
            command, moving, component, producer, name, parameters, references, true,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn from_command(
        command: AlignmentCommand,
        moving: EntityId,
        component: EntityId,
        producer: EntityId,
        name: String,
        parameters: AlignmentParameters,
        references: Vec<AlignmentInput>,
        existing: bool,
    ) -> Result<Self, String> {
        let choice = |anchor: &AlignmentAnchor| -> Result<AnchorChoice, String> {
            let input = if let AlignmentAnchor::Geometry { reference } = anchor {
                Some(
                    references
                        .iter()
                        .find(|r| r.intent.id == *reference)
                        .cloned()
                        .ok_or("Saved alignment is missing its unique geometry link")?,
                )
            } else {
                None
            };
            Ok(AnchorChoice {
                anchor: anchor.clone(),
                input,
                label: "Saved canonical anchor".into(),
            })
        };
        Ok(Self {
            command,
            moving,
            component,
            producer,
            name,
            stamp: FormStamp::new(),
            source: if existing {
                Some(choice(&parameters.source)?)
            } else {
                None
            },
            target: if existing {
                Some(choice(&parameters.target)?)
            } else {
                None
            },
            mode: parameters.mode,
            axis_faces: false,
            offset: [
                DimensionField::from_dimension(&parameters.offset[0], "mm")?,
                DimensionField::from_dimension(&parameters.offset[1], "mm")?,
                DimensionField::from_dimension(&parameters.offset[2], "mm")?,
            ],
            roll: DimensionField::from_dimension(&parameters.roll, "deg")?,
            opposed: parameters.opposed,
            up: parameters.up,
            picking: None,
            highlight: None,
            pending: None,
            error: None,
        })
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn picking(&self) -> bool {
        self.picking.is_some()
    }
    pub fn highlight(&self) -> Option<Selection> {
        self.highlight.clone()
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<AlignmentCommand, String> {
        if !enabled {
            return Err("Native alignment preview is unavailable".into());
        }
        if self.picking.is_some() {
            return Err("Finish or cancel anchor picking before Preview".into());
        }
        if self.pending.is_some() {
            return Err("Choose an explicit semantic face anchor before Preview".into());
        }
        let source = self
            .source
            .as_ref()
            .ok_or("Choose the moving source anchor")?;
        let target = self.target.as_ref().ok_or("Choose a target anchor")?;
        let parameters = AlignmentParameters {
            source: source.anchor.clone(),
            target: target.anchor.clone(),
            mode: self.mode,
            offset: [
                self.offset[0].resolve(document)?,
                self.offset[1].resolve(document)?,
                self.offset[2].resolve(document)?,
            ],
            roll: self.roll.resolve(document)?,
            opposed: self.opposed,
            up: self.up,
        };
        parameters.validate().map_err(|e| e.to_string())?;
        let references = [source.input.clone(), target.input.clone()]
            .into_iter()
            .flatten()
            .collect();
        Ok(match &self.command {
            AlignmentCommand::Part(request) => {
                let mut request = request.clone();
                request.parameters = parameters;
                request.references = references;
                AlignmentCommand::Part(request)
            }
            AlignmentCommand::Instance(request) => {
                let mut request = request.clone();
                request.parameters = parameters;
                request.references = references;
                AlignmentCommand::Instance(request)
            }
            AlignmentCommand::EditPart { feature, .. } => AlignmentCommand::EditPart {
                feature: *feature,
                parameters,
                references,
            },
            AlignmentCommand::EditInstance {
                driver, instance, ..
            } => AlignmentCommand::EditInstance {
                driver: *driver,
                instance: *instance,
                parameters,
                references,
            },
        })
    }
    fn remember(&mut self, command: &AlignmentCommand) {
        let parameters = match command {
            AlignmentCommand::Part(request) => &request.parameters,
            AlignmentCommand::Instance(request) => &request.parameters,
            AlignmentCommand::EditPart { parameters, .. }
            | AlignmentCommand::EditInstance { parameters, .. } => parameters,
        };
        for (field, value) in self.offset.iter_mut().zip(&parameters.offset) {
            field.remember(value);
        }
        self.roll.remember(&parameters.roll);
    }
    fn set_anchor(&mut self, side: Side, choice: AnchorChoice) {
        match side {
            Side::Source => self.source = Some(choice),
            Side::Target => self.target = Some(choice),
        }
        self.picking = None;
        self.pending = None;
        self.highlight = None;
        self.error = None;
    }
    fn anchor_controls(&mut self, ui: &mut egui::Ui, document: &Document, side: Side) -> bool {
        let mut changed = false;
        let title = match side {
            Side::Source => "Moving anchor · component local",
            Side::Target => "Target anchor · world",
        };
        ui.strong(title);
        ui.label(
            match side {
                Side::Source => self.source.as_ref(),
                Side::Target => self.target.as_ref(),
            }
            .map_or("No anchor selected", |a| a.label.as_str()),
        );
        ui.horizontal_wrapped(|ui| {
            if ui.button("Pick geometry").on_hover_text("Capture named current B-rep topology; no display coordinates are stored").clicked(){self.picking=Some(side);self.pending=None;self.error=None;changed=true;}
            controls::inspector_combo(ui,("alignment-datum",side==Side::Source)).selected_text("Choose datum…").show_ui(ui,|ui| {
                for datum in Datum::ALL {if ui.selectable_label(false,format!("{} {}",if side==Side::Source && self.moving!=self.component {"Component"}else{"World"},datum.label())).clicked(){let f=datum.frame();self.set_anchor(side,AnchorChoice{anchor:AlignmentAnchor::Frame{frame:multicad_core::references::ReferenceFrame{origin_mm:f.origin,u:f.u,v:f.v}},input:None,label:format!("Explicit {} datum",datum.label())});changed=true;}}
                for entity in document.entities().filter(|e|matches!(e.data,EntityData::Plane{..})){if ui.selectable_label(false,&entity.name).clicked(){self.set_anchor(side,AnchorChoice{anchor:AlignmentAnchor::Plane{plane:entity.id},input:None,label:format!("Construction plane · {}",entity.name)});changed=true;}}
            }).response.on_hover_text("Select an explicit world/component basis or a persistent construction-plane identity");
        });
        changed
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState<'_>,
    ) -> Option<ModelEvent> {
        controls::inspector_layout(ui);
        ui.strong("Referenced alignment");
        ui.label(&self.name);
        ui.small("Part transforms change the definition; Instance alignment changes only its occurrence. Source and target remain persistent references.");
        let mut changed = false;
        let before = self.mode;
        controls::inspector_combo(ui, "alignment-mode")
            .selected_text(match self.mode {
                AlignmentMode::Frame => "Face / frame",
                AlignmentMode::Axis => "Edge / axis / normal",
                AlignmentMode::Position => "Point / origin",
            })
            .show_ui(ui, |ui| {
                for (mode, label) in [
                    (AlignmentMode::Frame, "Face / frame"),
                    (AlignmentMode::Axis, "Edge / axis / normal"),
                    (AlignmentMode::Position, "Point / origin"),
                ] {
                    changed |= ui.selectable_value(&mut self.mode, mode, label).changed();
                }
            });
        if before != self.mode {
            self.source = None;
            self.target = None;
            self.pending = None;
            self.picking = None;
            self.highlight = None;
        }
        if self.mode == AlignmentMode::Axis {
            changed|=ui.checkbox(&mut self.axis_faces,"Pick face normals instead of edges").on_hover_text("Normal alignment uses the real plane normal and explicit Up vector; face UV is not inferred").changed();
        }
        ui.separator();
        changed |= self.anchor_controls(ui, document, Side::Source);
        changed |= self.anchor_controls(ui, document, Side::Target);
        if self.picking.is_some() {
            ui.small("Click the current model to capture this anchor");
            if ui
                .button("Cancel picking")
                .on_hover_text("Keep the previous anchor and discard only the pending capture")
                .clicked()
            {
                self.picking = None;
                self.highlight = None;
                self.pending = None;
                changed = true;
            }
        }
        if let Some(pending) = &self.pending {
            let mut selected = None;
            ui.label("This face has several semantic anchors. Choose one explicitly:");
            for anchor in &pending.anchors {
                if ui.button(format!("{} · {}",anchor.role,anchor.source)).on_hover_text("Use this persistent authored semantic frame, rather than an arbitrary native plane basis").clicked(){selected=Some(anchor.clone());}
            }
            if let Some(anchor) = selected {
                let pending = self.pending.take().expect("shown capture");
                let mut input = pending.input;
                input.intent.attachment = Some(Attachment {
                    anchor,
                    offset_mm: [0.; 3],
                    angle_rad: 0.,
                });
                let reference = input.intent.id;
                self.set_anchor(
                    pending.side,
                    AnchorChoice {
                        anchor: AlignmentAnchor::Geometry { reference },
                        input: Some(input),
                        label: pending.label,
                    },
                );
                changed = true;
            }
        }
        ui.separator();
        ui.strong(if self.mode == AlignmentMode::Position {
            "Offsets · world XYZ"
        } else {
            "Offsets · target frame"
        });
        let labels = if self.mode == AlignmentMode::Position {
            ["X offset", "Y offset", "Z offset"]
        } else {
            ["U offset", "V offset", "Normal offset"]
        };
        for (field, label) in self.offset.iter_mut().zip(labels) {
            changed|=field.show(ui,label,"Length along the target alignment frame; unit-aware parameter formulas are retained");
        }
        {
            if self.mode == AlignmentMode::Position {
                ui.small(
                    "Point alignment preserves rotation; Roll must be zero and opposed axes off.",
                );
            }
            changed |= self.roll.show(
                ui,
                "Roll",
                "Signed rotation about the target normal after matching anchors",
            );
            changed |= ui
                .checkbox(&mut self.opposed, "Opposed normals / axes")
                .on_hover_text("Reverse the target normal using a proper rotation")
                .changed();
        }
        if self.mode == AlignmentMode::Axis {
            ui.label("World Up vector");
            for (value, label) in self.up.iter_mut().zip(["Up X", "Up Y", "Up Z"]) {
                ui.horizontal(|ui| {
                    ui.label(label);
                    changed |= ui
                        .add(egui::DragValue::new(value).speed(0.1))
                        .on_hover_text(
                            "Explicit roll reference; a parallel or zero Up vector is rejected",
                        )
                        .changed();
                });
            }
        }
        let validation = self.request(document, enabled);
        if let Err(error) = &validation {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        form::preview_status(ui, self.stamp, preview);
        let action = form::actions(
            ui,
            validation.is_ok() && self.picking.is_none(),
            changed,
            self.stamp,
            preview,
        );
        if action == Some(FormAction::Cancel) {
            return Some(ModelEvent::Cancel);
        }
        if changed {
            self.stamp.changed();
        }
        match action {
            Some(FormAction::Preview) => validation.ok().map(|request| {
                self.remember(&request);
                ModelEvent::Preview {
                    stamp: self.stamp,
                    request: ModelRequest::Alignment(Box::new(request)),
                }
            }),
            Some(FormAction::Apply) if !changed && preview.ready(self.stamp) => {
                Some(ModelEvent::Apply)
            }
            _ if changed => Some(ModelEvent::Changed),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;

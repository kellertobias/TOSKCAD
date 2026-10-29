use super::{
    ExtrudeCapabilities, ExtrudeEvent, ExtrudePanel, FormStamp, PreviewState,
    advanced::AdvancedPanel,
    alignment_panel::AlignmentPanel,
    architecture::{ArchitectureCommand, ArchitecturePanel},
    boolean::BooleanPanel,
    boolean_edit::BooleanEditPanel,
    cabinet::{CabinetCommand, CabinetPanel},
    edge_treatment::EdgeTreatmentPanel,
    group_edit::GroupEditPanel,
    joint::{JointCommand, JointPanel},
    machining::{MachiningCommand, MachiningPanel},
    placement::PlacementPanel,
    primitive::PrimitivePanel,
    revolve::RevolvePanel,
    scaling::ScalePanel,
    split::{SplitEvent, SplitPanel},
    wall::{WallCommand, WallPanel},
};
use crate::egui;
use multicad_core::{
    commands::{Actor, Session},
    document::{Document, EntityData},
    model::{
        self, BodyInput, BooleanRequest, ExtrudeRequest, ModelPrepared, PrimitiveRequest,
        RevolveRequest,
    },
};

/// Typed tool intent, independent of display meshes and native model ownership.
#[derive(Clone, Debug)]
pub enum ModelRequest {
    Extrude(Box<ExtrudeRequest>),
    Primitive(Box<PrimitiveRequest>),
    Revolve(Box<RevolveRequest>),
    Booleans(Vec<BooleanRequest>),
    ExtrudeBoolean(Box<model::ExtrudeBooleanRequest>),
    Split(Box<model::SplitRequest>),
    ExtrudeGroupEdit(Box<model::ExtrudeGroupEdit>),
    GroupedExtrude(Box<model::GroupedExtrudeRequest>),
    Placement(Box<super::PlacementCommand>),
    Scale(Box<super::ScaleCommand>),
    EdgeTreatment(Box<super::EdgeTreatmentCommand>),
    Advanced(Box<super::AdvancedCommand>),
    Alignment(Box<super::AlignmentCommand>),
    Wall(Box<WallCommand>),
    Architecture(Box<ArchitectureCommand>),
    Cabinet(Box<CabinetCommand>),
    Joint(Box<JointCommand>),
    Machining(Box<MachiningCommand>),
}
pub(super) struct ToolProof {
    pub input: BodyInput,
    pub consumed: bool,
}
impl ModelRequest {
    pub(super) fn prepare_edit(
        &self,
        session: &Session,
        actor: Actor,
        feature: multicad_core::document::EntityId,
    ) -> Result<ModelPrepared, String> {
        match self {
            Self::Advanced(command) => {
                if command.edit_id() != Some(feature) {
                    return Err("Advanced edit changed its producer identity".into());
                }
                return command.prepare(session, actor);
            }
            Self::EdgeTreatment(command) => {
                if command.edit_id() != Some(feature) {
                    return Err("Edge-treatment edit changed its producer identity".into());
                }
                return command.prepare(session, actor);
            }
            Self::Scale(command) => {
                if command.edit_id() != Some(feature) {
                    return Err("Scale edit changed its saved producer identity".into());
                }
                return command.prepare(session, actor);
            }
            Self::Placement(command) => {
                if command.edit_id() != Some(feature) {
                    return Err("Placement edit changed its saved producer identity".into());
                }
                return command.prepare(session, actor);
            }
            Self::Alignment(command) => {
                if command.edit_id() != Some(feature) {
                    return Err("Alignment edit changed its saved producer identity".into());
                }
                return command.prepare(session, actor);
            }
            Self::Wall(command) => {
                if command.edit_id() != Some(feature) {
                    return Err("Wall edit changed its producer identity".into());
                }
                return command.prepare(session, actor);
            }
            Self::Architecture(command) => {
                if command.edit_id() != Some(feature) {
                    return Err("Slab or stair edit changed its producer identity".into());
                }
                return command.prepare(session, actor);
            }
            Self::Cabinet(command) => {
                if command.edit_id() != Some(feature) {
                    return Err("Panel edit changed its producer identity".into());
                }
                return command.prepare(session, actor);
            }
            Self::Extrude(request) => model::prepare_extrude_edit_with_faces(
                session,
                session.revision(),
                actor,
                feature,
                &request.parameters,
                &request.faces,
            ),
            Self::Primitive(request) => model::prepare_primitive_edit(
                session,
                session.revision(),
                actor,
                feature,
                &request.parameters,
            ),
            Self::Revolve(request) => model::prepare_revolve_edit(
                session,
                session.revision(),
                actor,
                feature,
                &request.parameters,
            ),
            Self::Booleans(requests) if requests.len() == 1 => {
                let request = &requests[0];
                model::prepare_boolean_edit(
                    session,
                    session.revision(),
                    actor,
                    feature,
                    &model::BooleanEdit {
                        operation: request.operation,
                        tools: request.tools.clone(),
                        keep_tools: request.keep_tools,
                    },
                )
            }
            _ => return Err("Edit one applied feature at a time".into()),
        }
        .map_err(|error| error.to_string())
    }
    pub(super) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        match self {
            Self::Advanced(command) => {
                if command.edit_id().is_some() {
                    return Err("Reopen the saved advanced feature before editing".into());
                }
                return command.prepare(session, actor);
            }
            Self::EdgeTreatment(command) => {
                if command.edit_id().is_some() {
                    return Err("Reopen the saved edge treatment before editing".into());
                }
                return command.prepare(session, actor);
            }
            Self::Scale(command) => {
                if command.edit_id().is_some() {
                    return Err("Reopen the saved scale before editing its source".into());
                }
                return command.prepare(session, actor);
            }
            Self::Placement(command) => {
                if command.edit_id().is_some() {
                    return Err("Reopen the saved placement before editing its source".into());
                }
                return command.prepare(session, actor);
            }
            Self::Alignment(command) => {
                if command.edit_id().is_some() {
                    return Err("Reopen the saved alignment before editing its source".into());
                }
                return command.prepare(session, actor);
            }
            Self::Wall(command) => {
                if command.edit_id().is_some() {
                    return Err("Reopen the saved wall before editing it".into());
                }
                return command.prepare(session, actor);
            }
            Self::Architecture(command) => {
                if command.edit_id().is_some() {
                    return Err("Reopen the saved slab or stair before editing it".into());
                }
                return command.prepare(session, actor);
            }
            Self::Cabinet(command) => {
                if command.edit_id().is_some() {
                    return Err("Reopen the saved panel before editing it".into());
                }
                return command.prepare(session, actor);
            }
            Self::Joint(command) => return command.prepare(session, actor),
            Self::Machining(command) => return command.prepare(session, actor),
            Self::Extrude(request) => {
                model::prepare_extrude(session, session.revision(), actor, request)
            }
            Self::Primitive(request) => {
                model::prepare_primitive(session, session.revision(), actor, request)
            }
            Self::Revolve(request) => {
                model::prepare_revolve(session, session.revision(), actor, request)
            }
            Self::Booleans(requests) => {
                model::prepare_booleans(session, session.revision(), actor, requests)
            }
            Self::ExtrudeBoolean(request) => {
                model::prepare_extrude_boolean(session, session.revision(), actor, request)
            }
            Self::Split(request) => {
                model::prepare_split(session, session.revision(), actor, request)
            }
            Self::GroupedExtrude(request) => {
                model::prepare_grouped_extrude(session, session.revision(), actor, request)
            }
            Self::ExtrudeGroupEdit(_) => {
                return Err(
                    "Reopen the recorded extrusion operation before editing its target set".into(),
                );
            }
        }
        .map_err(|error| error.to_string())
    }
    /// Final membership follows producer versions: later history can reactivate
    /// a Part whose earlier producer was consumed by a Boolean operation.
    pub(super) fn tool_proof(&self, document: &Document) -> Result<Vec<ToolProof>, String> {
        let inputs: Vec<_> = match self {
            Self::Booleans(requests) => requests
                .iter()
                .flat_map(|request| request.tools.iter().map(|tool| (tool, !request.keep_tools)))
                .collect(),
            Self::ExtrudeBoolean(request) => request
                .booleans
                .iter()
                .flat_map(|request| request.tools.iter().map(|tool| (tool, !request.keep_tools)))
                .collect(),
            Self::Split(request) => vec![
                (&request.target, true),
                (&request.tool, !request.keep_tools),
            ],
            _ => return Ok(vec![]),
        };
        let mut tools = std::collections::BTreeMap::new();
        for (tool, consume) in inputs {
            let Some(EntityData::Part { features, .. }) =
                document.get(tool.body).map(|entity| &entity.data)
            else {
                return Err("Model operation input body is unavailable".into());
            };
            let head = features
                .iter()
                .rev()
                .copied()
                .find(|id| {
                    matches!(
                        document.get(*id).map(|entity| &entity.data),
                        Some(EntityData::Feature {
                            suppressed: false,
                            ..
                        })
                    )
                })
                .ok_or("Model operation input has no current producer")?;
            let consumed = consume && head == tool.producer;
            let entry = tools.entry(tool.body).or_insert((
                BodyInput {
                    body: tool.body,
                    producer: head,
                },
                false,
            ));
            entry.1 |= consumed;
        }
        Ok(tools
            .into_values()
            .map(|(input, consumed)| ToolProof { input, consumed })
            .collect())
    }
}
pub(super) enum ModelEvent {
    Changed,
    Preview {
        stamp: FormStamp,
        request: ModelRequest,
    },
    Apply,
    Cancel,
}
pub(super) enum ModelForm {
    Extrude(Box<ExtrudePanel>),
    Primitive(Box<PrimitivePanel>),
    Revolve(Box<RevolvePanel>),
    Boolean(Box<BooleanPanel>),
    BooleanEdit(Box<BooleanEditPanel>),
    Split(Box<SplitPanel>),
    ExtrudeGroup(Box<GroupEditPanel>),
    Placement(Box<PlacementPanel>),
    Scale(Box<ScalePanel>),
    EdgeTreatment(Box<EdgeTreatmentPanel>),
    Advanced(Box<AdvancedPanel>),
    Alignment(Box<AlignmentPanel>),
    Wall(Box<WallPanel>),
    Architecture(Box<ArchitecturePanel>),
    Cabinet(Box<CabinetPanel>),
    Joint(Box<JointPanel>),
    Machining(Box<MachiningPanel>),
}
impl ModelForm {
    pub fn stamp(&self) -> FormStamp {
        match self {
            Self::Extrude(panel) => panel.stamp().expect("active extrusion form"),
            Self::Primitive(panel) => panel.stamp(),
            Self::Revolve(panel) => panel.stamp(),
            Self::Boolean(panel) => panel.stamp(),
            Self::BooleanEdit(panel) => panel.stamp(),
            Self::Split(panel) => panel.stamp(),
            Self::ExtrudeGroup(panel) => panel.stamp(),
            Self::Placement(panel) => panel.stamp(),
            Self::Scale(panel) => panel.stamp(),
            Self::EdgeTreatment(panel) => panel.stamp(),
            Self::Advanced(panel) => panel.stamp(),
            Self::Alignment(panel) => panel.stamp(),
            Self::Wall(panel) => panel.stamp(),
            Self::Architecture(panel) => panel.stamp(),
            Self::Cabinet(panel) => panel.stamp(),
            Self::Joint(panel) => panel.stamp(),
            Self::Machining(panel) => panel.stamp(),
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Extrude(_) => "Extrude",
            Self::Primitive(panel) => panel.label(),
            Self::Revolve(_) => "Revolve",
            Self::Boolean(_) => "Boolean",
            Self::BooleanEdit(_) => "Boolean",
            Self::Split(_) => "Split",
            Self::ExtrudeGroup(_) => "Extrusion operation",
            Self::Placement(panel) => panel.label(),
            Self::Scale(panel) => panel.label(),
            Self::EdgeTreatment(panel) => panel.label(),
            Self::Advanced(panel) => panel.label(),
            Self::Alignment(_) => "Referenced alignment",
            Self::Wall(panel) => panel.label(),
            Self::Architecture(panel) => panel.label(),
            Self::Cabinet(panel) => panel.label(),
            Self::Joint(panel) => panel.label(),
            Self::Machining(panel) => panel.label(),
        }
    }
    pub fn request(
        &self,
        document: &Document,
        capabilities: ExtrudeCapabilities,
    ) -> Result<ModelRequest, String> {
        match self {
            Self::Advanced(panel) => panel
                .request(document, capabilities.new_body)
                .map(|command| ModelRequest::Advanced(Box::new(command))),
            Self::EdgeTreatment(panel) => panel
                .request(document, capabilities.new_body)
                .map(|command| ModelRequest::EdgeTreatment(Box::new(command))),
            Self::Alignment(panel) => panel
                .request(document, capabilities.new_body)
                .map(|command| ModelRequest::Alignment(Box::new(command))),
            Self::Wall(panel) => panel
                .request(document, capabilities.new_body)
                .map(|command| ModelRequest::Wall(Box::new(command))),
            Self::Architecture(panel) => panel
                .request(document, capabilities.new_body)
                .map(|command| ModelRequest::Architecture(Box::new(command))),
            Self::Cabinet(panel) => panel
                .request(document, capabilities.new_body)
                .map(|command| ModelRequest::Cabinet(Box::new(command))),
            Self::Joint(panel) => panel
                .request(document, capabilities.new_body)
                .map(|command| ModelRequest::Joint(Box::new(command))),
            Self::Machining(panel) => panel
                .request(document, capabilities.new_body)
                .map(|command| ModelRequest::Machining(Box::new(command))),
            Self::Placement(panel) => panel
                .request(document, capabilities.new_body)
                .map(|command| ModelRequest::Placement(Box::new(command))),
            Self::Scale(panel) => panel
                .request(document, capabilities.new_body)
                .map(|command| ModelRequest::Scale(Box::new(command))),
            Self::Extrude(panel) => panel
                .request(document, capabilities)
                .map(|request| ModelRequest::Extrude(Box::new(request))),
            Self::Primitive(panel) => panel
                .request(document, capabilities.new_body)
                .map(|request| ModelRequest::Primitive(Box::new(request))),
            Self::Revolve(panel) => panel
                .request(document, capabilities.new_body)
                .map(|request| ModelRequest::Revolve(Box::new(request))),
            Self::Boolean(panel) => panel
                .request(document, capabilities.new_body)
                .map(ModelRequest::Booleans),
            Self::BooleanEdit(panel) => panel
                .request(document, capabilities.new_body)
                .map(|request| ModelRequest::Booleans(vec![request])),
            Self::Split(panel) => panel
                .request(document, capabilities.new_body)
                .map(|request| ModelRequest::Split(Box::new(request))),
            Self::ExtrudeGroup(panel) => panel
                .request(document, capabilities)
                .map(|request| ModelRequest::ExtrudeGroupEdit(Box::new(request))),
        }
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        capabilities: ExtrudeCapabilities,
        preview: PreviewState<'_>,
    ) -> Option<ModelEvent> {
        match self {
            Self::Extrude(panel) => panel
                .show(ui, document, capabilities, preview)
                .map(|event| match event {
                    ExtrudeEvent::Changed(_) => ModelEvent::Changed,
                    ExtrudeEvent::Preview { stamp, request } => ModelEvent::Preview {
                        stamp,
                        request: ModelRequest::Extrude(request),
                    },
                    ExtrudeEvent::Apply(_) => ModelEvent::Apply,
                    ExtrudeEvent::Cancel => ModelEvent::Cancel,
                }),
            Self::Primitive(panel) => panel.show(ui, document, capabilities.new_body, preview),
            Self::Revolve(panel) => panel.show(ui, document, capabilities.new_body, preview),
            Self::Boolean(panel) => panel.show(ui, document, capabilities.new_body, preview),
            Self::BooleanEdit(panel) => panel.show(ui, document, capabilities.new_body, preview),
            Self::Split(panel) => panel
                .show(ui, document, capabilities.new_body, preview)
                .map(|event| match event {
                    SplitEvent::Changed => ModelEvent::Changed,
                    SplitEvent::Preview { stamp, request } => ModelEvent::Preview {
                        stamp,
                        request: ModelRequest::Split(request),
                    },
                    SplitEvent::Apply => ModelEvent::Apply,
                    SplitEvent::Cancel => ModelEvent::Cancel,
                }),
            Self::ExtrudeGroup(panel) => panel.show(ui, document, capabilities, preview),
            Self::Placement(panel) => panel.show(ui, document, capabilities.new_body, preview),
            Self::Scale(panel) => panel.show(ui, document, capabilities.new_body, preview),
            Self::EdgeTreatment(panel) => panel.show(ui, document, capabilities.new_body, preview),
            Self::Advanced(panel) => panel.show(ui, document, capabilities.new_body, preview),
            Self::Alignment(panel) => panel.show(ui, document, capabilities.new_body, preview),
            Self::Wall(panel) => panel.show(ui, document, capabilities.new_body, preview),
            Self::Architecture(panel) => panel.show(ui, document, capabilities.new_body, preview),
            Self::Cabinet(panel) => panel.show(ui, document, capabilities.new_body, preview),
            Self::Joint(panel) => panel.show(ui, document, capabilities.new_body, preview),
            Self::Machining(panel) => panel.show(ui, document, capabilities.new_body, preview),
        }
    }
    pub fn extrusion(&self) -> Option<&ExtrudePanel> {
        match self {
            Self::Extrude(panel) => Some(panel),
            Self::ExtrudeGroup(panel) => Some(&panel.extrusion),
            _ => None,
        }
    }
    pub fn extrusion_mut(&mut self) -> Option<&mut ExtrudePanel> {
        match self {
            Self::Extrude(panel) => Some(panel),
            Self::ExtrudeGroup(panel) => Some(&mut panel.extrusion),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FormAction {
    Preview,
    Apply,
    Cancel,
}
pub(super) fn actions(
    ui: &mut egui::Ui,
    valid: bool,
    changed: bool,
    stamp: FormStamp,
    preview: PreviewState<'_>,
) -> Option<FormAction> {
    use crate::inspector_actions::{self, Button, Owner};
    let clicked = inspector_actions::row(
        ui,
        Owner::Model,
        Some(Button::new(
            "Preview",
            valid,
            "Validate these exact local inputs using native geometry",
        )),
        Button::new(
            "Apply",
            valid && !changed && preview.ready(stamp),
            "Commit the exact transaction whose matching native preview succeeded",
        ),
        Button::new(
            "Cancel",
            true,
            "Discard the form and its uncommitted native preview",
        ),
    );
    if clicked.cancel {
        Some(FormAction::Cancel)
    } else if clicked.apply {
        Some(FormAction::Apply)
    } else if clicked.preview {
        Some(FormAction::Preview)
    } else {
        None
    }
}
pub(super) fn preview_status(ui: &mut egui::Ui, stamp: FormStamp, preview: PreviewState<'_>) {
    match preview {
        PreviewState::Pending { stamp: ready } if ready == stamp => {
            ui.label("Building native preview…");
        }
        PreviewState::Ready { stamp: ready } if ready == stamp => {
            ui.label("Native preview ready");
        }
        PreviewState::Failed {
            stamp: failed,
            message,
        } if failed == stamp => {
            ui.colored_label(egui::Color32::LIGHT_RED, message);
        }
        _ => {}
    }
}

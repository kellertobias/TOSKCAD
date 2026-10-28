use super::{
    ExtrudeCapabilities, ExtrudeEvent, ExtrudePanel, FormStamp, PreviewState,
    boolean::BooleanPanel, primitive::PrimitivePanel, revolve::RevolvePanel,
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
}
pub(super) struct ToolProof {
    pub input: BodyInput,
    pub consumed: bool,
}
impl ModelRequest {
    pub(super) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        match self {
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
        }
        .map_err(|error| error.to_string())
    }
    /// Final membership follows producer versions: later history can reactivate
    /// a Part whose earlier producer was consumed by a Boolean operation.
    pub(super) fn tool_proof(&self, document: &Document) -> Result<Vec<ToolProof>, String> {
        let Self::Booleans(requests) = self else {
            return Ok(vec![]);
        };
        let mut tools = std::collections::BTreeMap::new();
        for request in requests {
            for tool in &request.tools {
                let Some(EntityData::Part { features, .. }) =
                    document.get(tool.body).map(|entity| &entity.data)
                else {
                    return Err("Boolean tool body is unavailable".into());
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
                    .ok_or("Boolean tool has no current producer")?;
                let consumed = !request.keep_tools && head == tool.producer;
                let entry = tools.entry(tool.body).or_insert((
                    BodyInput {
                        body: tool.body,
                        producer: head,
                    },
                    false,
                ));
                entry.1 |= consumed;
            }
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
}
impl ModelForm {
    pub fn stamp(&self) -> FormStamp {
        match self {
            Self::Extrude(panel) => panel.stamp().expect("active extrusion form"),
            Self::Primitive(panel) => panel.stamp(),
            Self::Revolve(panel) => panel.stamp(),
            Self::Boolean(panel) => panel.stamp(),
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Extrude(_) => "Extrude",
            Self::Primitive(panel) => panel.label(),
            Self::Revolve(_) => "Revolve",
            Self::Boolean(_) => "Boolean",
        }
    }
    pub fn request(
        &self,
        document: &Document,
        capabilities: ExtrudeCapabilities,
    ) -> Result<ModelRequest, String> {
        match self {
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
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(valid, egui::Button::new("Preview"))
            .on_hover_text("Validate these exact local inputs using native geometry")
            .clicked()
        {
            action = Some(FormAction::Preview);
        }
        if ui
            .add_enabled(
                valid && !changed && preview.ready(stamp),
                egui::Button::new("Apply"),
            )
            .on_hover_text("Commit the exact transaction whose matching native preview succeeded")
            .clicked()
        {
            action = Some(FormAction::Apply);
        }
        if ui
            .button("Cancel")
            .on_hover_text("Discard the form and its uncommitted native preview")
            .clicked()
        {
            action = Some(FormAction::Cancel);
        }
    });
    action
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

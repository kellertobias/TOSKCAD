//! Reopen a Boolean without rebinding its frozen producer dependencies.
use super::{
    FormStamp, PreviewState,
    form::{self, FormAction, ModelEvent, ModelRequest},
};
use crate::{controls, egui};
use multicad_core::{
    document::{Document, EntityId},
    model::{self, BooleanRequest, Operation},
};

pub(super) struct BooleanEditPanel {
    request: BooleanRequest,
    stamp: FormStamp,
}
impl BooleanEditPanel {
    pub fn new(document: &Document, feature: EntityId) -> Result<Self, String> {
        let source = model::read_boolean(document, feature).map_err(|error| error.to_string())?;
        Ok(Self {
            request: BooleanRequest {
                feature,
                name: document
                    .get(feature)
                    .ok_or("The applied feature is missing")?
                    .name
                    .clone(),
                target: source.target,
                tools: source.tools,
                operation: source.operation,
                keep_tools: source.keep_tools,
            },
            stamp: FormStamp::new(),
        })
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<BooleanRequest, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        for input in std::iter::once(&self.request.target).chain(&self.request.tools) {
            input
                .resolve_producer(document)
                .map_err(|error| format!("The saved Boolean input needs repair: {error}"))?;
        }
        Ok(self.request.clone())
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState<'_>,
    ) -> Option<ModelEvent> {
        controls::inspector_layout(ui);
        ui.strong("Boolean operation");
        ui.small("Edit the operation and tool retention. Saved target and tool producer versions remain fixed; replacing tools is not available in this form.");
        let mut changed = false;
        controls::inspector_combo(ui, "boolean-edit-operation")
            .selected_text(label(self.request.operation))
            .show_ui(ui, |ui| {
                for operation in [Operation::Join, Operation::Cut, Operation::Intersect] {
                    changed |= ui
                        .selectable_value(&mut self.request.operation, operation, label(operation))
                        .changed();
                }
            });
        ui.strong("Saved target");
        input_label(ui, document, &self.request.target);
        ui.strong("Saved tools");
        for input in &self.request.tools {
            input_label(ui, document, input);
        }
        changed|=ui.checkbox(&mut self.request.keep_tools,"Keep tool bodies").on_hover_text("Change consumption of these saved producer versions without switching to newer body history").changed();
        if changed {
            self.stamp.changed();
        }
        form::preview_status(ui, self.stamp, preview);
        let validation = self.request(document, enabled);
        if let Err(error) = &validation {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        match form::actions(ui, validation.is_ok(), changed, self.stamp, preview) {
            Some(FormAction::Cancel) => Some(ModelEvent::Cancel),
            Some(FormAction::Preview) => validation.ok().map(|request| ModelEvent::Preview {
                stamp: self.stamp,
                request: ModelRequest::Booleans(vec![request]),
            }),
            Some(FormAction::Apply) if !changed => Some(ModelEvent::Apply),
            _ if changed => Some(ModelEvent::Changed),
            _ => None,
        }
    }
}
fn label(operation: Operation) -> &'static str {
    match operation {
        Operation::Join => "Join",
        Operation::Cut => "Subtract",
        Operation::Intersect => "Intersect",
        _ => "Unsupported operation",
    }
}
fn input_label(ui: &mut egui::Ui, document: &Document, input: &model::BodyInput) {
    let body = document
        .get(input.body)
        .map_or("Missing body", |entity| entity.name.as_str());
    let producer = document
        .get(input.producer)
        .map_or("Missing producer", |entity| entity.name.as_str());
    ui.label(format!("{body} · {producer}"))
        .on_hover_text(format!(
            "Saved body: {}\nSaved producer: {}",
            input.body, input.producer
        ));
}

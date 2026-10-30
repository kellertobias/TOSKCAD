use super::{
    FormStamp, PreviewState,
    body_choices::{self, BodyChoice},
    form::{self, FormAction, ModelEvent, ModelRequest},
    names::GeneratedNames,
};
use crate::{controls, egui, viewport::Scene};
use multicad_core::{
    document::{Document, EntityId},
    model::{BooleanRequest, MAX_BOOLEAN_TARGETS, MAX_BOOLEAN_TOOLS, Operation},
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct BooleanPanel {
    stamp: FormStamp,
    choices: Vec<BodyChoice>,
    features: BTreeMap<EntityId, (EntityId, String)>,
    targets: BTreeSet<EntityId>,
    tools: BTreeSet<EntityId>,
    operation: Operation,
    keep_tools: bool,
    filter: String,
    error: Option<String>,
}
impl BooleanPanel {
    pub fn available(document: &Document, revision: u64, scene: &Scene) -> Result<usize, String> {
        body_choices::available(document, revision, scene)
    }
    pub fn new(
        document: &Document,
        revision: u64,
        scene: &Scene,
        selected: &BTreeSet<EntityId>,
    ) -> Result<Self, String> {
        let choices = body_choices::choices(document, revision, scene)?;
        if choices.len() < 2 {
            return Err("Boolean modeling needs at least two available native bodies".into());
        }
        let mut names = GeneratedNames::new(document);
        let features = choices
            .iter()
            .map(|choice| {
                (
                    choice.input.body,
                    (EntityId::new(), names.numbered("Boolean feature", "")),
                )
            })
            .collect();
        let targets = choices
            .iter()
            .map(|choice| choice.input.body)
            .filter(|id| selected.contains(id))
            .collect();
        Ok(Self {
            stamp: FormStamp::new(),
            choices,
            features,
            targets,
            tools: BTreeSet::new(),
            operation: Operation::Cut,
            keep_tools: false,
            filter: String::new(),
            error: None,
        })
    }
    /// Preset the operation chosen from the ribbon's Boolean menu. Only
    /// implemented Join, Subtract and Intersect operations are accepted.
    pub fn with_operation(mut self, operation: Operation) -> Self {
        if matches!(
            operation,
            Operation::Join | Operation::Cut | Operation::Intersect
        ) {
            self.operation = operation;
        }
        self
    }
    pub fn operation(&self) -> Operation {
        self.operation
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    pub fn request(
        &self,
        document: &Document,
        enabled: bool,
    ) -> Result<Vec<BooleanRequest>, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if self.targets.is_empty() || self.targets.len() > MAX_BOOLEAN_TARGETS {
            return Err(format!(
                "Choose between 1 and {MAX_BOOLEAN_TARGETS} explicit target bodies"
            ));
        }
        if self.tools.is_empty() || self.tools.len() > MAX_BOOLEAN_TOOLS {
            return Err(format!(
                "Choose between 1 and {MAX_BOOLEAN_TOOLS} explicit tool bodies"
            ));
        }
        if !self.targets.is_disjoint(&self.tools) {
            return Err("A target cannot also be a tool in the same Boolean operation".into());
        }
        if !matches!(
            self.operation,
            Operation::Join | Operation::Cut | Operation::Intersect
        ) {
            return Err(
                "Choose Join, Subtract or Intersect; Split outputs are not implemented".into(),
            );
        }
        let input = |id| {
            let input = self
                .choices
                .iter()
                .find(|choice| choice.input.body == id)
                .ok_or("Body selection is unavailable; restart Boolean")?
                .input
                .clone();
            input
                .resolve_producer(document)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>(input)
        };
        let tools = self
            .tools
            .iter()
            .map(|id| input(*id))
            .collect::<Result<Vec<_>, _>>()?;
        self.targets
            .iter()
            .map(|id| {
                let (feature, name) = self
                    .features
                    .get(id)
                    .ok_or("Target identity was not allocated; restart Boolean")?;
                Ok(BooleanRequest {
                    feature: *feature,
                    name: name.clone(),
                    target: input(*id)?,
                    tools: tools.clone(),
                    operation: self.operation,
                    keep_tools: self.keep_tools,
                })
            })
            .collect()
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState<'_>,
    ) -> Option<ModelEvent> {
        controls::inspector_layout(ui);
        ui.strong("Boolean bodies");
        ui.small("Choose target bodies and separate tool bodies. Each target retains its identity and receives its own result in one undoable edit.");
        ui.label("Operation");
        let mut changed = false;
        controls::inspector_combo(ui, "boolean-operation")
            .selected_text(operation_label(self.operation))
            .show_ui(ui, |ui| {
                for operation in [Operation::Join, Operation::Cut, Operation::Intersect] {
                    changed |= ui
                        .selectable_value(
                            &mut self.operation,
                            operation,
                            operation_label(operation),
                        )
                        .changed();
                }
                ui.add_enabled(false, egui::Button::new("Split"))
                    .on_disabled_hover_text("Split is not available in this build.");
            });
        ui.add(
            egui::TextEdit::singleline(&mut self.filter)
                .desired_width(ui.available_width())
                .hint_text("Filter bodies"),
        )
        .on_hover_text("Filter the lists without changing selected targets or tools");
        ui.separator();
        ui.strong(format!("Targets · {}", self.targets.len()));
        changed |= Self::body_list(
            ui,
            "boolean-targets",
            &self.choices,
            &self.filter,
            &mut self.targets,
            &mut self.tools,
        );
        ui.strong(format!("Tools · {}", self.tools.len()));
        changed |= Self::body_list(
            ui,
            "boolean-tools",
            &self.choices,
            &self.filter,
            &mut self.tools,
            &mut self.targets,
        );
        changed|=ui.checkbox(&mut self.keep_tools,"Keep tool bodies").on_hover_text("Keep each selected tool's current final producer as an output body. Otherwise this operation consumes that exact producer version; its source and history remain in the document.").changed();
        ui.small(
            "Targets are explicit. Automatic component cut targeting is not applied by this tool.",
        );
        form::preview_status(ui, self.stamp, preview);
        let validation = self.request(document, enabled);
        if let Err(error) = &validation {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        let action = form::actions(ui, validation.is_ok(), changed, self.stamp, preview);
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        self.finish_frame(changed, action, document, enabled, preview)
    }
    fn body_list(
        ui: &mut egui::Ui,
        salt: &str,
        choices: &[BodyChoice],
        filter: &str,
        selection: &mut BTreeSet<EntityId>,
        opposite: &mut BTreeSet<EntityId>,
    ) -> bool {
        let mut changed = false;
        let filter = filter.to_lowercase();
        egui::ScrollArea::vertical()
            .id_salt(salt)
            .max_height(150.)
            .show(ui, |ui| {
                for choice in choices.iter().filter(|choice| {
                    filter.is_empty() || choice.label.to_lowercase().contains(&filter)
                }) {
                    let mut checked = selection.contains(&choice.input.body);
                    if ui
                        .push_id(choice.input.body, |ui| {
                            ui.checkbox(&mut checked, &choice.label)
                        })
                        .inner
                        .on_hover_text(format!("{}\nProduced by {}", choice.path, choice.producer))
                        .changed()
                    {
                        if checked {
                            selection.insert(choice.input.body);
                            opposite.remove(&choice.input.body);
                        } else {
                            selection.remove(&choice.input.body);
                        }
                        changed = true;
                    }
                }
            });
        changed
    }
    fn finish_frame(
        &mut self,
        changed: bool,
        action: Option<FormAction>,
        document: &Document,
        enabled: bool,
        preview: PreviewState<'_>,
    ) -> Option<ModelEvent> {
        if action == Some(FormAction::Cancel) {
            return Some(ModelEvent::Cancel);
        }
        if changed {
            self.stamp.changed();
            self.error = None;
            if action != Some(FormAction::Preview) {
                return Some(ModelEvent::Changed);
            }
        }
        match action {
            Some(FormAction::Preview) => match self.request(document, enabled) {
                Ok(requests) => {
                    self.error = None;
                    Some(ModelEvent::Preview {
                        stamp: self.stamp,
                        request: ModelRequest::Booleans(requests),
                    })
                }
                Err(error) => {
                    self.error = Some(error);
                    None
                }
            },
            Some(FormAction::Apply)
                if preview.ready(self.stamp) && self.request(document, enabled).is_ok() =>
            {
                Some(ModelEvent::Apply)
            }
            _ => None,
        }
    }
}
fn operation_label(operation: Operation) -> &'static str {
    match operation {
        Operation::Join => "Join",
        Operation::Cut => "Subtract",
        Operation::Intersect => "Intersect",
        Operation::Split => "Split",
        Operation::NewBody => "New body",
    }
}

#[cfg(test)]
mod tests;

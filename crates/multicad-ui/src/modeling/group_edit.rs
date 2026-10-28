//! Edit an explicitly recorded extrusion operation as one captured target set.
//! Saved target predecessors never come from the current final cut bodies.
use super::{
    ExtrudeCapabilities, ExtrudeEvent, ExtrudePanel, FormStamp, PreviewState, body_choices,
    form::{ModelEvent, ModelRequest},
    names::GeneratedNames,
};
use crate::{controls, egui, viewport::Scene};
use multicad_core::{
    document::{Document, EntityId},
    model::{BodyInput, ExtrudeGroupEdit, ExtrudeGroupState, ExtrudeGroupTarget, Operation},
    project_file::ContentHash,
};
use std::collections::BTreeSet;

struct TargetRow {
    body: EntityId,
    label: String,
    predecessor: String,
    input: BodyInput,
    binding: ExtrudeGroupTarget,
    checked: bool,
    saved: bool,
}
pub(super) struct GroupEditPanel {
    pub extrusion: ExtrudePanel,
    state: ExtrudeGroupState,
    edit: ExtrudeGroupEdit,
    rows: Vec<TargetRow>,
    context_error: Option<String>,
    names: GeneratedNames,
    title: String,
    legacy: bool,
}
impl GroupEditPanel {
    pub fn new(document: &Document, state: ExtrudeGroupState) -> Result<Self, String> {
        let title = document
            .get(state.group)
            .ok_or("The recorded extrusion operation is missing")?
            .name
            .clone();
        Self::build(document, state, title, false)
    }
    pub fn legacy(
        document: &Document,
        state: ExtrudeGroupState,
        title: String,
    ) -> Result<Self, String> {
        Self::build(document, state, title, true)
    }
    fn build(
        document: &Document,
        state: ExtrudeGroupState,
        title: String,
        legacy: bool,
    ) -> Result<Self, String> {
        let edit = ExtrudeGroupEdit::from_state(&state).map_err(|error| error.to_string())?;
        let index =
            multicad_core::hierarchy::Index::new(document).map_err(|error| error.to_string())?;
        let rows = state
            .booleans
            .iter()
            .map(|request| {
                let entity = document
                    .get(request.target.body)
                    .ok_or("An operation target body is missing")?;
                let owner = index.parent(entity.id).map_err(|error| error.to_string())?;
                let path = crate::model_tree::container_path(document, &index, owner);
                document
                    .get(request.target.producer)
                    .ok_or("An operation's saved predecessor is missing")?;
                Ok(TargetRow {
                    body: entity.id,
                    label: format!("{} · {path}", entity.name),
                    predecessor: input_label(document, &request.target),
                    input: request.target.clone(),
                    binding: ExtrudeGroupTarget::Retain { body: entity.id },
                    checked: true,
                    saved: true,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut extrusion = ExtrudePanel::default();
        extrusion.begin_request(state.extrusion.clone())?;
        Ok(Self {
            extrusion,
            state,
            edit,
            rows,
            context_error: Some(
                "Current native bodies must be available before adding a new target".into(),
            ),
            names: GeneratedNames::new(document),
            title,
            legacy,
        })
    }
    pub fn stamp(&self) -> FormStamp {
        self.extrusion.stamp().expect("active grouped extrusion")
    }
    pub fn set_context(
        &mut self,
        document: &Document,
        revision: u64,
        scene: &Scene,
    ) -> Result<(), String> {
        let result = (|| {
            let info = scene
                .model_evaluation()
                .ok_or("Adding targets requires a source-bound native model")?;
            if info.source_hash != ContentHash::document(document)? {
                return Err("Target choices belong to different source content".into());
            }
            let tools: BTreeSet<_> = self
                .state
                .extrusion
                .outputs
                .iter()
                .map(|output| output.body)
                .collect();
            for choice in body_choices::choices(document, revision, scene)? {
                if tools.contains(&choice.input.body) {
                    continue;
                }
                if let Some(existing) = self.rows.iter().find(|row| row.body == choice.input.body) {
                    if let ExtrudeGroupTarget::Add { target, .. } = &existing.binding
                        && target != &choice.input
                    {
                        return Err(
                            "A newly selected target changed producer; reopen this operation"
                                .into(),
                        );
                    }
                    // A saved row always retains its frozen predecessor. Its
                    // current native head may be this operation or later work.
                    continue;
                }
                self.rows.push(TargetRow {
                    body: choice.input.body,
                    label: choice.label,
                    predecessor: input_label(document, &choice.input),
                    input: choice.input.clone(),
                    binding: ExtrudeGroupTarget::Add {
                        target: choice.input,
                        feature: EntityId::new(),
                        name: self.names.numbered("Extrusion result", ""),
                    },
                    checked: false,
                    saved: false,
                });
            }
            Ok(())
        })();
        self.context_error = result.as_ref().err().cloned();
        self.extrusion.set_picking_available(result.is_ok());
        result
    }
    pub fn request(
        &self,
        document: &Document,
        capabilities: ExtrudeCapabilities,
    ) -> Result<ExtrudeGroupEdit, String> {
        let extrusion = self.extrusion.request(document, capabilities)?;
        let mut edit = self.edit.clone();
        edit.parameters = extrusion.parameters;
        edit.faces = Some(extrusion.faces);
        if edit.operation == Operation::NewBody {
            edit.targets.clear();
            edit.keep_tools = false;
            return Ok(edit);
        }
        edit.targets = self
            .rows
            .iter()
            .filter(|row| row.checked)
            .map(|row| row.binding.clone())
            .collect();
        if edit.targets.is_empty() {
            return Err("Keep at least one affected target body".into());
        }
        Ok(edit)
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        capabilities: ExtrudeCapabilities,
        preview: PreviewState<'_>,
    ) -> Option<ModelEvent> {
        controls::inspector_layout(ui);
        ui.strong("Edit extrusion operation");
        ui.label(&self.title);
        if self.legacy {
            ui.small("This edit affects only the selected extrusion. Choosing a body operation creates an editable operation with the target bodies you select.");
        }
        ui.label("Operation");
        let mut changed = false;
        controls::inspector_combo(ui, "grouped-extrusion-operation")
            .selected_text(operation_label(self.edit.operation))
            .show_ui(ui, |ui| {
                for operation in [
                    Operation::NewBody,
                    Operation::Join,
                    Operation::Cut,
                    Operation::Intersect,
                ] {
                    changed |= ui
                        .selectable_value(
                            &mut self.edit.operation,
                            operation,
                            operation_label(operation),
                        )
                        .changed();
                }
            });
        if self.edit.operation != Operation::NewBody {
            ui.small("The checked bodies are an explicit captured target set. Changing dimensions does not rediscover targets. Saved predecessor versions stay fixed.");
            changed|=ui.checkbox(&mut self.edit.keep_tools,"Keep extrusion tool bodies").on_hover_text("Change retention of the group's saved extrusion producer versions without replacing their identities").changed();
            ui.strong(format!(
                "Affected bodies · {}",
                self.rows.iter().filter(|row| row.checked).count()
            ));
            for row in &mut self.rows {
                let hint = if row.saved {
                    "Saved predecessor; changing the current body history will not silently replace this input"
                } else {
                    "A newly added target explicitly pins this current native producer"
                };
                changed |= ui
                    .push_id(row.body, |ui| ui.checkbox(&mut row.checked, &row.label))
                    .inner
                    .on_hover_text(format!(
                        "{hint}\nInput: {}\nProducer: {}",
                        row.predecessor, row.input.producer
                    ))
                    .changed();
                if row.checked {
                    ui.small(format!(
                        "{} input: {}",
                        if row.saved { "Saved" } else { "New" },
                        row.predecessor
                    ));
                }
            }
            if let Some(error) = &self.context_error {
                ui.small(error);
            }
        } else {
            ui.small("Keep the existing extrusion body identities. Any owned body-operation results are removed only after a successful Preview and Apply; their targets return to the saved upstream results.");
        }
        if !self.legacy {
            controls::inspector_collapsing(
                ui,
                "group-captured-context",
                "Original capture context",
                |ui| {
                    if let Some(scope) = &self.state.definition.captured_scope {
                        ui.small("Historical context only; it is not an active automatic rule.");
                        let component = scope
                            .component
                            .map(|id| {
                                document
                                    .get(id)
                                    .map(|entity| entity.name.clone())
                                    .unwrap_or_else(|| format!("Unavailable component {id}"))
                            })
                            .unwrap_or_else(|| "Document root".into());
                        ui.label(format!("Component: {component}"));
                        for (label, ids) in
                            [("Included", &scope.include), ("Excluded", &scope.exclude)]
                        {
                            for id in ids {
                                ui.label(format!(
                                    "{label}: {}",
                                    document
                                        .get(*id)
                                        .map(|entity| entity.name.clone())
                                        .unwrap_or_else(|| id.to_string())
                                ));
                            }
                        }
                    } else {
                        ui.small("Body-operation targets are selected explicitly.");
                    }
                },
            );
        }
        if changed {
            self.extrusion.changed();
        }
        if let Err(error) = self.request(document, capabilities) {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        ui.separator();
        let event = self.extrusion.show(ui, document, capabilities, preview);
        match event {
            Some(ExtrudeEvent::Cancel) => Some(ModelEvent::Cancel),
            Some(ExtrudeEvent::Preview { stamp, .. }) => match self.request(document, capabilities)
            {
                Ok(request) => Some(ModelEvent::Preview {
                    stamp,
                    request: ModelRequest::ExtrudeGroupEdit(Box::new(request)),
                }),
                Err(_) => Some(ModelEvent::Changed),
            },
            Some(ExtrudeEvent::Apply(_))
                if !changed && self.request(document, capabilities).is_ok() =>
            {
                Some(ModelEvent::Apply)
            }
            Some(ExtrudeEvent::Changed(_)) => Some(ModelEvent::Changed),
            _ if changed => Some(ModelEvent::Changed),
            _ => None,
        }
    }
}
fn operation_label(operation: Operation) -> &'static str {
    match operation {
        Operation::NewBody => "New body",
        Operation::Join => "Join / merge",
        Operation::Cut => "Cut / remove material",
        Operation::Intersect => "Intersect",
        _ => "Unsupported operation",
    }
}
fn input_label(document: &Document, input: &BodyInput) -> String {
    let name = document
        .get(input.producer)
        .map_or("Unavailable producer", |entity| entity.name.as_str());
    let position = document
        .get(input.body)
        .and_then(|entity| match &entity.data {
            multicad_core::document::EntityData::Part { features, .. } => {
                features.iter().position(|id| *id == input.producer)
            }
            _ => None,
        });
    position
        .map(|index| format!("{name} · history step {}", index + 1))
        .unwrap_or_else(|| name.into())
}
#[cfg(test)]
mod tests;

//! Source-neutral controls for a bounded inside/outside body split.
use super::{
    FormStamp, PreviewState,
    body_choices::{self, BodyChoice},
    form::{self, FormAction},
    names::GeneratedNames,
};
use crate::{controls, egui, viewport::Scene};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{BodyInput, SplitDefinition, SplitOutput, SplitRequest, SplitSide},
    project_file::ContentHash,
};
use std::collections::BTreeSet;

pub(super) enum SplitEvent {
    Changed,
    Preview {
        stamp: FormStamp,
        request: Box<SplitRequest>,
    },
    Apply,
    Cancel,
}
pub(super) struct SplitPanel {
    document: EntityId,
    stamp: FormStamp,
    choices: Vec<BodyChoice>,
    target: Option<EntityId>,
    tool: Option<EntityId>,
    outputs: [SplitOutput; 2],
    keep_tools: bool,
    error: Option<String>,
}
impl SplitPanel {
    pub fn available(document: &Document, revision: u64, scene: &Scene) -> Result<usize, String> {
        if scene.model_evaluation().is_none() {
            return Err("Split requires current native model evaluation metadata".into());
        }
        body_choices::available(document, revision, scene)
    }
    pub fn new(
        document: &Document,
        revision: u64,
        scene: &Scene,
        selected: &BTreeSet<EntityId>,
    ) -> Result<Self, String> {
        if Self::available(document, revision, scene)? < 2 {
            return Err("Split needs one current target body and one separate tool body".into());
        }
        let info = scene.model_evaluation().expect("checked metadata");
        if info.source_hash != ContentHash::document(document)? {
            return Err(
                "The native bodies belong to different source content; rebuild before splitting"
                    .into(),
            );
        }
        let choices = body_choices::choices(document, revision, scene)?;
        let selected: Vec<_> = choices
            .iter()
            .filter(|c| selected.contains(&c.input.body))
            .map(|c| c.input.body)
            .collect();
        let target = if selected.len() == 1 {
            Some(selected[0])
        } else {
            None
        };
        let mut names = GeneratedNames::new(document);
        let outputs = [SplitSide::InsideTool, SplitSide::OutsideTool].map(|side| SplitOutput {
            feature: EntityId::new(),
            body: EntityId::new(),
            name: names.numbered(
                match side {
                    SplitSide::InsideTool => "Split inside",
                    SplitSide::OutsideTool => "Split outside",
                },
                "",
            ),
            side,
        });
        Ok(Self {
            document: document.id(),
            stamp: FormStamp::new(),
            choices,
            target,
            tool: None,
            outputs,
            keep_tools: false,
            error: None,
        })
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    fn input(&self, document: &Document, id: EntityId) -> Result<BodyInput, String> {
        let input = self
            .choices
            .iter()
            .find(|c| c.input.body == id)
            .ok_or("Body selection is unavailable; restart Split")?
            .input
            .clone();
        input
            .resolve_producer(document)
            .map_err(|e| e.to_string())?;
        let EntityData::Part { features, .. } = &document.get(id).expect("resolved body").data
        else {
            unreachable!()
        };
        let mut head = None;
        for feature in features {
            let Some(EntityData::Feature { suppressed, .. }) =
                document.get(*feature).map(|e| &e.data)
            else {
                return Err("A body history needs repair before splitting".into());
            };
            if !suppressed {
                head = Some(*feature);
            }
        }
        if head != Some(input.producer) {
            return Err(
                "A selected body changed; restart Split with the current native bodies".into(),
            );
        }
        Ok(input)
    }
    pub fn request(&self, document: &Document, enabled: bool) -> Result<SplitRequest, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if document.id() != self.document {
            return Err("Split belongs to another document; restart the tool".into());
        }
        let target = self.target.ok_or("Choose one target body")?;
        let tool = self.tool.ok_or("Choose one separate tool body")?;
        if target == tool {
            return Err("The target and tool must be different bodies".into());
        }
        let target = self.input(document, target)?;
        let tool = self.input(document, tool)?;
        for output in &self.outputs {
            if output.name.trim().is_empty() || output.name.len() > 1000 {
                return Err("Give both output parts a name of 1–1000 bytes".into());
            }
            if document.get(output.body).is_some() || document.get(output.feature).is_some() {
                return Err("Split output identities are already in use; restart the tool".into());
            }
            SplitDefinition {
                output: multicad_core::model::BodyOutput { body: output.body },
                target: target.clone(),
                tool: tool.clone(),
                side: output.side,
                keep_tools: self.keep_tools,
            }
            .validate()
            .map_err(|e| e.to_string())?;
        }
        Ok(SplitRequest {
            target,
            tool,
            outputs: self.outputs.clone(),
            keep_tools: self.keep_tools,
        })
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState<'_>,
    ) -> Option<SplitEvent> {
        controls::inspector_layout(ui);
        ui.strong("Split body");
        ui.small("Create an inside part and an outside part from one target and one tool. Each side must be a single connected piece.");
        ui.label("Target body");
        let mut changed = Self::body_picker(ui, "split-target", &self.choices, &mut self.target);
        ui.label("Tool body");
        changed |= Self::body_picker(ui, "split-tool", &self.choices, &mut self.tool);
        ui.separator();
        ui.strong("New parts");
        for output in &mut self.outputs {
            ui.push_id(output.feature,|ui|{
                ui.label(match output.side{SplitSide::InsideTool=>"Inside tool",SplitSide::OutsideTool=>"Outside tool"}).on_hover_text(format!("Part: {}\nProducer: {}\nThese identities stay fixed while you edit this split.",output.body,output.feature));
                changed|=ui.add(egui::TextEdit::singleline(&mut output.name).desired_width(ui.available_width()).hint_text("Part name")).on_hover_text("Name this new part without changing its persistent identity").changed();
            });
        }
        changed|=ui.checkbox(&mut self.keep_tools,"Keep tool body").on_hover_text("Retain the tool's current producer as a final body. The original target becomes the two new parts; its source stays in the document.").changed();
        ui.small("A tool that creates several separate pieces on either side needs a different cut. Preview checks both sides before Apply is available.");
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
    fn body_picker(
        ui: &mut egui::Ui,
        id: &str,
        choices: &[BodyChoice],
        selected: &mut Option<EntityId>,
    ) -> bool {
        let label = selected
            .and_then(|id| choices.iter().find(|c| c.input.body == id))
            .map_or("Choose body", |c| c.label.as_str());
        let mut changed = false;
        controls::inspector_combo(ui, id)
            .selected_text(label)
            .height(220.)
            .show_ui(ui, |ui| {
                changed |= ui.selectable_value(selected, None, "Choose body").changed();
                for choice in choices {
                    changed |= ui
                        .selectable_value(selected, Some(choice.input.body), &choice.label)
                        .on_hover_text(format!("{}\nProduced by {}", choice.path, choice.producer))
                        .changed();
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
    ) -> Option<SplitEvent> {
        if action == Some(FormAction::Cancel) {
            return Some(SplitEvent::Cancel);
        }
        if changed {
            self.stamp.changed();
            self.error = None;
            if action != Some(FormAction::Preview) {
                return Some(SplitEvent::Changed);
            }
        }
        match action {
            Some(FormAction::Preview) => match self.request(document, enabled) {
                Ok(request) => {
                    self.error = None;
                    Some(SplitEvent::Preview {
                        stamp: self.stamp,
                        request: Box::new(request),
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
                Some(SplitEvent::Apply)
            }
            _ => None,
        }
    }
}
#[cfg(test)]
mod tests;

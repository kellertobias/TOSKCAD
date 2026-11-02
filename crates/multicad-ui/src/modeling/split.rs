//! Explicit source-neutral Split fields; native proof precedes every commit.
use super::{
    FormStamp, PreviewState,
    body_choices::{self, BodyChoice},
    dimension::DimensionField,
    form::{self, FormAction},
    names::GeneratedNames,
};
use crate::{controls, egui, viewport::Scene};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{
        self, BodyInput, SplitOperationRequest, SplitOutputV2, SplitPlane, SplitSideV2,
        SplitTarget, SplitTool,
    },
    project_file::ContentHash,
    sketch::Datum,
};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitInputField {
    Targets,
    Tool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolKind {
    Model,
    Plane,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlaneChoice {
    Datum(Datum),
    Existing(EntityId),
}
pub(super) enum SplitEvent {
    Changed,
    Preview {
        stamp: FormStamp,
        request: Box<SplitOperationRequest>,
    },
    Apply,
    Cancel,
}
pub(super) struct SplitPanel {
    document: EntityId,
    stamp: FormStamp,
    choices: Vec<BodyChoice>,
    planes: Vec<(EntityId, String)>,
    targets: BTreeSet<EntityId>,
    tool: Option<EntityId>,
    tool_kind: ToolKind,
    plane: Option<PlaneChoice>,
    plane_offset: DimensionField,
    field: SplitInputField,
    selection_generation: u64,
    outputs: BTreeMap<(EntityId, SplitSideV2), SplitOutputV2>,
    names: GeneratedNames,
    keep_tools: bool,
    execute_requested: bool,
    auto_apply: bool,
    error: Option<String>,
}
impl SplitPanel {
    pub fn available(document: &Document, revision: u64, scene: &Scene) -> Result<usize, String> {
        if scene.model_evaluation().is_none() {
            return Err("Split requires current native evaluation metadata".into());
        }
        body_choices::available(document, revision, scene)
    }
    pub fn new(
        document: &Document,
        revision: u64,
        scene: &Scene,
        selected: &BTreeSet<EntityId>,
    ) -> Result<Self, String> {
        Self::available(document, revision, scene)?;
        if scene
            .model_evaluation()
            .expect("checked metadata")
            .source_hash
            != ContentHash::document(document)?
        {
            return Err(
                "The native bodies belong to different source content; rebuild before splitting"
                    .into(),
            );
        }
        let mut panel = Self::from_source(document, selected)?;
        panel.choices = body_choices::choices(document, revision, scene)?;
        panel
            .targets
            .retain(|id| panel.choices.iter().any(|choice| choice.input.body == *id));
        Ok(panel)
    }
    pub fn from_source(document: &Document, selected: &BTreeSet<EntityId>) -> Result<Self, String> {
        let choices = body_choices::source_choices(document)?;
        let targets = selected
            .iter()
            .copied()
            .filter(|id| choices.iter().any(|choice| choice.input.body == *id))
            .collect();
        let mut panel = Self {
            document: document.id(),
            stamp: FormStamp::new(),
            choices,
            planes: document
                .entities()
                .filter(|entity| matches!(entity.data, EntityData::Plane { .. }))
                .map(|entity| (entity.id, entity.name.clone()))
                .collect(),
            targets,
            tool: None,
            tool_kind: ToolKind::Model,
            plane: None,
            plane_offset: DimensionField::from_dimension(&model::length(0.), "mm")?,
            field: SplitInputField::Targets,
            selection_generation: 0,
            outputs: BTreeMap::new(),
            names: GeneratedNames::new(document),
            keep_tools: false,
            execute_requested: false,
            auto_apply: false,
            error: None,
        };
        panel.ensure_outputs();
        Ok(panel)
    }
    pub fn stamp(&self) -> FormStamp {
        self.stamp
    }
    fn sides(&self) -> [SplitSideV2; 2] {
        if self.tool_kind == ToolKind::Model {
            [SplitSideV2::InsideTool, SplitSideV2::OutsideTool]
        } else {
            [SplitSideV2::PositivePlane, SplitSideV2::NegativePlane]
        }
    }
    fn ensure_outputs(&mut self) {
        for target in &self.targets {
            for side in self.sides() {
                self.outputs
                    .entry((*target, side))
                    .or_insert_with(|| SplitOutputV2 {
                        feature: EntityId::new(),
                        body: EntityId::new(),
                        name: self.names.numbered(
                            match side {
                                SplitSideV2::InsideTool => "Split inside",
                                SplitSideV2::OutsideTool => "Split outside",
                                SplitSideV2::PositivePlane => "Split positive",
                                SplitSideV2::NegativePlane => "Split negative",
                            },
                            "",
                        ),
                        side,
                    });
            }
        }
    }
    fn focus(&mut self, field: SplitInputField) {
        self.auto_apply = false;
        self.field = field;
        self.selection_generation = self.selection_generation.wrapping_add(1);
    }
    pub fn selection(&self) -> (u64, SplitInputField, BTreeSet<EntityId>) {
        let ids = match self.field {
            SplitInputField::Targets => self.targets.clone(),
            SplitInputField::Tool => match self.tool_kind {
                ToolKind::Model => self.tool.into_iter().collect(),
                ToolKind::Plane => match self.plane {
                    Some(PlaneChoice::Existing(id)) => BTreeSet::from([id]),
                    _ => BTreeSet::new(),
                },
            },
        };
        (self.selection_generation, self.field, ids)
    }
    pub fn select(
        &mut self,
        document: &Document,
        selected: &BTreeSet<EntityId>,
    ) -> Result<bool, String> {
        let before = (self.targets.clone(), self.tool, self.tool_kind, self.plane);
        match self.field {
            SplitInputField::Targets => {
                if selected
                    .iter()
                    .any(|id| !self.choices.iter().any(|choice| choice.input.body == *id))
                {
                    return Err("Targets require body definitions. Choose the Splitting tool field to select a plane.".into());
                }
                self.targets = selected.clone();
                if self.tool.is_some_and(|id| self.targets.contains(&id)) {
                    self.tool = None;
                }
            }
            SplitInputField::Tool => {
                if selected.len() > 1 {
                    return Err("Choose one splitting model or plane; the target field supports multiple bodies.".into());
                }
                if let Some(id) = selected.first().copied() {
                    if self.choices.iter().any(|choice| choice.input.body == id) {
                        self.tool_kind = ToolKind::Model;
                        self.tool = Some(id);
                        self.targets.remove(&id);
                    } else if matches!(
                        document.get(id).map(|entity| &entity.data),
                        Some(EntityData::Plane { .. })
                    ) {
                        self.tool_kind = ToolKind::Plane;
                        self.plane = Some(PlaneChoice::Existing(id));
                        self.keep_tools = false;
                    } else {
                        return Err("Choose a body model or canonical construction plane as the splitting tool.".into());
                    }
                } else if self.tool_kind == ToolKind::Model {
                    self.tool = None;
                }
            }
        }
        let changed = before != (self.targets.clone(), self.tool, self.tool_kind, self.plane);
        if changed {
            self.ensure_outputs();
            self.stamp.changed();
            self.selection_generation = self.selection_generation.wrapping_add(1);
            self.auto_apply = false;
            self.error = None;
        }
        Ok(changed)
    }
    pub fn confirm(&mut self, document: &Document) -> Result<(), String> {
        if self.field == SplitInputField::Targets {
            if self.targets.is_empty() {
                return Err("Select at least one target body, then press Enter to choose the splitting tool.".into());
            }
            self.focus(SplitInputField::Tool);
            return Ok(());
        }
        self.request(document, true)?;
        self.execute_requested = true;
        Ok(())
    }
    fn input(&self, document: &Document, id: EntityId) -> Result<BodyInput, String> {
        let input = self
            .choices
            .iter()
            .find(|choice| choice.input.body == id)
            .ok_or("Body selection is unavailable; restart Split")?
            .input
            .clone();
        input
            .resolve_producer(document)
            .map_err(|error| error.to_string())?;
        let EntityData::Part { features, .. } = &document.get(id).expect("resolved body").data
        else {
            unreachable!()
        };
        let head = features.iter().rev().copied().find(|feature| {
            matches!(
                document.get(*feature).map(|entity| &entity.data),
                Some(EntityData::Feature {
                    suppressed: false,
                    ..
                })
            )
        });
        if head != Some(input.producer) {
            return Err("A selected body changed; restart Split with current sources".into());
        }
        Ok(input)
    }
    fn plane_source(&self, document: &Document) -> Result<SplitPlane, String> {
        let offset = self.plane_offset.resolve(document)?;
        match self
            .plane
            .ok_or("Choose an explicit plane; no default cutting plane is assumed")?
        {
            PlaneChoice::Existing(plane) => Ok(SplitPlane::Existing { plane, offset }),
            PlaneChoice::Datum(datum) => {
                let (axis, u, v) = match datum {
                    Datum::Xy => (2, [1., 0., 0.], [0., 1., 0.]),
                    Datum::Xz => (1, [0., 0., 1.], [1., 0., 0.]),
                    Datum::Yz => (0, [0., 1., 0.], [0., 0., 1.]),
                };
                let mut origin = std::array::from_fn(|_| model::length(0.));
                origin[axis] = offset;
                Ok(SplitPlane::Authored {
                    frame: Box::new(model::PrimitiveFrame { origin, u, v }),
                })
            }
        }
    }
    pub fn request(
        &self,
        document: &Document,
        enabled: bool,
    ) -> Result<SplitOperationRequest, String> {
        if !enabled {
            return Err("The native modeling preview provider is unavailable".into());
        }
        if document.id() != self.document {
            return Err("Split belongs to another document; restart the tool".into());
        }
        if self.targets.is_empty() || self.targets.len() > model::MAX_SPLIT_TARGETS {
            return Err("Choose between 1 and 128 target bodies".into());
        }
        let tool = match self.tool_kind {
            ToolKind::Model => {
                let tool = self
                    .tool
                    .ok_or("Choose one splitting model or select Plane")?;
                if self.targets.contains(&tool) {
                    return Err("A target cannot also be the splitting tool".into());
                }
                SplitTool::Body {
                    input: self.input(document, tool)?,
                }
            }
            ToolKind::Plane => SplitTool::Plane {
                source: self.plane_source(document)?,
            },
        };
        let mut targets = vec![];
        for id in &self.targets {
            let outputs = self
                .sides()
                .map(|side| self.outputs.get(&(*id, side)).cloned());
            let [Some(first), Some(second)] = outputs else {
                return Err("Split output identities need refresh".into());
            };
            for output in [&first, &second] {
                if output.name.trim().is_empty()
                    || output.name.len() > 1000
                    || document.get(output.body).is_some()
                    || document.get(output.feature).is_some()
                {
                    return Err(
                        "Name both new parts and keep their unused persistent identities".into(),
                    );
                }
            }
            targets.push(SplitTarget {
                input: self.input(document, *id)?,
                outputs: [first, second],
            });
        }
        Ok(SplitOperationRequest {
            targets,
            tool,
            keep_tools: self.tool_kind == ToolKind::Model && self.keep_tools,
        })
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        enabled: bool,
        preview: PreviewState<'_>,
    ) -> Option<SplitEvent> {
        let enter = ui.is_enabled() && !egui::Popup::is_any_open(ui.ctx()) && ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::Enter, pressed: true, repeat: false, modifiers, .. } if *modifiers == egui::Modifiers::NONE)));
        controls::inspector_layout(ui);
        ui.strong("Split");
        ui.small("Select target bodies and a separate model or plane. Enter advances from Targets to Splitting tool, then executes after native validation.");
        let mut changed = false;
        let target_label = format!(
            "Targets · {} selected{}",
            self.targets.len(),
            if self.field == SplitInputField::Targets {
                " · active"
            } else {
                ""
            }
        );
        if ui
            .add(
                egui::Button::new(target_label)
                    .selected(self.field == SplitInputField::Targets)
                    .min_size(egui::vec2(ui.available_width(), 32.)),
            )
            .clicked()
        {
            self.focus(SplitInputField::Targets);
        }
        let tool_count = usize::from(match self.tool_kind {
            ToolKind::Model => self.tool.is_some(),
            ToolKind::Plane => self.plane.is_some(),
        });
        let tool_label = format!(
            "Splitting tool · {tool_count} selected{}",
            if self.field == SplitInputField::Tool {
                " · active"
            } else {
                ""
            }
        );
        if ui
            .add(
                egui::Button::new(tool_label)
                    .selected(self.field == SplitInputField::Tool)
                    .min_size(egui::vec2(ui.available_width(), 32.)),
            )
            .clicked()
        {
            self.focus(SplitInputField::Tool);
        }
        ui.horizontal(|ui| {
            for (kind, label) in [(ToolKind::Model, "Model"), (ToolKind::Plane, "Plane")] {
                if ui
                    .selectable_value(&mut self.tool_kind, kind, label)
                    .changed()
                {
                    self.focus(SplitInputField::Tool);
                    self.keep_tools = false;
                    changed = true;
                }
            }
        });
        if self.tool_kind == ToolKind::Model {
            let label = self
                .tool
                .and_then(|id| self.choices.iter().find(|choice| choice.input.body == id))
                .map_or("Choose splitting model", |choice| choice.label.as_str());
            let mut picked = self.tool;
            controls::inspector_combo(ui, "split-model")
                .selected_text(label)
                .height(260.)
                .show_ui(ui, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    let height = ui.spacing().interact_size.y;
                    egui::ScrollArea::vertical().max_height(220.).show_rows(
                        ui,
                        height,
                        self.choices.len(),
                        |ui, rows| {
                            for row in rows {
                                let choice = &self.choices[row];
                                if ui
                                    .selectable_label(
                                        picked == Some(choice.input.body),
                                        &choice.label,
                                    )
                                    .on_hover_text(format!(
                                        "{}\nProduced by {}",
                                        choice.path, choice.producer
                                    ))
                                    .clicked()
                                {
                                    picked = Some(choice.input.body);
                                }
                            }
                        },
                    );
                });
            if picked != self.tool {
                self.tool = picked;
                if let Some(id) = picked {
                    self.targets.remove(&id);
                }
                self.focus(SplitInputField::Tool);
                changed = true;
            }
        } else {
            let label = match self.plane {
                None => "Choose a plane".into(),
                Some(PlaneChoice::Datum(datum)) => format!("{} plane", datum.label()),
                Some(PlaneChoice::Existing(id)) => document
                    .get(id)
                    .map_or("Missing plane".into(), |entity| entity.name.clone()),
            };
            let mut selected = self.plane;
            controls::inspector_combo(ui, "split-plane")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (datum, label) in [
                        (Datum::Xy, "XY · +Z normal"),
                        (Datum::Xz, "XZ · +Y normal"),
                        (Datum::Yz, "YZ · +X normal"),
                    ] {
                        ui.selectable_value(&mut selected, Some(PlaneChoice::Datum(datum)), label);
                    }
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    let height = ui.spacing().interact_size.y;
                    egui::ScrollArea::vertical().max_height(160.).show_rows(
                        ui,
                        height,
                        self.planes.len(),
                        |ui, rows| {
                            for row in rows {
                                let (id, name) = &self.planes[row];
                                if ui
                                    .selectable_label(
                                        selected == Some(PlaneChoice::Existing(*id)),
                                        name,
                                    )
                                    .on_hover_text(name)
                                    .clicked()
                                {
                                    selected = Some(PlaneChoice::Existing(*id));
                                }
                            }
                        },
                    );
                });
            if selected != self.plane {
                self.plane = selected;
                self.focus(SplitInputField::Tool);
                changed = true;
            }
            changed |= self.plane_offset.show(
                ui,
                "Plane offset",
                "Signed distance along the plane normal; values or =$parameter remain associative",
            );
        }
        ui.separator();
        ui.label("Target bodies");
        egui::ScrollArea::vertical()
            .id_salt("split-targets")
            .max_height(140.)
            .show_rows(
                ui,
                ui.spacing().interact_size.y,
                self.choices.len(),
                |ui, rows| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    for row in rows {
                        let choice = &self.choices[row];
                        let mut picked = self.targets.contains(&choice.input.body);
                        if ui
                            .checkbox(&mut picked, &choice.label)
                            .on_hover_text(&choice.label)
                            .changed()
                        {
                            if picked {
                                self.targets.insert(choice.input.body);
                                if self.tool == Some(choice.input.body) {
                                    self.tool = None;
                                }
                            } else {
                                self.targets.remove(&choice.input.body);
                            }
                            self.field = SplitInputField::Targets;
                            changed = true;
                        }
                    }
                },
            );
        if self.tool_kind == ToolKind::Model {
            changed |= ui
                .checkbox(&mut self.keep_tools, "Keep splitting model")
                .changed();
        }
        self.ensure_outputs();
        egui::CollapsingHeader::new("Output part names").show(ui, |ui| {
            for target in &self.targets {
                if let Some(entity) = document.get(*target) {
                    ui.strong(&entity.name);
                }
                for side in self.sides() {
                    if let Some(output) = self.outputs.get_mut(&(*target, side)) {
                        ui.push_id(output.feature, |ui| {
                            ui.label(match side {
                                SplitSideV2::InsideTool => "Inside splitting model",
                                SplitSideV2::OutsideTool => "Outside splitting model",
                                SplitSideV2::PositivePlane => "Positive plane side",
                                SplitSideV2::NegativePlane => "Negative plane side",
                            });
                            changed |= ui
                                .add(
                                    egui::TextEdit::singleline(&mut output.name)
                                        .desired_width(ui.available_width()),
                                )
                                .changed();
                        });
                    }
                }
            }
        });
        ui.small("Only closed native solids are supported as models. Both output sides must be nonempty and connected; a failed target cancels the whole operation.");
        if changed {
            self.selection_generation = self.selection_generation.wrapping_add(1);
            self.auto_apply = false;
        }
        form::preview_status(ui, self.stamp, preview);
        if preview.ready(self.stamp) {
            ui.small(match self.tool_kind {
                ToolKind::Model => "Preview: Targets become inside and outside parts at the Splitting tool's boundary.",
                ToolKind::Plane => "Preview: Targets become positive and negative parts relative to the Splitting tool's normal.",
            });
        }
        let validation = self.request(document, enabled);
        if let Err(error) = &validation {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        let mut action = form::actions(ui, validation.is_ok(), changed, self.stamp, preview);
        if action == Some(FormAction::Cancel) {
            self.auto_apply = false;
            self.execute_requested = false;
            return Some(SplitEvent::Cancel);
        }
        if action.is_none() && enter && !ui.ctx().text_edit_focused() {
            ui.input_mut(|input| {
                input.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            });
            if let Err(error) = self.confirm(document) {
                self.error = Some(error);
            }
        }
        if matches!(preview, PreviewState::Failed { .. }) {
            self.auto_apply = false;
        }
        if self.execute_requested {
            self.execute_requested = false;
            self.auto_apply = true;
            action = Some(if preview.ready(self.stamp) && !changed {
                FormAction::Apply
            } else {
                FormAction::Preview
            });
        }
        if self.auto_apply
            && preview.ready(self.stamp)
            && !changed
            && !ui.input(|input| input.pointer.any_down())
            && !ui.ctx().text_edit_focused()
            && !egui::Popup::is_any_open(ui.ctx())
        {
            action = Some(FormAction::Apply);
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        self.finish_frame(changed, action, document, enabled, preview)
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
            self.auto_apply = false;
            self.execute_requested = false;
            return Some(SplitEvent::Cancel);
        }
        if changed {
            if action != Some(FormAction::Preview) {
                self.auto_apply = false;
            }
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
                    self.auto_apply = false;
                    self.error = Some(error);
                    None
                }
            },
            Some(FormAction::Apply)
                if preview.ready(self.stamp) && self.request(document, enabled).is_ok() =>
            {
                self.auto_apply = false;
                Some(SplitEvent::Apply)
            }
            _ => None,
        }
    }
}
#[cfg(test)]
mod tests;

//! Source-bound first phase of an extrusion cut. The second phase prepares the
//! final composite transaction on the original Session, never on these tools.
use crate::viewport::{Scene, SceneStamp};
use crate::{controls, egui};
use multicad_core::{
    commands::{Actor, Prepared, Session},
    document::{Document, EntityData, EntityId},
    model::{
        self, BodyInput, CutTargetProbe, CutTargetQuery, CutTargetSelection, EvaluationOptions,
        ExtrudeBooleanRequest, ExtrudeRequest, Operation,
    },
    project_file::ContentHash,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct OperationControls {
    pub operation: Operation,
    pub keep_tools: bool,
    pub component: Option<EntityId>,
    pub include: BTreeSet<EntityId>,
    pub exclude: BTreeSet<EntityId>,
    pub targets: BTreeSet<EntityId>,
    choices: Vec<(EntityId, String, bool)>,
    selection: Option<CutTargetSelection>,
    fresh: bool,
    native: BTreeMap<EntityId, BodyInput>,
    context_error: Option<String>,
}
impl OperationControls {
    pub fn new(document: &Document) -> Self {
        let index = multicad_core::hierarchy::Index::new(document).ok();
        let mut choices: Vec<_> = document
            .entities()
            .filter_map(|entity| match entity.data {
                EntityData::Part { .. } => {
                    let path = index
                        .as_ref()
                        .map(|index| {
                            crate::model_tree::container_path(
                                document,
                                index,
                                index.parent(entity.id).ok().flatten(),
                            )
                        })
                        .unwrap_or_else(|| "Unknown owner".into());
                    let hidden = index.as_ref().is_some_and(|index| {
                        index.effective_visibility(entity.id).ok() == Some(false)
                    });
                    Some((
                        entity.id,
                        format!(
                            "{} · {path}{}",
                            entity.name,
                            if hidden { " · hidden" } else { "" }
                        ),
                        true,
                    ))
                }
                EntityData::Assembly { .. } => Some((
                    entity.id,
                    format!(
                        "{} (component)",
                        index
                            .as_ref()
                            .map(|index| crate::model_tree::container_path(
                                document,
                                index,
                                Some(entity.id)
                            ))
                            .unwrap_or_else(|| format!("{} · {}", entity.name, entity.id))
                    ),
                    false,
                )),
                _ => None,
            })
            .collect();
        choices.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        Self {
            operation: Operation::NewBody,
            keep_tools: false,
            component: document.hierarchy().active_component,
            include: BTreeSet::new(),
            exclude: BTreeSet::new(),
            targets: BTreeSet::new(),
            choices,
            selection: None,
            fresh: false,
            native: BTreeMap::new(),
            context_error: Some(
                "Wait for a source-current native model before choosing explicit targets".into(),
            ),
        }
    }
    pub fn set_context(
        &mut self,
        document: &Document,
        revision: u64,
        scene: &Scene,
    ) -> Result<(), String> {
        self.native.clear();
        let result: Result<(), String> = (|| {
            let info = scene
                .model_evaluation()
                .ok_or("Operation targets require a source-bound native evaluation")?;
            if info.source_hash != ContentHash::document(document)? {
                return Err("Operation targets belong to different source content".into());
            }
            for choice in super::body_choices::choices(document, revision, scene)? {
                if info.consumed_bodies.contains(&choice.input.body) {
                    return Err("A consumed body was returned as an available target".into());
                }
                if let Some(entry) = self
                    .choices
                    .iter_mut()
                    .find(|entry| entry.0 == choice.input.body)
                {
                    entry.1 = choice.label;
                }
                self.native.insert(choice.input.body, choice.input);
            }
            Ok(())
        })();
        self.context_error = result.as_ref().err().cloned();
        result
    }
    pub fn invalidate(&mut self) {
        self.fresh = false;
    }
    pub fn receive(&mut self, selection: CutTargetSelection) {
        self.selection = Some(selection);
        self.fresh = true;
    }
    pub fn explicit_targets(&self, document: &Document) -> Result<Vec<BodyInput>, String> {
        if let Some(error) = &self.context_error {
            return Err(error.clone());
        }
        self.targets
            .iter()
            .map(|body| {
                let native = self
                    .native
                    .get(body)
                    .ok_or("The selected target is not a current unconsumed native output")?;
                let Some(EntityData::Part { features, .. }) =
                    document.get(*body).map(|entity| &entity.data)
                else {
                    return Err("An extrusion target Part is missing".into());
                };
                let producer = features
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
                    .ok_or("An extrusion target has no current producer")?;
                if native.producer != producer {
                    return Err("The selected target producer changed; reopen the extrusion".into());
                }
                Ok(native.clone())
            })
            .collect()
    }
    pub fn show(&mut self, ui: &mut egui::Ui) -> bool {
        controls::inspector_layout(ui);
        let mut changed = false;
        ui.label("Operation");
        controls::inspector_combo(ui, "extrude-body-operation")
            .selected_text(operation_label(self.operation))
            .show_ui(ui, |ui| {
                for operation in [
                    Operation::NewBody,
                    Operation::Join,
                    Operation::Cut,
                    Operation::Intersect,
                ] {
                    changed |= ui
                        .selectable_value(
                            &mut self.operation,
                            operation,
                            operation_label(operation),
                        )
                        .changed();
                }
            });
        if self.operation == Operation::NewBody {
            if changed {
                self.invalidate();
            }
            return changed;
        }
        changed |= ui
            .checkbox(&mut self.keep_tools, "Keep extrusion tool bodies")
            .on_hover_text(
                "Keep the generated extrusion Parts alongside the resulting target Parts",
            )
            .changed();
        if self.operation == Operation::Cut {
            ui.small("Cut finds visible intersecting bodies in the chosen component scope. Preview computes exact solid intersections.");
            ui.label("Cut scope").on_hover_text("Starts with the active component when this extrusion opens; explicit includes can reach other bodies or components");
            controls::inspector_combo(ui, "extrude-cut-component")
                .selected_text(
                    self.component
                        .and_then(|id| {
                            self.choices
                                .iter()
                                .find(|entry| entry.0 == id)
                                .map(|entry| entry.1.as_str())
                        })
                        .unwrap_or("Document root"),
                )
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut self.component, None, "Document root")
                        .changed();
                    for (id, label, is_body) in &self.choices {
                        if !is_body {
                            changed |= ui
                                .selectable_value(&mut self.component, Some(*id), label)
                                .changed();
                        }
                    }
                });
            if let Some(selection) = &self.selection {
                ui.strong(if self.fresh {
                    "Exact affected bodies"
                } else {
                    "Previous exact set; Preview to update"
                });
                for target in &selection.targets {
                    ui.label(choice_label(&self.choices, target.body));
                }
                if selection.targets.is_empty() {
                    ui.small("No intersecting targets");
                }
                controls::inspector_collapsing(
                    ui,
                    "extrude-cut-excludes",
                    "Exclude affected bodies",
                    |ui| {
                        let bodies: BTreeSet<_> = selection
                            .defaults
                            .iter()
                            .chain(&selection.targets)
                            .map(|target| target.body)
                            .chain(self.exclude.iter().copied())
                            .collect();
                        for body in bodies {
                            let mut excluded = self.exclude.contains(&body);
                            if ui
                                .checkbox(&mut excluded, choice_label(&self.choices, body))
                                .changed()
                            {
                                if excluded {
                                    self.exclude.insert(body);
                                } else {
                                    self.exclude.remove(&body);
                                }
                                changed = true;
                            }
                        }
                    },
                );
                for id in &selection.not_intersecting {
                    ui.small(format!(
                        "No solid overlap: {}",
                        choice_label(&self.choices, *id)
                    ));
                }
            }
            controls::inspector_collapsing(
                ui,
                "extrude-cut-includes",
                "Include other bodies or components",
                |ui| {
                    for (id, label, is_body) in &self.choices {
                        let mut included = self.include.contains(id);
                        if ui
                            .add_enabled(
                                !is_body || self.native.contains_key(id),
                                egui::Checkbox::new(&mut included, label),
                            )
                            .on_disabled_hover_text(
                                "This Part has no current unconsumed native output",
                            )
                            .changed()
                        {
                            if included {
                                self.include.insert(*id);
                            } else {
                                self.include.remove(id);
                            }
                            changed = true;
                        }
                    }
                },
            );
        } else {
            ui.strong("Explicit target bodies");
            if let Some(error) = &self.context_error {
                ui.small(error);
            }
            for (id, label, is_body) in &self.choices {
                if !is_body {
                    continue;
                }
                let mut selected = self.targets.contains(id);
                if ui
                    .add_enabled(
                        self.native.contains_key(id),
                        egui::Checkbox::new(&mut selected, label),
                    )
                    .on_disabled_hover_text("This Part has no current unconsumed native output")
                    .changed()
                {
                    if selected {
                        self.targets.insert(*id);
                    } else {
                        self.targets.remove(id);
                    }
                    changed = true;
                }
            }
        }
        if changed {
            self.invalidate();
        }
        changed
    }
}
fn choice_label(choices: &[(EntityId, String, bool)], id: EntityId) -> String {
    choices
        .iter()
        .find(|entry| entry.0 == id)
        .map(|entry| entry.1.clone())
        .unwrap_or_else(|| id.to_string())
}
fn operation_label(operation: Operation) -> &'static str {
    match operation {
        Operation::NewBody => "New body",
        Operation::Join => "Join / merge",
        Operation::Cut => "Cut / remove material",
        Operation::Intersect => "Intersect",
        Operation::Split => "Split",
    }
}

pub(super) struct CutDiscovery {
    pub prepared: Prepared,
    pub options: EvaluationOptions,
    pub content: ContentHash,
    extrusion: ExtrudeRequest,
}
impl CutDiscovery {
    pub fn prepare(
        session: &Session,
        actor: Actor,
        extrusion: ExtrudeRequest,
        component: Option<EntityId>,
        include: Vec<EntityId>,
        exclude: Vec<EntityId>,
    ) -> Result<Self, String> {
        let tools = model::prepare_extrude(session, session.revision(), actor, &extrusion)
            .map_err(|error| error.to_string())?;
        let content = ContentHash::document(tools.prepared.document())?;
        let options = EvaluationOptions {
            cut_targets: vec![CutTargetProbe {
                id: EntityId::new(),
                query: CutTargetQuery {
                    tools: extrusion
                        .outputs
                        .iter()
                        .map(|output| output.feature)
                        .collect(),
                    component,
                    include,
                    exclude,
                },
            }],
        };
        options.validate().map_err(|error| error.to_string())?;
        Ok(Self {
            prepared: tools.prepared,
            options,
            content,
            extrusion,
        })
    }
    pub fn selection<'a>(
        &self,
        session: &Session,
        scene: &'a Scene,
        expected: SceneStamp,
    ) -> Result<&'a CutTargetSelection, String> {
        session
            .validate_preview(&self.prepared)
            .map_err(|error| error.to_string())?;
        if scene.stamp() != expected
            || expected.document != session.document().id()
            || expected.revision != session.revision()
        {
            return Err("Cut target results belong to an obsolete modeling request".into());
        }
        let info = scene
            .model_evaluation()
            .ok_or("The native provider did not return exact cut targets")?;
        if info.source_hash != self.content {
            return Err("Cut target results describe different extrusion source".into());
        }
        if info.cut_targets.len() != 1 || info.cut_targets[0].id != self.options.cut_targets[0].id {
            return Err("Cut target results do not match the requested probe".into());
        }
        for output in &self.extrusion.outputs {
            if !scene
                .reference_instances()
                .iter()
                .any(|instance| instance.id == output.body && instance.geometry == output.feature)
                || !scene
                    .geometry_snapshot(output.feature)
                    .is_some_and(|snapshot| !snapshot.elements.is_empty())
            {
                return Err(
                    "Native target discovery did not retain the exact extrusion tools".into(),
                );
            }
        }
        let selection = info.cut_targets[0]
            .result
            .as_ref()
            .map_err(|error| error.to_string())?;
        for target in selection.defaults.iter().chain(&selection.targets) {
            target
                .resolve_producer(session.document())
                .map_err(|error| error.to_string())?;
            let head = match &session
                .document()
                .get(target.body)
                .ok_or("Cut target body is missing")?
                .data
            {
                EntityData::Part { features, .. } => features.iter().rev().copied().find(|id| {
                    matches!(
                        session.document().get(*id).map(|entity| &entity.data),
                        Some(EntityData::Feature {
                            suppressed: false,
                            ..
                        })
                    )
                }),
                _ => None,
            };
            if head != Some(target.producer) {
                return Err(
                    "Cut target results reference a historical rather than current body producer"
                        .into(),
                );
            }
            if !scene
                .reference_instances()
                .iter()
                .any(|instance| instance.id == target.body && instance.geometry == target.producer)
            {
                return Err("Cut target results do not match current native body producers".into());
            }
        }
        Ok(selection)
    }
    pub fn extrusion(&self) -> &ExtrudeRequest {
        &self.extrusion
    }
}

/// Stable final producers across parameter edits, target overrides and repeated
/// previews in one form. Removing then restoring a target reuses its identity.
#[derive(Default)]
pub(super) struct BooleanIdentities {
    features: BTreeMap<EntityId, EntityId>,
    group: Option<model::ExtrudeGroupDraft>,
}
impl BooleanIdentities {
    pub fn new(document: &Document) -> Self {
        let name = super::names::GeneratedNames::new(document).numbered("Extrusion operation", "");
        Self {
            features: BTreeMap::new(),
            group: Some(model::ExtrudeGroupDraft::new(name, None)),
        }
    }
    pub fn compose(
        &mut self,
        extrusion: ExtrudeRequest,
        operation: Operation,
        targets: Vec<BodyInput>,
        keep_tools: bool,
    ) -> Result<ExtrudeBooleanRequest, String> {
        if !matches!(
            operation,
            Operation::Join | Operation::Cut | Operation::Intersect
        ) {
            return Err("Choose Join, Cut or Intersect for an extrusion body operation".into());
        }
        if targets.is_empty() {
            return Err("No affected bodies; change the extent or choose explicit targets".into());
        }
        let mut request = ExtrudeBooleanRequest::new(extrusion, operation, targets, keep_tools);
        request.group = Some(
            self.group
                .get_or_insert_with(|| model::ExtrudeGroupDraft::new("Extrusion operation", None))
                .clone(),
        );
        for boolean in &mut request.booleans {
            boolean.feature = *self.features.entry(boolean.target.body).or_default();
        }
        Ok(request)
    }
}

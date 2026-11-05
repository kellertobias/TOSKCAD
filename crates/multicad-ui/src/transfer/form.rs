use super::{TransferCaptureKind, TransferLane, TransferPick, TransferSelection, fields::Fields};
use crate::{
    controls, egui,
    icons::{self, Icon},
    viewport::Scene,
};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    materials,
    model::{
        self, AlignmentInput, BodyInput, OccurrencePath, TransferDepth, TransferRequest,
        TransferSelection as SourceSelection, TransferSource, TransferTarget,
        TransferTargetParameters,
    },
    references::GeometryIntent,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SourceMode {
    Circles,
    FaceHoles,
    FaceBoundary,
}
impl SourceMode {
    fn label(self) -> &'static str {
        match self {
            Self::Circles => "Individual circles / cylinders",
            Self::FaceHoles => "All circular holes on a face",
            Self::FaceBoundary => "Face outer boundary",
        }
    }
}
#[derive(Clone, Debug)]
pub(super) struct Choice {
    pub path: OccurrencePath,
    pub component: EntityId,
    pub producer: EntityId,
    pub label: String,
    component_name: String,
    discriminator: String,
    ambiguous_name: bool,
    help: String,
}
pub(super) struct Target {
    pub id: EntityId,
    pub feature: EntityId,
    pub input: Option<BodyInput>,
    pub path: Option<OccurrencePath>,
    pub face: Option<GeometryIntent>,
    pub parameters: TransferTargetParameters,
}
pub(super) struct Form {
    pub group: EntityId,
    pub name: String,
    pub editing: bool,
    pub version: u64,
    pub source: Option<Choice>,
    pub mode: SourceMode,
    pub captures: Vec<GeometryIntent>,
    pub targets: Vec<Target>,
    pub pick: Option<TransferLane>,
    pub replace_capture: Option<EntityId>,
    pub fields: Fields,
    pub choices: Vec<Choice>,
}
impl Form {
    pub fn new(
        selection: TransferSelection,
        document: &Document,
        scene: Option<&Arc<Scene>>,
    ) -> Result<Self, String> {
        let mut form = Self {
            group: EntityId::new(),
            name: "Transfer features".into(),
            editing: false,
            version: 0,
            source: None,
            mode: SourceMode::Circles,
            captures: vec![],
            targets: vec![],
            pick: None,
            replace_capture: None,
            fields: Fields::default(),
            choices: catalogue(document, scene)?,
        };
        if let Some(path) = selection.source {
            form.source = form.choices.iter().find(|c| c.path == path).cloned();
            if form.source.is_some() {
                form.pick = Some(TransferLane::Source);
            }
        }
        for path in selection.targets {
            let row = form.add_target();
            if let Some(choice) = form.choices.iter().find(|c| c.path == path).cloned() {
                let target = form
                    .targets
                    .iter_mut()
                    .find(|t| t.id == row)
                    .expect("new row");
                target.input = Some(BodyInput {
                    body: choice.component,
                    producer: choice.producer,
                });
                target.path = Some(choice.path);
            }
        }
        if form.targets.is_empty() {
            form.add_target();
        }
        Ok(form)
    }
    pub fn from_request(
        request: TransferRequest,
        document: &Document,
        scene: Option<&Arc<Scene>>,
    ) -> Result<Self, String> {
        let source = current_producer(document, request.source.component)
            .ok()
            .map(|producer| {
                make_choice(
                    document,
                    request.source.path,
                    request.source.component,
                    producer,
                )
            });
        let ids = request.source.selection.captures();
        let mode = match request.source.selection {
            SourceSelection::Circles { .. } => SourceMode::Circles,
            SourceSelection::FaceHoles { .. } => SourceMode::FaceHoles,
            SourceSelection::FaceBoundary { .. } => SourceMode::FaceBoundary,
        };
        let find = |id| {
            request
                .references
                .iter()
                .find(|r| r.intent.id == id)
                .map(|r| r.intent.clone())
                .ok_or("A stored transfer capture is missing".to_string())
        };
        let captures = ids.into_iter().map(find).collect::<Result<Vec<_>, _>>()?;
        let targets = request
            .targets
            .into_iter()
            .map(|t| {
                Ok(Target {
                    id: t.id,
                    feature: t.feature,
                    input: Some(t.input),
                    path: Some(t.path),
                    face: Some(find(t.face)?),
                    parameters: t.parameters,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            group: request.group,
            name: request.name,
            editing: true,
            version: 0,
            source,
            mode,
            captures,
            targets,
            pick: None,
            replace_capture: None,
            fields: Fields::default(),
            choices: catalogue(document, scene)?,
        })
    }
    pub fn request(&self) -> Result<TransferRequest, String> {
        if let Some(error) = &self.fields.error {
            return Err(error.clone());
        }
        let source = self.source.as_ref().ok_or("Choose the source occurrence")?;
        let selection = match self.mode {
            SourceMode::Circles => {
                if self.captures.is_empty() {
                    return Err("Select at least one source circle or cylindrical face".into());
                }
                SourceSelection::Circles {
                    references: self.captures.iter().map(|r| r.id).collect(),
                }
            }
            SourceMode::FaceHoles => SourceSelection::FaceHoles {
                face: self
                    .captures
                    .first()
                    .ok_or("Select the source hole-pattern face")?
                    .id,
            },
            SourceMode::FaceBoundary => SourceSelection::FaceBoundary {
                face: self
                    .captures
                    .first()
                    .ok_or("Select the source outline face")?
                    .id,
            },
        };
        let mut references = self
            .captures
            .iter()
            .map(|intent| AlignmentInput {
                source: source.component,
                intent: intent.clone(),
            })
            .collect::<Vec<_>>();
        let targets = self
            .targets
            .iter()
            .map(|target| {
                let input = target
                    .input
                    .clone()
                    .ok_or("Choose every receiving occurrence")?;
                let face = target
                    .face
                    .as_ref()
                    .ok_or("Select every receiving planar face")?;
                references.push(AlignmentInput {
                    source: input.producer,
                    intent: face.clone(),
                });
                Ok(TransferTarget {
                    id: target.id,
                    feature: target.feature,
                    input,
                    path: target
                        .path
                        .clone()
                        .ok_or("Choose every receiving occurrence")?,
                    face: face.id,
                    parameters: target.parameters.clone(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if targets.is_empty() {
            return Err("Add at least one receiving Part".into());
        }
        Ok(TransferRequest {
            group: self.group,
            name: self.name.clone(),
            source: TransferSource {
                component: source.component,
                path: source.path.clone(),
                selection,
            },
            targets,
            references,
        })
    }
    pub fn changed(&mut self) {
        self.version = self.version.saturating_add(1);
    }
    pub fn add_target(&mut self) -> EntityId {
        let id = EntityId::new();
        self.targets.push(Target {
            id,
            feature: EntityId::new(),
            input: None,
            path: None,
            face: None,
            parameters: Default::default(),
        });
        self.changed();
        id
    }
    pub fn set_occurrence(
        &mut self,
        lane: TransferLane,
        path: OccurrencePath,
    ) -> Result<(), String> {
        let choice = self
            .choices
            .iter()
            .find(|c| c.path == path)
            .cloned()
            .ok_or("Choose a current authored Part occurrence")?;
        match lane {
            TransferLane::Source => {
                if self
                    .source
                    .as_ref()
                    .is_none_or(|s| s.component != choice.component)
                {
                    self.captures.clear();
                }
                self.source = Some(choice);
                self.pick = Some(lane);
            }
            TransferLane::Target { target } => {
                let target = self
                    .targets
                    .iter_mut()
                    .find(|t| t.id == target)
                    .ok_or("The receiving row no longer exists")?;
                if self.editing
                    && target
                        .input
                        .as_ref()
                        .is_some_and(|i| i.body != choice.component)
                {
                    return Err("An applied receiving Part is fixed; create another transfer to change its definition".into());
                }
                if target
                    .input
                    .as_ref()
                    .is_none_or(|i| i.body != choice.component)
                {
                    target.input = Some(BodyInput {
                        body: choice.component,
                        producer: choice.producer,
                    });
                    target.face = None;
                }
                target.path = Some(choice.path);
                self.pick = Some(lane);
            }
        }
        self.replace_capture = None;
        self.changed();
        Ok(())
    }
    pub fn picker(&self) -> Option<TransferPick> {
        let lane = self.pick?;
        match lane {
            TransferLane::Source => {
                let source = self.source.as_ref()?;
                Some(TransferPick {
                    lane,
                    occurrence: source.path.clone(),
                    component: source.component,
                    producer: source.producer,
                    kind: if self.mode == SourceMode::Circles {
                        TransferCaptureKind::CircularFeature
                    } else {
                        TransferCaptureKind::PlanarFace
                    },
                    replace_capture: self.replace_capture,
                })
            }
            TransferLane::Target { target } => {
                let target = self.targets.iter().find(|t| t.id == target)?;
                let input = target.input.as_ref()?;
                Some(TransferPick {
                    lane,
                    occurrence: target.path.clone()?,
                    component: input.body,
                    producer: input.producer,
                    kind: TransferCaptureKind::PlanarFace,
                    replace_capture: target.face.as_ref().map(|f| f.id),
                })
            }
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, document: &Document) -> bool {
        self.fields.start();
        let mut changed = false;
        ui.label("Transfer name");
        changed |= ui.text_edit_singleline(&mut self.name).changed();
        ui.small("Source geometry remains independent. Receiving features follow the chosen occurrences and their placements.");
        let mut source = self.source.as_ref().map(|c| c.path.clone());
        if choose(
            ui,
            "transfer-source",
            "Source occurrence",
            &mut source,
            &self.choices.iter().collect::<Vec<_>>(),
            self.source.as_ref().map(|source| source.label.clone()),
        ) && let Some(path) = source
        {
            if let Err(e) = self.set_occurrence(TransferLane::Source, path) {
                self.fields.error = Some(e);
            }
            changed = true;
        }
        ui.label("Source geometry");
        let old = self.mode;
        controls::inspector_combo(ui, "transfer-mode")
            .selected_text(self.mode.label())
            .show_ui(ui, |ui| {
                for mode in [
                    SourceMode::Circles,
                    SourceMode::FaceHoles,
                    SourceMode::FaceBoundary,
                ] {
                    if ui
                        .selectable_value(&mut self.mode, mode, mode.label())
                        .clicked()
                    {
                        ui.close();
                    }
                }
            }).response.on_hover_text("Choose individual circular features, all circular holes on a face, or the face's closed outer boundary.");
        if self.mode != old {
            self.captures.clear();
            self.replace_capture = None;
            self.pick = self.source.as_ref().map(|_| TransferLane::Source);
            if self.mode == SourceMode::FaceBoundary {
                for target in &mut self.targets {
                    target.parameters.diameter = None;
                }
            }
            self.fields.clear();
            changed = true;
        }
        if self.mode == SourceMode::FaceHoles {
            ui.small("All circular inner loops of this face follow source pattern count and pitch. An empty pattern makes no cuts.");
        } else if self.mode == SourceMode::FaceBoundary {
            ui.small("Transfer the face’s outer outline. Nonzero clearance supports circles and strictly convex line outlines; native Preview checks support.");
        }
        if ui
            .add_enabled(
                self.source.is_some(),
                egui::Button::new(if self.pick == Some(TransferLane::Source) {
                    "Finish source selection"
                } else {
                    "Select source geometry"
                }),
            )
            .on_hover_text("Pick exact source geometry in the selected Part. Circles supply hole centers; face modes supply a hole pattern or outer outline.")
            .clicked()
        {
            self.pick = if self.pick == Some(TransferLane::Source) {
                None
            } else {
                Some(TransferLane::Source)
            };
            self.replace_capture = None;
        }
        let mut remove = None;
        for (index, capture) in self.captures.iter().enumerate() {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "{} reference {}",
                    if self.mode == SourceMode::Circles {
                        "Circular"
                    } else {
                        "Face"
                    },
                    index + 1
                ));
                if ui
                    .small_button("Reselect")
                    .on_hover_text("Replace this reference while retaining its identity")
                    .clicked()
                {
                    self.pick = Some(TransferLane::Source);
                    self.replace_capture = Some(capture.id);
                }
                if icons::mini_button(ui, Icon::Trash, "Remove source reference").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            let removed = self.captures.remove(index);
            if self.replace_capture == Some(removed.id) {
                self.replace_capture = None;
            }
            changed = true;
        }
        ui.small("Threaded-hole transfer is not supported. Native Preview refuses known thread references before Apply; create the receiving thread explicitly.");
        ui.separator();
        ui.strong("Receiving Parts");
        ui.colored_label(
            egui::Color32::LIGHT_YELLOW,
            "Cuts edit each Part definition. Every copy of that Part receives the same local cuts.",
        );
        let mut remove = None;
        let mut reorder = None;
        for (index, target) in self.targets.iter_mut().enumerate() {
            let lane = TransferLane::Target { target: target.id };
            ui.push_id(target.id.to_string(), |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(format!("Receiving Part {}", index + 1));
                    if icons::mini_button(ui, Icon::ArrowUp, "Move receiving row up").clicked()
                        && index > 0
                    {
                        reorder = Some((index, index - 1));
                    }
                    if icons::mini_button(ui, Icon::ArrowDown, "Move receiving row down").clicked()
                    {
                        reorder = Some((index, index + 1));
                    }
                    if !self.editing
                        && icons::mini_button(ui, Icon::Trash, "Remove receiving Part").clicked()
                    {
                        remove = Some(index);
                    }
                });
                let choices = self
                    .choices
                    .iter()
                    .filter(|c| {
                        !self.editing
                            || target.input.as_ref().is_some_and(|i| i.body == c.component)
                    })
                    .collect::<Vec<_>>();
                let mut path = target.path.clone();
                if choose(
                    ui,
                    "receiver",
                    "Occurrence",
                    &mut path,
                    &choices,
                    target.path.as_ref().map(|p|{let component=target.input.as_ref().map_or(p.root,|i|i.body);format!("{} · {}",document.get(component).map_or("Missing Part",|e|e.name.as_str()),path_name(document,p))}),
                ) && let Some(choice) = choices.iter().find(|c| Some(&c.path) == path.as_ref())
                {
                    if target
                        .input
                        .as_ref()
                        .is_none_or(|i| i.body != choice.component)
                    {
                        target.input = Some(BodyInput {
                            body: choice.component,
                            producer: choice.producer,
                        });
                        target.face = None;
                    }
                    target.path = path;
                    self.pick = Some(lane);
                    self.replace_capture = None;
                    changed = true;
                }
                if let Some(input) = &target.input {
                    if let Ok(properties) = materials::resolve(document, input.body) {
                        ui.small(format!(
                            "Material: {}",
                            properties
                                .material
                                .and_then(|id| document.get(id))
                                .map_or("Unassigned", |e| e.name.as_str())
                        ));
                    }
                    if ui
                        .button(if self.pick == Some(lane) {
                            "Picking receiving face…"
                        } else if target.face.is_some() {
                            "Reselect receiving face"
                        } else {
                            "Select receiving face"
                        })
                        .on_hover_text("Choose the receiving planar face before the transferred cuts. Configure cut direction and depth below.")
                        .clicked()
                    {
                        self.pick = Some(lane);
                        self.replace_capture = None;
                    }
                }
                if self.mode != SourceMode::FaceBoundary {
                    let mut override_diameter = target.parameters.diameter.is_some();
                    if ui
                        .checkbox(&mut override_diameter, "Override source diameter")
                        .changed()
                    {
                        target.parameters.diameter = override_diameter.then(|| model::length(4.));
                        self.fields.clear();
                        changed = true;
                    }
                    if let Some(diameter) = &mut target.parameters.diameter {
                        self.fields.length(
                            ui,
                            document,
                            format!("{}/diameter", target.id),
                            "Hole diameter",
                            diameter,
                        );
                    }
                    ui.small(
                        "Final diameter = source or override diameter + 2 × radial clearance.",
                    );
                }
                self.fields.length(
                    ui,
                    document,
                    format!("{}/clearance", target.id),
                    if self.mode == SourceMode::FaceBoundary {
                        "Outline clearance (+ expands)"
                    } else {
                        "Radial clearance"
                    },
                    &mut target.parameters.clearance,
                );
                let mut through = matches!(target.parameters.depth, TransferDepth::Through);
                if ui
                    .checkbox(&mut through, "Through receiving body")
                    .changed()
                {
                    target.parameters.depth = if through {
                        TransferDepth::Through
                    } else {
                        TransferDepth::Blind {
                            depth: model::length(10.),
                        }
                    };
                    self.fields.clear();
                    changed = true;
                }
                if let TransferDepth::Blind { depth } = &mut target.parameters.depth {
                    self.fields.length(
                        ui,
                        document,
                        format!("{}/depth", target.id),
                        "Blind cut depth",
                        depth,
                    );
                }
                ui.label("Cut direction");
                controls::inspector_combo(ui, "cut-direction")
                    .selected_text(if target.parameters.reverse {
                        "Opposite face normal (−)"
                    } else {
                        "Along face normal (+)"
                    })
                    .show_ui(ui, |ui| {
                        for (reverse, label) in [
                            (true, "Opposite face normal (−)"),
                            (false, "Along face normal (+)"),
                        ] {
                            if ui
                                .selectable_value(&mut target.parameters.reverse, reverse, label)
                                .clicked()
                            {
                                changed = true;
                                ui.close();
                            }
                        }
                    });
                ui.separator();
            });
        }
        if let Some(index) = remove {
            let id = self.targets.remove(index).id;
            if self.pick == Some(TransferLane::Target { target: id }) {
                self.pick = None;
            }
            changed = true;
        }
        if let Some((from, to)) = reorder
            && to < self.targets.len()
        {
            self.targets.swap(from, to);
            changed = true;
        }
        if !self.editing
            && self.targets.len() < model::MAX_TRANSFER_TARGETS
            && ui.button("Add receiving Part").clicked()
        {
            self.add_target();
            changed = true;
        }
        if self.editing {
            ui.small("Receiving definitions and upstream geometry stay fixed in an applied transfer. Create another transfer to add/remove receiving Parts.");
        }
        changed |= self.fields.changed;
        if changed {
            self.changed();
        }
        changed
    }
}
fn choose(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    path: &mut Option<OccurrencePath>,
    choices: &[&Choice],
    fallback: Option<String>,
) -> bool {
    ui.label(label);
    let mut changed = false;
    let selected = path
        .as_ref()
        .and_then(|path| choices.iter().find(|choice| &choice.path == path));
    let name = selected
        .map(|choice| selected_name(ui, choice))
        .or(fallback)
        .unwrap_or("Choose Part occurrence…".into());
    let help=selected.map(|choice|choice.help.clone()).unwrap_or_else(||path.as_ref().map_or("Choose an authored Part occurrence. The path selects which copy supplies the assembly placement.".into(),|path|format!("Stored occurrence path IDs: {}",path_ids(path))));
    controls::inspector_combo(ui, id)
        .selected_text(name)
        .show_ui(ui, |ui| {
            for choice in choices {
                if ui
                    .selectable_label(path.as_ref() == Some(&choice.path), &choice.label)
                    .on_hover_text(&choice.help)
                    .clicked()
                {
                    changed |= path.as_ref() != Some(&choice.path);
                    *path = Some(choice.path.clone());
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(help);
    changed
}
pub(super) fn catalogue(
    document: &Document,
    scene: Option<&Arc<Scene>>,
) -> Result<Vec<Choice>, String> {
    catalogue_scene(document, scene.map(Arc::as_ref))
}
pub(super) fn catalogue_scene(
    document: &Document,
    scene: Option<&Scene>,
) -> Result<Vec<Choice>, String> {
    let Some(info) = scene.and_then(|s| s.assembly_evaluation()) else {
        return authored_catalogue(document);
    };
    let mut seen = BTreeSet::new();
    let mut choices=info.leaves.iter().map(|leaf|{if !matches!(document.get(leaf.component).map(|e|&e.data),Some(EntityData::Part{..}))||!seen.insert(leaf.path.clone()){return Err("Native transfer catalogue contains a missing Part or duplicate occurrence path".into());}Ok(make_choice(document,leaf.path.clone(),leaf.component,leaf.producer))}).collect::<Result<Vec<_>,String>>()?;
    decorate_choices(document, &mut choices);
    Ok(choices)
}
pub(super) fn path_name(document: &Document, path: &OccurrencePath) -> String {
    std::iter::once(path.root)
        .chain(path.members.iter().copied())
        .map(|id| {
            document
                .get(id)
                .map_or("Missing occurrence".into(), |e| e.name.clone())
        })
        .collect::<Vec<_>>()
        .join(" / ")
}
fn current_producer(document: &Document, part: EntityId) -> Result<EntityId, String> {
    let Some(EntityData::Part { features, .. }) = document.get(part).map(|e| &e.data) else {
        return Err("The transfer source Part is missing".into());
    };
    features
        .iter()
        .rev()
        .copied()
        .find(|id| {
            matches!(
                document.get(*id).map(|e| &e.data),
                Some(EntityData::Feature {
                    suppressed: false,
                    ..
                })
            )
        })
        .ok_or("The transfer source Part has no active producer".into())
}

fn authored_catalogue(document: &Document) -> Result<Vec<Choice>, String> {
    let mut choices = model::transfer_occurrence_catalog(document)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|entry| make_choice(document, entry.path, entry.component, entry.producer))
        .collect::<Vec<_>>();
    decorate_choices(document, &mut choices);
    Ok(choices)
}
fn path_ids(path: &OccurrencePath) -> String {
    std::iter::once(path.root)
        .chain(path.members.iter().copied())
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(" / ")
}
fn make_choice(
    document: &Document,
    path: OccurrencePath,
    component: EntityId,
    producer: EntityId,
) -> Choice {
    let component_name = document
        .get(component)
        .map_or("Missing Part".into(), |e| e.name.clone());
    let ids = path_ids(&path);
    let digest = multicad_core::project_file::ContentHash::of(ids.as_bytes()).to_string();
    let discriminator = digest[..6].to_owned();
    let readable = path_name(document, &path);
    Choice {
        label: format!("{component_name} · {readable} [{discriminator}]"),
        help: format!(
            "Part: {component_name}\nOccurrence: {readable}\nPath ID: {discriminator}\nFull path IDs: {ids}\nComponent ID: {component}"
        ),
        path,
        component,
        producer,
        component_name,
        discriminator,
        ambiguous_name: true,
    }
}
fn decorate_choices(document: &Document, choices: &mut [Choice]) {
    let digests = choices
        .iter()
        .map(|choice| {
            multicad_core::project_file::ContentHash::of(path_ids(&choice.path).as_bytes())
                .to_string()
        })
        .collect::<Vec<_>>();
    let mut resolved = vec![false; choices.len()];
    let mut name_counts = BTreeMap::new();
    for choice in choices.iter() {
        *name_counts
            .entry(choice.component_name.clone())
            .or_insert(0usize) += 1;
    }
    for width in (6..=64).step_by(2) {
        let mut counts = BTreeMap::new();
        for hash in &digests {
            *counts.entry(&hash[..width]).or_insert(0usize) += 1;
        }
        for (i, choice) in choices.iter_mut().enumerate() {
            if !resolved[i] && counts[&digests[i][..width]] == 1 {
                choice.discriminator = digests[i][..width].into();
                resolved[i] = true;
            }
        }
        if resolved.iter().all(|resolved| *resolved) {
            break;
        }
    }
    for (i, choice) in choices.iter_mut().enumerate() {
        if !resolved[i] {
            choice.discriminator = path_ids(&choice.path);
        }
        choice.ambiguous_name = name_counts[&choice.component_name] > 1;
        let readable = path_name(document, &choice.path);
        choice.label = format!(
            "{} · {readable} [{}]",
            choice.component_name, choice.discriminator
        );
        choice.help = format!(
            "Part: {}\nOccurrence: {readable}\nPath ID: {}\nFull path IDs: {}\nComponent ID: {}",
            choice.component_name,
            choice.discriminator,
            path_ids(&choice.path),
            choice.component
        );
    }
}
fn selected_name(ui: &egui::Ui, choice: &Choice) -> String {
    if !choice.ambiguous_name {
        return choice.component_name.clone();
    }
    let suffix = format!(" ·{}", choice.discriminator);
    let font = egui::TextStyle::Button.resolve(ui.style());
    let width = ui.available_width().min(220.) - 28.;
    let measure = |text: String| {
        ui.painter()
            .layout_no_wrap(text, font.clone(), ui.visuals().text_color())
            .size()
            .x
    };
    let mut name = choice.component_name.clone();
    if measure(format!("{name}{suffix}")) <= width {
        return format!("{name}{suffix}");
    }
    while name.chars().count() > 1 && measure(format!("{name}…{suffix}")) > width {
        name.pop();
    }
    format!("{name}…{suffix}")
}

#[cfg(test)]
mod label_tests {
    use super::*;
    use multicad_core::{
        commands::{Actor, Channel, Session},
        model::{
            AssemblyRequest, ComponentPlacementRequest, PrimitiveParameters, PrimitiveRequest,
            length,
        },
    };
    fn fixture() -> (Document, Vec<EntityId>) {
        let mut session =
            Session::new(Document::new("Readable assembly choices").unwrap()).unwrap();
        let actor = || Actor::local(Channel::Ui);
        let mut parts = vec![];
        let mut members = vec![];
        for name in [
            "Independent drawer rail",
            "Wood receiver",
            "Printed receiver",
            "Sheet receiver",
        ] {
            let part = PrimitiveRequest::new(
                name,
                PrimitiveParameters::Box {
                    size: [length(10.), length(10.), length(10.)],
                    frame: Default::default(),
                },
            );
            session
                .commit(
                    model::prepare_primitive(&session, session.revision(), actor(), &part)
                        .unwrap()
                        .prepared,
                )
                .unwrap();
            parts.push(part.body);
            let member = ComponentPlacementRequest::new(part.body, Default::default());
            session
                .commit(
                    model::prepare_place_component(&session, session.revision(), actor(), &member)
                        .unwrap()
                        .prepared,
                )
                .unwrap();
            members.push(member.instance);
        }
        let assembly = AssemblyRequest::new("Fixture members", members, parts.clone());
        session
            .commit(
                model::prepare_assembly(&session, session.revision(), actor(), &assembly)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        for _ in 0..2 {
            let root = ComponentPlacementRequest::new(assembly.assembly, Default::default());
            session
                .commit(
                    model::prepare_place_component(&session, session.revision(), actor(), &root)
                        .unwrap()
                        .prepared,
                )
                .unwrap();
        }
        (session.document().clone(), parts)
    }
    #[test]
    fn identical_instance_names_show_part_first_unique_copy_ids_and_full_hover_paths() {
        let (document, parts) = fixture();
        let choices = authored_catalogue(&document).unwrap();
        assert_eq!(choices.len(), 8);
        let original = model::transfer_occurrence_catalog(&document)
            .unwrap()
            .into_iter()
            .map(|c| (c.path, c.component, c.producer))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            choices
                .iter()
                .map(|c| (c.path.clone(), c.component, c.producer))
                .collect::<BTreeSet<_>>(),
            original
        );
        assert!(
            choices
                .iter()
                .all(|c| path_name(&document, &c.path) == "Placed component / Placed component")
        );
        assert_eq!(
            choices
                .iter()
                .map(|c| &c.label)
                .collect::<BTreeSet<_>>()
                .len(),
            8
        );
        assert_eq!(
            choices
                .iter()
                .map(|c| &c.discriminator)
                .collect::<BTreeSet<_>>()
                .len(),
            8
        );
        for part in parts {
            let same = choices
                .iter()
                .filter(|c| c.component == part)
                .collect::<Vec<_>>();
            assert_eq!(same.len(), 2);
            let name = &document.get(part).unwrap().name;
            for choice in &same {
                assert!(choice.label.starts_with(name));
                assert!(choice.label.contains("Placed component / Placed component"));
                assert!(
                    choice
                        .label
                        .ends_with(&format!("[{}]", choice.discriminator))
                );
                for id in
                    std::iter::once(choice.path.root).chain(choice.path.members.iter().copied())
                {
                    assert!(choice.help.contains(&id.to_string()));
                }
                assert!(choice.help.contains(&part.to_string()));
            }
            assert_ne!(same[0].label, same[1].label);
        }
        let mut reordered = choices.clone();
        reordered.reverse();
        decorate_choices(&document, &mut reordered);
        for choice in &reordered {
            let before = choices.iter().find(|c| c.path == choice.path).unwrap();
            assert_eq!(choice.label, before.label);
            assert_eq!(choice.discriminator, before.discriminator);
        }
    }
    #[test]
    fn selected_text_at_narrow_inspector_width_keeps_part_name_and_copy_identity() {
        let (document, _) = fixture();
        let choices = authored_catalogue(&document).unwrap();
        let ctx = egui::Context::default();
        let mut selected = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(250., 900.),
                )),
                ..Default::default()
            },
            |ui| {
                for choice in &choices {
                    let name = selected_name(ui, choice);
                    assert!(
                        name.starts_with(choice.component_name.split_whitespace().next().unwrap())
                    );
                    assert!(name.contains(&choice.discriminator));
                    selected.push(name);
                    let mut path = Some(choice.path.clone());
                    choose(
                        ui,
                        &choice.discriminator,
                        "Occurrence",
                        &mut path,
                        &choices.iter().collect::<Vec<_>>(),
                        None,
                    );
                    assert_eq!(path, Some(choice.path.clone()));
                }
            },
        );
        output.textures_delta.clear();
        assert_eq!(selected.iter().collect::<BTreeSet<_>>().len(), 8);
        for name in [
            "Independent drawer rail",
            "Wood receiver",
            "Printed receiver",
            "Sheet receiver",
        ] {
            assert!(
                selected.iter().any(|label| label.starts_with(name)),
                "{name}: {selected:?}"
            );
        }
    }
}

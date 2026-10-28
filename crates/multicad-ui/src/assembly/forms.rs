use super::{
    AnchorPick, AnchorSide, AssemblySelection, AssemblyTool,
    widgets::{self, Fields},
};
use crate::{controls, egui};
use multicad_core::{
    commands::{Actor, Session},
    document::{Document, EntityData, EntityId},
    model::{
        self, AssemblyPrepared, AssemblyRequest, ComponentPlacementRequest, JointAnchor, JointKind,
        JointPoseValue, JointRequest, JointValues, NamedPoseRequest,
    },
};
#[derive(Clone)]
pub(super) enum Operation {
    Place(Box<ComponentPlacementRequest>),
    Assembly(AssemblyRequest),
    Joint(Box<JointRequest>),
    Pose(NamedPoseRequest),
    ApplyPose(EntityId),
    ReadOnly,
}
pub(super) struct Form {
    pub tool: AssemblyTool,
    pub operation: Operation,
    pub editing: bool,
    pub pick: Option<AnchorSide>,
    pub fields: Fields,
    pub version: u64,
    pub pose: Option<EntityId>,
    pub explode_mm: f64,
    pub preserve_placement: bool,
}
impl Form {
    pub fn new(
        tool: AssemblyTool,
        document: &Document,
        selection: AssemblySelection,
    ) -> Result<Self, String> {
        let operation = match tool {
            AssemblyTool::Place => Operation::Place(Box::new(ComponentPlacementRequest::new(
                selection.component.unwrap_or(missing()),
                Default::default(),
            ))),
            AssemblyTool::CreateAssembly => Operation::Assembly(AssemblyRequest::new(
                "Assembly",
                selection.instances,
                vec![],
            )),
            AssemblyTool::Relationship => Operation::Joint(Box::new(JointRequest::new(
                selection.instances.first().copied().unwrap_or(missing()),
                Default::default(),
                vec![],
            ))),
            AssemblyTool::Motion => selection
                .joint
                .or_else(|| {
                    selection
                        .instances
                        .first()
                        .and_then(|id| model::joint_driver(document, *id).ok().flatten())
                })
                .map(|id| {
                    JointRequest::existing(document, id).map(|r| Operation::Joint(Box::new(r)))
                })
                .transpose()
                .map_err(|e| e.to_string())?
                .unwrap_or_else(|| {
                    Operation::Joint(Box::new(JointRequest::new(
                        missing(),
                        Default::default(),
                        vec![],
                    )))
                }),
            AssemblyTool::NamedPose => {
                Operation::Pose(NamedPoseRequest::new("Closed", joint_values(document)))
            }
            _ => Operation::ReadOnly,
        };
        Ok(Self {
            tool,
            operation,
            editing: tool == AssemblyTool::Motion,
            pick: None,
            fields: Fields::default(),
            version: 0,
            pose: selection.pose,
            explode_mm: 40.,
            preserve_placement: tool == AssemblyTool::Relationship,
        })
    }
    pub fn existing(document: &Document, id: EntityId) -> Result<Self, String> {
        let (tool, operation) = if let Ok(request) = JointRequest::existing(document, id) {
            (
                AssemblyTool::Relationship,
                Operation::Joint(Box::new(request)),
            )
        } else if let Ok(request) = AssemblyRequest::existing(document, id) {
            (AssemblyTool::CreateAssembly, Operation::Assembly(request))
        } else if let Ok(request) = NamedPoseRequest::existing(document, id) {
            (AssemblyTool::NamedPose, Operation::Pose(request))
        } else {
            return Err("Choose an existing relationship, reusable assembly or named pose".into());
        };
        Ok(Self {
            tool,
            operation,
            editing: true,
            pick: None,
            fields: Fields::default(),
            version: 0,
            pose: None,
            explode_mm: 40.,
            preserve_placement: false,
        })
    }
    pub fn prepare(&self, session: &Session, actor: Actor) -> Result<AssemblyPrepared, String> {
        if let Some(error) = &self.fields.error {
            return Err(error.clone());
        }
        let revision = session.revision();
        let prepared = match &self.operation {
            Operation::Place(request) => {
                model::prepare_place_component(session, revision, actor, request)
            }
            Operation::Assembly(request) => {
                if self.editing {
                    model::prepare_assembly_edit(session, revision, actor, request)
                } else {
                    model::prepare_assembly(session, revision, actor, request)
                }
            }
            Operation::Joint(request) => {
                if self.editing {
                    model::prepare_joint_edit(session, revision, actor, request)
                } else {
                    model::prepare_joint(session, revision, actor, request)
                }
            }
            Operation::Pose(request) => {
                if self.editing {
                    model::prepare_named_pose_edit(session, revision, actor, request)
                } else {
                    model::prepare_named_pose(session, revision, actor, request)
                }
            }
            Operation::ApplyPose(pose) => {
                model::prepare_apply_pose(session, revision, actor, *pose)
            }
            Operation::ReadOnly => return Err("This view does not create source edits".into()),
        };
        prepared.map_err(|e| e.to_string())
    }
    pub fn changed(&mut self) {
        self.version = self.version.saturating_add(1);
    }
    pub fn show(&mut self, ui: &mut egui::Ui, document: &Document) -> bool {
        self.fields.start();
        let mut changed = false;
        let instances = choices(document, |d| matches!(d, EntityData::Instance { .. }));
        match &mut self.operation {
            Operation::Place(request) => {
                ui.small("Place another occurrence of a Part or reusable assembly. The component definition stays shared.");
                let components = choices(document, |d| {
                    matches!(d, EntityData::Part { .. } | EntityData::Assembly { .. })
                });
                let mut chosen = Some(request.component).filter(|id| !id.is_nil());
                if widgets::choose(
                    ui,
                    "assembly-component",
                    "Component",
                    &mut chosen,
                    &components,
                    None,
                ) {
                    request.component = chosen.unwrap_or(missing());
                    changed = true;
                }
                ui.label("Occurrence name");
                changed |= ui.text_edit_singleline(&mut request.name).changed();
                self.fields.vector(
                    ui,
                    document,
                    "place-translation",
                    "Translation",
                    &mut request.parameters.translation,
                );
                self.fields.vector(
                    ui,
                    document,
                    "place-pivot",
                    "Rotation pivot",
                    &mut request.parameters.pivot,
                );
                self.fields.dimension(
                    ui,
                    document,
                    "place-angle",
                    "Rotation",
                    &mut request.parameters.angle,
                    "deg",
                );
                direction(
                    ui,
                    "Rotation axis",
                    &mut request.parameters.axis,
                    &mut changed,
                );
            }
            Operation::Assembly(request) => {
                ui.small("A reusable definition contains the chosen member occurrences. Place the resulting assembly to reuse it.");
                ui.label("Assembly name");
                changed |= ui.text_edit_singleline(&mut request.name).changed();
                ui.label("Members (local placements)");
                for (id, name) in &instances {
                    let mut yes = request.members.contains(id);
                    if ui.checkbox(&mut yes, name).changed() {
                        changed = true;
                        if yes {
                            request.members.push(*id);
                        } else {
                            request.members.retain(|i| i != id);
                        }
                    }
                }
                controls::inspector_collapsing(
                    ui,
                    "definition-parts",
                    "Definition-only Parts",
                    |ui| {
                        ui.small("Choose Parts used only as reusable definitions, so they do not also appear as standalone solids.");
                        for (id, name) in
                            choices(document, |d| matches!(d, EntityData::Part { .. }))
                        {
                            let mut yes = request.definition_parts.contains(&id);
                            if ui.checkbox(&mut yes, name).changed() {
                                changed = true;
                                if yes {
                                    request.definition_parts.push(id);
                                } else {
                                    request.definition_parts.retain(|i| *i != id);
                                }
                            }
                        }
                    },
                );
            }
            Operation::Joint(request) => {
                if self.tool == AssemblyTool::Motion {
                    let joints = choices(
                        document,
                        |d| matches!(d,EntityData::Extension{definition,..}if definition.type_id==model::JOINT_TYPE),
                    );
                    let mut selected =
                        Some(request.driver).filter(|id| document.get(*id).is_some());
                    if widgets::choose(
                        ui,
                        "motion-joint",
                        "Relationship",
                        &mut selected,
                        &joints,
                        None,
                    ) && let Some(id) = selected
                        && let Ok(next) = JointRequest::existing(document, id)
                    {
                        **request = next;
                        self.fields.reset();
                        changed = true;
                    }
                    ui.small("Set exact joint coordinates within the inclusive limits. Preview checks the complete assembled model.");
                } else {
                    let old = request.parameters.kind;
                    ui.label("Relationship type");
                    controls::inspector_combo(ui, "joint-kind")
                        .selected_text(old.label())
                        .show_ui(ui, |ui| {
                            for kind in [
                                JointKind::Fixed,
                                JointKind::Slider,
                                JointKind::Revolute,
                                JointKind::Cylindrical,
                                JointKind::Planar,
                            ] {
                                if ui
                                    .selectable_value(
                                        &mut request.parameters.kind,
                                        kind,
                                        kind.label(),
                                    )
                                    .clicked()
                                {
                                    ui.close();
                                }
                            }
                        });
                    if old != request.parameters.kind {
                        request.parameters.values = JointValues::default();
                        request.parameters.limits = Default::default();
                        self.fields.reset();
                        changed = true;
                    }
                    let mut owner = Some(request.owner).filter(|id| !id.is_nil());
                    if !self.editing
                        && widgets::choose(
                            ui,
                            "joint-moving",
                            "Moving occurrence",
                            &mut owner,
                            &instances,
                            None,
                        )
                    {
                        request.owner = owner.unwrap_or(missing());
                        request.parameters.moving = Default::default();
                        retain_references(request);
                        self.fields.reset();
                        self.pick = None;
                        self.preserve_placement = true;
                        changed = true;
                    }
                    if self.editing {
                        ui.label(format!(
                            "Moving: {}",
                            widgets::name(document, request.owner)
                        ));
                    }
                    let parents = instances
                        .iter()
                        .filter(|(id, _)| *id != request.owner)
                        .cloned()
                        .collect::<Vec<_>>();
                    if widgets::choose(
                        ui,
                        "joint-parent",
                        "Relative to",
                        &mut request.parameters.parent,
                        &parents,
                        Some("World"),
                    ) {
                        request.parameters.target = Default::default();
                        retain_references(request);
                        self.fields.reset();
                        self.pick = None;
                        self.preserve_placement = true;
                        changed = true;
                    }
                    if ui.button("Use current placement").on_hover_text("Capture the moving occurrence’s present position and directions as a numeric target anchor").clicked(){self.preserve_placement=true;changed=true;}
                    ui.small("Coordinates follow the target anchor: slider Z; hinge about Z; planar X/Y and rotation about Z.");
                    for side in [AnchorSide::Moving, AnchorSide::Target] {
                        let (label, instance, anchor) = match side {
                            AnchorSide::Moving => (
                                "Moving local anchor",
                                Some(request.owner),
                                &mut request.parameters.moving,
                            ),
                            AnchorSide::Target => (
                                "Target local anchor",
                                request.parameters.parent,
                                &mut request.parameters.target,
                            ),
                        };
                        controls::inspector_collapsing(
                            ui,
                            format!("anchor-{side:?}"),
                            label,
                            |ui| {
                                if instance.is_some_and(|id|!id.is_nil())&&ui.button(if self.pick==Some(side){"Picking native frame…"}else{"Pick face"}).on_hover_text("Select an explicitly named native face frame on this occurrence").clicked(){self.pick=Some(side);}
                                if matches!(anchor, JointAnchor::Geometry { .. }) {
                                    ui.label("Referenced native frame");
                                    if ui.small_button("Use numeric local frame").clicked() {
                                        *anchor = Default::default();
                                        self.fields.reset();
                                        changed = true;
                                    }
                                }
                                if let JointAnchor::Frame { frame } = anchor {
                                    self.fields.frame(
                                        ui,
                                        document,
                                        &format!("anchor/{side:?}"),
                                        frame,
                                    );
                                }
                            },
                        );
                    }
                    retain_references(request);
                }
                self.fields.values(
                    ui,
                    document,
                    "joint-value",
                    request.parameters.kind,
                    &mut request.parameters.values,
                );
                if self.tool != AssemblyTool::Motion {
                    self.fields.limits(
                        ui,
                        document,
                        request.parameters.kind,
                        &mut request.parameters.limits,
                    );
                } else {
                    ui.small(format!("Limits: {:?}", request.parameters.limits));
                }
            }
            Operation::Pose(request) => {
                ui.label("Pose name");
                changed |= ui.text_edit_singleline(&mut request.name).changed();
                ui.small("Store explicit joint coordinates as one named pose. Saving does not move the assembly.");
                ui.horizontal_wrapped(|ui| {
                    for name in ["Closed", "Half-open", "Open"] {
                        if ui.small_button(name).clicked() {
                            request.name = name.into();
                            changed = true;
                        }
                    }
                });
                if ui.button("Capture current joint values").clicked() {
                    request.values = joint_values(document);
                    self.fields.reset();
                    changed = true;
                }
                for value in &mut request.values {
                    if let Ok((source, _)) = model::read_joint(document, value.joint) {
                        controls::inspector_collapsing(
                            ui,
                            value.joint.to_string(),
                            &widgets::name(document, value.joint),
                            |ui| {
                                self.fields.values(
                                    ui,
                                    document,
                                    &format!("pose/{}", value.joint),
                                    source.parameters.kind,
                                    &mut value.values,
                                );
                            },
                        );
                    }
                }
                if !self.editing {
                    let poses = choices(
                        document,
                        |d| matches!(d,EntityData::Extension{definition,..}if definition.type_id==model::NAMED_POSE_TYPE),
                    );
                    widgets::choose(
                        ui,
                        "saved-pose",
                        "Saved poses",
                        &mut self.pose,
                        &poses,
                        None,
                    );
                    if let Some(pose) = self.pose
                        && ui.button("Apply saved pose").clicked()
                    {
                        self.operation = Operation::ApplyPose(pose);
                        changed = true;
                    }
                }
            }
            Operation::ApplyPose(id) => {
                ui.label(format!("Restore {}", widgets::name(document, *id)));
                ui.small(
                    "Preview all stored coordinates, then Apply them in one undoable transaction.",
                );
            }
            Operation::ReadOnly => {}
        }
        changed |= self.fields.changed;
        if changed {
            self.changed();
        }
        changed
    }
    pub fn anchor_pick(&self) -> Option<AnchorPick> {
        let Operation::Joint(request) = &self.operation else {
            return None;
        };
        let side = self.pick?;
        let instance = match side {
            AnchorSide::Moving => Some(request.owner),
            AnchorSide::Target => request.parameters.parent,
        }?;
        (!instance.is_nil()).then_some(AnchorPick { side, instance })
    }
}
fn retain_references(request: &mut JointRequest) {
    let wanted = [&request.parameters.moving, &request.parameters.target]
        .into_iter()
        .filter_map(|a| {
            if let JointAnchor::Geometry { reference } = a {
                Some(*reference)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    request.references.retain(|r| wanted.contains(&r.intent.id));
}
fn direction(ui: &mut egui::Ui, label: &str, axis: &mut [f64; 3], changed: &mut bool) {
    controls::inspector_collapsing(ui, label, label, |ui| {
        for (label, v) in ["X", "Y", "Z"].iter().zip(axis.iter_mut()) {
            ui.horizontal(|ui| {
                ui.label(*label);
                *changed |= ui.add(egui::DragValue::new(v).speed(0.01)).changed();
            });
        }
    });
}
fn choices(document: &Document, accept: impl Fn(&EntityData) -> bool) -> Vec<(EntityId, String)> {
    document
        .entities()
        .filter(|e| accept(&e.data))
        .map(|e| (e.id, e.name.clone()))
        .collect()
}
fn joint_values(document: &Document) -> Vec<JointPoseValue> {
    document
        .entities()
        .filter_map(|e| {
            model::read_joint(document, e.id)
                .ok()
                .map(|(s, _)| JointPoseValue {
                    joint: e.id,
                    values: s.parameters.values,
                })
        })
        .collect()
}

fn missing() -> EntityId {
    EntityId::parse("00000000-0000-0000-0000-000000000000").expect("nil UUID")
}

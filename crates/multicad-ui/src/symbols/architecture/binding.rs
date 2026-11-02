//! A graphical form holds one stable placement identity map. Source and native
//! preparation still use the shared core contract and existing preview worker.
use super::Fields;
use crate::{controls, egui};
use multicad_core::{
    commands::{Actor, Session},
    document::{Document, EntityId},
    model::{CreatedBody, ModelPrepared, fence_members, length},
    symbols::{self, architecture::*},
};
use std::collections::BTreeMap;
#[derive(Clone, Debug)]
pub enum Command {
    Place(Box<ArchitecturePlaceRequest>),
    Edit {
        request: Box<ArchitectureEditRequest>,
        output: ArchitectureOutput,
    },
}
impl Command {
    pub(crate) fn edit_id(&self) -> Option<EntityId> {
        match self {
            Self::Place(_) => None,
            Self::Edit { output, .. } => Some(output.anchors()[0]),
        }
    }
    pub(crate) fn prepare(&self, session: &Session, actor: Actor) -> Result<ModelPrepared, String> {
        let result = match self {
            Self::Place(request) => {
                prepare_architecture_place(session, session.revision(), actor, request)
            }
            Self::Edit { request, .. } => {
                prepare_architecture_edit(session, session.revision(), actor, request)
            }
        }
        .map_err(|e| e.to_string())?;
        let outputs = match result.output {
            ArchitectureOutput::Wall { feature, body } => vec![CreatedBody { feature, body }],
            ArchitectureOutput::Fence { run, .. } => fence_members(result.prepared.document(), run)
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|(_, feature, body)| CreatedBody { feature, body })
                .collect(),
        };
        Ok(ModelPrepared {
            prepared: result.prepared,
            outputs,
        })
    }
}
pub(crate) struct Binding {
    role: ArchitectureRole,
    output: ArchitectureOutput,
    record: Option<EntityId>,
    definition: Option<EntityId>,
    construction: Option<EntityId>,
    name: String,
    revision: String,
    fields: Option<Fields>,
    placement: Option<ArchitecturePlaceRequest>,
    choices: Vec<(EntityId, String)>,
    pin: Option<multicad_core::project_file::ContentHash>,
    source_stamp: Option<multicad_core::commands::SessionStamp>,
    anchor_source: Option<multicad_core::document::Entity>,
    error: Option<String>,
    snapshot: Option<symbols::SymbolDefinition>,
    preview_map: BTreeMap<EntityId, EntityId>,
}
impl Binding {
    pub(crate) fn new(document: &Document, output: ArchitectureOutput) -> Result<Self, String> {
        let role = output.role();
        let choices = symbols::definitions(document)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter_map(|(id, definition)| {
                blueprint_for_definition(&definition)
                    .ok()
                    .flatten()
                    .filter(|b| b.role == role)
                    .map(|_| (id, definition.name))
            })
            .collect();
        Ok(Self {
            role,
            output,
            record: None,
            definition: None,
            construction: None,
            name: String::new(),
            revision: String::new(),
            fields: None,
            placement: None,
            choices,
            pin: None,
            source_stamp: None,
            anchor_source: None,
            error: None,
            snapshot: None,
            preview_map: BTreeMap::new(),
        })
    }
    pub(crate) fn existing(document: &Document, output: EntityId) -> Result<Option<Self>, String> {
        let Some((record, value)) =
            architecture_instance_for_output(document, output).map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };
        let definition =
            symbols::read_definition(document, value.definition).map_err(|e| e.to_string())?;
        let mut binding = Self::new(document, value.output.clone())?;
        binding.record = Some(record);
        binding.definition = Some(value.definition);
        let blueprint = blueprint_for_definition(&definition)
            .map_err(|e| e.to_string())?
            .ok_or("Architectural interface is missing")?;
        binding.construction = Some(value.ids[&blueprint.construction]);
        binding.name = definition.name.clone();
        binding.revision = definition.revision.clone();
        binding.pin = Some(value.pin);
        binding.preview_map = value.ids.keys().map(|id| (*id, EntityId::new())).collect();
        binding.snapshot = Some(definition.clone());
        binding.anchor_source = document.get(value.output.anchors()[0]).cloned();
        binding.fields = Some(Fields::new(&definition, &value.inputs, document, None)?);
        Ok(Some(binding))
    }
    pub(crate) fn active(&self) -> bool {
        self.definition.is_some()
    }
    pub(crate) fn construction(&self) -> Option<EntityId> {
        self.construction
    }
    pub(crate) fn snapshot(&self) -> Option<&symbols::SymbolDefinition> {
        self.snapshot.as_ref()
    }
    pub(crate) fn first(&self) -> Option<EntityId> {
        self.choices.first().map(|(id, _)| *id)
    }
    pub(crate) fn select(
        &mut self,
        document: &Document,
        id: Option<EntityId>,
    ) -> Result<(), String> {
        if self.record.is_some() {
            return Err("Use explicit definition update to change a pinned construction".into());
        }
        self.error = None;
        self.pin = None;
        self.construction = None;
        let Some(id) = id else {
            self.definition = None;
            self.fields = None;
            self.placement = None;
            return Ok(());
        };
        let definition = symbols::read_definition(document, id).map_err(|e| e.to_string())?;
        let context = match self.role {
            ArchitectureRole::Wall => ArchitectureContext::Wall {
                path: vec![],
                closed: false,
                level: None,
                base_z: length(0.),
                flipped: false,
                details: None,
            },
            ArchitectureRole::Fence => ArchitectureContext::Fence {
                path: multicad_core::model::FencePath {
                    vertices: vec![],
                    closed: false,
                    base_z: length(0.),
                },
                level: None,
                post_positions: vec![],
            },
        };
        let mut placement = ArchitecturePlaceRequest::new(
            document,
            id,
            &definition.name,
            ArchitectureInputs::default(),
            context,
        )
        .map_err(|e| e.to_string())?;
        placement.output = self.output.clone();
        let blueprint = blueprint_for_definition(&definition)
            .map_err(|e| e.to_string())?
            .ok_or("Architectural interface is missing")?;
        self.construction = Some(placement.ids[&blueprint.construction]);
        self.snapshot = Some(definition.clone());
        self.preview_map = placement
            .ids
            .keys()
            .map(|id| (*id, EntityId::new()))
            .collect();
        self.fields = Some(Fields::new(
            &definition,
            &ArchitectureInputs::default(),
            document,
            None,
        )?);
        self.pin = Some(definition.pin().map_err(|e| e.to_string())?);
        self.name = definition.name;
        self.revision = definition.revision;
        self.definition = Some(id);
        self.placement = Some(placement);
        Ok(())
    }
    pub(crate) fn sync_source(&mut self, session: &Session) {
        if self.source_stamp == Some(session.stamp()) {
            return;
        }
        if self.source_stamp.is_some() && self.active() {
            self.error = Some("The document changed while this construction was being edited; reopen the editor before applying".into());
        }
        self.source_stamp = Some(session.stamp());
        if let Some(id) = self.definition {
            let current = symbols::read_definition(session.document(), id).and_then(|d| d.pin());
            if current.ok() != self.pin
                || self
                    .anchor_source
                    .as_ref()
                    .is_some_and(|old| session.document().get(old.id) != Some(old))
            {
                self.error = Some("This pinned construction or placement changed; reopen its editor before applying".into());
            }
        }
    }
    pub(crate) fn show_choice(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
    ) -> Result<bool, String> {
        if self.record.is_some() {
            ui.label(format!("{} · revision {}", self.name, self.revision));
            ui.small("Construction settings come from the embedded pin. Use Compare update for a newer revision.");
            return Ok(false);
        }
        let mut selected = self.definition;
        ui.label("Construction source");
        egui::ComboBox::from_id_salt("architectural-definition")
            .width(controls::inspector_width(ui))
            .selected_text(if self.active() {
                self.name.as_str()
            } else {
                "Local construction"
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut selected, None, "Local construction");
                for (id, name) in &self.choices {
                    ui.selectable_value(&mut selected, Some(*id), name);
                }
            });
        if selected != self.definition {
            self.select(document, selected)?;
            return Ok(true);
        }
        Ok(false)
    }
    pub(crate) fn show_fields(&mut self, ui: &mut egui::Ui) -> bool {
        self.fields.as_mut().is_some_and(|fields| fields.show(ui))
    }
    pub(crate) fn command(
        &self,
        document: &Document,
        name: &str,
        context: ArchitectureContext,
    ) -> Result<Command, String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        if context.role() != self.role {
            return Err("Architectural path context has the wrong role".into());
        }
        let inputs = self
            .fields
            .as_ref()
            .ok_or("Choose a reusable construction")?
            .resolve(document)?;
        if let Some(record) = self.record {
            Ok(Command::Edit {
                request: Box::new(ArchitectureEditRequest {
                    record,
                    inputs,
                    context: Some(context),
                }),
                output: self.output.clone(),
            })
        } else {
            let mut request = self
                .placement
                .clone()
                .ok_or("Choose a reusable construction")?;
            request.name = name.into();
            request.inputs = inputs;
            request.context = context;
            Ok(Command::Place(Box::new(request)))
        }
    }
}

pub(crate) struct FencePlan {
    pub layout: Result<multicad_core::model::FenceLayout, String>,
    pub automatic: Result<multicad_core::model::FencePostPlan, String>,
}
impl Binding {
    pub(crate) fn fence_plan(
        &self,
        document: &Document,
        context: ArchitectureContext,
    ) -> Result<FencePlan, String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let ArchitectureContext::Fence {
            path,
            level,
            post_positions,
        } = context
        else {
            return Err("Choose a fence path context".into());
        };
        let definition = self.snapshot.as_ref().ok_or("Choose a fence definition")?;
        let blueprint = blueprint_for_definition(definition)
            .map_err(|e| e.to_string())?
            .ok_or("Fence interface is missing")?;
        let inputs = self
            .fields
            .as_ref()
            .ok_or("Fence inputs are unavailable")?
            .resolve(document)?;
        let graph = configured_source(definition, &self.preview_map, &inputs, document)
            .map_err(|e| e.to_string())?;
        let stage = document
            .edited(
                &graph
                    .into_values()
                    .map(multicad_core::document::Mutation::Insert)
                    .collect::<Vec<_>>(),
            )
            .map_err(|e| e.to_string())?;
        let parameters = multicad_core::parameters::evaluate(&stage).map_err(|e| e.to_string())?;
        let construction = multicad_core::model::read_fence_type(
            &stage,
            self.preview_map[&blueprint.construction],
        )
        .map_err(|e| e.to_string())?;
        let mut source = multicad_core::model::FenceRunDefinition::new(
            self.preview_map[&blueprint.construction],
            path,
        );
        source.level = level;
        source.post_positions = post_positions;
        let layout =
            multicad_core::model::plan_fence_instance(&stage, &parameters, &construction, &source)
                .map_err(|e| e.to_string());
        source.post_positions.clear();
        let automatic = multicad_core::model::plan_fence_instance_posts(
            &stage,
            &parameters,
            &construction,
            &source,
        )
        .map_err(|e| e.to_string());
        Ok(FencePlan { layout, automatic })
    }
}

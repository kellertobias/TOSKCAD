use super::{
    SymbolSelection, SymbolTool,
    fields::{Overrides, PoseInputs},
};
use multicad_core::{
    commands::{Actor, Session},
    document::{Document, EntityData, EntityId},
    symbols::{
        self, SymbolAuthorRequest, SymbolDefinition, SymbolEditRequest, SymbolKind,
        SymbolPlaceRequest, SymbolPrepared, SymbolUpdateRequest, SymbolUpgradeReport,
    },
};
use std::collections::BTreeMap;
pub(super) enum Operation {
    Library,
    ChooseEdit {
        selected: Option<EntityId>,
    },
    Author(SymbolAuthorRequest),
    ArchitecturalAuthor {
        request: symbols::architecture::ArchitectureAuthorRequest,
        editor: super::architecture::author::Interface,
    },
    Import(SymbolDefinition),
    Place {
        definition: Option<EntityId>,
        name: String,
        request: Option<Box<SymbolPlaceRequest>>,
        values: Overrides,
        pose: PoseInputs,
    },
    ArchitecturalEdit {
        record: EntityId,
        output: symbols::architecture::ArchitectureOutput,
        definition: EntityId,
        values: super::architecture::Fields,
    },
    Edit {
        request: SymbolEditRequest,
        definition: SymbolDefinition,
        values: Overrides,
    },
    Update {
        definition: Option<EntityId>,
        replacement: Option<SymbolDefinition>,
        request: Option<Box<SymbolUpdateRequest>>,
        values: BTreeMap<EntityId, Overrides>,
        architecture_values: BTreeMap<EntityId, super::architecture::Fields>,
        report: Option<SymbolUpgradeReport>,
    },
}
pub(super) struct Form {
    pub tool: SymbolTool,
    pub operation: Operation,
    pub version: u64,
    pub filter: String,
}
impl Form {
    pub fn new(
        tool: SymbolTool,
        document: &Document,
        selection: SymbolSelection,
    ) -> Result<Self, String> {
        let operation = match tool {
            SymbolTool::Library => Operation::Library,
            SymbolTool::Create => {
                let root = selection
                    .component
                    .and_then(|id| document.get(id))
                    .and_then(|e| match e.data {
                        EntityData::Part { .. } | EntityData::Assembly { .. } => Some(e.id),
                        EntityData::Instance { component, .. } => Some(component),
                        _ => None,
                    })
                    .unwrap_or_default();
                let exposed = document
                    .entities()
                    .filter_map(|e| {
                        matches!(e.data, EntityData::Parameter { exposed: true, .. })
                            .then_some(e.id)
                    })
                    .collect();
                let name = document.get(root).map_or("New symbol", |e| e.name.as_str());
                Operation::Author(SymbolAuthorRequest::new(
                    name,
                    root,
                    SymbolKind::Dynamic,
                    exposed,
                    selection
                        .plan_sketch
                        .map(|sketch| symbols::SymbolPlan { sketch }),
                ))
            }
            SymbolTool::Place => Operation::Place {
                definition: None,
                name: "Placed symbol".into(),
                request: None,
                values: Overrides::default(),
                pose: PoseInputs::default(),
            },
            SymbolTool::Edit => {
                if let Some(instance) = selection.instance {
                    return Self::existing(document, instance);
                } else {
                    Operation::ChooseEdit { selected: None }
                }
            }
            SymbolTool::Update => {
                let definition = selection.instance.and_then(|id| {
                    symbols::read_instance(document, id)
                        .ok()
                        .map(|i| i.definition)
                        .or_else(|| {
                            symbols::architecture::architecture_instance_for_output(document, id)
                                .ok()
                                .flatten()
                                .map(|(_, i)| i.definition)
                        })
                });
                Operation::Update {
                    definition,
                    replacement: None,
                    request: None,
                    values: BTreeMap::new(),
                    architecture_values: BTreeMap::new(),
                    report: None,
                }
            }
        };
        let mut form = Self {
            tool,
            operation,
            version: 0,
            filter: String::new(),
        };
        if tool == SymbolTool::Create {
            let constructor = selection.component.and_then(|id| {
                if let Some(EntityData::Part { features, .. }) = document.get(id).map(|e| &e.data) {
                    for feature in features {
                        if let Ok(wall) = multicad_core::model::read_wall(document, *feature) {
                            return Some(wall.wall_type);
                        }
                        if let Ok(member) =
                            multicad_core::model::read_fence_member(document, *feature)
                        {
                            return multicad_core::model::read_fence_run(document, member.run)
                                .ok()
                                .map(|run| run.fence_type);
                        }
                    }
                }
                super::architecture::author::construction_role(document, id).map(|_| id)
            });
            if let Some(construction) = constructor
                && !symbols::owned_entities(document)
                    .map_err(|e| e.to_string())?
                    .contains(&construction)
                && super::architecture::author::construction_role(document, construction).is_some()
            {
                form.author_construction(document, construction)?;
            }
        }
        Ok(form)
    }
    pub fn author_construction(
        &mut self,
        document: &Document,
        construction: EntityId,
    ) -> Result<(), String> {
        let (mut editor, mut interface) =
            super::architecture::author::Interface::new(document, construction)?;
        let mut root = EntityId::new();
        for entity in document.entities() {
            if let Ok(stored) = symbols::architecture::read_blueprint(document, entity.id)
                && stored.construction == construction
                && stored.role == interface.role
            {
                root = entity.id;
                interface = stored;
                editor.restore(&interface);
                break;
            }
        }
        let dependencies = interface
            .dependencies(document)
            .map_err(|e| e.to_string())?;
        let exposed = document
            .entities()
            .filter(|e| {
                dependencies.contains(&e.id)
                    && matches!(e.data, EntityData::Parameter { exposed: true, .. })
            })
            .map(|e| e.id)
            .collect();
        let name = document
            .get(construction)
            .map_or("Architectural construction", |e| e.name.as_str());
        let mut symbol = SymbolAuthorRequest::new(name, root, SymbolKind::Dynamic, exposed, None);
        let definitions = symbols::definitions(document).map_err(|e| e.to_string())?;
        if let Some((_, prior)) = definitions
            .iter()
            .find(|(_, d)| d.root == root && d.template.id() == document.id())
        {
            symbol.symbol = prior.symbol;
            symbol.exposed = prior
                .exposed
                .iter()
                .filter(|id| dependencies.contains(id))
                .copied()
                .collect();
            let mut next = definitions
                .iter()
                .filter(|(_, d)| d.symbol == prior.symbol)
                .count()
                + 1;
            while definitions
                .iter()
                .any(|(_, d)| d.symbol == prior.symbol && d.revision == next.to_string())
            {
                next += 1;
            }
            symbol.revision = next.to_string();
            symbol.name = document
                .get(root)
                .map_or_else(|| name.to_string(), |e| e.name.clone());
        }
        self.tool = SymbolTool::Create;
        self.operation = Operation::ArchitecturalAuthor {
            request: symbols::architecture::ArchitectureAuthorRequest {
                blueprint: root,
                interface,
                symbol,
            },
            editor,
        };
        self.changed();
        Ok(())
    }
    pub fn existing(document: &Document, id: EntityId) -> Result<Self, String> {
        if let Some((record, instance)) =
            symbols::architecture::architecture_instance_for_output(document, id)
                .map_err(|e| e.to_string())?
        {
            let definition = symbols::read_definition(document, instance.definition)
                .map_err(|e| e.to_string())?;
            return Ok(Self {
                tool: SymbolTool::Edit,
                operation: Operation::ArchitecturalEdit {
                    record,
                    output: instance.output,
                    definition: instance.definition,
                    values: super::architecture::Fields::new(
                        &definition,
                        &instance.inputs,
                        document,
                        None,
                    )?,
                },
                version: 0,
                filter: String::new(),
            });
        }
        let instance = symbols::read_instance(document, id).map_err(|e| e.to_string())?;
        let record = id;
        let definition =
            symbols::read_definition(document, instance.definition).map_err(|e| e.to_string())?;
        let values = Overrides::new(&definition, &instance.overrides)?;
        Ok(Self {
            tool: SymbolTool::Edit,
            operation: Operation::Edit {
                request: SymbolEditRequest {
                    record,
                    overrides: instance.overrides,
                },
                definition,
                values,
            },
            version: 0,
            filter: String::new(),
        })
    }
    pub fn changed(&mut self) {
        self.version = self.version.saturating_add(1);
    }
    pub fn authoring(&self) -> bool {
        !matches!(self.operation, Operation::Library)
    }
    pub fn place(&mut self, document: &Document, id: EntityId) -> Result<(), String> {
        let definition = symbols::read_definition(document, id).map_err(|e| e.to_string())?;
        self.tool = SymbolTool::Place;
        self.operation = Operation::Place {
            definition: Some(id),
            name: definition.name.clone(),
            request: None,
            values: Overrides::new(&definition, &BTreeMap::new())?,
            pose: PoseInputs::default(),
        };
        self.changed();
        Ok(())
    }
    pub fn import(&mut self, definition: SymbolDefinition) {
        self.tool = SymbolTool::Library;
        self.operation = Operation::Import(definition);
        self.changed();
    }
    pub fn set_replacement(
        &mut self,
        document: &Document,
        replacement: SymbolDefinition,
    ) -> Result<(), String> {
        let Operation::Update {
            definition: Some(id),
            replacement: stored,
            request,
            values,
            architecture_values,
            report,
        } = &mut self.operation
        else {
            return Err("Choose the embedded definition to update first".into());
        };
        let comparison =
            symbols::compare_update(document, *id, &replacement).map_err(|e| e.to_string())?;
        let prepared = SymbolUpdateRequest::new(document, *id, replacement.clone())
            .map_err(|e| e.to_string())?;
        let inputs = prepared
            .instance_overrides
            .iter()
            .map(|(id, overrides)| {
                Overrides::new(&replacement, overrides).map(|fields| (*id, fields))
            })
            .collect::<Result<_, _>>()?;
        let old = symbols::read_definition(document, *id).map_err(|e| e.to_string())?;
        let architecture = prepared
            .architecture_inputs
            .iter()
            .map(|(id, inputs)| {
                super::architecture::Fields::new(&replacement, inputs, document, Some(&old))
                    .map(|fields| (*id, fields))
            })
            .collect::<Result<_, _>>()?;
        *architecture_values = architecture;
        *stored = Some(replacement);
        *request = Some(Box::new(prepared));
        *values = inputs;
        *report = Some(comparison);
        self.changed();
        Ok(())
    }
    pub fn prepare(&mut self, session: &Session, actor: Actor) -> Result<SymbolPrepared, String> {
        let revision = session.revision();
        let prepared = match &mut self.operation {
            Operation::ChooseEdit { .. } => return Err("Choose a symbol instance to edit".into()),
            Operation::Library => {
                return Err("Choose an embedded symbol to place, or import a definition".into());
            }
            Operation::Author(request) => {
                symbols::prepare_author(session, revision, actor, request)
            }
            Operation::ArchitecturalAuthor { request, editor } => {
                editor.write(&mut request.interface);
                symbols::architecture::prepare_architecture_author(
                    session, revision, actor, request,
                )
            }
            Operation::Import(definition) => {
                symbols::prepare_import(session, revision, actor, definition)
            }
            Operation::Place {
                definition,
                name,
                request,
                values,
                pose,
            } => {
                let id = definition.ok_or("Choose an embedded symbol definition")?;
                let source =
                    symbols::read_definition(session.document(), id).map_err(|e| e.to_string())?;
                let overrides = values.resolve(&source)?;
                let placement = pose.resolve()?;
                if let Some(request) = request {
                    request.name = name.clone();
                    request.overrides = overrides;
                    request.placement = placement;
                } else {
                    *request = Some(Box::new(
                        SymbolPlaceRequest::new(
                            session.document(),
                            id,
                            name.clone(),
                            overrides,
                            placement,
                        )
                        .map_err(|e| e.to_string())?,
                    ));
                }
                symbols::prepare_place(
                    session,
                    revision,
                    actor,
                    request.as_ref().expect("allocated placement"),
                )
            }
            Operation::ArchitecturalEdit {
                record,
                output,
                definition,
                values,
            } => {
                let result = symbols::architecture::prepare_architecture_edit(
                    session,
                    revision,
                    actor,
                    &symbols::architecture::ArchitectureEditRequest {
                        record: *record,
                        inputs: values.resolve(session.document())?,
                        context: None,
                    },
                );
                result.map(|result| SymbolPrepared {
                    prepared: result.prepared,
                    definition: *definition,
                    instances: vec![output.anchors()[1]],
                    plans: BTreeMap::new(),
                })
            }
            Operation::Edit {
                request,
                definition,
                values,
            } => {
                request.overrides = values.resolve(definition)?;
                symbols::prepare_edit(session, revision, actor, request)
            }
            Operation::Update {
                request,
                replacement,
                values,
                architecture_values,
                report,
                ..
            } => {
                let request = request
                    .as_mut()
                    .ok_or("Choose an incoming revision and review its changes")?;
                let replacement = replacement.as_ref().ok_or("Choose an incoming revision")?;
                request.instance_overrides = values
                    .iter()
                    .map(|(id, value)| value.resolve(replacement).map(|v| (*id, v)))
                    .collect::<Result<_, _>>()?;
                request.architecture_inputs = architecture_values
                    .iter()
                    .map(|(id, fields)| {
                        fields
                            .resolve(session.document())
                            .map(|inputs| (*id, inputs))
                    })
                    .collect::<Result<_, _>>()?;
                *report = Some(
                    symbols::compare_update_request(session.document(), request)
                        .map_err(|e| e.to_string())?,
                );
                symbols::prepare_update(session, revision, actor, request)
            }
        };
        prepared.map_err(|e| e.to_string())
    }
}

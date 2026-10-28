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
    Import(SymbolDefinition),
    Place {
        definition: Option<EntityId>,
        name: String,
        request: Option<Box<SymbolPlaceRequest>>,
        values: Overrides,
        pose: PoseInputs,
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
                });
                Operation::Update {
                    definition,
                    replacement: None,
                    request: None,
                    values: BTreeMap::new(),
                    report: None,
                }
            }
        };
        Ok(Self {
            tool,
            operation,
            version: 0,
            filter: String::new(),
        })
    }
    pub fn existing(document: &Document, id: EntityId) -> Result<Self, String> {
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
                symbols::prepare_update(session, revision, actor, request)
            }
        };
        prepared.map_err(|e| e.to_string())
    }
}

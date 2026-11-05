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
        library: Option<SymbolDefinition>,
        parent: Option<EntityId>,
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
    pub parent: Option<EntityId>,
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
                library: None,
                parent: selection.parent,
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
            parent: selection.parent,
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
                parent: None,
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
            parent: None,
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
            library: None,
            parent: self.parent,
            name: definition.name.clone(),
            request: None,
            values: Overrides::new(&definition, &BTreeMap::new())?,
            pose: PoseInputs::default(),
        };
        self.changed();
        Ok(())
    }
    pub fn place_library(
        &mut self,
        document: &Document,
        definition: &SymbolDefinition,
    ) -> Result<(), String> {
        let id = symbols::library_placement_definition(document, definition)
            .map_err(|e| e.to_string())?
            .unwrap_or_else(EntityId::new);
        // Validate generic-placement eligibility before replacing the current form.
        let request = SymbolPlaceRequest::from_definition(
            id,
            definition,
            definition.name.clone(),
            BTreeMap::new(),
            Default::default(),
        )
        .map_err(|e| e.to_string())?;
        let values = Overrides::new(definition, &BTreeMap::new())?;
        self.tool = SymbolTool::Place;
        self.operation = Operation::Place {
            definition: Some(id),
            library: Some(definition.clone()),
            parent: self.parent,
            name: definition.name.clone(),
            request: Some(Box::new(request)),
            values,
            pose: PoseInputs::default(),
        };
        self.changed();
        Ok(())
    }
    pub(super) fn placement_pose_mut(&mut self) -> Option<&mut PoseInputs> {
        match &mut self.operation {
            Operation::Place { pose, .. } => Some(pose),
            _ => None,
        }
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
                library,
                parent,
                name,
                request,
                values,
                pose,
            } => {
                let id = definition.ok_or("Choose a symbol definition")?;
                let source = match library.as_ref() {
                    Some(content) => content.clone(),
                    None => symbols::read_definition(session.document(), id)
                        .map_err(|e| e.to_string())?,
                };
                let overrides = values.resolve(&source)?;
                let placement = pose.resolve()?;
                if let Some(request) = request {
                    request.name = name.clone();
                    request.overrides = overrides;
                    request.placement = placement;
                } else {
                    *request = Some(Box::new(
                        SymbolPlaceRequest::from_definition(
                            id,
                            &source,
                            name.clone(),
                            overrides,
                            placement,
                        )
                        .map_err(|e| e.to_string())?,
                    ));
                }
                let request = request.as_ref().expect("allocated placement");
                match library.as_ref() {
                    Some(content) => symbols::prepare_library_place(
                        session, revision, actor, content, request, *parent,
                    ),
                    None => symbols::prepare_place_in(session, revision, actor, request, *parent),
                }
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

#[cfg(test)]
mod placement_tests {
    use super::*;
    use multicad_core::{
        commands::{Channel, Request},
        document::{Entity, Mutation},
        model::{self, PrimitiveParameters, PrimitiveRequest, length},
        parameters::BoundExpression,
    };
    fn actor() -> Actor {
        Actor::local(Channel::Ui)
    }
    fn definition() -> (SymbolDefinition, EntityId) {
        let parameter = Entity::new(
            "width",
            EntityData::Parameter {
                expression: "10 mm".into(),
                unit: "mm".into(),
                exposed: false,
                inputs: vec![],
            },
        );
        let width = parameter.id;
        let document = Document::new("Author")
            .unwrap()
            .edited(&[Mutation::Insert(parameter)])
            .unwrap();
        let mut author = Session::new(document).unwrap();
        let part = PrimitiveRequest::new(
            "Source box",
            PrimitiveParameters::Box {
                size: [
                    multicad_core::constraint_sketch::Dimension::Expression {
                        expression: BoundExpression::bind(author.document(), "=$width", "mm", None)
                            .unwrap(),
                    },
                    length(20.),
                    length(30.),
                ],
                frame: Default::default(),
            },
        );
        let prepared = model::prepare_primitive(&author, 0, actor(), &part).unwrap();
        author.commit(prepared.prepared).unwrap();
        let request = SymbolAuthorRequest::new(
            "Reusable box",
            part.body,
            SymbolKind::Dynamic,
            vec![width],
            None,
        );
        let prepared =
            symbols::prepare_author(&author, author.revision(), actor(), &request).unwrap();
        author.commit(prepared.prepared).unwrap();
        (
            symbols::read_definition(author.document(), request.definition).unwrap(),
            width,
        )
    }
    #[test]
    fn detached_library_placement_keeps_ids_through_pose_and_override_edits_and_commits_once_in_parent()
     {
        let (definition, width) = definition();
        let parent = Entity::new("Assembly", EntityData::Assembly { instances: vec![] });
        let parent_id = parent.id;
        let document = Document::new("Receiver")
            .unwrap()
            .edited(&[Mutation::Insert(parent)])
            .unwrap();
        let mut session = Session::new(document.clone()).unwrap();
        let mut form = Form::new(
            SymbolTool::Library,
            session.document(),
            SymbolSelection {
                parent: Some(parent_id),
                ..Default::default()
            },
        )
        .unwrap();
        form.place_library(session.document(), &definition).unwrap();
        let first = form.prepare(&session, actor()).unwrap();
        let original = match &form.operation {
            Operation::Place {
                request: Some(request),
                ..
            } => (**request).clone(),
            _ => panic!("placement"),
        };
        assert_eq!(session.document(), &document);
        assert!(symbols::definitions(session.document()).unwrap().is_empty());
        if let Operation::Place { values, pose, .. } = &mut form.operation {
            values.0.get_mut(&width).unwrap().enabled = true;
            values.0.get_mut(&width).unwrap().text = "14".into();
            pose.translation[0] = "25".into();
        }
        form.changed();
        let next = form.prepare(&session, actor()).unwrap();
        let updated = match &form.operation {
            Operation::Place {
                request: Some(request),
                ..
            } => (**request).clone(),
            _ => panic!("placement"),
        };
        assert_eq!(first.definition, next.definition);
        assert_eq!(first.instances, next.instances);
        assert_eq!(original.ids, updated.ids);
        assert_eq!(original.record, updated.record);
        assert_eq!(original.wrapper, updated.wrapper);
        assert_eq!(original.member, updated.member);
        assert_eq!(updated.placement.translation_mm, [25., 0., 0.]);
        assert_eq!(updated.overrides[&width].value, 14.);
        session.commit(next.prepared).unwrap();
        assert_eq!(session.revision(), 1);
        assert_eq!(
            symbols::read_definition(session.document(), updated.definition).unwrap(),
            definition
        );
        let placed = symbols::read_instance(session.document(), updated.record).unwrap();
        assert_eq!(placed.overrides[&width].value, 14.);
        assert_eq!(
            multicad_core::hierarchy::Index::new(session.document())
                .unwrap()
                .parent(updated.instance)
                .unwrap(),
            Some(parent_id)
        );
        session.undo(1, actor()).unwrap();
        assert_eq!(session.document(), &document);
        assert!(!session.can_undo());
        session.redo(2, actor()).unwrap();
        assert!(symbols::read_instance(session.document(), updated.record).is_ok());
    }
    #[test]
    fn detached_place_cancel_is_neutral_and_exact_embedded_pin_reuses_definition() {
        let (definition, _) = definition();
        let mut session = Session::new(Document::new("Receiver").unwrap()).unwrap();
        let before = session.document().clone();
        let mut form =
            Form::new(SymbolTool::Library, session.document(), Default::default()).unwrap();
        form.place_library(session.document(), &definition).unwrap();
        let prepared = form.prepare(&session, actor()).unwrap();
        drop(prepared);
        drop(form);
        assert_eq!(session.document(), &before);
        assert_eq!(session.revision(), 0);
        assert!(!session.can_undo());
        session
            .commit(
                symbols::prepare_import(&session, 0, actor(), &definition)
                    .unwrap()
                    .prepared,
            )
            .unwrap();
        let embedded = symbols::definitions(session.document()).unwrap()[0].0;
        let mut form =
            Form::new(SymbolTool::Library, session.document(), Default::default()).unwrap();
        form.place_library(session.document(), &definition).unwrap();
        let prepared = form.prepare(&session, actor()).unwrap();
        assert_eq!(prepared.definition, embedded);
        session.commit(prepared.prepared).unwrap();
        assert_eq!(symbols::definitions(session.document()).unwrap().len(), 1);
        // A stale review cannot be committed after another owner edit.
        form.place_library(session.document(), &definition).unwrap();
        let stale = form.prepare(&session, actor()).unwrap();
        let edit = session
            .prepare(Request {
                expected_revision: session.revision(),
                actor: actor(),
                label: "Owner edit".into(),
                edits: vec![Mutation::RenameDocument("Changed receiver".into())],
            })
            .unwrap();
        session.commit(edit).unwrap();
        let changed = session.document().clone();
        assert!(session.commit(stale.prepared).is_err());
        assert_eq!(session.document(), &changed);
    }
}

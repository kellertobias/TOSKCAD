use super::{request::driver_chain, *};
use crate::{
    commands::{Actor, CommandError, Request, Session},
    document::{Entity, EntityData, EntityId, Mutation, Placement},
    hierarchy,
    model::{
        self, BodyOutput, PlacementBase, PlacementBatchPrepared, RigidPose,
        boolean::available,
        common::{error, revision},
        placement::Driver,
    },
};
use std::collections::{BTreeMap, BTreeSet};

fn fresh(
    document: &crate::document::Document,
    id: EntityId,
    ids: &mut BTreeSet<EntityId>,
) -> Result<(), CommandError> {
    if id.is_nil() || id == document.id() || document.get(id).is_some() || !ids.insert(id) {
        return Err(error(
            "Scale outputs require distinct fresh non-nil identities",
        ));
    }
    Ok(())
}
fn feature(id: EntityId, source: &ScaleDefinition) -> Result<Entity, CommandError> {
    let mut entity = Entity::new(
        "Scale",
        EntityData::Feature {
            definition: source.definition()?,
            inputs: source.inputs()?,
            suppressed: false,
        },
    );
    entity.id = id;
    Ok(entity)
}
fn instance_source(
    document: &crate::document::Document,
    instance: EntityId,
) -> Result<(EntityId, Placement, Option<EntityId>), CommandError> {
    let Some(EntityData::Instance {
        component,
        placement,
        placement_source,
    }) = document.get(instance).map(|e| &e.data)
    else {
        return Err(error("Select a supported Part Instance to scale"));
    };
    if !matches!(
        document.get(*component).map(|e| &e.data),
        Some(EntityData::Part { .. })
    ) {
        return Err(error(
            "Scale currently supports Part occurrences; subassemblies require an explicit assembly operation",
        ));
    }
    Ok((*component, *placement, *placement_source))
}
fn base(placement: Placement, driver: Option<EntityId>) -> Result<PlacementBase, CommandError> {
    Ok(match driver {
        Some(driver) => PlacementBase::Previous { driver },
        None => PlacementBase::Literal {
            pose: RigidPose::from_placement(placement)?,
        },
    })
}
fn clone_drivers(
    document: &crate::document::Document,
    source: EntityId,
    owner: EntityId,
    drivers: &BTreeMap<EntityId, EntityId>,
    references: &BTreeMap<EntityId, EntityId>,
    ids: &mut BTreeSet<EntityId>,
    edits: &mut Vec<Mutation>,
) -> Result<(), CommandError> {
    let chain = driver_chain(document, source)?;
    let expected = chain.iter().map(|(id, _)| *id).collect::<BTreeSet<_>>();
    let captured = chain
        .iter()
        .flat_map(|(_, driver)| match driver {
            Driver::Alignment(_, inputs) => inputs.iter().map(|i| i.intent.id).collect(),
            _ => Vec::new(),
        })
        .collect::<BTreeSet<_>>();
    if expected != drivers.keys().copied().collect()
        || captured != references.keys().copied().collect()
    {
        return Err(error(
            "The copied placement source changed; select it again before scaling",
        ));
    }
    for id in drivers.values().chain(references.values()) {
        fresh(document, *id, ids)?;
    }
    for (id, mut driver) in chain {
        match &mut driver {
            Driver::Numeric(source) => source.owner = owner,
            Driver::Alignment(source, inputs) => {
                source.owner = owner;
                for input in inputs {
                    let old = input.intent.id;
                    let new = references[&old];
                    input.intent.id = new;
                    for anchor in [&mut source.parameters.source, &mut source.parameters.target] {
                        if let model::AlignmentAnchor::Geometry { reference } = anchor
                            && *reference == old
                        {
                            *reference = new;
                        }
                    }
                }
            }
        }
        if let PlacementBase::Previous { driver: previous } = driver.base() {
            driver.set_base(PlacementBase::Previous {
                driver: drivers[&previous],
            });
        }
        edits.push(Mutation::Insert(driver.entity(drivers[&id])?));
    }
    Ok(())
}
/// Source-only preparation. The complete candidate still needs native B-rep
/// validation before an interactive or external client can confirm it.
pub fn prepare_scale_batch(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &ScaleBatchRequest,
) -> Result<PlacementBatchPrepared, CommandError> {
    revision(session, expected_revision)?;
    request.parameters.resolve(session.parameters())?;
    if request.operations.is_empty() || request.operations.len() > model::MAX_TRANSFORM_TARGETS {
        return Err(error("Select 1-128 explicit scale targets"));
    }
    if request
        .operations
        .iter()
        .any(|op| matches!(op, ScaleOperation::Copy { .. }) != request.is_copy())
    {
        return Err(error(
            "Scale and scaled-copy actions must be separate explicit commands",
        ));
    }
    let document = session.document();
    let index = hierarchy::Index::new(document).map_err(|e| error(e.to_string()))?;
    let mut hierarchy = document.hierarchy().clone();
    let mut ids = BTreeSet::new();
    let mut selected = BTreeSet::new();
    let mut part_targets = BTreeSet::new();
    let mut edits = vec![];
    let mut instances = vec![];
    for target in request.targets() {
        if !selected.insert(target.entity()) {
            return Err(error("An explicit scale target cannot appear twice"));
        }
        if !request.is_copy()
            && let ScaleTarget::Part { input } = target
        {
            part_targets.insert(input.body);
        }
    }
    if !request.is_copy() {
        for target in request.targets() {
            if let ScaleTarget::Instance { instance } = target {
                let (component, _, _) = instance_source(document, instance)?;
                if model::scale_component_ancestry(document, component)?
                    .iter()
                    .any(|id| part_targets.contains(id))
                {
                    return Err(error(
                        "Choose a Part definition or its dependent Instance; definition scaling already propagates to linked occurrences",
                    ));
                }
            }
        }
    }
    for operation in &request.operations {
        match operation {
            ScaleOperation::Definition {
                input,
                feature: output,
            } => {
                fresh(document, *output, &mut ids)?;
                available(document, input, true)?;
                let source = ScaleDefinition {
                    input: ScaleInput::Part {
                        source: input.clone(),
                    },
                    output: BodyOutput { body: input.body },
                    parameters: request.parameters.clone(),
                };
                let mut part = document
                    .get(input.body)
                    .ok_or_else(|| error("Scale source Part is missing"))?
                    .clone();
                let EntityData::Part { features, .. } = &mut part.data else {
                    return Err(error("Scale definition target must be a Part"));
                };
                features.push(*output);
                edits.extend([
                    Mutation::Insert(feature(*output, &source)?),
                    Mutation::Replace(part),
                ]);
            }
            ScaleOperation::Occurrence {
                instance,
                component,
                feature: output,
            } => {
                fresh(document, *component, &mut ids)?;
                fresh(document, *output, &mut ids)?;
                let (source_component, placement, driver) = instance_source(document, *instance)?;
                let mut owner = document
                    .get(*instance)
                    .ok_or_else(|| error("Scale target is missing"))?
                    .clone();
                let EntityData::Instance {
                    component: current, ..
                } = &mut owner.data
                else {
                    unreachable!()
                };
                *current = *component;
                let source = ScaleDefinition {
                    input: ScaleInput::Component {
                        component: source_component,
                        owner: *instance,
                        base: base(placement, driver)?,
                    },
                    output: BodyOutput { body: *component },
                    parameters: request.parameters.clone(),
                };
                edits.extend([
                    Mutation::Insert(feature(*output, &source)?),
                    Mutation::Insert(component_part(
                        document,
                        source_component,
                        *component,
                        *output,
                    )?),
                    Mutation::Replace(owner),
                ]);
                instances.push(*instance);
            }
            ScaleOperation::Copy {
                source,
                instance,
                component,
                feature: output,
                drivers,
                references,
            } => {
                for id in [instance, component, output] {
                    fresh(document, *id, &mut ids)?;
                }
                let (source_component, placement, driver) = match source {
                    ScaleTarget::Part { input } => {
                        available(document, input, true)?;
                        if !drivers.is_empty() || !references.is_empty() {
                            return Err(error(
                                "Part copies cannot contain occurrence driver remaps",
                            ));
                        }
                        (input.body, Placement::default(), None)
                    }
                    ScaleTarget::Instance { instance: original } => {
                        let (component, placement, driver) = instance_source(document, *original)?;
                        clone_drivers(
                            document, *original, *instance, drivers, references, &mut ids,
                            &mut edits,
                        )?;
                        (component, placement, driver.map(|id| drivers[&id]))
                    }
                };
                let definition = ScaleDefinition {
                    input: ScaleInput::Component {
                        component: source_component,
                        owner: *instance,
                        base: base(placement, driver)?,
                    },
                    output: BodyOutput { body: *component },
                    parameters: request.parameters.clone(),
                };
                let source_name = document
                    .get(source.entity())
                    .map_or("object", |e| e.name.as_str())
                    .chars()
                    .take(200)
                    .collect::<String>();
                let mut owner = Entity::new(
                    format!("Scaled copy of {source_name}"),
                    EntityData::Instance {
                        component: *component,
                        placement,
                        placement_source: driver,
                    },
                );
                owner.id = *instance;
                edits.extend([
                    Mutation::Insert(feature(*output, &definition)?),
                    Mutation::Insert(component_part(
                        document,
                        source_component,
                        *component,
                        *output,
                    )?),
                    Mutation::Insert(owner),
                ]);
                instances.push(*instance);
                if let Some(parent) = index
                    .parent(source.entity())
                    .map_err(|e| error(e.to_string()))?
                {
                    hierarchy.members.entry(parent).or_default().push(*instance);
                }
            }
        }
    }
    if document
        .entities()
        .filter(|e| matches!(e.data, EntityData::Instance { .. }))
        .count()
        + if request.is_copy() {
            instances.len()
        } else {
            0
        }
        > model::MAX_PLACED_INSTANCES
    {
        return Err(error("Scaled copies exceed the native occurrence budget"));
    }
    if hierarchy != *document.hierarchy() {
        edits.push(Mutation::SetHierarchy(hierarchy));
    }
    let prepared = session.prepare_scoped(Request {
        expected_revision,
        actor,
        label: if request.is_copy() {
            "Create scaled copies"
        } else {
            "Scale selected objects"
        }
        .into(),
        edits,
    })?;
    Ok(PlacementBatchPrepared {
        prepared,
        outputs: request.outputs(),
        instances,
    })
}
fn component_part(
    document: &crate::document::Document,
    source: EntityId,
    body: EntityId,
    feature: EntityId,
) -> Result<Entity, CommandError> {
    let Some(EntityData::Part { material, tags, .. }) = document.get(source).map(|e| &e.data)
    else {
        return Err(error("Owned scale component source must be a Part"));
    };
    let mut part = Entity::new(
        "Scaled occurrence geometry",
        EntityData::Part {
            features: vec![feature],
            material: *material,
            tags: tags.clone(),
        },
    );
    part.id = body;
    Ok(part)
}
pub fn prepare_scale_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature_id: EntityId,
    parameters: &ScaleParameters,
) -> Result<PlacementBatchPrepared, CommandError> {
    revision(session, expected_revision)?;
    parameters.resolve(session.parameters())?;
    let mut source = read_scale(session.document(), feature_id)?;
    source.parameters = parameters.clone();
    let mut feature = session
        .document()
        .get(feature_id)
        .ok_or_else(|| error("Scale feature is missing"))?
        .clone();
    let EntityData::Feature {
        definition,
        inputs,
        suppressed,
    } = &mut feature.data
    else {
        return Err(error("Choose an applied Scale feature"));
    };
    if *suppressed {
        return Err(error("Unsuppress the Scale feature before editing it"));
    }
    *definition = source.definition()?;
    *inputs = source.inputs()?;
    let (outputs, instances) = match source.input {
        ScaleInput::Part { .. } => (
            vec![model::CreatedBody {
                body: source.output.body,
                feature: feature_id,
            }],
            vec![],
        ),
        ScaleInput::Component { owner, .. } => (vec![], vec![owner]),
    };
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit Scale".into(),
        edits: vec![Mutation::Replace(feature)],
    })?;
    Ok(PlacementBatchPrepared {
        prepared,
        outputs,
        instances,
    })
}

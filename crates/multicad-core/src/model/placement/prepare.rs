use super::*;
use crate::model::{
    BodyInput, MAX_TRANSFORM_TARGETS,
    boolean::available,
    common::{error, revision},
};
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    document::{Entity, Mutation, Placement},
    hierarchy,
};
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InstanceCopySource {
    Part { source: BodyInput },
    Instance { instance: EntityId },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceCopyRequest {
    pub source: InstanceCopySource,
    pub instance: EntityId,
    pub driver: EntityId,
    pub name: String,
    pub parameters: NumericTransform,
}
impl InstanceCopyRequest {
    pub fn new(source: InstanceCopySource, parameters: NumericTransform) -> Self {
        Self {
            source,
            instance: EntityId::new(),
            driver: EntityId::new(),
            name: "Placed copy".into(),
            parameters,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceMoveRequest {
    pub instance: EntityId,
    pub driver: EntityId,
    pub parameters: NumericTransform,
}
impl InstanceMoveRequest {
    pub fn new(instance: EntityId, parameters: NumericTransform) -> Self {
        Self {
            instance,
            driver: EntityId::new(),
            parameters,
        }
    }
}
pub struct PlacementPrepared {
    pub prepared: Prepared,
    pub instances: Vec<EntityId>,
}
fn driver_entity(
    id: EntityId,
    source: &InstancePlacementDefinition,
) -> Result<Entity, CommandError> {
    let mut entity = Entity::new(
        "Instance placement",
        EntityData::Extension {
            definition: source.definition()?,
            inputs: source.inputs()?,
        },
    );
    entity.id = id;
    Ok(entity)
}
fn fresh(
    document: &Document,
    id: EntityId,
    ids: &mut BTreeSet<EntityId>,
) -> Result<(), CommandError> {
    if id.is_nil() || id == document.id() || document.get(id).is_some() || !ids.insert(id) {
        return Err(error(
            "Placement identities must be new, distinct and non-nil",
        ));
    }
    Ok(())
}
fn part_component(document: &Document, id: EntityId) -> Result<(), CommandError> {
    if !matches!(
        document.get(id).map(|e| &e.data),
        Some(EntityData::Part { .. } | EntityData::Assembly { .. })
    ) {
        return Err(error(
            "This placement tool requires an Instance of an existing Part or Assembly",
        ));
    }
    Ok(())
}
fn copy_base(
    document: &Document,
    instance: EntityId,
    owner: EntityId,
    ids: &mut BTreeSet<EntityId>,
    edits: &mut Vec<Mutation>,
) -> Result<(EntityId, Placement, PlacementBase), CommandError> {
    if crate::model::joint_driver(document, instance)?.is_some() {
        return Err(error(
            "Use Place component to copy relationship-controlled geometry, then create or edit its own joint",
        ));
    }
    let Some(EntityData::Instance {
        component,
        placement,
        placement_source,
    }) = document.get(instance).map(|e| &e.data)
    else {
        return Err(error("Choose an existing Instance to copy"));
    };
    part_component(document, *component)?;
    let Some(mut driver) = *placement_source else {
        return Ok((
            *component,
            *placement,
            PlacementBase::Literal {
                pose: RigidPose::from_placement(*placement)?,
            },
        ));
    };
    check_next_driver(document, driver, instance)?;
    let mut visited = BTreeSet::new();
    let mut sources = vec![];
    loop {
        if visited.len() >= MAX_PLACEMENT_CHAIN || !visited.insert(driver) {
            return Err(error(
                "Instance placement chain exceeds its copy budget or contains a cycle",
            ));
        }
        let source = Driver::read(document, driver)?;
        if source.owner() != instance {
            return Err(error("Copied placement driver belongs to another instance"));
        }
        let next = match source.base() {
            PlacementBase::Previous { driver } => Some(driver),
            _ => None,
        };
        sources.push((driver, source));
        match next {
            Some(next) => driver = next,
            None => break,
        }
    }
    let mut remap = BTreeMap::new();
    for (id, _) in &sources {
        let copied = EntityId::new();
        fresh(document, copied, ids)?;
        remap.insert(*id, copied);
    }
    for (id, mut source) in sources {
        source.copy_owner(owner);
        if let PlacementBase::Previous { driver } = source.base() {
            source.set_base(PlacementBase::Previous {
                driver: remap[&driver],
            });
        }
        edits.push(Mutation::Insert(source.entity(remap[&id])?));
    }
    Ok((
        *component,
        *placement,
        PlacementBase::Previous {
            driver: remap[placement_source.as_ref().unwrap()],
        },
    ))
}
/// Duplicate only the explicit source as a true placed occurrence. Existing
/// placement-driver chains are cloned with fresh identities and a new owner.
pub fn prepare_instance_copies(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    requests: &[InstanceCopyRequest],
) -> Result<PlacementPrepared, CommandError> {
    revision(session, expected_revision)?;
    if requests.is_empty() || requests.len() > MAX_TRANSFORM_TARGETS {
        return Err(error("Choose 1–128 explicit copies"));
    }
    let document = session.document();
    if document
        .entities()
        .filter(|e| matches!(e.data, EntityData::Instance { .. }))
        .count()
        .saturating_add(requests.len())
        > MAX_PLACED_INSTANCES
    {
        return Err(error("Placed copies exceed the native occurrence budget"));
    }

    let index = hierarchy::Index::new(document).map_err(|e| error(e.to_string()))?;
    let mut hierarchy = document.hierarchy().clone();
    let mut ids = BTreeSet::new();
    let mut edits = vec![];
    let mut instances = vec![];
    for request in requests {
        fresh(document, request.instance, &mut ids)?;
        fresh(document, request.driver, &mut ids)?;
        request.parameters.resolve(session.parameters())?;
        if request.name.trim().is_empty() || request.name.len() > 1000 {
            return Err(error("Copy name must contain 1–1000 bytes"));
        }
        let (component, placement, base, parent) = match &request.source {
            InstanceCopySource::Part { source } => {
                available(document, source, true)?;
                (
                    source.body,
                    Placement::default(),
                    PlacementBase::Literal {
                        pose: RigidPose::default(),
                    },
                    index
                        .parent(source.body)
                        .map_err(|e| error(e.to_string()))?,
                )
            }
            InstanceCopySource::Instance { instance } => {
                let copied_driver_start = edits.len();
                let (mut component, placement, base) =
                    copy_base(document, *instance, request.instance, &mut ids, &mut edits)?;
                if let Some((wrapper, owned, private_rekey)) = crate::symbols::copy_owned_variant(
                    document,
                    *instance,
                    request.instance,
                    &request.name,
                )? {
                    for edit in &mut edits[copied_driver_start..] {
                        let Mutation::Insert(entity) = edit else {
                            return Err(error(
                                "Copied symbol placement has an unexpected source edit",
                            ));
                        };
                        *entity = crate::symbols::rekey_copied_driver(entity, &private_rekey)?;
                    }
                    for edit in owned {
                        let Mutation::Insert(entity) = &edit else {
                            return Err(error(
                                "Copied symbol variant has an unexpected source edit",
                            ));
                        };
                        fresh(document, entity.id, &mut ids)?;
                        edits.push(edit);
                    }
                    component = wrapper;
                }
                (
                    component,
                    placement,
                    base,
                    index.parent(*instance).map_err(|e| error(e.to_string()))?,
                )
            }
        };
        let source = InstancePlacementDefinition {
            owner: request.instance,
            base,
            parameters: request.parameters.clone(),
        };
        let mut instance = Entity::new(
            &request.name,
            EntityData::Instance {
                component,
                placement,
                placement_source: Some(request.driver),
            },
        );
        instance.id = request.instance;
        edits.extend([
            Mutation::Insert(driver_entity(request.driver, &source)?),
            Mutation::Insert(instance),
        ]);
        if let Some(parent) = parent {
            hierarchy
                .members
                .entry(parent)
                .or_default()
                .push(request.instance);
        }
        instances.push(request.instance);
    }
    if hierarchy != *document.hierarchy() {
        edits.push(Mutation::SetHierarchy(hierarchy));
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Create placed copies".into(),
        edits,
    })?;
    Ok(PlacementPrepared {
        prepared,
        instances,
    })
}
pub fn prepare_instance_moves(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    requests: &[InstanceMoveRequest],
) -> Result<PlacementPrepared, CommandError> {
    revision(session, expected_revision)?;
    if requests.is_empty() || requests.len() > MAX_TRANSFORM_TARGETS {
        return Err(error("Choose 1–128 explicit instance targets"));
    }
    let document = session.document();
    let mut ids = BTreeSet::new();
    let mut selected = BTreeSet::new();
    let mut edits = vec![];
    let mut instances = vec![];
    for request in requests {
        if !selected.insert(request.instance) {
            return Err(error("Choose each instance once"));
        }
        fresh(document, request.driver, &mut ids)?;
        request.parameters.resolve(session.parameters())?;
        let mut instance = document
            .get(request.instance)
            .ok_or_else(|| error("Instance target is missing"))?
            .clone();
        let EntityData::Instance {
            component,
            placement,
            placement_source,
        } = &mut instance.data
        else {
            return Err(error("Choose an existing Instance"));
        };
        part_component(document, *component)?;
        if crate::model::joint_driver(document, request.instance)?.is_some() {
            return Err(error(
                "Edit this Instance's joint values instead of adding a free placement after its relationship",
            ));
        }
        let base = match *placement_source {
            Some(driver) => {
                check_next_driver(document, driver, request.instance)?;
                PlacementBase::Previous { driver }
            }
            None => PlacementBase::Literal {
                pose: RigidPose::from_placement(*placement)?,
            },
        };
        let source = InstancePlacementDefinition {
            owner: instance.id,
            base,
            parameters: request.parameters.clone(),
        };
        *placement_source = Some(request.driver);
        edits.extend([
            Mutation::Insert(driver_entity(request.driver, &source)?),
            Mutation::Replace(instance),
        ]);
        instances.push(request.instance);
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Move / rotate instances".into(),
        edits,
    })?;
    Ok(PlacementPrepared {
        prepared,
        instances,
    })
}
pub fn prepare_instance_placement_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    driver: EntityId,
    parameters: &NumericTransform,
) -> Result<PlacementPrepared, CommandError> {
    revision(session, expected_revision)?;
    let mut source = read_instance_placement(session.document(), driver)?;
    parameters.resolve(session.parameters())?;
    source.parameters = parameters.clone();
    let mut entity = session.document().get(driver).expect("read driver").clone();
    entity.data = EntityData::Extension {
        definition: source.definition()?,
        inputs: source.inputs()?,
    };
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit instance placement".into(),
        edits: vec![Mutation::Replace(entity)],
    })?;
    Ok(PlacementPrepared {
        prepared,
        instances: vec![source.owner],
    })
}

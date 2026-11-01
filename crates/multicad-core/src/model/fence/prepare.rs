//! Source-only atomic preparations. Native evaluation must prove every affected
//! member and downstream result before presenting an Apply-capable candidate.
use super::*;
use crate::{
    commands::{Actor, CommandError, MAX_TRANSACTION_EDITS, Request, Session},
    document::{Entity, Mutation},
    materials::{self, LibraryDefinition},
    model::{
        BodyOutput, CreatedBody, ModelPrepared,
        common::{error, revision},
    },
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct FenceTypeRequest {
    pub id: EntityId,
    pub name: String,
    pub construction: FenceConstruction,
}
impl FenceTypeRequest {
    pub fn new(name: impl Into<String>, construction: FenceConstruction) -> Self {
        Self {
            id: EntityId::new(),
            name: name.into(),
            construction,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FenceRunRequest {
    pub id: EntityId,
    pub name: String,
    pub definition: FenceRunDefinition,
}
impl FenceRunRequest {
    pub fn new(name: impl Into<String>, definition: FenceRunDefinition) -> Self {
        Self {
            id: EntityId::new(),
            name: name.into(),
            definition,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FenceRunChange {
    Create(FenceRunRequest),
    Edit(FenceRunRequest),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FenceChange {
    pub new_type: Option<FenceTypeRequest>,
    /// An explicit shared edit. Every run using this type is reconciled in the
    /// same transaction, including a current run's simultaneous override repair.
    pub edit_type: Option<FenceTypeRequest>,
    pub run: Option<FenceRunChange>,
}

#[derive(Clone, Debug)]
pub struct FencePrepared {
    pub runs: Vec<EntityId>,
    pub members: Vec<(EntityId, FenceMemberKey, CreatedBody)>,
    pub model: ModelPrepared,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FenceMemberSpec {
    pub key: FenceMemberKey,
    pub name: String,
    pub material: EntityId,
}

fn short_name(value: &str) -> String {
    let mut chars = value.chars();
    let mut result: String = chars.by_ref().take(100).collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}
impl FenceLayout {
    /// Names follow traversal order; semantic keys remain independent of names.
    pub fn member_specs(&self) -> Vec<FenceMemberSpec> {
        let mut result = Vec::with_capacity(self.posts.len() + self.infills.len());
        let mut visited = BTreeSet::new();
        for bay in &self.bays {
            for post in [bay.key.start, bay.key.end] {
                if visited.insert(post) {
                    result.push(FenceMemberSpec {
                        key: FenceMemberKey::Post { post },
                        name: format!("Post {}", visited.len()),
                        material: self.construction.post_material,
                    });
                }
            }
        }
        let bays: BTreeMap<_, _> = self
            .bays
            .iter()
            .enumerate()
            .map(|(i, b)| (b.key, i + 1))
            .collect();
        result.extend(self.infills.iter().map(|member| FenceMemberSpec {
            key: FenceMemberKey::Infill { infill: member.key },
            name: format!(
                "{} / Bay {} / Member {}",
                short_name(&member.name),
                bays[&member.key.bay],
                member.key.ordinal + 1
            ),
            material: member.material,
        }));
        result
    }
}

pub fn fence_type_users(
    document: &Document,
    fence_type: EntityId,
) -> Result<Vec<EntityId>, DocumentError> {
    let mut result = Vec::new();
    for entity in fence_runs(document) {
        if read_fence_run(document, entity.id)?.fence_type == fence_type {
            result.push(entity.id);
        }
    }
    Ok(result)
}
fn check_name(name: &str) -> Result<(), CommandError> {
    if name.trim().is_empty() || name.len() > 1000 {
        return Err(error("Fence names must contain 1-1000 bytes"));
    }
    Ok(())
}
fn type_entity(request: &FenceTypeRequest) -> Result<Entity, CommandError> {
    check_name(&request.name)?;
    request.construction.validate()?;
    let mut entity = Entity::new(
        request.name.clone(),
        EntityData::Extension {
            definition: records::definition(FENCE_TYPE_KIND, &request.construction)?,
            inputs: request.construction.inputs(),
        },
    );
    entity.id = request.id;
    Ok(entity)
}

pub fn prepare_fence_change(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    change: &FenceChange,
) -> Result<FencePrepared, CommandError> {
    revision(session, expected_revision)?;
    if change.new_type.is_none() && change.edit_type.is_none() && change.run.is_none() {
        return Err(error("Choose a fence change"));
    }
    if change.new_type.is_some() && change.edit_type.is_some() {
        return Err(error(
            "Choose a shared construction edit or a separate duplicated type",
        ));
    }
    let document = session.document();
    let mut edits = Vec::new();
    let mut affected = BTreeSet::new();
    if let Some(request) = &change.new_type {
        if request.id.is_nil() || document.get(request.id).is_some() {
            return Err(error("New fence type needs a new nonempty identity"));
        }
        edits.push(Mutation::Insert(type_entity(request)?));
    }
    if let Some(request) = &change.edit_type {
        read_fence_type(document, request.id)?;
        affected.extend(fence_type_users(document, request.id)?);
        let mut entity = type_entity(request)?;
        entity.description = document
            .get(request.id)
            .expect("existing type")
            .description
            .clone();
        edits.push(Mutation::Replace(entity));
    }
    let mut hierarchy = document.hierarchy().clone();
    if let Some(change) = &change.run {
        let (request, create) = match change {
            FenceRunChange::Create(r) => (r, true),
            FenceRunChange::Edit(r) => (r, false),
        };
        check_name(&request.name)?;
        request.definition.validate()?;
        let mut entity = Entity::new(request.name.clone(), request.definition.entity_data()?);
        entity.id = request.id;
        if create {
            if request.id.is_nil()
                || request.id == request.definition.group
                || document.get(request.id).is_some()
                || document.get(request.definition.group).is_some()
            {
                return Err(error(
                    "New fence run and group need new distinct identities",
                ));
            }
            let mut group = Entity::new(request.name.clone(), EntityData::Group {});
            group.id = request.definition.group;
            edits.push(Mutation::Insert(group));
            edits.push(Mutation::Insert(entity));
            hierarchy
                .members
                .entry(hierarchy.active_component.unwrap_or(document.id()))
                .or_default()
                .push(request.definition.group);
            hierarchy
                .members
                .insert(request.definition.group, Vec::new());
        } else {
            let previous = read_fence_run(document, request.id)?;
            if request.definition.group != previous.group {
                return Err(error("Fence edits retain their existing group identity"));
            }
            entity.description = document
                .get(request.id)
                .expect("existing run")
                .description
                .clone();
            let mut group = document.get(previous.group).expect("checked group").clone();
            group.name = request.name.clone();
            edits.push(Mutation::Replace(group));
            edits.push(Mutation::Replace(entity));
        }
        affected.insert(request.id);
    }
    // Type and run edits stage together, before resolving either. A type's
    // removed automatic post can thus be explicitly repaired on the current run.
    let stage = document.edited(&edits)?;
    let parameters = crate::parameters::evaluate(&stage)?;
    for request in change.new_type.iter().chain(change.edit_type.iter()) {
        request.construction.resolve(&stage, &parameters)?;
    }
    let mut existing: BTreeMap<EntityId, BTreeMap<FenceMemberKey, CreatedBody>> = BTreeMap::new();
    let mut properties = BTreeMap::new();
    for entity in document.entities() {
        if matches!(&entity.data, EntityData::Feature { definition, .. } if definition.type_id == FENCE_MEMBER_TYPE)
        {
            let source = read_fence_member(document, entity.id)?;
            existing.entry(source.run).or_default().insert(
                source.key,
                CreatedBody {
                    feature: entity.id,
                    body: source.output.body,
                },
            );
        }
        if let Some(LibraryDefinition::PartProperties(p)) = materials::read(entity)? {
            properties.insert(p.part, entity.id);
        }
    }
    let mut removed = BTreeSet::new();
    let mut members = Vec::new();
    for run in &affected {
        let source = read_fence_run(&stage, *run)?;
        let layout = resolve_fence_run(&stage, &parameters, &source).map_err(|reason| {
            error(format!(
                "{}: {reason}",
                stage.get(*run).expect("staged run").name
            ))
        })?;
        let specs = layout.member_specs();
        let mut previous = existing.remove(run).unwrap_or_default();
        for spec in specs {
            let name = format!(
                "{} / {}",
                short_name(&stage.get(*run).expect("staged run").name),
                spec.name
            );
            let output = if let Some(output) = previous.remove(&spec.key) {
                let mut part = document
                    .get(output.body)
                    .expect("checked member Part")
                    .clone();
                part.name = name.clone();
                let EntityData::Part { material, .. } = &mut part.data else {
                    unreachable!()
                };
                *material = Some(spec.material);
                if &part != document.get(output.body).expect("existing Part") {
                    edits.push(Mutation::Replace(part));
                }
                let mut feature = document
                    .get(output.feature)
                    .expect("checked member")
                    .clone();
                feature.name = format!("{name} feature");
                if &feature != document.get(output.feature).expect("existing feature") {
                    edits.push(Mutation::Replace(feature));
                }
                output
            } else {
                let output = CreatedBody {
                    feature: EntityId::new(),
                    body: EntityId::new(),
                };
                let member = FenceMemberDefinition {
                    run: *run,
                    key: spec.key,
                    output: BodyOutput { body: output.body },
                };
                let mut feature = Entity::new(
                    format!("{name} feature"),
                    EntityData::Feature {
                        definition: member.definition()?,
                        inputs: member.inputs(),
                        suppressed: false,
                    },
                );
                feature.id = output.feature;
                let mut part = Entity::new(
                    name,
                    EntityData::Part {
                        features: vec![output.feature],
                        material: Some(spec.material),
                        tags: BTreeSet::new(),
                    },
                );
                part.id = output.body;
                edits.extend([Mutation::Insert(feature), Mutation::Insert(part)]);
                hierarchy
                    .members
                    .entry(source.group)
                    .or_default()
                    .extend([output.body, output.feature]);
                output
            };
            members.push((*run, spec.key, output));
        }
        for output in previous.values() {
            let Some(EntityData::Part { features, .. }) =
                document.get(output.body).map(|e| &e.data)
            else {
                unreachable!()
            };
            if features != &[output.feature] {
                return Err(error(
                    "A removed fence member has downstream features; explicitly repair them before changing its layout",
                ));
            }
            removed.extend([output.body, output.feature]);
            if let Some(properties) = properties.get(&output.body) {
                removed.insert(*properties);
            }
        }
    }
    // External dimensions, views and model references may not silently retarget
    // a removed post/picket. Deleting their source requires explicit repair.
    for entity in stage.entities() {
        if !removed.contains(&entity.id)
            && entity
                .references()
                .iter()
                .any(|link| removed.contains(&link.target))
        {
            return Err(error(format!(
                "{} references a removed fence member; repair it before changing the layout",
                entity.name
            )));
        }
    }
    edits.extend(removed.iter().copied().map(Mutation::Remove));
    for children in hierarchy.members.values_mut() {
        children.retain(|id| !removed.contains(id));
    }
    if &hierarchy != document.hierarchy() {
        edits.push(Mutation::SetHierarchy(hierarchy));
    }
    if edits.len() > MAX_TRANSACTION_EDITS {
        return Err(error(
            "Shared fence change exceeds the atomic transaction budget; simplify the runs or duplicate the construction type",
        ));
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit fence construction or run".into(),
        edits,
    })?;
    Ok(FencePrepared {
        runs: affected.into_iter().collect(),
        model: ModelPrepared {
            prepared,
            outputs: members
                .iter()
                .map(|(_, _, output)| output.clone())
                .collect(),
        },
        members,
    })
}

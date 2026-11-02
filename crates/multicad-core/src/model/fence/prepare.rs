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
    let (derived, members) =
        reconcile_members(document, &stage, &parameters, &affected, hierarchy)?;
    edits.extend(derived);
    if edits.len() > MAX_TRANSACTION_EDITS {
        return Err(error("Fence change exceeds the atomic transaction budget"));
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

/// Derive real member source records from the staged canonical run/type.
/// `document` is the pre-edit source so recipe kind conversions cannot retain
/// an old panel identity as a rail or picket.
type MemberOutputs = Vec<(EntityId, FenceMemberKey, CreatedBody)>;

fn reconcile_members(
    document: &Document,
    stage: &Document,
    parameters: &crate::parameters::EvaluatedParameters,
    affected: &BTreeSet<EntityId>,
    mut hierarchy: crate::hierarchy::Hierarchy,
) -> Result<(Vec<Mutation>, MemberOutputs), CommandError> {
    let mut edits = Vec::new();
    let mut existing: BTreeMap<EntityId, BTreeMap<FenceMemberKey, CreatedBody>> = BTreeMap::new();
    let mut properties = BTreeMap::new();
    for entity in stage.entities() {
        if matches!(&entity.data, EntityData::Feature { definition, .. } if definition.type_id == FENCE_MEMBER_TYPE)
        {
            let source = read_fence_member(stage, entity.id)?;
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
    for run in affected {
        let source = read_fence_run(stage, *run)?;
        let layout = resolve_fence_run(stage, parameters, &source).map_err(|reason| {
            error(format!(
                "{}: {reason}",
                stage.get(*run).expect("staged run").name
            ))
        })?;
        let specs = layout.member_specs();
        let mut previous = existing.remove(run).unwrap_or_default();
        let old_construction = document
            .get(*run)
            .map(|_| {
                read_fence_run(document, *run)
                    .and_then(|source| read_fence_type(document, source.fence_type))
            })
            .transpose()?;
        let new_construction = read_fence_type(stage, source.fence_type)?;
        for spec in specs {
            let name = format!(
                "{} / {}",
                short_name(&stage.get(*run).expect("staged run").name),
                spec.name
            );
            // A panel becoming a rail or an array of pickets has no retained
            // physical-member identity, even if its recipe ID stays stable.
            // Keep the old output in `previous` so removal/reference repair
            // follows the same atomic path as any other deleted member.
            let already_replaced = previous
                .get(&spec.key)
                .is_some_and(|output| document.get(output.feature).is_none());
            let same_family = already_replaced
                || match spec.key {
                    FenceMemberKey::Post { .. } => true,
                    FenceMemberKey::Infill { infill } => old_construction
                        .as_ref()
                        .and_then(|old| old.infills.iter().find(|r| r.id == infill.recipe))
                        .zip(
                            new_construction
                                .infills
                                .iter()
                                .find(|r| r.id == infill.recipe),
                        )
                        .is_some_and(|(old, new)| {
                            std::mem::discriminant(&old.construction)
                                == std::mem::discriminant(&new.construction)
                        }),
                };
            let retained = if same_family {
                previous.remove(&spec.key)
            } else {
                None
            };
            let output = if let Some(output) = retained {
                let mut part = stage.get(output.body).expect("checked member Part").clone();
                part.name = name.clone();
                let EntityData::Part { material, .. } = &mut part.data else {
                    unreachable!()
                };
                *material = Some(spec.material);
                if &part != stage.get(output.body).expect("existing Part") {
                    edits.push(Mutation::Replace(part));
                }
                let mut feature = stage.get(output.feature).expect("checked member").clone();
                feature.name = format!("{name} feature");
                if &feature != stage.get(output.feature).expect("existing feature") {
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
            let Some(EntityData::Part { features, .. }) = stage.get(output.body).map(|e| &e.data)
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
    if &hierarchy != stage.hierarchy() {
        edits.push(Mutation::SetHierarchy(hierarchy));
    }
    if edits.len() > MAX_TRANSACTION_EDITS {
        return Err(error(
            "Shared fence change exceeds the atomic transaction budget; simplify the runs or duplicate the construction type",
        ));
    }
    Ok((edits, members))
}

/// Reconcile parameter-driven topology in the same source transaction. This is
/// source validation; callers still need complete native preflight before Apply.
pub(crate) fn reconcile_fence_sources(
    before: &crate::parameters::EvaluatedParameters,
    after: &crate::parameters::EvaluatedParameters,
) -> Result<Vec<Mutation>, CommandError> {
    let document = &before.document;
    let stage = &after.document;
    let parameters_changed = before.values != after.values;
    let mut affected = BTreeSet::new();
    for entity in fence_runs(stage) {
        let source = read_fence_run(stage, entity.id)?;
        let old = document.get(entity.id);
        // Avoid resolving layouts for unrelated edits, including member metadata.
        let level_changed = source
            .level
            .is_some_and(|id| document.get(id) != stage.get(id));
        if !parameters_changed
            && old == Some(entity)
            && document.get(source.fence_type) == stage.get(source.fence_type)
            && !level_changed
        {
            continue;
        }
        let layout = resolve_fence_run(stage, after, &source)?;
        let previous = old
            .map(|_| {
                let run = read_fence_run(document, entity.id)?;
                resolve_fence_run(document, before, &run)
            })
            .transpose()?;
        if previous.as_ref() != Some(&layout) || old.map(|e| &e.name) != Some(&entity.name) {
            affected.insert(entity.id);
        }
    }
    if affected.is_empty() {
        return Ok(Vec::new());
    }
    reconcile_members(document, stage, after, &affected, stage.hierarchy().clone())
        .map(|(edits, _)| edits)
}

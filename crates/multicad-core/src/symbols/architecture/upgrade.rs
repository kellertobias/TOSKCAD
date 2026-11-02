//! Architectural participants in the existing pinned-symbol compare/update.
use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureSourceChange {
    pub id: EntityId,
    pub name: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureUpgradeChange {
    pub record: EntityId,
    pub output: ArchitectureOutput,
    pub added_inputs: Vec<EntityId>,
    pub removed_inputs: Vec<EntityId>,
    pub changed_inputs: Vec<EntityId>,
    pub removed_layers: Vec<ArchitectureSourceChange>,
    pub changed_recipes: Vec<ArchitectureSourceChange>,
    pub removed_members: Vec<EntityId>,
}

pub(in crate::symbols) fn compatible_roles(
    old: &SymbolDefinition,
    new: &SymbolDefinition,
) -> Result<(), DocumentError> {
    let old = blueprint_for_definition(old)?.map(|b| b.role);
    let new = blueprint_for_definition(new)?.map(|b| b.role);
    if old != new {
        return Err(invalid(
            "Changing between ordinary symbols, Wall and Fence requires a new symbol identity and placement",
        ));
    }
    Ok(())
}
fn signatures(
    definition: &SymbolDefinition,
    blueprint: &ArchitectureBlueprint,
) -> Result<BTreeMap<EntityId, serde_json::Value>, DocumentError> {
    let mut construction = ArchitectureConstruction::read(
        &definition.template,
        blueprint.role,
        blueprint.construction,
    )?;
    let parameters =
        crate::parameters::evaluate(&definition.template).map_err(|e| invalid(e.to_string()))?;
    let mut values = BTreeMap::new();
    for slot in &blueprint.dimensions {
        let value = slot.target.dimension_mut(&mut construction)?;
        let mm = value
            .value_in(crate::parameters::Unit::Millimeter, Some(&parameters))
            .map_err(invalid)?;
        values.insert(
            slot.id,
            serde_json::json!({"dimension":slot, "default":value, "default_mm":mm}),
        );
    }
    for slot in &blueprint.materials {
        values.insert(slot.id, serde_json::json!({"material":slot, "default":slot.target.material_mut(&mut construction)?}));
    }
    for slot in &blueprint.options {
        values.insert(
            slot.id,
            serde_json::json!({"option":slot, "default":slot.target.value(&construction)?}),
        );
    }
    Ok(values)
}
fn changed_target(
    old: &ArchitectureBlueprint,
    new: &ArchitectureBlueprint,
    inputs: &ArchitectureInputs,
) -> Result<(), DocumentError> {
    for id in inputs.dimensions.keys() {
        if let Some((old, new)) = old
            .dimensions
            .iter()
            .find(|s| s.id == *id)
            .zip(new.dimensions.iter().find(|s| s.id == *id))
            && old.target != new.target
        {
            return Err(invalid(format!(
                "{} changed its dimension target; explicitly reset this override before updating",
                old.name
            )));
        }
    }
    for id in inputs.materials.keys() {
        if let Some((old, new)) = old
            .materials
            .iter()
            .find(|s| s.id == *id)
            .zip(new.materials.iter().find(|s| s.id == *id))
            && old.target != new.target
        {
            return Err(invalid(format!(
                "{} changed its material target; explicitly reset this override before updating",
                old.name
            )));
        }
    }
    for id in inputs.options.keys() {
        if let Some((old, new)) = old
            .options
            .iter()
            .find(|s| s.id == *id)
            .zip(new.options.iter().find(|s| s.id == *id))
            && old.target != new.target
        {
            return Err(invalid(format!(
                "{} changed its option target; explicitly reset this override before updating",
                old.name
            )));
        }
    }
    Ok(())
}
pub(in crate::symbols) fn changes(
    document: &Document,
    definition: EntityId,
    old: &SymbolDefinition,
    new: &SymbolDefinition,
) -> Result<Vec<ArchitectureUpgradeChange>, DocumentError> {
    let Some(previous) = blueprint_for_definition(old)? else {
        return Ok(Vec::new());
    };
    let next = blueprint_for_definition(new)?
        .ok_or_else(|| invalid("Architectural update requires a compatible role"))?;
    let before = signatures(old, &previous)?;
    let after = signatures(new, &next)?;
    let mut removed_layers = Vec::new();
    let mut changed_recipes = Vec::new();
    match (
        ArchitectureConstruction::read(&old.template, previous.role, previous.construction)?,
        ArchitectureConstruction::read(&new.template, next.role, next.construction)?,
    ) {
        (ArchitectureConstruction::Wall(old), ArchitectureConstruction::Wall(new)) => {
            for layer in &old.layers.layers {
                if !new.layers.layers.iter().any(|l| l.id == layer.id) {
                    removed_layers.push(ArchitectureSourceChange {
                        id: layer.id,
                        name: layer.name.clone(),
                    });
                }
            }
        }
        (ArchitectureConstruction::Fence(old), ArchitectureConstruction::Fence(new)) => {
            for recipe in &old.infills {
                if !new.infills.iter().any(|r| {
                    r.id == recipe.id
                        && std::mem::discriminant(&r.construction)
                            == std::mem::discriminant(&recipe.construction)
                }) {
                    changed_recipes.push(ArchitectureSourceChange {
                        id: recipe.id,
                        name: recipe.name.clone(),
                    });
                }
            }
        }
        _ => return Err(invalid("Architectural update roles disagree")),
    }
    Ok(architecture_instances(document, definition)?
        .into_iter()
        .map(|(record, value)| ArchitectureUpgradeChange {
            record,
            output: value.output,
            added_inputs: after
                .keys()
                .filter(|id| !before.contains_key(id))
                .copied()
                .collect(),
            removed_inputs: before
                .keys()
                .filter(|id| !after.contains_key(id))
                .copied()
                .collect(),
            changed_inputs: before
                .keys()
                .filter(|id| after.contains_key(id) && before.get(id) != after.get(id))
                .copied()
                .collect(),
            removed_layers: removed_layers.clone(),
            changed_recipes: changed_recipes.clone(),
            removed_members: vec![],
        })
        .collect())
}

pub(in crate::symbols) fn update_edits(
    document: &Document,
    request: &SymbolUpdateRequest,
    old: &SymbolDefinition,
) -> Result<Vec<Mutation>, CommandError> {
    let previous = architecture_instances(document, request.definition)?;
    let records: BTreeSet<_> = previous.iter().map(|(id, _)| *id).collect();
    if request
        .architecture_inputs
        .keys()
        .copied()
        .collect::<BTreeSet<_>>()
        != records
        || request
            .architecture_contexts
            .keys()
            .any(|id| !records.contains(id))
    {
        return Err(invalid("An architectural update must explicitly cover each affected record and only its receiving contexts").into());
    }
    if previous.is_empty() {
        return Ok(Vec::new());
    }
    let before_blueprint = blueprint_for_definition(old)?
        .ok_or_else(|| invalid("Previous architectural construction is missing"))?;
    let after_blueprint = blueprint_for_definition(&request.replacement)?
        .ok_or_else(|| invalid("Incoming architectural construction is missing"))?;
    let keys = super::super::remap::identities(&request.replacement)?;
    let mut edits = Vec::new();
    for (record, mut value) in previous {
        let map = request
            .maps
            .get(&record)
            .ok_or_else(|| invalid("Architectural update identity map is missing"))?;
        for id in keys.iter().filter(|id| value.ids.contains_key(id)) {
            if map.get(id) != value.ids.get(id) {
                return Err(invalid("A compatible architectural source identity cannot be reallocated during update").into());
            }
        }
        for id in keys.iter().filter(|id| !value.ids.contains_key(id)) {
            if let Some(mapped) = map.get(id)
                && (value.ids.values().any(|old| old == mapped)
                    || document.get(*mapped).is_some()
                    || *mapped == document.id())
            {
                return Err(invalid(
                    "New architectural source identities require fresh private IDs",
                )
                .into());
            }
        }
        let before = configured_source(old, &value.ids, &value.inputs, document)?;
        let inputs = &request.architecture_inputs[&record];
        changed_target(&before_blueprint, &after_blueprint, inputs)?;
        value.inputs = inputs.clone();
        value.ids = map.clone();
        value.pin = request.replacement.pin()?;
        let after = configured_source(&request.replacement, &value.ids, &value.inputs, document)?;
        for id in before.keys().filter(|id| !after.contains_key(id)) {
            edits.push(Mutation::Remove(*id));
        }
        for (id, entity) in after {
            if before.contains_key(&id) {
                if document.get(id) != Some(&entity) {
                    edits.push(Mutation::Replace(entity));
                }
            } else {
                if document.get(id).is_some() {
                    return Err(invalid(
                        "New architectural identity collides with existing source",
                    )
                    .into());
                }
                edits.push(Mutation::Insert(entity));
            }
        }
        let mut entity = document.get(record).expect("checked record").clone();
        entity.data = value.entity_data()?;
        edits.push(Mutation::Replace(entity));
        let mut output = if let Some(context) = request.architecture_contexts.get(&record) {
            edits.extend(super::wall_details::mutations(document, context)?);
            super::edit::context_entity(document, &value, context)?
        } else {
            document
                .get(value.output.anchors()[0])
                .expect("checked output")
                .clone()
        };
        let construction = value.ids[&after_blueprint.construction];
        match &mut output.data {
            EntityData::Feature {
                definition, inputs, ..
            } if value.output.role() == ArchitectureRole::Wall => {
                let mut wall = model::WallDefinition::from_definition(definition)?;
                wall.wall_type = construction;
                *definition = wall.definition()?;
                *inputs = wall.inputs();
            }
            EntityData::Extension { definition, .. }
                if value.output.role() == ArchitectureRole::Fence =>
            {
                let mut run: model::FenceRunDefinition =
                    serde_json::from_value(definition.data.clone())
                        .map_err(|e| invalid(e.to_string()))?;
                run.fence_type = construction;
                output.data = run.entity_data()?;
            }
            _ => return Err(invalid("Architectural output role changed unexpectedly").into()),
        }
        if document.get(output.id) != Some(&output) {
            edits.push(Mutation::Replace(output));
        }
    }
    Ok(edits)
}

/// Plan semantic removals even when an external reference correctly prevents
/// preparing the complete update. Fresh temporary construction sources leave
/// the existing owned graphs and references intact; nothing is committed.
pub(in crate::symbols) fn planned_removals(
    document: &Document,
    request: &SymbolUpdateRequest,
    old: &SymbolDefinition,
) -> Result<BTreeMap<EntityId, Vec<EntityId>>, DocumentError> {
    let incoming = blueprint_for_definition(&request.replacement)?
        .ok_or_else(|| invalid("Incoming construction is missing"))?;
    if incoming.role != ArchitectureRole::Fence {
        return Ok(BTreeMap::new());
    }
    let old_blueprint = blueprint_for_definition(old)?
        .ok_or_else(|| invalid("Previous construction is missing"))?;
    let ArchitectureConstruction::Fence(before) = ArchitectureConstruction::read(
        &old.template,
        old_blueprint.role,
        old_blueprint.construction,
    )?
    else {
        return Err(invalid("Previous fence construction is missing"));
    };
    let ArchitectureConstruction::Fence(after) = ArchitectureConstruction::read(
        &request.replacement.template,
        incoming.role,
        incoming.construction,
    )?
    else {
        return Err(invalid("Incoming fence construction is missing"));
    };
    let keys = super::super::remap::identities(&request.replacement)?;
    let mut result = BTreeMap::new();
    for (record, value) in architecture_instances(document, request.definition)? {
        let ArchitectureOutput::Fence { run, .. } = value.output else {
            continue;
        };
        let fresh: BTreeMap<_, _> = keys.iter().map(|id| (*id, EntityId::new())).collect();
        let inverse: BTreeMap<_, _> = fresh
            .iter()
            .map(|(source, private)| (*private, *source))
            .collect();
        let inputs = request
            .architecture_inputs
            .get(&record)
            .ok_or_else(|| invalid("Architectural input repair is missing"))?;
        changed_target(&old_blueprint, &incoming, inputs)?;
        let graph = configured_source(&request.replacement, &fresh, inputs, document)?;
        let stage = document.edited(
            &graph
                .into_values()
                .map(Mutation::Insert)
                .collect::<Vec<_>>(),
        )?;
        let parameters = crate::parameters::evaluate(&stage).map_err(|e| invalid(e.to_string()))?;
        let construction = model::read_fence_type(&stage, fresh[&incoming.construction])?;
        let mut source = model::read_fence_run(document, run)?;
        if let Some(ArchitectureContext::Fence {
            path,
            level,
            post_positions,
        }) = request.architecture_contexts.get(&record)
        {
            source.path = path.clone();
            source.level = *level;
            source.post_positions = post_positions.clone();
        }
        let layout = model::plan_fence_instance(&stage, &parameters, &construction, &source)?;
        let desired_map = request
            .maps
            .get(&record)
            .ok_or_else(|| invalid("Architectural identity map is missing"))?;
        if desired_map.keys().copied().collect::<BTreeSet<_>>() != keys {
            return Err(invalid("Architectural identity map is incomplete"));
        }
        let desired = layout
            .member_specs()
            .into_iter()
            .map(|spec| {
                let mut key = spec.key;
                if let model::FenceMemberKey::Infill { infill } = &mut key {
                    infill.recipe = desired_map[&inverse[&infill.recipe]];
                }
                key
            })
            .collect::<BTreeSet<_>>();
        let replaced_families = before
            .infills
            .iter()
            .filter(|recipe| {
                !after.infills.iter().any(|next| {
                    next.id == recipe.id
                        && std::mem::discriminant(&next.construction)
                            == std::mem::discriminant(&recipe.construction)
                })
            })
            .map(|recipe| value.ids[&recipe.id])
            .collect::<BTreeSet<_>>();
        let removed = model::fence_members(document, run)?.into_iter().filter_map(|(key, _, body)| {
            let replaced = matches!(key, model::FenceMemberKey::Infill { infill } if replaced_families.contains(&infill.recipe));
            (replaced || !desired.contains(&key)).then_some(body)
        }).collect();
        result.insert(record, removed);
    }
    Ok(result)
}

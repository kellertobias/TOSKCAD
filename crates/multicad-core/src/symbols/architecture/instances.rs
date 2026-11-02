//! A pin owns only the private reusable construction. The existing canonical
//! wall/fence record remains the single authority for receiving path/context.
use super::*;
use crate::{model, project_file::ContentHash};
use std::collections::{BTreeMap, BTreeSet};

pub const PLACEMENT_TYPE: &str = "symbols.architecture_instance";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArchitectureOutput {
    Wall { feature: EntityId, body: EntityId },
    Fence { run: EntityId, group: EntityId },
}
impl ArchitectureOutput {
    pub fn role(&self) -> ArchitectureRole {
        match self {
            Self::Wall { .. } => ArchitectureRole::Wall,
            Self::Fence { .. } => ArchitectureRole::Fence,
        }
    }
    pub fn anchors(&self) -> [EntityId; 2] {
        match self {
            Self::Wall { feature, body } => [*feature, *body],
            Self::Fence { run, group } => [*run, *group],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureInstance {
    pub definition: EntityId,
    pub pin: ContentHash,
    pub ids: BTreeMap<EntityId, EntityId>,
    pub inputs: ArchitectureInputs,
    pub output: ArchitectureOutput,
}
impl ArchitectureInstance {
    pub fn entity_data(&self) -> Result<EntityData, DocumentError> {
        Ok(EntityData::Extension {
            definition: super::super::source::encode(PLACEMENT_TYPE, self)?,
            inputs: self.links(),
        })
    }
    fn links(&self) -> Vec<Link> {
        let mut links = vec![Link::new(self.definition, "definition")];
        for anchor in self.output.anchors() {
            links.push(Link::new(anchor, "architectural_output"));
        }
        for value in self.inputs.dimensions.values() {
            if let crate::constraint_sketch::Dimension::Expression { expression } = value {
                links.extend(
                    expression
                        .bindings
                        .iter()
                        .map(|b| Link::new(b.target, "receiving_parameter")),
                );
            }
        }
        for value in self.inputs.materials.values() {
            if let ArchitectureMaterialValue::Project { material } = value {
                links.push(Link::new(*material, "receiving_material"));
            }
        }
        links.sort_by(|a, b| (&a.role, a.target).cmp(&(&b.role, b.target)));
        links.dedup();
        links
    }
}
pub fn read_architecture_instance(
    document: &Document,
    id: EntityId,
) -> Result<ArchitectureInstance, DocumentError> {
    let Some(EntityData::Extension { definition, inputs }) = document.get(id).map(|e| &e.data)
    else {
        return Err(invalid("Choose an architectural symbol ownership record"));
    };
    let value: ArchitectureInstance = super::super::source::decode(definition, PLACEMENT_TYPE)?;
    if inputs != &value.links() {
        return Err(invalid(
            "Architectural symbol dependencies differ from its typed inputs",
        ));
    }
    Ok(value)
}
pub fn architecture_instances(
    document: &Document,
    definition: EntityId,
) -> Result<Vec<(EntityId, ArchitectureInstance)>, DocumentError> {
    let mut values = Vec::new();
    for entity in document.entities() {
        if matches!(&entity.data, EntityData::Extension { definition, .. } if definition.type_id == PLACEMENT_TYPE)
        {
            let value = read_architecture_instance(document, entity.id)?;
            if value.definition == definition {
                values.push((entity.id, value));
            }
        }
    }
    Ok(values)
}
fn output_contract(
    document: &Document,
    value: &ArchitectureInstance,
    blueprint: &ArchitectureBlueprint,
) -> Result<(), DocumentError> {
    match value.output {
        ArchitectureOutput::Wall { feature, body } => {
            let wall = model::read_wall(document, feature)?;
            if wall.output.body != body
                || wall.wall_type != blueprint.construction
                || wall.thickness.is_some()
                || wall.height.is_some()
                || wall.justification.is_some()
                || wall
                    .layer_placement
                    .as_ref()
                    .is_some_and(|p| p.reference.is_some())
            {
                return Err(invalid(
                    "Architectural wall must retain its pinned construction; edit exposed inputs instead of private wall overrides",
                ));
            }
            for entity in document.entities() {
                if entity.id != feature
                    && matches!(&entity.data, EntityData::Feature { definition, .. } if definition.type_id == model::WALL_TYPE)
                    && model::read_wall(document, entity.id)?.wall_type == blueprint.construction
                {
                    return Err(invalid(
                        "Copy this architectural wall through its symbol placement so the new path receives independent private construction",
                    ));
                }
            }
            if !matches!(document.get(body).map(|e| &e.data), Some(EntityData::Part { material: None, features, .. }) if features.first() == Some(&feature))
            {
                return Err(invalid(
                    "Architectural wall output must retain its real wall feature and per-layer materials",
                ));
            }
        }
        ArchitectureOutput::Fence { run, group } => {
            let source = model::read_fence_run(document, run)?;
            if model::fence_type_users(document, blueprint.construction)? != vec![run] {
                return Err(invalid(
                    "Copy this architectural fence through its symbol placement so the new run receives independent private construction",
                ));
            }
            if source.group != group
                || source.fence_type != blueprint.construction
                || source.overrides != model::FenceInstanceOverrides::default()
            {
                return Err(invalid(
                    "Architectural fence must retain its pinned construction; edit exposed inputs instead of private run overrides",
                ));
            }
        }
    }
    Ok(())
}
pub(in crate::symbols) fn validate_instances(
    document: &Document,
    namespaced: &mut BTreeSet<EntityId>,
    owned: &mut BTreeSet<EntityId>,
    count: &mut usize,
) -> Result<(), DocumentError> {
    for entity in document.entities() {
        if !matches!(&entity.data, EntityData::Extension { definition, .. } if definition.type_id == PLACEMENT_TYPE)
        {
            continue;
        }
        *count += 1;
        if *count > MAX_SYMBOL_INSTANCES {
            return Err(invalid("Embedded symbol instance budget exceeded"));
        }
        let value = read_architecture_instance(document, entity.id)?;
        let definition = read_definition(document, value.definition)?;
        let blueprint = blueprint_for_definition(&definition)?.ok_or_else(|| {
            invalid("Architectural placement requires an architectural definition")
        })?;
        if blueprint.role != value.output.role() || value.pin != definition.pin()? {
            return Err(invalid(
                "Architectural role or pin differs from the embedded definition; use explicit update",
            ));
        }
        for id in value
            .ids
            .values()
            .copied()
            .chain([entity.id])
            .chain(value.output.anchors())
        {
            if id.is_nil()
                || id == document.id()
                || !namespaced.insert(id)
                || id == value.definition
            {
                return Err(invalid(
                    "Symbol placements must have distinct nonempty private identities and output anchors",
                ));
            }
        }
        for (source, mapped) in &value.ids {
            if definition.template.get(*source).is_none() && document.get(*mapped).is_some() {
                return Err(invalid(
                    "Local architectural identities cannot alias canonical document entities",
                ));
            }
        }
        let expected = configured_source(&definition, &value.ids, &value.inputs, document)?;
        for (id, expected) in expected {
            if !owned.insert(id) || document.get(id) != Some(&expected) {
                return Err(invalid(format!(
                    "Architectural symbol-owned entity {id} was independently edited or removed; edit exposed inputs or explicitly update the definition"
                )));
            }
        }
        let private = read_blueprint(document, value.ids[&definition.root])?;
        output_contract(document, &value, &private)?;
    }
    Ok(())
}
/// Ownership projection for an already validated source document. Real wall
/// and fence outputs remain ordinary architectural records and stay editable.
pub(in crate::symbols) fn private_entities(
    document: &Document,
) -> Result<BTreeSet<EntityId>, DocumentError> {
    let mut owned = BTreeSet::new();
    for entity in document.entities() {
        if matches!(&entity.data, EntityData::Extension { definition, .. } if definition.type_id == PLACEMENT_TYPE)
        {
            let value = read_architecture_instance(document, entity.id)?;
            owned.extend(
                value
                    .ids
                    .values()
                    .copied()
                    .filter(|id| document.get(*id).is_some()),
            );
        }
    }
    Ok(owned)
}

/// Resolve a selected real output without parsing embedded templates or pins.
/// UI callers cache this projection when their source/selection context changes.
pub fn architecture_instance_for_output(
    document: &Document,
    output: EntityId,
) -> Result<Option<(EntityId, ArchitectureInstance)>, DocumentError> {
    let anchor = match document.get(output).map(|e| &e.data) {
        Some(EntityData::Feature { definition, .. })
            if definition.type_id == model::FENCE_MEMBER_TYPE =>
        {
            model::read_fence_member(document, output)?.run
        }
        Some(EntityData::Part { features, .. }) => {
            let member = features.iter().find(|id| matches!(document.get(**id).map(|e| &e.data), Some(EntityData::Feature { definition, .. }) if definition.type_id == model::FENCE_MEMBER_TYPE));
            if let Some(member) = member {
                model::read_fence_member(document, *member)?.run
            } else {
                output
            }
        }
        _ => output,
    };
    for entity in document.entities() {
        if matches!(&entity.data, EntityData::Extension { definition, .. } if definition.type_id == PLACEMENT_TYPE)
        {
            let value = read_architecture_instance(document, entity.id)?;
            if value.output.anchors().contains(&anchor) || entity.id == output {
                return Ok(Some((entity.id, value)));
            }
        }
    }
    Ok(None)
}

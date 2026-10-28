use super::{source::invalid, *};
use crate::{
    constraint_sketch::source::Source,
    document::{Definition, DocumentError, Entity, EntityData, EntityId},
    model,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
// Each accepted opaque Definition first passes its real schema decoder. The
// value walk below therefore sees only these audited ID-bearing schemas.
fn provider(d: &Definition) -> Result<(), DocumentError> {
    match d.type_id.as_str() {
        crate::constraint_sketch::source::TYPE => {
            Source::from_definition(d)?;
        }
        model::PRIMITIVE_TYPE => {
            model::PrimitiveDefinition::from_definition(d)?;
        }
        model::EXTRUDE_TYPE => {
            model::ExtrudeDefinition::from_definition(d)?;
        }
        model::INSTANCE_PLACEMENT_TYPE => {
            model::InstancePlacementDefinition::from_definition(d)?;
        }
        model::JOINT_TYPE => {
            model::JointDefinition::from_definition(d)?;
        }
        model::ASSEMBLY_TYPE => {
            if d.schema_version != 1 {
                return Err(invalid("Unsupported assembly schema in symbol"));
            }
            let v: model::AssemblyDefinition =
                serde_json::from_value(d.data.clone()).map_err(|e| invalid(e.to_string()))?;
            v.definition()?;
        }
        model::NAMED_POSE_TYPE => {
            if d.schema_version != 1 {
                return Err(invalid("Unsupported assembly pose schema in symbol"));
            }
            let v: model::NamedPoseDefinition =
                serde_json::from_value(d.data.clone()).map_err(|e| invalid(e.to_string()))?;
            v.definition()?;
        }
        other => {
            return Err(invalid(format!(
                "Symbol packaging does not yet support provider {other}; no opaque IDs were rewritten"
            )));
        }
    }
    Ok(())
}
pub(super) fn validate_entity(e: &Entity) -> Result<(), DocumentError> {
    match &e.data {
        EntityData::Parameter { .. }
        | EntityData::Plane { .. }
        | EntityData::Part { .. }
        | EntityData::Assembly { .. }
        | EntityData::Instance { .. } => {}
        EntityData::Material {
            section_hatch: None,
            ..
        } => {}
        EntityData::Sketch { definition, .. }
        | EntityData::Feature { definition, .. }
        | EntityData::Extension { definition, .. } => provider(definition)?,
        _ => {
            return Err(invalid(
                "Symbol source has an unsupported entity or material hatch provider",
            ));
        }
    }
    Ok(())
}
fn collect(v: &Value, out: &mut BTreeSet<EntityId>) {
    match v {
        Value::String(s) => {
            if let Ok(id) = EntityId::parse(s) {
                out.insert(id);
            } else {
                for token in s.split('/') {
                    if let Ok(id) = EntityId::parse(token) {
                        out.insert(id);
                    }
                }
            }
        }
        Value::Array(a) => {
            for v in a {
                collect(v, out)
            }
        }
        Value::Object(o) => {
            for v in o.values() {
                collect(v, out)
            }
        }
        _ => {}
    }
}
pub(super) fn identities(def: &SymbolDefinition) -> Result<BTreeSet<EntityId>, DocumentError> {
    let mut out = BTreeSet::new();
    for entity in def.template.entities() {
        validate_entity(entity)?;
        out.insert(entity.id);
        // Exclude author labels/descriptions: UUID-looking prose is never identity.
        collect(
            &serde_json::to_value(&entity.data).map_err(|e| invalid(e.to_string()))?,
            &mut out,
        );
    }
    if out.len() > MAX_SYMBOL_ENTITIES * 8 || out.iter().any(EntityId::is_nil) {
        return Err(invalid(
            "Symbol's source/local identity budget is exceeded or contains nil IDs",
        ));
    }
    Ok(out)
}
fn rewrite(v: &mut Value, map: &BTreeMap<EntityId, EntityId>) {
    match v {
        Value::String(s) => {
            if let Ok(id) = EntityId::parse(s) {
                if let Some(new) = map.get(&id) {
                    *s = new.to_string();
                }
            }
            // Schema-owned identity paths occur in dimension/curve/member Origin roles.
            // Only complete slash-delimited UUID tokens are replaced, never substrings.
            else if s.contains('/') {
                *s = s
                    .split('/')
                    .map(|token| {
                        EntityId::parse(token)
                            .ok()
                            .and_then(|id| map.get(&id))
                            .map(ToString::to_string)
                            .unwrap_or_else(|| token.into())
                    })
                    .collect::<Vec<_>>()
                    .join("/");
            }
        }
        Value::Array(a) => {
            for v in a {
                rewrite(v, map)
            }
        }
        Value::Object(o) => {
            for v in o.values_mut() {
                rewrite(v, map)
            }
        }
        _ => {}
    }
}
pub(super) fn remap(
    e: &Entity,
    map: &BTreeMap<EntityId, EntityId>,
) -> Result<Entity, DocumentError> {
    validate_entity(e)?;
    let mut data = serde_json::to_value(&e.data).map_err(|e| invalid(e.to_string()))?;
    rewrite(&mut data, map);
    let mut result = e.clone();
    result.id = *map
        .get(&e.id)
        .ok_or_else(|| invalid("Incomplete symbol identity map"))?;
    result.data = serde_json::from_value(data).map_err(|e| invalid(e.to_string()))?;
    if matches!(result.data, EntityData::Parameter { .. }) {
        result.name = format!("s{}_{}", result.id.to_string().replace('-', ""), e.name);
    }
    validate_entity(&result)?;
    Ok(result)
}

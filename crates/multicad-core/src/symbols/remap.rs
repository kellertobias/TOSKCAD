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
        model::INSTANCE_ALIGNMENT_TYPE => {
            model::InstanceAlignmentDefinition::from_definition(d)?;
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
        // Weldment and cabinetry geometry (CAD-185). Their payloads name
        // canonical entities only by complete UUID strings or slash paths;
        // the typed decoders reject unknown fields, so nothing opaque is
        // walked. Document-level rules (weld marks, sequences, connections)
        // live in extensions a Part closure never reaches.
        model::MEMBER_TYPE => {
            model::MemberDefinition::from_definition(d)?;
        }
        model::PANEL_TYPE => {
            model::PanelDefinition::from_definition(d)?;
        }
        model::MACHINING_KIND => typed::<model::MachiningDefinition>(d)?.validate()?,
        model::MACHINING_CUT_TYPE => {
            typed::<model::MachiningCutDefinition>(d)?.definition()?;
        }
        model::HARDWARE_PATTERN_KIND => typed::<model::HardwarePatternDefinition>(d)?.validate()?,
        model::PANEL_JOINT_KIND => typed::<model::PanelJointDefinition>(d)?.validate()?,
        model::PANEL_JOINT_CUT_TYPE => {
            typed::<model::PanelJointCutDefinition>(d)?.definition()?;
        }
        model::LOOSE_TENON_TYPE => {
            typed::<model::LooseTenonDefinition>(d)?.definition()?;
        }
        other => {
            return Err(invalid(format!(
                "Symbol packaging does not yet support provider {other}; no opaque IDs were rewritten"
            )));
        }
    }
    Ok(())
}
fn typed<T: serde::de::DeserializeOwned>(d: &Definition) -> Result<T, DocumentError> {
    if d.schema_version != 1 {
        return Err(invalid(format!(
            "Unsupported {} schema {} in symbol",
            d.type_id, d.schema_version
        )));
    }
    serde_json::from_value(d.data.clone()).map_err(|e| invalid(format!("{}: {e}", d.type_id)))
}
pub(super) fn validate_entity(e: &Entity) -> Result<(), DocumentError> {
    // Library stock, material styles and Part properties: the typed reader
    // checks the schema and that the canonical links match the payload.
    if crate::materials::read(e)?.is_some() {
        return Ok(());
    }
    match &e.data {
        EntityData::Parameter { .. }
        | EntityData::Plane { .. }
        | EntityData::Part { .. }
        | EntityData::Assembly { .. }
        | EntityData::Instance { .. } => {}
        EntityData::Material { section_hatch, .. } => {
            if let Some(hatch) = section_hatch {
                if hatch.type_id != "materials.hatch.inline" || hatch.schema_version != 1 {
                    return Err(invalid(
                        "Symbol material requires a supported inline hatch schema",
                    ));
                }
                let style: crate::materials::HatchStyle =
                    serde_json::from_value(hatch.data.clone())
                        .map_err(|e| invalid(e.to_string()))?;
                crate::materials::LibraryDefinition::Hatch(style).validate()?;
            }
        }
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
        match &entity.data {
            EntityData::Part {
                features, material, ..
            } => {
                out.extend(features.iter().copied());
                out.extend(material);
            }
            EntityData::Material { .. } => {}
            _ => collect(
                &serde_json::to_value(&entity.data).map_err(|e| invalid(e.to_string()))?,
                &mut out,
            ),
        }
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
    let data = match &e.data {
        EntityData::Part {
            features,
            material,
            tags,
        } => EntityData::Part {
            features: features
                .iter()
                .map(|id| {
                    map.get(id)
                        .copied()
                        .ok_or_else(|| invalid("Missing mapped Part feature"))
                })
                .collect::<Result<_, _>>()?,
            material: material
                .map(|id| {
                    map.get(&id)
                        .copied()
                        .ok_or_else(|| invalid("Missing mapped Part material"))
                })
                .transpose()?,
            tags: tags.clone(),
        },
        EntityData::Material { .. } => e.data.clone(),
        _ => {
            let mut data = serde_json::to_value(&e.data).map_err(|e| invalid(e.to_string()))?;
            rewrite(&mut data, map);
            serde_json::from_value(data).map_err(|e| invalid(e.to_string()))?
        }
    };
    let mut result = e.clone();
    result.id = *map
        .get(&e.id)
        .ok_or_else(|| invalid("Incomplete symbol identity map"))?;
    result.data = data;
    if matches!(result.data, EntityData::Parameter { .. }) {
        result.name = format!("s{}", result.id.to_string().replace('-', ""));
    }
    validate_entity(&result)?;
    Ok(result)
}

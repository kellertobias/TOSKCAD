//! Apply exposed values to a private source graph. This is a source draft,
//! not a prepared Session or a native geometry receipt. The role provider must
//! compose its receiving path/members, validate and preflight before Apply.
use super::*;
use crate::{
    constraint_sketch::Dimension,
    parameters::{self, Unit},
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArchitectureMaterialValue {
    Preset { material: EntityId },
    Project { material: EntityId },
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureInputs {
    #[serde(default)]
    pub parameters: BTreeMap<EntityId, SymbolValue>,
    #[serde(default)]
    pub dimensions: BTreeMap<EntityId, Dimension>,
    #[serde(default)]
    pub materials: BTreeMap<EntityId, ArchitectureMaterialValue>,
    #[serde(default)]
    pub options: BTreeMap<EntityId, ArchitectureOptionValue>,
}
impl ArchitectureInputs {
    pub fn input_count(&self) -> usize {
        self.parameters.len() + self.dimensions.len() + self.materials.len() + self.options.len()
    }
}
fn input_error(name: &str, reason: impl std::fmt::Display) -> DocumentError {
    invalid(format!("{name}: {reason}"))
}
fn length_source(value: &Dimension, receiving: &Document) -> Result<(), DocumentError> {
    if !Unit::parse(value.unit()).is_some_and(|u| u.dimension() == parameters::Dimension::LENGTH) {
        return Err(invalid(
            "Exposed construction dimensions require length units",
        ));
    }
    match value {
        Dimension::Literal { value, .. } if !value.is_finite() => {
            return Err(invalid("Exposed dimension must be finite"));
        }
        Dimension::Expression { expression } => {
            for binding in &expression.bindings {
                if !matches!(
                    receiving.get(binding.target).map(|e| &e.data),
                    Some(EntityData::Parameter { .. })
                ) {
                    return Err(invalid(
                        "Exposed dimension references a missing receiving-project parameter",
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}
/// Build a private construction source draft with exposed values applied.
/// This returns no placement, Session preview or native geometry receipt.
/// Compose the role's receiving path/context and generated members before
/// validating the complete transaction and running native preflight.
pub fn configured_source(
    definition: &SymbolDefinition,
    ids: &BTreeMap<EntityId, EntityId>,
    values: &ArchitectureInputs,
    receiving: &Document,
) -> Result<BTreeMap<EntityId, Entity>, DocumentError> {
    if values.input_count() > MAX_SYMBOL_PARAMETERS {
        return Err(invalid("Architectural input budget exceeded"));
    }
    let blueprint = blueprint_for_definition(definition)?
        .ok_or_else(|| invalid("Choose an architectural construction definition"))?;
    for key in values.dimensions.keys() {
        if !blueprint.dimensions.iter().any(|s| s.id == *key) {
            return Err(invalid(
                "Dimension input is private, removed or not exposed by this definition",
            ));
        }
    }
    for key in values.materials.keys() {
        if !blueprint.materials.iter().any(|s| s.id == *key) {
            return Err(invalid(
                "Material input is private, removed or not exposed by this definition",
            ));
        }
    }
    for key in values.options.keys() {
        if !blueprint.options.iter().any(|s| s.id == *key) {
            return Err(invalid(
                "Construction option is private, removed or not exposed by this definition",
            ));
        }
    }
    let mut entities =
        super::super::materialize::mapped_source(definition, ids, &values.parameters)?;
    let root = entities
        .get(&ids[&definition.root])
        .ok_or_else(|| invalid("Private architectural blueprint is missing"))?;
    let EntityData::Extension {
        definition: root, ..
    } = &root.data
    else {
        return Err(invalid("Private architectural blueprint is malformed"));
    };
    let private = ArchitectureBlueprint::from_definition(root)?;
    let entity = entities
        .get(&private.construction)
        .ok_or_else(|| invalid("Private construction is missing"))?;
    let EntityData::Extension {
        definition: source, ..
    } = &entity.data
    else {
        return Err(invalid("Private construction is malformed"));
    };
    let mut construction = ArchitectureConstruction::decoded(private.role, source)?;
    for slot in &blueprint.dimensions {
        if let Some(value) = values.dimensions.get(&slot.id) {
            length_source(value, receiving).map_err(|e| input_error(&slot.name, e))?;
            let target = &private
                .dimensions
                .iter()
                .find(|s| s.id == ids[&slot.id])
                .ok_or_else(|| invalid("Mapped dimension input is missing"))?
                .target;
            *target
                .dimension_mut(&mut construction)
                .map_err(|e| input_error(&slot.name, e))? = value.clone();
        }
    }
    for slot in &blueprint.materials {
        if let Some(value) = values.materials.get(&slot.id) {
            let material = match value {
                ArchitectureMaterialValue::Preset { material } => {
                    if !slot.choices.contains(material) {
                        return Err(input_error(
                            &slot.name,
                            "Material preset is not an authored choice",
                        ));
                    }
                    ids.get(material).copied().ok_or_else(|| {
                        input_error(&slot.name, "Material preset is missing from the pin")
                    })?
                }
                ArchitectureMaterialValue::Project { material } => {
                    if !slot.allow_project_materials {
                        return Err(input_error(
                            &slot.name,
                            "Receiving-project materials are private for this input",
                        ));
                    }
                    if !matches!(
                        receiving.get(*material).map(|e| &e.data),
                        Some(EntityData::Material { .. })
                    ) {
                        return Err(input_error(
                            &slot.name,
                            "Choose a native Material from this project",
                        ));
                    }
                    *material
                }
            };
            let target = &private
                .materials
                .iter()
                .find(|s| s.id == ids[&slot.id])
                .ok_or_else(|| invalid("Mapped material input is missing"))?
                .target;
            *target
                .material_mut(&mut construction)
                .map_err(|e| input_error(&slot.name, e))? = material;
        }
    }
    for slot in &blueprint.options {
        if let Some(value) = values.options.get(&slot.id) {
            let index = slot
                .choices
                .iter()
                .position(|choice| choice == value)
                .ok_or_else(|| {
                    input_error(&slot.name, "Value is not an authored construction option")
                })?;
            let private = private
                .options
                .iter()
                .find(|s| s.id == ids[&slot.id])
                .ok_or_else(|| invalid("Mapped construction option is missing"))?;
            private
                .target
                .apply(&mut construction, &private.choices[index])
                .map_err(|e| input_error(&slot.name, e))?;
        }
    }
    entities
        .get_mut(&private.construction)
        .expect("checked construction")
        .data = construction.entity_data()?;
    Ok(entities)
}

//! Source-only architectural symbol roots. A blueprint owns an interface to a
//! canonical construction; receiving paths and generated physical members are
//! placement inputs and never part of this reusable source.
mod edit;
mod inputs;
mod instances;
mod placement;
mod remap;
mod targets;
mod upgrade;
mod wall_details;
pub use edit::{ArchitectureEditRequest, prepare_architecture_edit};
pub use inputs::{ArchitectureInputs, ArchitectureMaterialValue, configured_source};
pub use instances::{
    ArchitectureInstance, ArchitectureOutput, PLACEMENT_TYPE, architecture_instance_for_output,
    architecture_instances, read_architecture_instance,
};
pub(super) use instances::{private_entities, validate_instances};
pub use placement::{
    ArchitectureContext, ArchitecturePlaceRequest, ArchitecturePrepared, prepare_architecture_place,
};
pub use upgrade::{ArchitectureSourceChange, ArchitectureUpgradeChange};
pub(super) use upgrade::{
    changes as upgrade_changes, compatible_roles, planned_removals, update_edits as upgrade_edits,
};
pub use wall_details::{ArchitectureOpeningTypeRequest, ArchitectureWallDetails};
#[cfg(test)]
mod tests;
use super::{source::invalid, *};
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    model::{self, FenceConstruction, LayeredWallTypeDefinition, WallTypeConstruction},
};
pub(super) use remap::{local_identities, remap_data, validate_provider};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub use targets::*;

pub const BLUEPRINT_TYPE: &str = "symbols.architecture_blueprint";
pub const CAPABILITY: &str = "symbols.architecture.1";
pub const MAX_INPUTS: usize = 128;
pub const MAX_MATERIAL_CHOICES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectureRole {
    Wall,
    Fence,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ArchitectureConstruction {
    Wall(Box<LayeredWallTypeDefinition>),
    Fence(Box<FenceConstruction>),
}
impl ArchitectureConstruction {
    pub(super) fn decoded(
        role: ArchitectureRole,
        definition: &Definition,
    ) -> Result<Self, DocumentError> {
        match role {
            ArchitectureRole::Wall
                if definition.type_id == model::WALL_TYPE_KIND
                    && definition.schema_version == model::LAYERED_WALL_TYPE_SCHEMA =>
            {
                serde_json::from_value(definition.data.clone())
                    .map(|v| Self::Wall(Box::new(v)))
                    .map_err(|e| invalid(e.to_string()))
            }
            ArchitectureRole::Fence
                if definition.type_id == model::FENCE_TYPE_KIND
                    && definition.schema_version == model::FENCE_SCHEMA =>
            {
                serde_json::from_value(definition.data.clone())
                    .map(|v| Self::Fence(Box::new(v)))
                    .map_err(|e| invalid(e.to_string()))
            }
            _ => Err(invalid(
                "Architectural role and construction provider disagree",
            )),
        }
    }
    pub(super) fn entity_data(&self) -> Result<EntityData, DocumentError> {
        let (definition, inputs) = match self {
            Self::Wall(w) => (w.definition()?, w.inputs()),
            Self::Fence(f) => {
                f.validate()?;
                (
                    Definition {
                        type_id: model::FENCE_TYPE_KIND.into(),
                        schema_version: model::FENCE_SCHEMA,
                        data: serde_json::to_value(f).map_err(|e| invalid(e.to_string()))?,
                    },
                    f.inputs(),
                )
            }
        };
        Ok(EntityData::Extension { definition, inputs })
    }
    pub fn role(&self) -> ArchitectureRole {
        match self {
            Self::Wall(_) => ArchitectureRole::Wall,
            Self::Fence(_) => ArchitectureRole::Fence,
        }
    }
    pub fn read(
        document: &Document,
        role: ArchitectureRole,
        id: EntityId,
    ) -> Result<Self, DocumentError> {
        match role {
            ArchitectureRole::Wall => match model::read_wall_construction(document, id)? {
                WallTypeConstruction::Layered(value) => Ok(Self::Wall(Box::new(value))),
                WallTypeConstruction::Uniform(_) => Err(invalid(
                    "Architectural Wall blueprints currently require a layered wall construction",
                )),
            },
            ArchitectureRole::Fence => {
                model::read_fence_type(document, id).map(|value| Self::Fence(Box::new(value)))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureDimensionSlot {
    pub id: EntityId,
    pub name: String,
    pub target: ArchitectureDimensionTarget,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureMaterialSlot {
    pub id: EntityId,
    pub name: String,
    pub target: ArchitectureMaterialTarget,
    /// Authored preset Material identities, included in the pinned closure.
    pub choices: Vec<EntityId>,
    /// Permit a native Material supplied by the receiving project as well.
    pub allow_project_materials: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureOptionSlot {
    pub id: EntityId,
    pub name: String,
    pub target: ArchitectureOptionTarget,
    pub choices: Vec<ArchitectureOptionValue>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureBlueprint {
    pub role: ArchitectureRole,
    pub construction: EntityId,
    #[serde(default)]
    pub dimensions: Vec<ArchitectureDimensionSlot>,
    #[serde(default)]
    pub materials: Vec<ArchitectureMaterialSlot>,
    #[serde(default)]
    pub options: Vec<ArchitectureOptionSlot>,
}
impl ArchitectureBlueprint {
    pub fn new(role: ArchitectureRole, construction: EntityId) -> Self {
        Self {
            role,
            construction,
            dimensions: vec![],
            materials: vec![],
            options: vec![],
        }
    }
    pub fn input_count(&self) -> usize {
        self.dimensions.len() + self.materials.len() + self.options.len()
    }
    pub fn validate_shape(&self) -> Result<(), DocumentError> {
        if self.construction.is_nil() || self.input_count() > MAX_INPUTS {
            return Err(invalid(
                "Architectural blueprint needs a construction and at most 128 exposed inputs",
            ));
        }
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        for (id, name) in self
            .dimensions
            .iter()
            .map(|s| (s.id, &s.name))
            .chain(self.materials.iter().map(|s| (s.id, &s.name)))
            .chain(self.options.iter().map(|s| (s.id, &s.name)))
        {
            if id.is_nil()
                || id == self.construction
                || !ids.insert(id)
                || name.trim().is_empty()
                || name.len() > 256
                || !names.insert(name.to_lowercase())
            {
                return Err(invalid(
                    "Architectural input identities and labels must be nonempty, bounded and distinct",
                ));
            }
        }
        if self
            .dimensions
            .iter()
            .map(|s| &s.target)
            .collect::<BTreeSet<_>>()
            .len()
            != self.dimensions.len()
            || self
                .materials
                .iter()
                .map(|s| &s.target)
                .collect::<BTreeSet<_>>()
                .len()
                != self.materials.len()
            || self
                .options
                .iter()
                .map(|s| &s.target)
                .collect::<BTreeSet<_>>()
                .len()
                != self.options.len()
        {
            return Err(invalid("Expose each architectural input target only once"));
        }
        for slot in &self.materials {
            if slot.choices.len() > MAX_MATERIAL_CHOICES
                || slot.choices.iter().any(EntityId::is_nil)
                || slot.choices.iter().collect::<BTreeSet<_>>().len() != slot.choices.len()
                || (!slot.allow_project_materials && slot.choices.is_empty())
            {
                return Err(invalid(
                    "An architectural material input needs bounded distinct presets or receiving-project materials",
                ));
            }
        }
        for slot in &self.options {
            if slot.choices.is_empty()
                || slot.choices.len() > 32
                || slot
                    .choices
                    .iter()
                    .enumerate()
                    .any(|(i, choice)| slot.choices[..i].contains(choice))
            {
                return Err(invalid(
                    "Architectural option choices must be nonempty, bounded and distinct",
                ));
            }
        }
        Ok(())
    }
    pub fn validate(&self, document: &Document) -> Result<(), DocumentError> {
        self.validate_shape()?;
        let mut value = ArchitectureConstruction::read(document, self.role, self.construction)?;
        for id in self.local_ids() {
            if document.get(id).is_some() || document.id() == id {
                return Err(invalid(
                    "Architectural input identities cannot alias canonical entities",
                ));
            }
        }
        for slot in &self.dimensions {
            slot.target
                .dimension_mut(&mut value)
                .map_err(|e| invalid(format!("{}: {e}", slot.name)))?;
        }
        for slot in &self.materials {
            slot.target
                .material_mut(&mut value)
                .map_err(|e| invalid(format!("{}: {e}", slot.name)))?;
            for id in &slot.choices {
                if !matches!(
                    document.get(*id).map(|e| &e.data),
                    Some(EntityData::Material { .. })
                ) {
                    return Err(invalid(format!(
                        "{} requires native Material presets",
                        slot.name
                    )));
                }
            }
        }
        for slot in &self.options {
            for choice in &slot.choices {
                slot.target
                    .apply(&mut value.clone(), choice)
                    .map_err(|e| invalid(format!("{}: {e}", slot.name)))?;
            }
        }
        Ok(())
    }
    pub fn local_ids(&self) -> Vec<EntityId> {
        self.dimensions
            .iter()
            .map(|s| s.id)
            .chain(self.materials.iter().map(|s| s.id))
            .chain(self.options.iter().map(|s| s.id))
            .collect()
    }
    pub fn dependencies(&self, document: &Document) -> Result<BTreeSet<EntityId>, DocumentError> {
        let mut pending = vec![self.construction];
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            if seen.len() > MAX_SYMBOL_ENTITIES {
                return Err(invalid(
                    "Architectural construction dependencies exceed the symbol budget",
                ));
            }
            pending.extend(
                document
                    .get(id)
                    .ok_or(DocumentError::Missing(id))?
                    .references()
                    .iter()
                    .map(|l| l.target),
            );
        }
        Ok(seen)
    }
    pub fn inputs(&self) -> Vec<Link> {
        let mut links = vec![Link::new(self.construction, "construction")];
        let mut seen = BTreeSet::new();
        for slot in &self.materials {
            for material in &slot.choices {
                if seen.insert(*material) {
                    links.push(Link::new(
                        *material,
                        format!("material_choice:{}", seen.len()),
                    ));
                }
            }
        }
        links
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate_shape()?;
        super::source::encode(BLUEPRINT_TYPE, self)
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        let value: Self = super::source::decode(definition, BLUEPRINT_TYPE)?;
        value.validate_shape()?;
        Ok(value)
    }
}
pub fn is_blueprint(document: &Document, id: EntityId) -> bool {
    matches!(document.get(id).map(|e| &e.data),Some(EntityData::Extension{definition,..}) if definition.type_id == BLUEPRINT_TYPE)
}
pub fn read_blueprint(
    document: &Document,
    id: EntityId,
) -> Result<ArchitectureBlueprint, DocumentError> {
    let Some(EntityData::Extension { definition, inputs }) = document.get(id).map(|e| &e.data)
    else {
        return Err(invalid("Choose an architectural construction blueprint"));
    };
    let value = ArchitectureBlueprint::from_definition(definition)?;
    let expected = value.inputs();
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Architectural blueprint and canonical input links disagree",
        ));
    }
    value.validate(document)?;
    Ok(value)
}
pub fn prepare_blueprint(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    name: &str,
    value: &ArchitectureBlueprint,
) -> Result<(EntityId, Prepared), CommandError> {
    value.validate(session.document())?;
    let entity = Entity::new(
        name,
        EntityData::Extension {
            definition: value.definition()?,
            inputs: value.inputs(),
        },
    );
    let id = entity.id;
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Define architectural symbol interface".into(),
        edits: vec![Mutation::Insert(entity)],
    })?;
    Ok((id, prepared))
}
pub(super) fn validate_blueprints(document: &Document) -> Result<(), DocumentError> {
    for entity in document.entities() {
        if is_blueprint(document, entity.id) {
            read_blueprint(document, entity.id)?;
        }
    }
    Ok(())
}
pub fn blueprint_for_definition(
    definition: &SymbolDefinition,
) -> Result<Option<ArchitectureBlueprint>, DocumentError> {
    if is_blueprint(&definition.template, definition.root) {
        read_blueprint(&definition.template, definition.root).map(Some)
    } else {
        Ok(None)
    }
}
pub fn required_capability(document: &Document) -> bool {
    document.entities().any(|e|matches!(&e.data,EntityData::Extension{definition,..} if definition.type_id==PLACEMENT_TYPE) || is_blueprint(document,e.id) || matches!(&e.data,EntityData::Extension{definition,..} if definition.type_id==DEFINITION_TYPE && serde_json::from_value::<SymbolDefinition>(definition.data.clone()).is_ok_and(|value|is_blueprint(&value.template,value.root))))
}

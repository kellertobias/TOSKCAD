//! Separate instance placement drivers. A driver never depends on its owner,
//! so source geometry and occurrence pose have explicit acyclic dependencies.
mod prepare;
use super::{NumericTransform, RigidPose, common::parameter_links, source::invalid};
use crate::{
    document::{Definition, Document, DocumentError, EntityData, EntityId, Link},
    parameters::EvaluatedParameters,
};
pub use prepare::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const INSTANCE_PLACEMENT_TYPE: &str = "model.instance_placement";
pub const INSTANCE_PLACEMENT_SCHEMA: u32 = 1;
pub const INSTANCE_PLACEMENT_CAPABILITY: &str = "model.instance_placement.1";
pub const MAX_PLACEMENT_CHAIN: usize = 128;
pub const MAX_PLACED_INSTANCES: usize = 4096;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlacementBase {
    Literal { pose: RigidPose },
    Previous { driver: EntityId },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstancePlacementDefinition {
    pub owner: EntityId,
    pub base: PlacementBase,
    pub parameters: NumericTransform,
}
impl InstancePlacementDefinition {
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.owner.is_nil() {
            return Err(invalid("Placement driver needs a non-nil owning instance"));
        }
        match self.base {
            PlacementBase::Literal { pose } => pose.validate()?,
            PlacementBase::Previous { driver } if driver.is_nil() || driver == self.owner => {
                return Err(invalid("Previous placement driver identity is invalid"));
            }
            _ => {}
        }
        self.parameters.validate()
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: INSTANCE_PLACEMENT_TYPE.into(),
            schema_version: INSTANCE_PLACEMENT_SCHEMA,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.type_id != INSTANCE_PLACEMENT_TYPE
            || definition.schema_version != INSTANCE_PLACEMENT_SCHEMA
        {
            return Err(invalid("Unsupported instance-placement provider/version"));
        }
        let source: Self =
            serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))?;
        source.validate()?;
        Ok(source)
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        let mut links = parameter_links(self.parameters.dimensions());
        if let PlacementBase::Previous { driver } = self.base {
            links.push(Link::new(driver, "previous_pose"));
        }
        Ok(links)
    }
}
pub fn read_instance_placement(
    document: &Document,
    driver: EntityId,
) -> Result<InstancePlacementDefinition, DocumentError> {
    let Some(EntityData::Extension { definition, inputs }) = document.get(driver).map(|e| &e.data)
    else {
        return Err(invalid(
            "Placement driver is missing or has the wrong entity kind",
        ));
    };
    let source = InstancePlacementDefinition::from_definition(definition)?;
    let expected = source.inputs()?;
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Instance placement and canonical dependency links disagree",
        ));
    }
    if document
        .get(source.owner)
        .is_some_and(|e| !matches!(e.data, EntityData::Instance { .. }))
    {
        return Err(invalid("Placement driver owner must be an Instance"));
    }
    for link in inputs
        .iter()
        .filter(|link| link.role.starts_with("parameter/"))
    {
        if document
            .get(link.target)
            .is_some_and(|e| !matches!(e.data, EntityData::Parameter { .. }))
        {
            return Err(invalid(
                "Placement parameter dependency has the wrong entity kind",
            ));
        }
    }
    if let PlacementBase::Previous { driver: previous } = source.base
        && let Some(entity) = document.get(previous)
    {
        let EntityData::Extension { definition, .. } = &entity.data else {
            return Err(invalid("Previous pose must be a placement driver"));
        };
        let previous = InstancePlacementDefinition::from_definition(definition)?;
        if previous.owner != source.owner {
            return Err(invalid(
                "A placement-driver chain cannot be shared by different owning instances",
            ));
        }
    }
    Ok(source)
}
pub fn resolve_instance_pose(
    document: &Document,
    instance: EntityId,
    parameters: &EvaluatedParameters,
) -> Result<RigidPose, DocumentError> {
    let Some(EntityData::Instance {
        placement,
        placement_source,
        ..
    }) = document.get(instance).map(|e| &e.data)
    else {
        return Err(invalid("Choose an existing Instance"));
    };
    let Some(mut driver) = *placement_source else {
        return RigidPose::from_placement(*placement);
    };
    let mut visited = BTreeSet::new();
    let mut controls = vec![];
    let base = loop {
        if visited.len() >= MAX_PLACEMENT_CHAIN || !visited.insert(driver) {
            return Err(invalid(
                "Instance placement chain exceeds 128 steps or contains a cycle",
            ));
        }
        let source = read_instance_placement(document, driver)?;
        if source.owner != instance {
            return Err(invalid("Placement driver belongs to a different instance"));
        }
        controls.push(source.parameters);
        match source.base {
            PlacementBase::Literal { pose } => break pose,
            PlacementBase::Previous { driver: previous } => driver = previous,
        }
    };
    controls.into_iter().rev().try_fold(base, |pose, control| {
        pose.then(control.resolve(parameters)?)
    })
}
pub(super) fn check_next_driver(
    document: &Document,
    mut driver: EntityId,
    owner: EntityId,
) -> Result<(), DocumentError> {
    let mut visited = BTreeSet::new();
    loop {
        if visited.len() >= MAX_PLACEMENT_CHAIN - 1 || !visited.insert(driver) {
            return Err(invalid(
                "Adding this placement exceeds the 128-step chain budget or creates a cycle; edit an existing placement instead",
            ));
        }
        let source = read_instance_placement(document, driver)?;
        if source.owner != owner {
            return Err(invalid("Placement driver belongs to another instance"));
        }
        match source.base {
            PlacementBase::Literal { .. } => return Ok(()),
            PlacementBase::Previous { driver: previous } => driver = previous,
        }
    }
}

pub(super) fn validate_document(document: &Document) -> Result<(), DocumentError> {
    for entity in document.entities() {
        match &entity.data {
            EntityData::Extension { definition, .. }
                if definition.type_id == INSTANCE_PLACEMENT_TYPE =>
            {
                read_instance_placement(document, entity.id)?;
            }
            EntityData::Instance {
                placement_source: Some(driver),
                ..
            } if document.get(*driver).is_some()
                && read_instance_placement(document, *driver)?.owner != entity.id =>
            {
                return Err(invalid(
                    "Instance placement source belongs to a different owner",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

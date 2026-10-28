//! Separate instance placement drivers. A driver never depends on its owner,
//! so source geometry and occurrence pose have explicit acyclic dependencies.
mod batch;
mod prepare;
use super::{NumericTransform, RigidPose, common::parameter_links, source::invalid};
use crate::{
    document::{Definition, Document, DocumentError, EntityData, EntityId, Link},
    parameters::EvaluatedParameters,
};
pub use batch::*;
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
        && header(&entity.data)?.0 != source.owner
    {
        return Err(invalid(
            "A placement-driver chain cannot be shared by different owning instances",
        ));
    }
    Ok(source)
}
pub(super) fn header(data: &EntityData) -> Result<(EntityId, PlacementBase), DocumentError> {
    let EntityData::Extension { definition, .. } = data else {
        return Err(invalid(
            "Previous pose must be an instance placement/alignment driver",
        ));
    };
    match definition.type_id.as_str() {
        INSTANCE_PLACEMENT_TYPE => {
            let s = InstancePlacementDefinition::from_definition(definition)?;
            Ok((s.owner, s.base))
        }
        super::INSTANCE_ALIGNMENT_TYPE => {
            let s = super::InstanceAlignmentDefinition::from_definition(definition)?;
            Ok((s.owner, s.base))
        }
        _ => Err(invalid("Unsupported instance placement driver")),
    }
}
#[derive(Clone)]
pub(super) enum Driver {
    Numeric(InstancePlacementDefinition),
    Alignment(
        super::InstanceAlignmentDefinition,
        Vec<super::AlignmentInput>,
    ),
}
impl Driver {
    pub(super) fn read(document: &Document, id: EntityId) -> Result<Self, DocumentError> {
        let entity = document
            .get(id)
            .ok_or_else(|| invalid("Instance driver is missing"))?;
        let EntityData::Extension { definition, .. } = &entity.data else {
            return Err(invalid("Instance driver has the wrong entity kind"));
        };
        match definition.type_id.as_str() {
            INSTANCE_PLACEMENT_TYPE => Ok(Self::Numeric(read_instance_placement(document, id)?)),
            super::INSTANCE_ALIGNMENT_TYPE => {
                let (s, r) = super::read_instance_alignment(document, id)?;
                Ok(Self::Alignment(s, r))
            }
            _ => Err(invalid("Unsupported instance driver")),
        }
    }
    pub(super) fn owner(&self) -> EntityId {
        match self {
            Self::Numeric(s) => s.owner,
            Self::Alignment(s, _) => s.owner,
        }
    }
    pub(super) fn base(&self) -> PlacementBase {
        match self {
            Self::Numeric(s) => s.base.clone(),
            Self::Alignment(s, _) => s.base.clone(),
        }
    }
    pub(super) fn set_base(&mut self, base: PlacementBase) {
        match self {
            Self::Numeric(s) => s.base = base,
            Self::Alignment(s, _) => s.base = base,
        }
    }
    pub(super) fn copy_owner(&mut self, owner: EntityId) {
        match self {
            Self::Numeric(s) => s.owner = owner,
            Self::Alignment(s, refs) => {
                s.owner = owner;
                for input in refs {
                    let old = input.intent.id;
                    let new = EntityId::new();
                    input.intent.id = new;
                    for anchor in [&mut s.parameters.source, &mut s.parameters.target] {
                        if let super::AlignmentAnchor::Geometry { reference } = anchor
                            && *reference == old
                        {
                            *reference = new;
                        }
                    }
                }
            }
        }
    }
    pub(super) fn entity(&self, id: EntityId) -> Result<crate::document::Entity, DocumentError> {
        let (definition, inputs) = match self {
            Self::Numeric(s) => (s.definition()?, s.inputs()?),
            Self::Alignment(s, r) => (s.definition()?, s.inputs(r)?),
        };
        let mut e = crate::document::Entity::new(
            "Copied instance placement",
            EntityData::Extension { definition, inputs },
        );
        e.id = id;
        Ok(e)
    }
}
pub fn resolve_instance_pose(
    document: &Document,
    instance: EntityId,
    parameters: &EvaluatedParameters,
) -> Result<RigidPose, DocumentError> {
    resolve_instance_pose_with_snapshots(
        document,
        instance,
        parameters,
        0,
        &std::collections::BTreeMap::new(),
    )
}
pub fn resolve_instance_pose_with_snapshots(
    document: &Document,
    instance: EntityId,
    parameters: &EvaluatedParameters,
    revision: u64,
    snapshots: &std::collections::BTreeMap<EntityId, crate::references::TopologySnapshot>,
) -> Result<RigidPose, DocumentError> {
    let Some(EntityData::Instance {
        placement,
        placement_source,
        ..
    }) = document.get(instance).map(|e| &e.data)
    else {
        return Err(invalid("Choose an existing Instance"));
    };
    let base = match placement_source {
        Some(driver) => PlacementBase::Previous { driver: *driver },
        None => PlacementBase::Literal {
            pose: RigidPose::from_placement(*placement)?,
        },
    };
    resolve_placement_base_with_snapshots(
        document, instance, &base, parameters, revision, snapshots,
    )
}
/// Evaluate a frozen preceding rigid chain independently of the Instance's
/// later controls. Geometry operations can conjugate into that local frame.
pub fn resolve_placement_base_with_snapshots(
    document: &Document,
    instance: EntityId,
    base: &PlacementBase,
    parameters: &EvaluatedParameters,
    revision: u64,
    snapshots: &std::collections::BTreeMap<EntityId, crate::references::TopologySnapshot>,
) -> Result<RigidPose, DocumentError> {
    let mut driver = match base {
        PlacementBase::Literal { pose } => {
            pose.validate()?;
            return Ok(*pose);
        }
        PlacementBase::Previous { driver } => *driver,
    };
    let mut visited = BTreeSet::new();
    let mut controls = vec![];
    let base = loop {
        if visited.len() >= MAX_PLACEMENT_CHAIN || !visited.insert(driver) {
            return Err(invalid(
                "Instance placement chain exceeds 128 steps or contains a cycle",
            ));
        }
        let source = Driver::read(document, driver)?;
        if source.owner() != instance {
            return Err(invalid("Placement driver belongs to a different instance"));
        }
        let base = source.base();
        controls.push(source);
        match base {
            PlacementBase::Literal { pose } => break pose,
            PlacementBase::Previous { driver: previous } => driver = previous,
        }
    };
    controls.into_iter().rev().try_fold(base, |pose, control| {
        let delta = match control {
            Driver::Numeric(s) => s.parameters.resolve(parameters)?,
            Driver::Alignment(s, r) => super::resolve_alignment(
                document,
                revision,
                parameters,
                &s.parameters,
                &r,
                snapshots,
                pose,
            )?,
        };
        pose.then(delta)
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
        let source = Driver::read(document, driver)?;
        if source.owner() != owner {
            return Err(invalid("Placement driver belongs to another instance"));
        }
        match source.base() {
            PlacementBase::Literal { .. } => return Ok(()),
            PlacementBase::Previous { driver: previous } => driver = previous,
        }
    }
}
pub(super) fn validate_document(document: &Document) -> Result<(), DocumentError> {
    for entity in document.entities() {
        match &entity.data {
            EntityData::Extension { definition, .. }
                if [INSTANCE_PLACEMENT_TYPE, super::INSTANCE_ALIGNMENT_TYPE]
                    .contains(&definition.type_id.as_str()) =>
            {
                Driver::read(document, entity.id)?;
            }
            EntityData::Instance {
                placement_source: Some(driver),
                ..
            } if document.get(*driver).is_some()
                && Driver::read(document, *driver)?.owner() != entity.id =>
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

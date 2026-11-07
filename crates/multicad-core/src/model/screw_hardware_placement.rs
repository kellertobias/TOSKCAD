//! Absolute axis-driven poses for connection-owned outer hardware Instances.
//! Owning identity is validated metadata, never a geometry dependency.
use super::{
    NumericTransform, PlacementBase, RigidPose,
    common::parameter_links,
    length, read_screw_connection_axis,
    source::{check_dimension, invalid},
};
use crate::{
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, EntityData, EntityId, Link},
    parameters::{EvaluatedParameters, Unit},
    references::TopologySnapshot,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub const SCREW_HARDWARE_PLACEMENT_TYPE: &str = "model.screw_hardware_placement";
pub const SCREW_HARDWARE_PLACEMENT_CAPABILITY: &str = "model.screw_hardware_placement.1";
pub const ASSOCIATIVE_SCREW_HARDWARE_CAPABILITY: &str =
    "model.screw_hardware_placement.associative.1";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrewHardwareConnectionBinding {
    pub connection: EntityId,
    /// Stable role key inside the canonical connection record.
    pub role: EntityId,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrewHardwarePlacementDefinition {
    pub owner: EntityId,
    pub axis: EntityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<ScrewHardwareConnectionBinding>,
    pub station: Dimension,
    pub roll: Dimension,
    /// Recognized component-local mounting frame; maps to the axis station.
    pub local_mount: RigidPose,
}
impl ScrewHardwarePlacementDefinition {
    pub fn base(&self) -> PlacementBase {
        PlacementBase::Literal {
            pose: RigidPose::default(),
        }
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.owner.is_nil() || self.axis.is_nil() || self.owner == self.axis {
            return Err(invalid(
                "Hardware placement requires distinct non-nil owner and axis identities",
            ));
        }
        check_dimension(&self.station, Unit::Millimeter)?;
        check_dimension(&self.roll, Unit::Radian)?;
        self.local_mount.validate()?;
        if let Some(binding) = &self.connection {
            if binding.connection.is_nil()
                || binding.role.is_nil()
                || binding.connection == self.owner
                || binding.connection == self.axis
            {
                return Err(invalid(
                    "Associative hardware needs a distinct owning connection and non-nil role key",
                ));
            }
            let zero =
                |d: &Dimension| matches!(d, Dimension::Literal { value, .. } if *value == 0.);
            if !zero(&self.station) || !zero(&self.roll) || self.local_mount != RigidPose::default()
            {
                return Err(invalid(
                    "Associative hardware stores no dormant station, roll or mount authority",
                ));
            }
        }
        Ok(())
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: SCREW_HARDWARE_PLACEMENT_TYPE.into(),
            schema_version: 1,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }
    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.type_id != SCREW_HARDWARE_PLACEMENT_TYPE || definition.schema_version != 1 {
            return Err(invalid(
                "Unsupported screw hardware placement provider/version",
            ));
        }
        let source: Self =
            serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))?;
        source.validate()?;
        Ok(source)
    }
    pub fn inputs(&self) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        let mut inputs = parameter_links(vec![&self.station, &self.roll]);
        inputs.push(Link::new(self.axis, "connection_axis"));
        if let Some(binding) = &self.connection {
            inputs.push(Link::new(binding.connection, "connection_record"));
        }
        Ok(inputs)
    }
    pub fn resolve(
        &self,
        document: &Document,
        parameters: &EvaluatedParameters,
        revision: u64,
        snapshots: &BTreeMap<EntityId, TopologySnapshot>,
    ) -> Result<RigidPose, DocumentError> {
        self.validate()?;
        require_root_hardware(document, self.owner)?;
        if let Some(binding) = &self.connection {
            return super::resolve_connection_hardware_pose(
                document, parameters, binding, self.owner, self.axis, revision, snapshots,
            );
        }
        let (axis, face) = read_screw_connection_axis(document, self.axis)?;
        let axis = axis.resolve(document, parameters, &face, revision, snapshots)?;
        let control = NumericTransform {
            translation: [length(0.), length(0.), self.station.clone()],
            angle: self.roll.clone(),
            ..Default::default()
        }
        .resolve(parameters)?;
        self.local_mount.inverse()?.then(control)?.then(axis)
    }
}
fn require_root_hardware(document: &Document, owner: EntityId) -> Result<(), DocumentError> {
    // Hierarchy folders do not affect physical poses. Placed assembly parents
    // need an explicit retained parent-local conversion, not a second world pose.
    if document.entities().any(|entity| matches!(&entity.data, EntityData::Assembly { instances } if instances.contains(&owner))) {
        return Err(invalid("Connection hardware currently requires a root occurrence; nested hardware needs an explicit parent pose binding"));
    }
    Ok(())
}
pub fn read_screw_hardware_placement(
    document: &Document,
    id: EntityId,
) -> Result<ScrewHardwarePlacementDefinition, DocumentError> {
    let Some(EntityData::Extension { definition, inputs }) = document.get(id).map(|e| &e.data)
    else {
        return Err(invalid(
            "Screw hardware placement is missing or has the wrong entity kind",
        ));
    };
    let source = ScrewHardwarePlacementDefinition::from_definition(definition)?;
    let expected = source.inputs()?;
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Hardware placement and canonical dependency links disagree",
        ));
    }
    if document
        .get(source.owner)
        .is_some_and(|e| !matches!(e.data, EntityData::Instance { .. }))
    {
        return Err(invalid("Hardware placement owner must be an Instance"));
    }
    if document.get(source.axis).is_some() {
        read_screw_connection_axis(document, source.axis)?;
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
                "Hardware station and roll must read actual Parameters",
            ));
        }
    }
    require_root_hardware(document, source.owner)?;
    if let Some(binding) = &source.connection
        && document.get(binding.connection).is_some()
    {
        let record = super::read_screw_connection(document, binding.connection)?;
        let row = record
            .hardware
            .iter()
            .find(|row| row.role_id == binding.role)
            .ok_or_else(|| invalid("Associative driver role is absent from its connection"))?;
        if record.axis != source.axis || row.instance != source.owner || row.driver != id {
            return Err(invalid(
                "Associative hardware role, driver, owner and axis disagree",
            ));
        }
        if document.get(source.owner).is_some_and(|entity|
            !matches!(&entity.data,EntityData::Instance{placement_source:Some(driver),..}if *driver==id)) {
            return Err(invalid("Associative hardware owner no longer uses this driver"));
        }
    }
    Ok(source)
}

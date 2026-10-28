use super::*;
use crate::{
    document::{Document, DocumentError, EntityData, EntityId},
    model::{AlignmentInput, NumericTransform, PlacementBase, RigidPose, length, source::invalid},
    parameters::EvaluatedParameters,
    references::{self, ReferenceFrame, Resolution, TopologySnapshot},
};
use std::collections::{BTreeMap, BTreeSet};
pub fn joint_driver(
    document: &Document,
    instance: EntityId,
) -> Result<Option<EntityId>, DocumentError> {
    let Some(EntityData::Instance {
        placement_source, ..
    }) = document.get(instance).map(|e| &e.data)
    else {
        return Err(invalid("Choose an existing Instance"));
    };
    let mut current = *placement_source;
    let mut visited = BTreeSet::new();
    let mut found = None;
    while let Some(id) = current {
        if visited.len() >= crate::model::MAX_PLACEMENT_CHAIN || !visited.insert(id) {
            return Err(invalid(
                "Joint driver chain contains a cycle or exceeds its budget",
            ));
        }
        let e = document
            .get(id)
            .ok_or_else(|| invalid("Placement driver missing"))?;
        let (owner, base) = if matches!(&e.data,EntityData::Extension{definition,..}if definition.type_id==JOINT_TYPE)
        {
            let (source, _) = read_joint(document, id)?;
            if found.replace(id).is_some() {
                return Err(invalid("An Instance may have only one incoming joint"));
            }
            (source.owner, source.base)
        } else {
            crate::model::placement::header(&e.data)?
        };
        if owner != instance {
            return Err(invalid(
                "Joint/placement driver belongs to another Instance",
            ));
        }
        current = if let PlacementBase::Previous { driver } = base {
            Some(driver)
        } else {
            None
        };
    }
    Ok(found)
}
fn anchor(
    _document: &Document,
    revision: u64,
    anchor: &JointAnchor,
    inputs: &[AlignmentInput],
    parameters: &EvaluatedParameters,
    snapshots: &BTreeMap<EntityId, TopologySnapshot>,
) -> Result<ReferenceFrame, DocumentError> {
    match anchor {
        JointAnchor::Frame { frame } => {
            let f = frame.resolve(parameters)?;
            Ok(ReferenceFrame {
                origin_mm: f.origin,
                u: f.u,
                v: f.v,
            })
        }
        JointAnchor::Geometry { reference } => {
            let input = inputs
                .iter()
                .find(|r| r.intent.id == *reference)
                .ok_or_else(|| invalid("Joint geometry capture is missing"))?;
            let snapshot = snapshots
                .get(&input.source)
                .filter(|s| s.owner == input.source)
                .ok_or_else(|| invalid("Joint source geometry is unavailable at this revision"))?;
            match references::resolve(&input.intent, snapshot, revision) {
                Resolution::Resolved {
                    frame: Some(frame), ..
                } => {
                    frame.validate().map_err(invalid)?;
                    Ok(frame)
                }
                diagnostic => Err(invalid(format!(
                    "Joint anchor needs an explicit stable named frame or repair: {diagnostic:?}"
                ))),
            }
        }
    }
}
fn pose(frame: ReferenceFrame) -> Result<RigidPose, DocumentError> {
    frame.validate().map_err(invalid)?;
    let n = frame.normal();
    Ok(RigidPose {
        translation_mm: frame.origin_mm,
        rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], n[i]]),
    })
}
pub fn resolve_joint(
    document: &Document,
    revision: u64,
    parameters: &EvaluatedParameters,
    source: &JointDefinition,
    inputs: &[AlignmentInput],
    snapshots: &BTreeMap<EntityId, TopologySnapshot>,
    base: RigidPose,
) -> Result<RigidPose, DocumentError> {
    source.inputs(inputs)?;
    let values = source.parameters.resolve(parameters)?;
    let moving = pose(anchor(
        document,
        revision,
        &source.parameters.moving,
        inputs,
        parameters,
        snapshots,
    )?)?;
    let target = pose(anchor(
        document,
        revision,
        &source.parameters.target,
        inputs,
        parameters,
        snapshots,
    )?)?;
    let parent = source
        .parameters
        .parent
        .map(|id| {
            crate::model::resolve_instance_pose_with_snapshots(
                document, id, parameters, revision, snapshots,
            )
        })
        .transpose()?
        .unwrap_or_default();
    let motion = NumericTransform {
        translation: values.translation_mm.map(length),
        angle: crate::constraint_sketch::Dimension::Literal {
            value: values.rotation_rad,
            unit: "rad".into(),
        },
        ..Default::default()
    }
    .resolve(parameters)?;
    let desired = moving.inverse()?.then(motion)?.then(target)?.then(parent)?;
    base.inverse()?.then(desired)
}

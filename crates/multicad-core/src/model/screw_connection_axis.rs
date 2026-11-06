//! Shared screw axis attached to an exact receiving predecessor and occurrence.
//! The axis consumes the captured face and pose suppliers, never a final Part
//! containing its own receiving cut or an aggregate purchased hardware solid.
use super::{
    AlignmentAnchor, AlignmentInput, AlignmentMode, AlignmentParameters, BodyInput, FaceInput,
    OccurrencePath, RigidPose, TransferPoseProjection, angle, common::parameter_links, length,
    source::invalid,
};
use crate::{
    constraint_sketch::Dimension,
    document::{Definition, Document, DocumentError, EntityData, EntityId, Link},
    parameters::EvaluatedParameters,
    references::{ReferenceFrame, TopologySnapshot},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SCREW_CONNECTION_AXIS_TYPE: &str = "model.screw_connection_axis";
pub const SCREW_CONNECTION_AXIS_CAPABILITY: &str = "model.screw_connection_axis.1";
const FACE_ROLE: &str = "screw_axis/face";
const POSE_ROLE: &str = "screw_axis/pose/";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrewConnectionAxisDefinition {
    pub receiver: BodyInput,
    pub occurrence: OccurrencePath,
    /// Identity of the single canonical face capture held in the source links.
    pub face: EntityId,
    pub offset: [Dimension; 3],
    pub roll: Dimension,
    pub reverse: bool,
}

impl ScrewConnectionAxisDefinition {
    pub fn new(receiver: BodyInput, occurrence: OccurrencePath, face: EntityId) -> Self {
        Self {
            receiver,
            occurrence,
            face,
            offset: std::array::from_fn(|_| length(0.)),
            roll: angle(0.),
            reverse: false,
        }
    }

    fn control(&self) -> AlignmentParameters {
        AlignmentParameters {
            source: AlignmentAnchor::Frame {
                frame: ReferenceFrame {
                    origin_mm: [0.; 3],
                    u: [1., 0., 0.],
                    v: [0., 1., 0.],
                },
            },
            target: AlignmentAnchor::Geometry {
                reference: self.face,
            },
            mode: AlignmentMode::Frame,
            offset: self.offset.clone(),
            roll: self.roll.clone(),
            opposed: self.reverse,
            up: [1., 0., 0.],
        }
    }

    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.receiver.body.is_nil()
            || self.receiver.producer.is_nil()
            || self.receiver.body == self.receiver.producer
            || self.face.is_nil()
            || self.occurrence.root.is_nil()
            || self.occurrence.members.len() > 32
            || self.occurrence.members.iter().any(|id| id.is_nil())
            || self
                .occurrence
                .members
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != self.occurrence.members.len()
        {
            return Err(invalid(
                "A screw axis needs an explicit receiving predecessor, face and bounded occurrence path",
            ));
        }
        self.control().validate()
    }

    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        Ok(Definition {
            type_id: SCREW_CONNECTION_AXIS_TYPE.into(),
            schema_version: 1,
            data: serde_json::to_value(self).map_err(|e| invalid(e.to_string()))?,
        })
    }

    pub fn from_definition(definition: &Definition) -> Result<Self, DocumentError> {
        if definition.type_id != SCREW_CONNECTION_AXIS_TYPE || definition.schema_version != 1 {
            return Err(invalid("Unsupported screw-axis provider or version"));
        }
        let source: Self =
            serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))?;
        source.validate()?;
        Ok(source)
    }

    pub fn inputs(&self, face: &FaceInput) -> Result<Vec<Link>, DocumentError> {
        self.validate()?;
        super::validate_profile_face(face)?;
        if face.source != self.receiver.producer || face.intent.id != self.face {
            return Err(invalid(
                "The screw-axis face must belong to its exact receiving predecessor",
            ));
        }
        let mut links = parameter_links(self.offset.iter().chain([&self.roll]).collect::<Vec<_>>());
        links.push(Link {
            target: face.source,
            role: FACE_ROLE.into(),
            geometry: Some(face.intent.clone()),
        });
        for (index, id) in std::iter::once(self.occurrence.root)
            .chain(self.occurrence.members.iter().copied())
            .enumerate()
        {
            links.push(Link::new(id, format!("{POSE_ROLE}{index}")));
        }
        Ok(links)
    }

    /// Resolve the named predecessor face in component space, then apply only
    /// the actual occurrence pose. Supplied topology must be current and owned
    /// by that predecessor. Missing, ambiguous or stale captures require repair.
    pub fn resolve(
        &self,
        document: &Document,
        parameters: &EvaluatedParameters,
        face: &FaceInput,
        revision: u64,
        snapshots: &BTreeMap<EntityId, TopologySnapshot>,
    ) -> Result<RigidPose, DocumentError> {
        self.inputs(face)?;
        self.receiver.resolve_producer(document)?;
        if !matches!(
            document
                .get(self.receiver.producer)
                .map(|entity| &entity.data),
            Some(EntityData::Feature {
                suppressed: false,
                ..
            })
        ) {
            return Err(invalid(
                "The screw-axis receiving predecessor is suppressed",
            ));
        }
        let local = super::resolve_alignment(
            document,
            revision,
            parameters,
            &self.control(),
            &[AlignmentInput {
                source: face.source,
                intent: face.intent.clone(),
            }],
            snapshots,
            RigidPose::default(),
        )?;
        if !matches!(
            snapshots
                .get(&face.source)
                .map(|snapshot| crate::references::resolve(&face.intent, snapshot, revision)),
            Some(crate::references::Resolution::Resolved {
                geometry: Some(crate::references::AnalyticGeometry::Plane { .. }),
                ..
            })
        ) {
            return Err(invalid(
                "The screw axis requires a current native planar receiving face",
            ));
        }
        local.then(super::resolve_occurrence_pose(
            document,
            &self.occurrence,
            self.receiver.body,
            parameters,
            revision,
            snapshots,
        )?)
    }
}

pub fn read_screw_connection_axis(
    document: &Document,
    id: EntityId,
) -> Result<(ScrewConnectionAxisDefinition, FaceInput), DocumentError> {
    let Some(EntityData::Extension { definition, inputs }) = document.get(id).map(|e| &e.data)
    else {
        return Err(invalid(
            "The screw-axis source is missing or has the wrong kind",
        ));
    };
    let source = ScrewConnectionAxisDefinition::from_definition(definition)?;
    let mut faces = inputs.iter().filter(|link| link.role == FACE_ROLE);
    let (Some(link), None) = (faces.next(), faces.next()) else {
        return Err(invalid(
            "A screw axis needs exactly one receiving face capture",
        ));
    };
    let face = FaceInput {
        source: link.target,
        intent: link
            .geometry
            .clone()
            .ok_or_else(|| invalid("The screw-axis face capture is missing"))?,
    };
    let expected = source.inputs(&face)?;
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Screw-axis source and canonical dependency links disagree",
        ));
    }
    Ok((source, face))
}

pub fn screw_connection_axis_pose_projection(
    document: &Document,
    id: EntityId,
) -> Result<Option<TransferPoseProjection>, DocumentError> {
    if !matches!(document.get(id).map(|e| &e.data), Some(EntityData::Extension { definition, .. })
        if definition.type_id == SCREW_CONNECTION_AXIS_TYPE)
    {
        return Ok(None);
    }
    let (source, _) = read_screw_connection_axis(document, id)?;
    super::occurrence_pose_projection(document, &source.occurrence, None).map(Some)
}

pub(crate) fn screw_axis_evaluation_dependencies(
    document: &Document,
    id: EntityId,
) -> Result<Option<BTreeSet<EntityId>>, DocumentError> {
    let Some(projection) = screw_connection_axis_pose_projection(document, id)? else {
        return Ok(None);
    };
    // Only a validated typed source can project these exact occurrence links.
    // Face geometry and parameter inputs remain actual geometry dependencies.
    let mut dependencies = document
        .get(id)
        .expect("validated axis")
        .references()
        .into_iter()
        .filter(|link| !link.role.starts_with(POSE_ROLE))
        .map(|link| link.target)
        .collect::<BTreeSet<_>>();
    dependencies.extend(projection.geometry_dependencies);
    Ok(Some(dependencies))
}

pub(super) fn validate_document(document: &Document) -> Result<(), DocumentError> {
    for entity in document.entities() {
        if matches!(&entity.data, EntityData::Extension { definition, .. }
            if definition.type_id == SCREW_CONNECTION_AXIS_TYPE)
        {
            read_screw_connection_axis(document, entity.id)?;
        }
    }
    Ok(())
}

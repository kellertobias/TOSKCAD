//! A single source transaction for explicit mixed definition/occurrence edits.
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    document::{EntityData, EntityId},
    model::{
        self, AlignmentRequest, CreatedBody, InstanceAlignmentRequest, InstanceCopyRequest,
        InstanceMoveRequest, MAX_TRANSFORM_TARGETS, TransformRequest,
        common::{error, revision},
    },
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementBatchRequest {
    #[serde(default)]
    pub transforms: Vec<TransformRequest>,
    #[serde(default)]
    pub alignments: Vec<AlignmentRequest>,
    #[serde(default)]
    pub instances: Vec<InstanceMoveRequest>,
    #[serde(default)]
    pub instance_alignments: Vec<InstanceAlignmentRequest>,
    #[serde(default)]
    pub copies: Vec<InstanceCopyRequest>,
}
pub struct PlacementBatchPrepared {
    pub prepared: Prepared,
    pub outputs: Vec<CreatedBody>,
    pub instances: Vec<EntityId>,
}
impl PlacementBatchRequest {
    fn validate(&self, session: &Session) -> Result<(), CommandError> {
        let count = self.transforms.len()
            + self.alignments.len()
            + self.instances.len()
            + self.instance_alignments.len()
            + self.copies.len();
        if !(1..=MAX_TRANSFORM_TARGETS).contains(&count) {
            return Err(error("Choose 1–128 explicit placement targets"));
        }
        if !self.copies.is_empty() && self.copies.len() != count {
            return Err(error(
                "Copy and move/alignment must be separate explicit actions",
            ));
        }
        let mut bodies = BTreeSet::new();
        for body in self
            .transforms
            .iter()
            .map(|r| r.input.body)
            .chain(self.alignments.iter().map(|r| r.input.body))
        {
            if !bodies.insert(body) {
                return Err(error(
                    "A Part cannot be moved and aligned twice in the same action",
                ));
            }
        }
        let mut instances = BTreeSet::new();
        for instance in self
            .instances
            .iter()
            .map(|r| r.instance)
            .chain(self.instance_alignments.iter().map(|r| r.instance))
        {
            if !instances.insert(instance) {
                return Err(error(
                    "An Instance cannot be moved and aligned twice in the same action",
                ));
            }
            if let Some(EntityData::Instance { component, .. }) =
                session.document().get(instance).map(|e| &e.data)
                && bodies.contains(component)
            {
                return Err(error(
                    "Choose the Part definition or its Instance, not both: a Part edit already moves the geometry of every linked Instance",
                ));
            }
        }
        Ok(())
    }
}
/// Prepare against the caller's current revision. Internal derived Sessions
/// only combine source edits; the caller receives one attributable transaction
/// and must validate the complete candidate using the native model evaluator.
pub fn prepare_placement_batch(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &PlacementBatchRequest,
) -> Result<PlacementBatchPrepared, CommandError> {
    revision(session, expected_revision)?;
    request.validate(session)?;
    let mut candidate = session.document().clone();
    let mut outputs = vec![];
    let mut instances = vec![];
    if !request.transforms.is_empty() {
        let stage = Session::new(candidate)?;
        let result = model::prepare_transforms(
            &stage,
            stage.revision(),
            actor.clone(),
            &request.transforms,
        )?;
        candidate = result.prepared.document().clone();
        outputs.extend(result.outputs);
    }
    if !request.alignments.is_empty() {
        let stage = Session::new(candidate)?;
        let result = model::prepare_alignments(
            &stage,
            stage.revision(),
            actor.clone(),
            &request.alignments,
        )?;
        candidate = result.prepared.document().clone();
        outputs.extend(result.outputs);
    }
    if !request.instances.is_empty() {
        let stage = Session::new(candidate)?;
        let result = model::prepare_instance_moves(
            &stage,
            stage.revision(),
            actor.clone(),
            &request.instances,
        )?;
        candidate = result.prepared.document().clone();
        instances.extend(result.instances);
    }
    if !request.instance_alignments.is_empty() {
        let stage = Session::new(candidate)?;
        let result = model::prepare_instance_alignments(
            &stage,
            stage.revision(),
            actor.clone(),
            &request.instance_alignments,
        )?;
        candidate = result.prepared.document().clone();
        instances.extend(result.instances);
    }
    if !request.copies.is_empty() {
        let stage = Session::new(candidate)?;
        let result = model::prepare_instance_copies(
            &stage,
            stage.revision(),
            actor.clone(),
            &request.copies,
        )?;
        candidate = result.prepared.document().clone();
        instances.extend(result.instances);
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: if request.copies.is_empty() {
            "Place selected objects"
        } else {
            "Create placed copies"
        }
        .into(),
        edits: model::extrude_boolean::delta(session.document(), &candidate)?,
    })?;
    if prepared.document() != &candidate {
        return Err(error(
            "Placement preparation changed the derived source candidate",
        ));
    }
    Ok(PlacementBatchPrepared {
        prepared,
        outputs,
        instances,
    })
}

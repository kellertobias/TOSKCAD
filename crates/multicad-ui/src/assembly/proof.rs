use crate::viewport::Scene;
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{self, AssemblyLeafReceipt, OccurrencePath, RigidPose},
    parameters,
    project_file::ContentHash,
};
use std::collections::{BTreeMap, BTreeSet};
/// Compare native leaf coverage against the authored component graph. Source
/// traversal defines expectations; only the native receipt supplies results.
pub(crate) fn validate(document: &Document, revision: u64, scene: &Scene) -> Result<(), String> {
    let receipt = scene
        .assembly_evaluation()
        .ok_or("Assembly Apply requires a native leaf-occurrence receipt")?;
    receipt.validate().map_err(|e| e.to_string())?;
    if receipt.source_hash != ContentHash::document(document)? {
        return Err("Native leaf receipt belongs to different candidate source".into());
    }
    let parameters = parameters::evaluate(document).map_err(|e| e.to_string())?;
    let members = model::assembly_member_instances(document).map_err(|e| e.to_string())?;
    let definitions = model::assembly_definition_parts(document).map_err(|e| e.to_string())?;
    let snapshots = scene.reference_snapshots();
    let mut expected = BTreeMap::new();
    let consumed = &scene
        .model_evaluation()
        .ok_or("Missing native evaluation metadata")?
        .consumed_bodies;
    let mut context = Walk {
        document,
        parameters: &parameters,
        revision,
        snapshots: &snapshots,
        expected: &mut expected,
        visited: BTreeSet::new(),
    };
    for e in document.entities() {
        match e.data {
            EntityData::Part { .. }
                if !definitions.contains(&e.id)
                    && !consumed.contains(&e.id)
                    && model::scale_component_owner(document, e.id)
                        .map_err(|e| e.to_string())?
                        .is_none() =>
            {
                context.component(
                    e.id,
                    OccurrencePath {
                        root: e.id,
                        members: vec![],
                    },
                    RigidPose::default(),
                )?
            }
            EntityData::Instance { component, .. } if !members.contains(&e.id) => {
                let pose = model::resolve_instance_pose_with_snapshots(
                    document,
                    e.id,
                    &parameters,
                    revision,
                    &snapshots,
                )
                .map_err(|e| e.to_string())?;
                context.component(
                    component,
                    OccurrencePath {
                        root: e.id,
                        members: vec![],
                    },
                    pose,
                )?;
            }
            EntityData::Assembly{ref instances} if !instances.is_empty()&&!document.entities().any(|other|matches!(other.data,EntityData::Instance{component,..}if component==e.id))&&!document.entities().any(|other|model::read_assembly(document,other.id).is_ok_and(|source|source.assembly==e.id))=>{
                context.component(e.id,OccurrencePath{root:e.id,members:vec![]},RigidPose::default())?;
            }
            _ => {}
        }
    }
    if expected.len() != receipt.leaves.len() {
        return Err("Native leaf receipt omits or adds physical occurrences".into());
    }
    for actual in &receipt.leaves {
        let wanted = expected
            .get(&actual.path)
            .ok_or("Native leaf receipt contains an unexpected occurrence path")?;
        if actual.component != wanted.component
            || actual.producer != wanted.producer
            || !pose_matches(actual.pose, wanted.pose)
        {
            return Err(
                "Native leaf receipt disagrees with the requested component, producer or pose"
                    .into(),
            );
        }
    }
    Ok(())
}
struct Walk<'a> {
    document: &'a Document,
    parameters: &'a parameters::EvaluatedParameters,
    revision: u64,
    snapshots: &'a BTreeMap<EntityId, multicad_core::references::TopologySnapshot>,
    expected: &'a mut BTreeMap<OccurrencePath, AssemblyLeafReceipt>,
    visited: BTreeSet<EntityId>,
}
impl Walk<'_> {
    fn component(
        &mut self,
        component: EntityId,
        path: OccurrencePath,
        pose: RigidPose,
    ) -> Result<(), String> {
        if path.members.len() > 32
            || self.expected.len() >= model::MAX_ASSEMBLY_LEAVES
            || !self.visited.insert(component)
        {
            return Err(
                "Assembly component graph contains a cycle or exceeds its leaf/depth budget".into(),
            );
        }
        match self.document.get(component).map(|e| &e.data) {
            Some(EntityData::Part { features, .. }) => {
                let producer = features
                    .iter()
                    .rev()
                    .copied()
                    .find(|id| {
                        matches!(
                            self.document.get(*id).map(|e| &e.data),
                            Some(EntityData::Feature {
                                suppressed: false,
                                ..
                            })
                        )
                    })
                    .ok_or("Assembly leaf Part has no active producer")?;
                self.expected.insert(
                    path.clone(),
                    AssemblyLeafReceipt {
                        path,
                        component,
                        producer,
                        pose,
                    },
                );
            }
            Some(EntityData::Assembly { instances }) => {
                for instance in instances {
                    let Some(EntityData::Instance { component, .. }) =
                        self.document.get(*instance).map(|e| &e.data)
                    else {
                        return Err("Assembly member is not an Instance".into());
                    };
                    let mut child = path.clone();
                    child.members.push(*instance);
                    let local = model::resolve_instance_pose_with_snapshots(
                        self.document,
                        *instance,
                        self.parameters,
                        self.revision,
                        self.snapshots,
                    )
                    .map_err(|e| e.to_string())?;
                    self.component(
                        *component,
                        child,
                        local.then(pose).map_err(|e| e.to_string())?,
                    )?;
                }
            }
            _ => return Err("Assembly component definition is missing".into()),
        }
        self.visited.remove(&component);
        Ok(())
    }
}
fn pose_matches(a: RigidPose, b: RigidPose) -> bool {
    a.translation_mm
        .iter()
        .zip(b.translation_mm)
        .all(|(a, b)| (a - b).abs() <= 1e-8)
        && a.rotation
            .iter()
            .flatten()
            .zip(b.rotation.iter().flatten())
            .all(|(a, b)| (a - b).abs() <= 1e-9)
}

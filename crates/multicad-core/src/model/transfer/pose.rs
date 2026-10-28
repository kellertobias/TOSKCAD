//! Pose-only dependencies are explicit semantic links, never geometry inputs
//! to a whole assembly containing the receiving feature itself.
use super::*;
use crate::{
    document::{Document, DocumentError, Entity, EntityData, EntityId},
    model::{self, OccurrencePath, PlacementBase, RigidPose, source::invalid},
    parameters::EvaluatedParameters,
    project_file::ContentHash,
    references::TopologySnapshot,
};
use std::collections::{BTreeMap, BTreeSet};
pub struct TransferPoseProjection {
    pub geometry_dependencies: BTreeSet<EntityId>,
    pub fingerprint: ContentHash,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TransferOccurrence {
    pub path: OccurrencePath,
    pub component: EntityId,
    pub producer: EntityId,
}
/// Authored physical leaf identities/current heads, independent of aggregate
/// geometry. These choices are not a native preview or valid cut receipt.
pub fn transfer_occurrence_catalog(
    document: &Document,
) -> Result<Vec<TransferOccurrence>, DocumentError> {
    let parts = crate::exchange::resolve_parts(document, &Default::default())
        .map_err(|e| invalid(e.to_string()))?;
    let mut output = vec![];
    for part in parts {
        let reference = part.reference;
        let path = if reference.occurrence_path.is_empty() {
            OccurrencePath {
                root: reference.part,
                members: vec![],
            }
        } else if let Some(root) = reference.root_assembly {
            let first = reference.occurrence_path[0];
            if matches!(document.get(first).map(|e|&e.data),Some(EntityData::Instance{component,..})if *component==root)
            {
                OccurrencePath {
                    root: first,
                    members: reference.occurrence_path[1..].to_vec(),
                }
            } else {
                OccurrencePath {
                    root,
                    members: reference.occurrence_path,
                }
            }
        } else {
            OccurrencePath {
                root: reference.occurrence_path[0],
                members: reference.occurrence_path[1..].to_vec(),
            }
        };
        let Some(EntityData::Part { features, .. }) = document.get(reference.part).map(|e| &e.data)
        else {
            continue;
        };
        let Some(producer) = features.iter().rev().find(|id| {
            matches!(
                document.get(**id).map(|e| &e.data),
                Some(EntityData::Feature {
                    suppressed: false,
                    ..
                })
            )
        }) else {
            continue;
        };
        validate_transfer_path(document, &path, reference.part)?;
        output.push(TransferOccurrence {
            path,
            component: reference.part,
            producer: *producer,
        });
    }
    Ok(output)
}
fn raw_steps(document: &Document, path: &OccurrencePath) -> Vec<EntityId> {
    if matches!(
        document.get(path.root).map(|e| &e.data),
        Some(EntityData::Assembly { .. } | EntityData::Part { .. })
    ) {
        path.members.clone()
    } else {
        std::iter::once(path.root)
            .chain(path.members.iter().copied())
            .collect()
    }
}
fn suffixes<'a>(
    a: &'a [EntityId],
    b: &'a [EntityId],
    same_root: bool,
) -> (&'a [EntityId], &'a [EntityId]) {
    let common = if same_root {
        a.iter().zip(b).take_while(|(a, b)| a == b).count()
    } else {
        0
    };
    (&a[common..], &b[common..])
}
/// Validate identity and membership without demanding evaluated geometry. A
/// broken saved path remains readable; new/edited commands call this preflight.
pub fn validate_transfer_path(
    document: &Document,
    path: &OccurrencePath,
    component: EntityId,
) -> Result<Vec<EntityId>, DocumentError> {
    let mut steps = vec![];
    let mut current = match document.get(path.root).map(|e| &e.data) {
        Some(EntityData::Part { .. }) if path.members.is_empty() && path.root == component => {
            return Ok(steps);
        }
        Some(EntityData::Instance { component, .. }) => {
            steps.push(path.root);
            *component
        }
        Some(EntityData::Assembly { .. }) => path.root,
        _ => {
            return Err(invalid(
                "Transfer occurrence root is missing/replaced; explicitly repair its stored path",
            ));
        }
    };
    for member in &path.members {
        let Some(EntityData::Assembly { instances }) = document.get(current).map(|e| &e.data)
        else {
            return Err(invalid(
                "Transfer path no longer expands through its stored Assembly; repair the binding",
            ));
        };
        if !instances.contains(member) {
            return Err(invalid(
                "Transfer member no longer belongs to the stored Assembly; explicitly repair, no nearest-part fallback",
            ));
        }
        let Some(EntityData::Instance { component, .. }) = document.get(*member).map(|e| &e.data)
        else {
            return Err(invalid("Transfer member Instance is missing or replaced"));
        };
        steps.push(*member);
        current = *component;
    }
    if current != component
        || !matches!(
            document.get(current).map(|e| &e.data),
            Some(EntityData::Part { .. })
        )
    {
        return Err(invalid(
            "Transfer path resolves to a different source/receiving Part; repair its component binding",
        ));
    }
    Ok(steps)
}
pub fn resolve_transfer_relative_pose(
    document: &Document,
    source: &TransferSource,
    target: &TransferTarget,
    parameters: &EvaluatedParameters,
    revision: u64,
    snapshots: &BTreeMap<EntityId, TopologySnapshot>,
) -> Result<RigidPose, DocumentError> {
    let a = validate_transfer_path(document, &source.path, source.component)?;
    let b = validate_transfer_path(document, &target.path, target.input.body)?;
    let (a, b) = suffixes(&a, &b, source.path.root == target.path.root);
    let resolve = |steps: &[EntityId]| {
        steps.iter().try_fold(RigidPose::default(), |parent, id| {
            let local = model::resolve_instance_pose_with_snapshots(
                document, *id, parameters, revision, snapshots,
            )?;
            local.then(parent)
        })
    };
    resolve(a)?.then(resolve(b)?.inverse()?)
}
enum Job {
    Instance(EntityId),
    Driver(EntityId),
}
struct Collector<'a> {
    document: &'a Document,
    geometry: BTreeSet<EntityId>,
    poses: BTreeMap<EntityId, Option<&'a Entity>>,
    structure: BTreeMap<EntityId, serde_json::Value>,
    instances: BTreeSet<EntityId>,
    drivers: BTreeSet<EntityId>,
    pending: Vec<Job>,
}
impl Collector<'_> {
    fn structural(&mut self, id: EntityId) -> Result<(), DocumentError> {
        let mut pending = vec![id];
        while let Some(id) = pending.pop() {
            if self.structure.contains_key(&id) {
                continue;
            }
            if self.structure.len() >= 8192 {
                return Err(invalid(
                    "Transfer structural dependency closure exceeds its 8192-source budget",
                ));
            }
            let value = match self.document.get(id).map(|e| &e.data) {
                Some(EntityData::Instance { component, .. }) => {
                    serde_json::json!({"instance":id,"component":component})
                }
                Some(EntityData::Assembly { instances }) => {
                    serde_json::json!({"assembly":id,"members":instances})
                }
                Some(EntityData::Part { .. }) => serde_json::json!({"part":id}),
                Some(other) => serde_json::json!({"replaced":other.kind()}),
                None => serde_json::Value::Null,
            };
            self.structure.insert(id, value);
            if let Some(EntityData::Instance { component, .. }) =
                self.document.get(id).map(|e| &e.data)
            {
                pending.push(*component);
            }
        }
        Ok(())
    }
    fn previous(&mut self, base: &PlacementBase) {
        if let PlacementBase::Previous { driver } = base {
            self.pending.push(Job::Driver(*driver));
        }
    }
    fn run(&mut self) -> Result<(), DocumentError> {
        while let Some(job) = self.pending.pop() {
            if self.instances.len() + self.drivers.len() > 8192 || self.structure.len() > 8192 {
                return Err(invalid(
                    "Transfer pose dependency closure exceeds its 8192-source budget",
                ));
            }
            match job {
                Job::Instance(id) => {
                    if !self.instances.insert(id) {
                        continue;
                    }
                    self.structural(id)?;
                    self.poses.insert(id, self.document.get(id));
                    if let Some(EntityData::Instance {
                        placement_source: Some(driver),
                        ..
                    }) = self.document.get(id).map(|e| &e.data)
                    {
                        self.pending.push(Job::Driver(*driver));
                    }
                }
                Job::Driver(id) => {
                    if !self.drivers.insert(id) {
                        continue;
                    }
                    let entity = self.document.get(id);
                    self.poses.insert(id, entity);
                    let Some(EntityData::Extension { definition, inputs }) =
                        entity.map(|e| &e.data)
                    else {
                        continue;
                    };
                    let known = match definition.type_id.as_str() {
                        model::INSTANCE_PLACEMENT_TYPE => {
                            if let Ok(source) =
                                model::InstancePlacementDefinition::from_definition(definition)
                            {
                                self.previous(&source.base);
                                true
                            } else {
                                false
                            }
                        }
                        model::INSTANCE_ALIGNMENT_TYPE => {
                            if let Ok(source) =
                                model::InstanceAlignmentDefinition::from_definition(definition)
                            {
                                self.previous(&source.base);
                                true
                            } else {
                                false
                            }
                        }
                        model::JOINT_TYPE => {
                            if let Ok(source) = model::JointDefinition::from_definition(definition)
                            {
                                self.previous(&source.base);
                                if let Some(parent) = source.parameters.parent {
                                    self.pending.push(Job::Instance(parent));
                                }
                                true
                            } else {
                                false
                            }
                        }
                        _ => false,
                    };
                    for link in inputs {
                        if !known
                            || link.geometry.is_some()
                            || link.role.starts_with("parameter/")
                            || link.role.ends_with("/plane")
                        {
                            self.geometry.insert(link.target);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
/// Stored typed group/links are checked before deferring anything. Missing or
/// replaced occurrence paths remain fingerprinted and fail native binding, not
/// file loading. Dynamic driver suppliers are discovered from the current doc.
pub fn transfer_pose_projection(
    document: &Document,
    entity: EntityId,
) -> Result<Option<TransferPoseProjection>, DocumentError> {
    if !matches!(document.get(entity).map(|e|&e.data),Some(EntityData::Extension{definition,..})if definition.type_id==TRANSFER_GROUP_TYPE)
    {
        return Ok(None);
    }
    let (source, _) = read_transfer(document, entity)?;
    let mut collector = Collector {
        document,
        geometry: BTreeSet::new(),
        poses: BTreeMap::new(),
        structure: BTreeMap::new(),
        instances: BTreeSet::new(),
        drivers: BTreeSet::new(),
        pending: vec![],
    };
    let a = raw_steps(document, &source.source.path);
    for target in &source.targets {
        for path in [&source.source.path, &target.path] {
            for id in std::iter::once(path.root).chain(path.members.iter().copied()) {
                collector.structural(id)?;
            }
        }
        let b = raw_steps(document, &target.path);
        let (a, b) = suffixes(&a, &b, source.source.path.root == target.path.root);
        collector
            .pending
            .extend(a.iter().chain(b).copied().map(Job::Instance));
    }
    collector.run()?;
    let bytes = serde_json::to_vec(&(collector.structure, collector.poses))
        .map_err(|e| invalid(e.to_string()))?;
    Ok(Some(TransferPoseProjection {
        geometry_dependencies: collector.geometry,
        fingerprint: ContentHash::of(&bytes),
    }))
}
pub fn transfer_evaluation_dependencies(
    document: &Document,
    entity: EntityId,
) -> Result<BTreeSet<EntityId>, DocumentError> {
    let Some(owner) = document.get(entity) else {
        return Ok(BTreeSet::new());
    };
    let projection = transfer_pose_projection(document, entity)?;
    let mut dependencies = owner
        .references()
        .into_iter()
        .filter(|link| projection.is_none() || !link.role.starts_with(TRANSFER_POSE_ROLE))
        .map(|link| link.target)
        .collect::<BTreeSet<_>>();
    if let Some(projection) = projection {
        dependencies.extend(projection.geometry_dependencies);
    }
    Ok(dependencies)
}

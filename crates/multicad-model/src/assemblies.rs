//! Exact reusable component compounds and canonical leaf-path geometry.
use crate::{ModelError, OccurrenceSnapshot, error};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{OccurrencePath, RigidPose},
    references::TopologySnapshot,
};
use multicad_kernel::{Bounds, GeometryPolicy, RigidTransform, Solid};
use std::{collections::BTreeMap, sync::Arc};
#[derive(Clone, Debug)]
pub struct LeafOccurrenceSnapshot {
    pub path: OccurrencePath,
    pub component: EntityId,
    pub producer: EntityId,
    pub pose: RigidPose,
    pub bounds: Bounds,
    pub topology: TopologySnapshot,
}
#[derive(Default)]
pub(crate) struct LeafStore {
    pub snapshots: BTreeMap<OccurrencePath, Arc<LeafOccurrenceSnapshot>>,
    pub solids: BTreeMap<OccurrencePath, Solid>,
}
impl LeafStore {
    fn insert(
        &mut self,
        snapshot: LeafOccurrenceSnapshot,
        solid: Option<Solid>,
    ) -> Result<(), ModelError> {
        if snapshot.path.members.len() > 32
            || self.snapshots.len() >= multicad_core::model::MAX_ASSEMBLY_LEAVES
        {
            return Err(error(
                Some(snapshot.path.root),
                format!(
                    "Assembly expansion exceeds the 32-level/{} source-leaf budget",
                    multicad_core::model::MAX_ASSEMBLY_LEAVES
                ),
            ));
        }
        let key = snapshot.path.clone();
        if self
            .snapshots
            .insert(key.clone(), Arc::new(snapshot))
            .is_some()
        {
            return Err(error(
                Some(key.root),
                "Assembly expansion produced a duplicate canonical leaf path",
            ));
        }
        if let Some(solid) = solid {
            self.solids.insert(key, solid);
        }
        Ok(())
    }
    pub(crate) fn part(
        &mut self,
        root: EntityId,
        component: EntityId,
        producer: EntityId,
        pose: RigidPose,
        bounds: Bounds,
        mut topology: TopologySnapshot,
    ) -> Result<(), ModelError> {
        topology.owner = root;
        self.insert(
            LeafOccurrenceSnapshot {
                path: OccurrencePath {
                    root,
                    members: vec![],
                },
                component,
                producer,
                pose,
                bounds,
                topology,
            },
            None,
        )
    }
    pub(crate) fn instance(
        &mut self,
        document: &Document,
        occurrence: &OccurrenceSnapshot,
        revision: u64,
    ) -> Result<(), ModelError> {
        if matches!(
            document.get(occurrence.component).map(|e| &e.data),
            Some(EntityData::Part { .. })
        ) {
            return self.part(
                occurrence.instance,
                occurrence.component,
                occurrence.producer,
                occurrence.pose,
                occurrence.bounds,
                occurrence.topology.clone(),
            );
        }
        let source = self
            .snapshots
            .values()
            .filter(|leaf| leaf.path.root == occurrence.component)
            .cloned()
            .collect::<Vec<_>>();
        if source.is_empty() {
            return Err(error(
                Some(occurrence.instance),
                "Reusable Assembly has no compiled canonical leaf geometry",
            ));
        }
        for leaf in source {
            let solid = self
                .solids
                .get(&leaf.path)
                .ok_or_else(|| {
                    error(
                        Some(occurrence.instance),
                        "Assembly leaf source geometry is unavailable",
                    )
                })?
                .transformed(rigid(occurrence.pose))
                .map_err(|e| error(Some(occurrence.instance), e))?;
            let path = OccurrencePath {
                root: occurrence.instance,
                members: leaf.path.members.clone(),
            };
            let snapshot = LeafOccurrenceSnapshot {
                path,
                component: leaf.component,
                producer: leaf.producer,
                pose: leaf
                    .pose
                    .then(occurrence.pose)
                    .map_err(|e| error(Some(occurrence.instance), e))?,
                bounds: solid
                    .bounds()
                    .map_err(|e| error(Some(occurrence.instance), e))?,
                topology: solid
                    .topology_snapshot(occurrence.instance, revision)
                    .map_err(|e| error(Some(occurrence.instance), e))?,
            };
            self.insert(snapshot, Some(solid))?;
        }
        Ok(())
    }
}
fn rigid(pose: RigidPose) -> RigidTransform {
    RigidTransform {
        rotation: pose.rotation,
        translation_mm: pose.translation_mm,
    }
}
pub(crate) fn component(
    document: &Document,
    id: EntityId,
    occurrences: &BTreeMap<EntityId, Arc<OccurrenceSnapshot>>,
    solids: &BTreeMap<EntityId, Solid>,
    store: &mut LeafStore,
    revision: u64,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let Some(EntityData::Assembly { instances }) = document.get(id).map(|e| &e.data) else {
        return Err(error(Some(id), "Assembly definition missing"));
    };
    if instances.is_empty() {
        return Err(error(
            Some(id),
            "A reusable Assembly must have explicit placed member Instances",
        ));
    }
    let mut members = vec![];
    let mut leaves = vec![];
    for member in instances {
        let occurrence = occurrences
            .get(member)
            .ok_or_else(|| error(Some(id), "Assembly member placement is unavailable"))?;
        let solid = solids
            .get(member)
            .ok_or_else(|| error(Some(id), "Assembly member native geometry is unavailable"))?
            .transformed(Default::default())
            .and_then(|s| s.with_occurrence_origin(id, *member))
            .map_err(|e| error(Some(id), e))?;
        if matches!(
            document.get(occurrence.component).map(|e| &e.data),
            Some(EntityData::Part { .. })
        ) {
            let leaf = LeafOccurrenceSnapshot {
                path: OccurrencePath {
                    root: id,
                    members: vec![*member],
                },
                component: occurrence.component,
                producer: occurrence.producer,
                pose: occurrence.pose,
                bounds: solid.bounds().map_err(|e| error(Some(id), e))?,
                topology: solid
                    .topology_snapshot(id, revision)
                    .map_err(|e| error(Some(id), e))?,
            };
            leaves.push((
                leaf,
                solid
                    .transformed(Default::default())
                    .map_err(|e| error(Some(id), e))?,
            ));
        } else {
            let source = store
                .snapshots
                .values()
                .filter(|leaf| leaf.path.root == occurrence.component)
                .cloned()
                .collect::<Vec<_>>();
            if source.is_empty() {
                return Err(error(
                    Some(id),
                    "Nested Assembly has no compiled leaf geometry",
                ));
            }
            for leaf in source {
                let world = store
                    .solids
                    .get(&leaf.path)
                    .ok_or_else(|| {
                        error(Some(id), "Nested Assembly leaf native geometry is missing")
                    })?
                    .transformed(rigid(occurrence.pose))
                    .and_then(|s| s.with_occurrence_origin(id, *member))
                    .map_err(|e| error(Some(id), e))?;
                let path = OccurrencePath {
                    root: id,
                    members: std::iter::once(*member)
                        .chain(leaf.path.members.iter().copied())
                        .collect(),
                };
                let snapshot = LeafOccurrenceSnapshot {
                    path,
                    component: leaf.component,
                    producer: leaf.producer,
                    pose: leaf
                        .pose
                        .then(occurrence.pose)
                        .map_err(|e| error(Some(id), e))?,
                    bounds: world.bounds().map_err(|e| error(Some(id), e))?,
                    topology: world
                        .topology_snapshot(id, revision)
                        .map_err(|e| error(Some(id), e))?,
                };
                leaves.push((snapshot, world));
            }
        }
        members.push(solid);
    }
    let compound = Solid::compound_named(&members.iter().collect::<Vec<_>>(), id, policy)
        .map_err(|e| error(Some(id), e))?;
    for (snapshot, solid) in leaves {
        store.insert(snapshot, Some(solid))?;
    }
    Ok(compound)
}

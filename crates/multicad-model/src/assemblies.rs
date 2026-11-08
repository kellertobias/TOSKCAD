//! Exact reusable component compounds and canonical leaf-path geometry.
use crate::{ModelError, OccurrenceSnapshot, compiler::checkpoint, error};
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
pub(crate) struct CachedLeaves {
    pub snapshots: BTreeMap<OccurrencePath, Arc<LeafOccurrenceSnapshot>>,
    pub solids: BTreeMap<OccurrencePath, Solid>,
}
impl CachedLeaves {
    pub fn shared(&self, revision: u64) -> Option<Self> {
        let snapshots = self
            .snapshots
            .iter()
            .map(|(path, leaf)| {
                let mut leaf = (**leaf).clone();
                leaf.topology.revision = revision;
                (path.clone(), Arc::new(leaf))
            })
            .collect();
        let solids = self
            .solids
            .iter()
            .map(|(path, solid)| Some((path.clone(), solid.share().ok()?)))
            .collect::<Option<_>>()?;
        Some(Self { snapshots, solids })
    }
    pub fn estimated_bytes(&self) -> usize {
        self.snapshots
            .values()
            .map(|leaf| leaf.topology.elements.len().saturating_mul(1280))
            .sum()
    }
}
impl LeafStore {
    pub(crate) fn capture(&self, root: EntityId) -> Option<CachedLeaves> {
        let snapshots = self
            .snapshots
            .iter()
            .filter(|(path, _)| path.root == root)
            .map(|(path, leaf)| (path.clone(), leaf.clone()))
            .collect();
        let solids = self
            .solids
            .iter()
            .filter(|(path, _)| path.root == root)
            .map(|(path, solid)| Some((path.clone(), solid.share().ok()?)))
            .collect::<Option<_>>()?;
        Some(CachedLeaves { snapshots, solids })
    }
    pub(crate) fn restore(&mut self, leaves: CachedLeaves) -> Result<(), ModelError> {
        // Check the entire set before inserting anything: a failed restoration
        // cannot leave partial paths or bypass ordinary occurrence budgets.
        if self.snapshots.len().saturating_add(leaves.snapshots.len())
            > multicad_core::model::MAX_ASSEMBLY_LEAVES
            || leaves
                .snapshots
                .keys()
                .any(|path| path.members.len() > 32 || self.snapshots.contains_key(path))
            || leaves
                .solids
                .keys()
                .any(|path| !leaves.snapshots.contains_key(path) || self.solids.contains_key(path))
        {
            return Err(error(
                None,
                "Cached assembly leaves exceed their budget or duplicate a canonical path",
            ));
        }
        self.snapshots.extend(leaves.snapshots);
        self.solids.extend(leaves.solids);
        Ok(())
    }
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
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<(), ModelError> {
        checkpoint(interrupt)?;
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
            checkpoint(interrupt)?;
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
pub(crate) struct ComponentInputs<'a> {
    pub document: &'a Document,
    pub occurrences: &'a BTreeMap<EntityId, Arc<OccurrenceSnapshot>>,
    pub solids: &'a BTreeMap<EntityId, Solid>,
    pub revision: u64,
    pub policy: GeometryPolicy,
    pub interrupt: Option<&'a dyn Fn() -> bool>,
}
pub(crate) fn component(
    inputs: ComponentInputs<'_>,
    id: EntityId,
    store: &mut LeafStore,
) -> Result<Solid, ModelError> {
    let ComponentInputs {
        document,
        occurrences,
        solids,
        revision,
        policy,
        interrupt,
    } = inputs;
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
        checkpoint(interrupt)?;
        let occurrence = occurrences
            .get(member)
            .ok_or_else(|| error(Some(id), "Assembly member placement is unavailable"))?;
        let solid = solids
            .get(member)
            .ok_or_else(|| error(Some(id), "Assembly member native geometry is unavailable"))?
            .share()
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
            leaves.push((leaf, solid.share().map_err(|e| error(Some(id), e))?));
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
                checkpoint(interrupt)?;
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
    let compound = Solid::compound_named_interruptible(
        &members.iter().collect::<Vec<_>>(),
        id,
        policy,
        interrupt,
    )
    .map_err(|e| error(Some(id), e))?;
    for (snapshot, solid) in leaves {
        store.insert(snapshot, Some(solid))?;
    }
    Ok(compound)
}

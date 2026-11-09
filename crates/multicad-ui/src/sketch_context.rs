//! Occurrence placement is an editing/display context, not a history parent.
use crate::viewport::{Scene, Transform};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::OccurrencePath,
};
use std::collections::BTreeSet;

/// Locate a displayed occurrence of the component that produces or consumes a
/// sketch/plane. Producer links keep feature history acyclic; this pose is never
/// written into the canonical sketch coordinates.
pub fn occurrence(
    document: &Document,
    seed: EntityId,
    scene: &Scene,
    preferred: Option<EntityId>,
) -> Option<(EntityId, Transform)> {
    if scene.stamp().document != document.id()
        || multicad_core::model::developed_sheet_sketch(document, seed)
            .is_ok_and(|chart| chart.is_some())
    {
        return None;
    }
    let mut seeds = BTreeSet::from([seed]);
    let plane_only = matches!(document.get(seed)?.data, EntityData::Plane { .. });
    let mut component_anchor = None;
    let mut current = seed;
    for _ in 0..256 {
        match &document.get(current)?.data {
            EntityData::Sketch { plane, .. } => current = *plane,
            EntityData::Plane {
                anchor: Some(anchor),
                ..
            } => {
                match document.get(anchor.target).map(|e| &e.data) {
                    // Old explicit instance references already resolve in world
                    // coordinates. Applying their placement again would be wrong.
                    Some(
                        EntityData::Instance { .. }
                        | EntityData::Part { .. }
                        | EntityData::Assembly { .. },
                    ) if anchor.geometry.is_some() => return None,
                    Some(EntityData::Feature { .. }) if anchor.geometry.is_some() => {
                        seeds.insert(anchor.target);
                        component_anchor = Some(anchor.target);
                        break;
                    }
                    Some(EntityData::Plane { .. })
                        if anchor.role == multicad_core::sketch::OFFSET_PLANE_ROLE =>
                    {
                        current = anchor.target
                    }
                    _ => break,
                }
            }
            _ => break,
        }
    }
    // An independent world datum remains a world datum even if some part was
    // originally extruded from a sketch on it. Sketch editing can still choose
    // a placed occurrence of that part's component-local definition.
    if plane_only && component_anchor.is_none() {
        return None;
    }
    let affected: BTreeSet<_> = document.affected(seeds).ok()?.into_iter().collect();
    let matches =
        |id: EntityId, geometry: EntityId| affected.contains(&id) || affected.contains(&geometry);
    // Downstream hardware can depend on a sheet's producer without owning the
    // sketch support. On reopening, match the canonical owning Part rather
    // than other affected leaves in the same displayed assembly.
    let owns_anchor = |part| {
        component_anchor.is_some_and(|anchor| {
            matches!(
                document.get(part).map(|entity| &entity.data),
                Some(EntityData::Part { features, .. }) if features.contains(&anchor)
            )
        })
    };
    let direct_owns_anchor = |instance: &crate::viewport::Instance| {
        let part = match document.get(instance.id).map(|entity| &entity.data) {
            Some(EntityData::Instance { component, .. }) => *component,
            _ => instance.id,
        };
        owns_anchor(part)
    };
    let direct_matches = |instance: &crate::viewport::Instance| {
        matches(instance.id, instance.geometry)
            && (component_anchor.is_none()
                || matches!(
                    document.get(instance.geometry).map(|entity| &entity.data),
                    Some(EntityData::Feature { .. }),
                ))
    };
    let direct = |instance: &crate::viewport::Instance| (instance.id, instance.transform);
    let direct_preferred = preferred.and_then(|id| {
        scene
            .reference_instances()
            .iter()
            .find(|i| i.id == id && direct_matches(i))
            .map(direct)
    });
    let nested = |root: EntityId| {
        component_anchor?;
        let mut leaves = scene.assembly_evaluation()?.leaves.iter().filter(|leaf| {
            leaf.path.root == root && !leaf.path.members.is_empty() && owns_anchor(leaf.component)
        });
        let leaf = leaves.next()?;
        if leaves.next().is_some() {
            return None;
        }
        Transform::from_rigid_pose(leaf.pose)
            .ok()
            .map(|pose| (root, pose))
    };
    let direct_owner = scene
        .reference_instances()
        .iter()
        .filter(|instance| direct_owns_anchor(instance))
        .min_by_key(|instance| instance.transform != Transform::default())
        .map(direct);
    preferred
        .and_then(|id| {
            scene
                .reference_instances()
                .iter()
                .find(|instance| instance.id == id && direct_owns_anchor(instance))
                .map(direct)
                .or_else(|| nested(id))
        })
        .or(direct_owner)
        .or_else(|| scene.instances().iter().find_map(|i| nested(i.id)))
        .or(direct_preferred)
        .or_else(|| {
            scene
                .reference_instances()
                .iter()
                .find(|i| direct_matches(i) && i.transform == Transform::default())
                .map(direct)
        })
        .or_else(|| {
            scene
                .reference_instances()
                .iter()
                .find(|i| direct_matches(i))
                .map(direct)
        })
}

/// The exact active display leaf picked at creation, including in assemblies
/// with multiple copies of the same sheet definition.
pub fn exact_occurrence(scene: &Scene, path: &OccurrencePath) -> Option<Transform> {
    if !scene.displays_instance(path.root) {
        return None;
    }
    let leaf = scene
        .assembly_evaluation()?
        .leaves
        .iter()
        .find(|leaf| &leaf.path == path)?;
    Transform::from_rigid_pose(leaf.pose).ok()
}

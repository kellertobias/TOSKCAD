//! Occurrence placement is an editing/display context, not a history parent.
use crate::viewport::{Scene, Transform};
use multicad_core::document::{Document, EntityData, EntityId};
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
    if scene.stamp().document != document.id() {
        return None;
    }
    let mut seeds = BTreeSet::from([seed]);
    let plane_only = matches!(document.get(seed)?.data, EntityData::Plane { .. });
    let mut component_anchor = false;
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
                        component_anchor = true;
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
    if plane_only && !component_anchor {
        return None;
    }
    let affected: BTreeSet<_> = document.affected(seeds).ok()?.into_iter().collect();
    let matches =
        |id: EntityId, geometry: EntityId| affected.contains(&id) || affected.contains(&geometry);
    let instance = preferred
        .and_then(|id| {
            scene
                .reference_instances()
                .iter()
                .find(|i| i.id == id && matches(i.id, i.geometry))
        })
        .or_else(|| {
            scene
                .reference_instances()
                .iter()
                .find(|i| matches(i.id, i.geometry) && i.transform == Transform::default())
        })
        .or_else(|| {
            scene
                .reference_instances()
                .iter()
                .find(|i| matches(i.id, i.geometry))
        })?;
    Some((instance.id, instance.transform))
}

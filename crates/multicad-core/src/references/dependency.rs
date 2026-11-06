use crate::{
    document::{Document, DocumentError, EntityId},
    project_file::ContentHash,
};
use std::collections::{BTreeMap, BTreeSet};
/// Bind a report to this source's canonical transitive dependencies, rather
/// than its consumer sketch. Constraint-only consumer edits can reuse a report;
/// changing any upstream producer/parameter/plane cannot. This is a stale-report
/// guard, not authentication of a caller-supplied topology snapshot.
pub fn dependency_hash(
    document: &Document,
    source: EntityId,
) -> Result<ContentHash, DocumentError> {
    let mut visited = BTreeSet::new();
    let mut pending = vec![source];
    let mut entities = BTreeMap::new();
    let mut pose_fingerprints = BTreeMap::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        if visited.len() > 4096 {
            return Err(DocumentError::Invalid(
                "Reference dependency closure exceeds the 4096-entity budget".into(),
            ));
        }
        let entity = document.get(id);
        if let Some(entity) = entity {
            pending.extend(document.evaluation_dependencies(id)?);
            if let Some(projection) = crate::model::transfer_pose_projection(document, entity.id)?
                .or(crate::model::screw_connection_axis_pose_projection(
                    document, entity.id,
                )?)
                .or(crate::sketch_external::external_pose_projection(
                    document, entity.id,
                )?)
            {
                pose_fingerprints.insert(id, projection.fingerprint);
            }
        }
        entities.insert(id, entity);
    }
    let bytes = if pose_fingerprints.is_empty() {
        serde_json::to_vec(&(document.id(), entities))
    } else {
        serde_json::to_vec(&(document.id(), entities, pose_fingerprints))
    }
    .map_err(|e| DocumentError::Invalid(e.to_string()))?;
    Ok(ContentHash::of(&bytes))
}

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
            pending.extend(entity.references().into_iter().map(|link| link.target));
        }
        entities.insert(id, entity);
    }
    let bytes = serde_json::to_vec(&(document.id(), entities))
        .map_err(|e| DocumentError::Invalid(e.to_string()))?;
    Ok(ContentHash::of(&bytes))
}

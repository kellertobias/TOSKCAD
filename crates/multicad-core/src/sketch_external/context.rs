//! Pure authored context choices. Exact native discovery still validates source
//! topology and current poses; this catalogue is not a geometry receipt.
use super::{ExternalQuery, OccurrenceContext, OccurrenceSource, source::invalid};
use crate::{
    document::{Document, DocumentError, EntityData, EntityId},
    model,
};
use std::collections::{BTreeMap, BTreeSet};
pub fn external_occurrence_catalog(
    document: &Document,
) -> Result<Vec<OccurrenceSource>, DocumentError> {
    Ok(model::transfer_occurrence_catalog(document)?
        .into_iter()
        .map(|c| OccurrenceSource {
            component: c.component,
            source: c.component,
            path: c.path,
        })
        .collect())
}
fn sketch_components(
    document: &Document,
    sketch: EntityId,
) -> Result<BTreeSet<EntityId>, DocumentError> {
    let Some(EntityData::Sketch { plane, .. }) = document.get(sketch).map(|e| &e.data) else {
        return Err(invalid("Choose a current sketch"));
    };
    let mut reverse = BTreeMap::<EntityId, BTreeSet<EntityId>>::new();
    for e in document.entities() {
        for dependency in document.evaluation_dependencies(e.id)? {
            reverse.entry(dependency).or_default().insert(e.id);
        }
    }
    let mut affected = BTreeSet::new();
    let mut pending = vec![sketch];
    while let Some(id) = pending.pop() {
        if !affected.insert(id) {
            continue;
        }
        if affected.len() > 16_384 {
            return Err(invalid(
                "Sketch ownership closure exceeds the 16384-source budget",
            ));
        }
        if let Some(next) = reverse.get(&id) {
            pending.extend(next);
        }
    }
    let mut plane_sources = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut pending = vec![*plane];
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        if visited.len() > 256 {
            return Err(invalid(
                "Sketch plane ownership chain exceeds the bounded context budget",
            ));
        }
        if let Some(EntityData::Plane {
            anchor: Some(link), ..
        }) = document.get(id).map(|e| &e.data)
        {
            plane_sources.insert(link.target);
            if matches!(
                document.get(link.target).map(|e| &e.data),
                Some(EntityData::Plane { .. })
            ) {
                pending.push(link.target);
            }
        }
    }
    Ok(document
        .entities()
        .filter_map(|e| match &e.data {
            EntityData::Part { features, .. }
                if features
                    .iter()
                    .any(|id| affected.contains(id) || plane_sources.contains(id))
                    || plane_sources.contains(&e.id) =>
            {
                Some(e.id)
            }
            _ => None,
        })
        .collect())
}
/// Every placed leaf of each explicitly consuming/mounting Part. Shared sketches
/// can have multiple valid contexts, which callers must present for selection.
pub fn sketch_occurrence_contexts(
    document: &Document,
    sketch: EntityId,
) -> Result<Vec<OccurrenceContext>, DocumentError> {
    let components = sketch_components(document, sketch)?;
    Ok(external_occurrence_catalog(document)?
        .into_iter()
        .filter(|c| components.contains(&c.component))
        .map(|c| OccurrenceContext {
            component: c.component,
            path: c.path,
        })
        .collect())
}
/// Strong preflight for NEW or repaired bindings. Stored readers intentionally
/// do not call this: upstream missing paths remain saveable and repairable.
pub fn validate_occurrence_query(
    document: &Document,
    query: &ExternalQuery,
) -> Result<(), DocumentError> {
    query.validate()?;
    model::validate_transfer_path(document, &query.source.path, query.source.component)?;
    match document.get(query.source.component).map(|e| &e.data) {
        Some(EntityData::Part { features, .. })
            if query.source.source == query.source.component
                || features.contains(&query.source.source) => {}
        _ => {
            return Err(invalid(
                "Selected source does not belong to the named component history",
            ));
        }
    }
    if query.source.source != query.source.component
        && !matches!(
            document.get(query.source.source).map(|e| &e.data),
            Some(EntityData::Feature {
                suppressed: false,
                ..
            })
        )
    {
        return Err(invalid(
            "Selected source producer is missing, suppressed or replaced",
        ));
    }
    let Some(EntityData::Sketch { .. }) = document.get(query.sketch).map(|e| &e.data) else {
        return Err(invalid("Receiving sketch is missing or replaced"));
    };
    if let Some(context) = &query.sketch_context {
        model::validate_transfer_path(document, &context.path, context.component)?;
        if !sketch_components(document, query.sketch)?.contains(&context.component) {
            return Err(invalid(
                "Consumer occurrence does not own a feature using this sketch or its mounting plane; choose world space or its actual Part context",
            ));
        }
    }
    Ok(())
}
/// Deferred pose-only links are recognized only after the complete stored v2
/// payload and exact link set have passed the canonical reader.
pub fn external_pose_projection(
    document: &Document,
    reference: EntityId,
) -> Result<Option<model::TransferPoseProjection>, DocumentError> {
    if !super::is_occurrence_reference(document, reference) {
        return Ok(None);
    }
    let request = super::read_occurrence(document, reference)?;
    let mut projection = model::occurrence_pose_projection(
        document,
        &request.query.source.path,
        request.query.sketch_context.as_ref().map(|c| &c.path),
    )?;
    let frame = match document.get(request.query.sketch).map(|e| &e.data) {
        Some(EntityData::Sketch {
            plane, definition, ..
        }) => {
            projection.geometry_dependencies.insert(*plane);
            let u = crate::constraint_sketch::source::Source::from_definition(definition)?.u;
            serde_json::json!({"sketch":request.query.sketch,"plane":plane,"u":u})
        }
        Some(other) => serde_json::json!({"replaced":other.kind()}),
        None => serde_json::Value::Null,
    };
    projection.fingerprint = crate::project_file::ContentHash::of(
        &serde_json::to_vec(&(projection.fingerprint, frame)).map_err(invalid)?,
    );
    Ok(Some(projection))
}
pub fn external_evaluation_dependencies(
    document: &Document,
    reference: EntityId,
) -> Result<Option<BTreeSet<EntityId>>, DocumentError> {
    let Some(projection) = external_pose_projection(document, reference)? else {
        return Ok(None);
    };
    let mut deps = document
        .get(reference)
        .ok_or(DocumentError::Missing(reference))?
        .references()
        .into_iter()
        .filter(|l| !l.role.starts_with(super::EXTERNAL_POSE_ROLE))
        .map(|l| l.target)
        .collect::<BTreeSet<_>>();
    deps.extend(projection.geometry_dependencies);
    Ok(Some(deps))
}

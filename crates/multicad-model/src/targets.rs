//! Exact, source-neutral cut target discovery on a current owned native model.
//! Scope and explicit overrides select candidate Parts; cached bounds only
//! prune work, and positive-volume native common geometry decides every hit.
use crate::{Model, ModelError, compiler::error};
pub use multicad_core::model::{CutTargetQuery, CutTargetSelection, MAX_CUT_TARGET_POOL};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    hierarchy,
    model::{BodyInput, MAX_BOOLEAN_TARGETS, MAX_BOOLEAN_TOOLS},
    project_file::ContentHash,
};
use multicad_kernel::{BooleanOperation, Bounds};
use std::collections::{BTreeMap, BTreeSet};
fn checkpoint(interrupt: Option<&dyn Fn() -> bool>) -> Result<(), ModelError> {
    if interrupt.is_some_and(|f| f()) {
        Err(error(None, "Cut target query superseded"))
    } else {
        Ok(())
    }
}
fn overlap(a: Bounds, b: Bounds) -> bool {
    (0..3).all(|i| a.max[i] > b.min[i] && b.max[i] > a.min[i])
}
fn unique(ids: &[EntityId], budget: usize, what: &str) -> Result<BTreeSet<EntityId>, ModelError> {
    if ids.len() > budget {
        return Err(error(
            None,
            format!("Cut target {what} exceeds the {budget}-item budget"),
        ));
    }
    let result: BTreeSet<_> = ids.iter().copied().collect();
    if result.len() != ids.len() || result.iter().any(EntityId::is_nil) {
        return Err(error(
            None,
            format!("Cut target {what} require unique non-nil identities"),
        ));
    }
    Ok(result)
}
impl Model {
    pub fn cut_targets(
        &self,
        document: &Document,
        revision: u64,
        query: &CutTargetQuery,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<CutTargetSelection, ModelError> {
        checkpoint(interrupt)?;
        let snapshot = self.snapshot();
        if snapshot.scope != crate::EvaluationScope::Full {
            return Err(error(
                None,
                "A source-only inspection model cannot discover cut targets",
            ));
        }
        if snapshot.stamp.document != document.id()
            || snapshot.stamp.revision != revision
            || snapshot.stamp.source_hash
                != ContentHash::document(document).map_err(|e| error(None, e))?
        {
            return Err(error(
                None,
                "Cut target query requires the current document/revision/source model; rebuild before selecting targets",
            ));
        }
        let tools = unique(&query.tools, MAX_BOOLEAN_TOOLS, "tools")?;
        if tools.is_empty() {
            return Err(error(None, "Choose at least one current tool producer"));
        }
        let includes = unique(&query.include, MAX_CUT_TARGET_POOL, "includes")?;
        let excludes = unique(&query.exclude, MAX_CUT_TARGET_POOL, "excludes")?;
        let mut tool_heads = BTreeMap::new();
        for (i, (body, producer)) in snapshot.bodies.iter().enumerate() {
            if i % 256 == 0 {
                checkpoint(interrupt)?;
            }
            if tools.contains(producer) && tool_heads.insert(*producer, *body).is_some() {
                return Err(error(
                    Some(*producer),
                    "Tool must uniquely identify one current unconsumed body's final producer",
                ));
            }
        }
        let mut tool_bodies = BTreeSet::new();
        for tool in &tools {
            let Some(body) = tool_heads.get(tool) else {
                return Err(error(
                    Some(*tool),
                    "Tool must uniquely identify one current unconsumed body's final producer",
                ));
            };
            if self.solid(*tool).is_none() || !snapshot.shapes.contains_key(tool) {
                return Err(error(
                    Some(*tool),
                    "Tool producer has no current exact native shape",
                ));
            }
            tool_bodies.insert(*body);
        }
        let index = hierarchy::Index::new(document).map_err(|e| error(None, e))?;
        let defaults: BTreeSet<_> = index
            .default_cut_bodies(query.component)
            .map_err(|e| error(query.component, e))?
            .into_iter()
            .filter(|id| snapshot.bodies.contains_key(id) && !tool_bodies.contains(id))
            .collect();
        let mut explicit = BTreeSet::new();
        let mut empty_components = BTreeSet::new();
        for id in includes {
            checkpoint(interrupt)?;
            match document.get(id).map(|e| &e.data) {
                Some(EntityData::Part { .. }) => {
                    if tool_bodies.contains(&id) {
                        return Err(error(
                            Some(id),
                            "A tool Part cannot explicitly include itself as a cut target",
                        ));
                    }
                    if !snapshot.bodies.contains_key(&id) {
                        return Err(error(
                            Some(id),
                            "Included Part has no current unconsumed final body output",
                        ));
                    }
                    explicit.insert(id);
                }
                Some(EntityData::Assembly { .. }) => {
                    let members: Vec<_> = index
                        .default_cut_bodies(Some(id))
                        .map_err(|e| error(Some(id), e))?
                        .into_iter()
                        .filter(|id| snapshot.bodies.contains_key(id) && !tool_bodies.contains(id))
                        .collect();
                    if members.is_empty() {
                        empty_components.insert(id);
                    }
                    explicit.extend(members);
                }
                _ => {
                    return Err(error(
                        Some(id),
                        "Cut target includes must identify an existing Part or component",
                    ));
                }
            }
        }
        for id in &excludes {
            if !matches!(
                document.get(*id).map(|e| &e.data),
                Some(EntityData::Part { .. })
            ) {
                return Err(error(
                    Some(*id),
                    "Cut target excludes must identify existing Parts",
                ));
            }
        }
        let pool: BTreeSet<_> = defaults.union(&explicit).copied().collect();
        if pool.len() > MAX_CUT_TARGET_POOL {
            return Err(error(
                None,
                format!(
                    "Cut target pool exceeds {MAX_CUT_TARGET_POOL} bodies; narrow its component scope or explicit includes"
                ),
            ));
        }
        let mut hits = BTreeMap::new();
        let mut producer_hits = BTreeMap::new();
        for body in pool {
            checkpoint(interrupt)?;
            let producer = snapshot.bodies[&body];
            if let Some(hit) = producer_hits.get(&producer) {
                hits.insert(body, *hit);
                continue;
            }
            let shape = snapshot.shapes.get(&producer).ok_or_else(|| {
                error(
                    Some(producer),
                    "Current target producer has no exact shape snapshot",
                )
            })?;
            let target = self.solid(producer).ok_or_else(|| {
                error(
                    Some(producer),
                    "Current target producer has no exact native solid",
                )
            })?;
            let mut hit = false;
            for tool in &tools {
                checkpoint(interrupt)?;
                let tool_shape = &snapshot.shapes[tool];
                if !overlap(shape.bounds, tool_shape.bounds) {
                    continue;
                }
                let common = target
                    .boolean(
                        self.solid(*tool).expect("validated tool"),
                        BooleanOperation::Intersect,
                        self.policy,
                    )
                    .map_err(|e| error(Some(body), e))?;
                checkpoint(interrupt)?;
                // Solid::boolean only returns Some after validating positive
                // finite native volume; contact-only results return None.
                if common.is_some() {
                    hit = true;
                    break;
                }
            }
            producer_hits.insert(producer, hit);
            hits.insert(body, hit);
        }
        let body_input = |id: &EntityId| BodyInput {
            body: *id,
            producer: snapshot.bodies[id],
        };
        let default_hits = defaults
            .iter()
            .filter(|id| hits[id])
            .map(body_input)
            .collect();
        let targets: Vec<_> = hits
            .iter()
            .filter(|(id, hit)| **hit && !excludes.contains(id))
            .map(|(id, _)| body_input(id))
            .collect();
        if targets.len() > MAX_BOOLEAN_TARGETS {
            return Err(error(
                None,
                format!(
                    "More than {MAX_BOOLEAN_TARGETS} exact cut targets remain; narrow the scope or exclude bodies before applying"
                ),
            ));
        }
        let mut not_intersecting = empty_components;
        not_intersecting.extend(explicit.into_iter().filter(|id| !hits[id]));
        checkpoint(interrupt)?;
        Ok(CutTargetSelection {
            defaults: default_hits,
            targets,
            not_intersecting: not_intersecting.into_iter().collect(),
        })
    }
}

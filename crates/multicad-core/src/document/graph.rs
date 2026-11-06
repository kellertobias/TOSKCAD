use super::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferenceProblem {
    MissingEntity,
    WrongKind {
        expected: Vec<EntityKind>,
        actual: EntityKind,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceDiagnostic {
    pub owner: EntityId,
    pub target: EntityId,
    pub role: String,
    pub problem: ReferenceProblem,
}
impl Document {
    pub fn reference_diagnostics(&self) -> Vec<ReferenceDiagnostic> {
        let mut output = Vec::new();
        for entity in self.entities() {
            for (link, expected) in entity.data.sites() {
                let problem = match self.get(link.target) {
                    None => Some(ReferenceProblem::MissingEntity),
                    Some(target)
                        if !expected.is_empty() && !expected.contains(&target.data.kind()) =>
                    {
                        Some(ReferenceProblem::WrongKind {
                            expected,
                            actual: target.data.kind(),
                        })
                    }
                    _ => None,
                };
                if let Some(problem) = problem {
                    output.push(ReferenceDiagnostic {
                        owner: entity.id,
                        target: link.target,
                        role: link.role,
                        problem,
                    });
                }
            }
        }
        output
    }
    pub fn evaluation_dependencies(
        &self,
        id: EntityId,
    ) -> Result<BTreeSet<EntityId>, DocumentError> {
        if let Some(deps) = crate::sketch_external::external_evaluation_dependencies(self, id)? {
            return Ok(deps);
        }
        if let Some(deps) = crate::model::screw_axis_evaluation_dependencies(self, id)? {
            return Ok(deps);
        }
        if let Some(deps) = crate::model::connection_receiver_evaluation_dependencies(self, id)? {
            return Ok(deps);
        }
        crate::model::transfer_evaluation_dependencies(self, id)
    }
    fn edges(&self) -> Result<BTreeMap<EntityId, BTreeSet<EntityId>>, DocumentError> {
        self.entities()
            .map(|e| Ok((e.id, self.evaluation_dependencies(e.id)?)))
            .collect()
    }
    /// Sources precede dependents; UUID ordering breaks ties deterministically.
    /// Missing references are diagnosed separately and never invented as nodes.
    pub fn topological_order(&self) -> Result<Vec<EntityId>, DocumentError> {
        let edges = self.edges()?;
        let mut degree = BTreeMap::new();
        let mut reverse: BTreeMap<EntityId, BTreeSet<EntityId>> = BTreeMap::new();
        for (id, inputs) in &edges {
            degree.insert(
                *id,
                inputs
                    .iter()
                    .filter(|input| edges.contains_key(input))
                    .count(),
            );
            for input in inputs {
                reverse.entry(*input).or_default().insert(*id);
            }
        }
        let mut ready: BTreeSet<_> = degree
            .iter()
            .filter_map(|(id, n)| (*n == 0).then_some(*id))
            .collect();
        let mut ordered = Vec::new();
        while let Some(id) = ready.pop_first() {
            ordered.push(id);
            if let Some(dependents) = reverse.get(&id) {
                for dependent in dependents {
                    let n = degree.get_mut(dependent).expect("known dependent");
                    *n -= 1;
                    if *n == 0 {
                        ready.insert(*dependent);
                    }
                }
            }
        }
        if ordered.len() != edges.len() {
            let remaining: BTreeSet<_> = degree
                .iter()
                .filter_map(|(id, n)| (*n > 0).then_some(*id))
                .collect();
            let mut path = Vec::new();
            let mut seen = BTreeMap::new();
            let mut current = *remaining.first().expect("unfinished node");
            loop {
                if let Some(index) = seen.insert(current, path.len()) {
                    let mut cycle = path[index..].to_vec();
                    cycle.push(current);
                    return Err(DocumentError::Cycle(cycle));
                }
                path.push(current);
                current = *edges[&current]
                    .iter()
                    .find(|id| remaining.contains(id))
                    .expect("unfinished dependency");
            }
        }
        Ok(ordered)
    }
    /// Includes transitive dependents of deleted source IDs, for safe repair and
    /// cache invalidation. Unrelated bodies are omitted from the rebuild order.
    pub fn affected(
        &self,
        sources: impl IntoIterator<Item = EntityId>,
    ) -> Result<Vec<EntityId>, DocumentError> {
        self.reachable_dependents(sources, false)
    }
    /// Invalidation also reaches organizational ancestors and their consumers;
    /// semantic producer/consumer queries retain affected()'s narrower meaning.
    pub fn invalidated(
        &self,
        sources: impl IntoIterator<Item = EntityId>,
    ) -> Result<Vec<EntityId>, DocumentError> {
        self.reachable_dependents(sources, true)
    }
    fn reachable_dependents(
        &self,
        sources: impl IntoIterator<Item = EntityId>,
        containment: bool,
    ) -> Result<Vec<EntityId>, DocumentError> {
        let mut reverse: BTreeMap<EntityId, BTreeSet<EntityId>> = BTreeMap::new();
        for e in self.entities() {
            for link in e.references() {
                reverse.entry(link.target).or_default().insert(e.id);
            }
        }
        if containment {
            for (container, members) in crate::hierarchy::invalidation_edges(self) {
                for member in members {
                    reverse.entry(member).or_default().insert(container);
                }
            }
        }
        if containment {
            for (owner, association) in crate::materials::invalidation_edges(self)? {
                reverse.entry(association).or_default().insert(owner);
            }
        }
        let mut affected: BTreeSet<_> = sources.into_iter().collect();
        let mut pending = affected.clone();
        while let Some(id) = pending.pop_first() {
            if let Some(dependents) = reverse.get(&id) {
                for dependent in dependents {
                    if affected.insert(*dependent) {
                        pending.insert(*dependent);
                    }
                }
            }
        }
        Ok(self
            .topological_order()?
            .into_iter()
            .filter(|id| affected.contains(id))
            .collect())
    }
}

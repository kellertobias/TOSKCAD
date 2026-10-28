use super::*;
use crate::{
    document::{Document, DocumentError, EntityData, EntityId},
    model::{self, BodyInput, CreatedBody, PlacementBase, placement::Driver, source::invalid},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScaleTarget {
    Part { input: BodyInput },
    Instance { instance: EntityId },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScaleOperation {
    Definition {
        input: BodyInput,
        feature: EntityId,
    },
    Occurrence {
        instance: EntityId,
        component: EntityId,
        feature: EntityId,
    },
    Copy {
        source: ScaleTarget,
        instance: EntityId,
        component: EntityId,
        feature: EntityId,
        drivers: BTreeMap<EntityId, EntityId>,
        references: BTreeMap<EntityId, EntityId>,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScaleBatchRequest {
    pub parameters: ScaleParameters,
    pub operations: Vec<ScaleOperation>,
}
impl ScaleTarget {
    pub fn entity(&self) -> EntityId {
        match self {
            Self::Part { input } => input.body,
            Self::Instance { instance } => *instance,
        }
    }
}
impl ScaleBatchRequest {
    /// All authored identities are allocated here, before preview submission.
    /// Repeated preparation and modifier toggles retain the exact same copies.
    pub fn new(
        document: &Document,
        targets: Vec<ScaleTarget>,
        parameters: ScaleParameters,
        copy: bool,
    ) -> Result<Self, DocumentError> {
        parameters.validate()?;
        if targets.is_empty() || targets.len() > model::MAX_TRANSFORM_TARGETS {
            return Err(invalid("Select 1-128 explicit scale targets"));
        }
        let mut selected = BTreeSet::new();
        let mut operations = vec![];
        for target in targets {
            if !selected.insert(target.entity()) {
                return Err(invalid("A scale target cannot be selected twice"));
            }
            let feature = EntityId::new();
            if copy {
                let mut drivers = BTreeMap::new();
                let mut references = BTreeMap::new();
                if let ScaleTarget::Instance { instance } = target {
                    for (id, source) in driver_chain(document, instance)? {
                        drivers.insert(id, EntityId::new());
                        if let Driver::Alignment(_, inputs) = source {
                            for input in inputs {
                                references
                                    .entry(input.intent.id)
                                    .or_insert_with(EntityId::new);
                            }
                        }
                    }
                }
                operations.push(ScaleOperation::Copy {
                    source: target,
                    instance: EntityId::new(),
                    component: EntityId::new(),
                    feature,
                    drivers,
                    references,
                });
            } else {
                operations.push(match target {
                    ScaleTarget::Part { input } => ScaleOperation::Definition { input, feature },
                    ScaleTarget::Instance { instance } => ScaleOperation::Occurrence {
                        instance,
                        component: EntityId::new(),
                        feature,
                    },
                });
            }
        }
        Ok(Self {
            parameters,
            operations,
        })
    }
    pub fn is_copy(&self) -> bool {
        self.operations
            .first()
            .is_some_and(|op| matches!(op, ScaleOperation::Copy { .. }))
    }
    pub fn instances(&self) -> Vec<EntityId> {
        self.operations
            .iter()
            .filter_map(|op| match op {
                ScaleOperation::Definition { .. } => None,
                ScaleOperation::Occurrence { instance, .. }
                | ScaleOperation::Copy { instance, .. } => Some(*instance),
            })
            .collect()
    }
    pub fn outputs(&self) -> Vec<CreatedBody> {
        self.operations
            .iter()
            .filter_map(|op| match op {
                ScaleOperation::Definition { input, feature } => Some(CreatedBody {
                    body: input.body,
                    feature: *feature,
                }),
                _ => None,
            })
            .collect()
    }
    pub fn targets(&self) -> Vec<ScaleTarget> {
        self.operations
            .iter()
            .map(|op| match op {
                ScaleOperation::Definition { input, .. } => ScaleTarget::Part {
                    input: input.clone(),
                },
                ScaleOperation::Occurrence { instance, .. } => ScaleTarget::Instance {
                    instance: *instance,
                },
                ScaleOperation::Copy { source, .. } => source.clone(),
            })
            .collect()
    }
}
pub(super) fn driver_chain(
    document: &Document,
    instance: EntityId,
) -> Result<Vec<(EntityId, Driver)>, DocumentError> {
    let Some(EntityData::Instance {
        placement_source, ..
    }) = document.get(instance).map(|e| &e.data)
    else {
        return Err(invalid("Choose an existing Instance to scale"));
    };
    let Some(mut driver) = *placement_source else {
        return Ok(vec![]);
    };
    let mut seen = BTreeSet::new();
    let mut sources = vec![];
    loop {
        if seen.len() >= model::MAX_PLACEMENT_CHAIN || !seen.insert(driver) {
            return Err(invalid(
                "Scale source placement exceeds its depth budget or contains a cycle",
            ));
        }
        let source = Driver::read(document, driver)?;
        if source.owner() != instance {
            return Err(invalid(
                "Scale source placement belongs to another Instance",
            ));
        }
        let next = source.base();
        sources.push((driver, source));
        match next {
            PlacementBase::Literal { .. } => return Ok(sources),
            PlacementBase::Previous { driver: previous } => driver = previous,
        }
    }
}

//! Cut features chained onto panel Parts. Joint and machining cuts each take
//! the Part's previous head as input, so several can stack on one panel and
//! one can be removed by re-attaching the cut after it.
use super::*;
use crate::{
    commands::CommandError,
    document::Mutation,
    hierarchy,
    model::{BodyInput, CreatedBody, common::error},
};

pub(super) fn head(document: &Document, part: EntityId) -> Result<EntityId, CommandError> {
    match document.get(part).map(|e| &e.data) {
        Some(EntityData::Part { features, .. }) => features
            .last()
            .copied()
            .ok_or_else(|| error("The panel has no features")),
        _ => Err(error("A panel Part is missing")),
    }
}
/// Append a cut feature built on the Part's current head. `build` receives
/// that input and returns the feature definition and links.
pub(super) fn append_cut(
    document: &mut Document,
    edits: &mut Vec<Mutation>,
    part: EntityId,
    name: String,
    build: impl FnOnce(BodyInput) -> Result<(Definition, Vec<Link>), CommandError>,
) -> Result<CreatedBody, CommandError> {
    let (definition, inputs) = build(BodyInput {
        body: part,
        producer: head(document, part)?,
    })?;
    let feature = Entity::new(
        name,
        EntityData::Feature {
            definition,
            inputs,
            suppressed: false,
        },
    );
    let id = feature.id;
    let mut entity = document.get(part).cloned().expect("panel Part");
    let EntityData::Part { features, .. } = &mut entity.data else {
        unreachable!()
    };
    features.push(id);
    let step = [Mutation::Insert(feature), Mutation::Replace(entity)];
    *document = document.edited(&step)?;
    edits.extend(step);
    let index = hierarchy::Index::new(document).map_err(|e| error(e.to_string()))?;
    if let Some(parent) = index.parent(part).map_err(|e| error(e.to_string()))? {
        let mut state = document.hierarchy().clone();
        state.members.entry(parent).or_default().push(id);
        let step = Mutation::SetHierarchy(state);
        *document = document.edited(std::slice::from_ref(&step))?;
        edits.push(step);
    }
    Ok(CreatedBody {
        feature: id,
        body: part,
    })
}
/// The input of a joint or machining cut.
fn cut_input(document: &Document, feature: EntityId) -> Option<BodyInput> {
    read_panel_joint_cut(document, feature)
        .map(|c| c.input)
        .or_else(|_| read_machining_cut(document, feature).map(|c| c.input))
        .ok()
}
/// The same cut feature built on another producer.
fn rewired(
    document: &Document,
    feature: EntityId,
    producer: EntityId,
) -> Result<Entity, CommandError> {
    let mut entity = document.get(feature).cloned().expect("cut feature");
    let (definition, inputs) = if let Ok(mut c) = read_panel_joint_cut(document, feature) {
        c.input.producer = producer;
        (c.definition()?, c.inputs())
    } else {
        let mut c = read_machining_cut(document, feature)?;
        c.input.producer = producer;
        (c.definition()?, c.inputs())
    };
    entity.data = EntityData::Feature {
        definition,
        inputs,
        suppressed: false,
    };
    Ok(entity)
}
/// Remove cut features from their Parts. A later joint or machining cut on
/// the same Part is re-attached to the removed cut's input; any other
/// feature that builds on a removed cut blocks the removal.
pub(super) fn remove_cuts(
    candidate: &mut Document,
    edits: &mut Vec<Mutation>,
    cuts: &[EntityId],
    what: &str,
) -> Result<(), CommandError> {
    for &feature in cuts {
        let input =
            cut_input(candidate, feature).ok_or_else(|| error("A cut feature is missing"))?;
        let later: Vec<_> = candidate
            .entities()
            .filter(|e| cut_input(candidate, e.id).is_some_and(|i| i.producer == feature))
            .map(|e| e.id)
            .collect();
        for next in later {
            let step = Mutation::Replace(rewired(candidate, next, input.producer)?);
            *candidate = candidate.edited(std::slice::from_ref(&step))?;
            edits.push(step);
        }
        if candidate.entities().any(|e| {
            e.id != feature
                && !matches!(&e.data, EntityData::Part { .. })
                && e.references().iter().any(|l| l.target == feature)
        }) {
            return Err(error(format!(
                "A later feature builds on this {what}'s cut; remove it first"
            )));
        }
        let mut part = candidate.get(input.body).cloned().expect("Part");
        if let EntityData::Part { features, .. } = &mut part.data {
            features.retain(|f| *f != feature);
        }
        let mut state = candidate.hierarchy().clone();
        for members in state.members.values_mut() {
            members.retain(|m| *m != feature);
        }
        let mut steps = vec![Mutation::Replace(part)];
        if &state != candidate.hierarchy() {
            steps.push(Mutation::SetHierarchy(state));
        }
        steps.push(Mutation::Remove(feature));
        *candidate = candidate.edited(&steps)?;
        edits.extend(steps);
    }
    Ok(())
}

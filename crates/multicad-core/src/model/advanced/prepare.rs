use super::*;
use crate::{
    commands::{Actor, CommandError, Request, Session},
    document::{Entity, EntityData, Mutation},
    hierarchy,
    model::{
        CreatedBody, ModelPrepared,
        boolean::available,
        common::{error, revision},
    },
};
use std::collections::BTreeSet;
fn feature(request: &AdvancedRequest) -> Result<Entity, CommandError> {
    let source = request.source()?;
    let mut entity = Entity::new(
        request.parameters.kind().label(),
        EntityData::Feature {
            definition: source.definition()?,
            inputs: source.inputs(&request.faces)?,
            suppressed: false,
        },
    );
    entity.id = request.feature;
    Ok(entity)
}
fn selection_sources(session: &Session, request: &AdvancedRequest) -> Result<(), CommandError> {
    request.parameters.resolve(session.parameters())?;
    for sketch in request
        .profiles
        .iter()
        .map(|p| p.sketch)
        .chain(request.path.iter().map(|p| p.sketch))
    {
        if !matches!(
            session.document().get(sketch).map(|e| &e.data),
            Some(EntityData::Sketch { .. })
        ) {
            return Err(error(
                "Choose existing source sketches before creating an advanced shape",
            ));
        }
    }
    Ok(())
}
pub fn prepare_advanced(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &AdvancedRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    selection_sources(session, request)?;
    if request.feature.is_nil()
        || request.feature == request.body
        || request.feature == session.document().id()
        || session.document().get(request.feature).is_some()
    {
        return Err(error(
            "Advanced feature needs a fresh non-nil producer identity",
        ));
    }
    let entity = feature(request)?;
    let mut edits = vec![Mutation::Insert(entity)];
    let prepared = if let Some(input) = &request.input {
        available(session.document(), input, true)?;
        let mut part = session
            .document()
            .get(input.body)
            .expect("validated Part")
            .clone();
        let EntityData::Part { features, .. } = &mut part.data else {
            unreachable!()
        };
        features.push(request.feature);
        edits.push(Mutation::Replace(part));
        let index = hierarchy::Index::new(session.document()).map_err(|e| error(e.to_string()))?;
        if let Some(parent) = index.parent(input.body).map_err(|e| error(e.to_string()))? {
            let mut hierarchy = session.document().hierarchy().clone();
            hierarchy
                .members
                .entry(parent)
                .or_default()
                .push(request.feature);
            edits.push(Mutation::SetHierarchy(hierarchy));
        }
        session.prepare(Request {
            expected_revision,
            actor,
            label: format!("{} Part", request.parameters.kind().label()),
            edits,
        })?
    } else {
        if request.body.is_nil()
            || request.body == session.document().id()
            || session.document().get(request.body).is_some()
        {
            return Err(error(
                "A sketch-based advanced shape needs a fresh Part identity",
            ));
        }
        let mut part = Entity::new(
            &request.name,
            EntityData::Part {
                features: vec![request.feature],
                material: None,
                tags: BTreeSet::new(),
            },
        );
        part.id = request.body;
        edits.push(Mutation::Insert(part));
        session.prepare_scoped(Request {
            expected_revision,
            actor,
            label: format!("Create {} Part", request.parameters.kind().label()),
            edits,
        })?
    };
    Ok(ModelPrepared {
        prepared,
        outputs: vec![CreatedBody {
            body: request.body,
            feature: request.feature,
        }],
    })
}
pub fn prepare_advanced_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &AdvancedRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    selection_sources(session, request)?;
    let (source, _) = read_advanced(session.document(), request.feature)?;
    if source.input != request.input
        || source.output.body != request.body
        || source.parameters.kind() != request.parameters.kind()
    {
        return Err(error(
            "An advanced edit must retain its operation, Part and frozen source identities",
        ));
    }
    let old = session
        .document()
        .get(request.feature)
        .expect("readable advanced feature");
    if matches!(
        old.data,
        EntityData::Feature {
            suppressed: true,
            ..
        }
    ) {
        return Err(error("Unsuppress the advanced feature before editing it"));
    }
    let mut entity = feature(request)?;
    entity.name = old.name.clone();
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: format!("Edit {}", request.parameters.kind().label()),
        edits: vec![Mutation::Replace(entity)],
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![CreatedBody {
            body: request.body,
            feature: request.feature,
        }],
    })
}

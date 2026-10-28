use super::*;
use crate::{
    commands::{Actor, CommandError, Request, Session},
    document::{Entity, EntityData, EntityId, Link, Mutation},
    model::{
        CreatedBody, ModelPrepared,
        boolean::available,
        common::{error, revision},
    },
};
use std::collections::BTreeSet;
fn fresh(document: &crate::document::Document, id: EntityId) -> Result<(), CommandError> {
    if id.is_nil() || id == document.id() || document.get(id).is_some() {
        return Err(error(
            "Transfer group/row/output identities must be fresh and non-nil",
        ));
    }
    Ok(())
}
fn bindings(
    session: &Session,
    source: &TransferDefinition,
    edit: bool,
) -> Result<(), CommandError> {
    validate_transfer_path(
        session.document(),
        &source.source.path,
        source.source.component,
    )?;
    if !matches!(
        session
            .document()
            .get(source.source.component)
            .map(|e| &e.data),
        Some(EntityData::Part { .. })
    ) {
        return Err(error(
            "Source hardware must be an existing independent Part",
        ));
    }
    for target in &source.targets {
        available(session.document(), &target.input, !edit)?;
        validate_transfer_path(session.document(), &target.path, target.input.body)?;
        target.parameters.resolve(session.parameters())?;
    }
    Ok(())
}
fn group(request: &TransferRequest, source: &TransferDefinition) -> Result<Entity, CommandError> {
    let mut entity = Entity::new(
        &request.name,
        EntityData::Extension {
            definition: source.definition()?,
            inputs: source.inputs(&request.references)?,
        },
    );
    entity.id = request.group;
    Ok(entity)
}
pub fn prepare_transfer(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &TransferRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let source = request.source()?;
    bindings(session, &source, false)?;
    fresh(session.document(), request.group)?;
    let mut identities = BTreeSet::from([request.group]);
    let mut edits = vec![Mutation::Insert(group(request, &source)?)];
    let mut outputs = vec![];
    for target in &source.targets {
        for id in [target.id, target.feature] {
            fresh(session.document(), id)?;
            if !identities.insert(id) {
                return Err(error("Transfer row/output identities must be distinct"));
            }
        }
        let definition = TransferFeatureDefinition {
            group: request.group,
            target: target.id,
        };
        let mut feature = Entity::new(
            "Transferred machining",
            EntityData::Feature {
                definition: definition.definition()?,
                inputs: vec![
                    Link::new(request.group, "transfer_group"),
                    Link::new(target.input.producer, "prior_solid"),
                ],
                suppressed: false,
            },
        );
        feature.id = target.feature;
        let mut part = session
            .document()
            .get(target.input.body)
            .expect("checked input")
            .clone();
        let EntityData::Part { features, .. } = &mut part.data else {
            unreachable!()
        };
        features.push(target.feature);
        edits.extend([Mutation::Insert(feature), Mutation::Replace(part)]);
        outputs.push(CreatedBody {
            body: target.input.body,
            feature: target.feature,
        });
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Transfer source holes and cutouts".into(),
        edits,
    })?;
    Ok(ModelPrepared { prepared, outputs })
}
pub fn prepare_transfer_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &TransferRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let (old, _) = read_transfer(session.document(), request.group)?;
    let source = request.source()?;
    if old.targets.len() != source.targets.len()
        || old.targets.iter().any(|prior| {
            !source.targets.iter().any(|next| {
                next.id == prior.id && next.feature == prior.feature && next.input == prior.input
            })
        })
    {
        return Err(error(
            "Applied transfer edits must preserve receiving row, output, Part and frozen input identities; edit source/parameters/paths or repair captures",
        ));
    }
    bindings(session, &source, true)?;
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit or repair machining transfer".into(),
        edits: vec![Mutation::Replace(group(request, &source)?)],
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: source
            .targets
            .iter()
            .map(|t| CreatedBody {
                body: t.input.body,
                feature: t.feature,
            })
            .collect(),
    })
}

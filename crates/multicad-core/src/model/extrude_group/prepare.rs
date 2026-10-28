use super::super::{
    self as model, BodyOutput, BooleanDefinition,
    boolean::available,
    common::{error, revision},
    prepare::termination_inputs,
};
use super::*;
use crate::{
    commands::{Actor, CommandError, ErrorCode, Request, Session},
    document::{Entity, EntityData, Mutation},
    hierarchy,
};
use std::collections::{BTreeMap, BTreeSet};

pub(in crate::model) fn creation_entity(
    document: &Document,
    draft: &ExtrudeGroupDraft,
    request: &model::ExtrudeBooleanRequest,
) -> Result<Entity, CommandError> {
    if draft.id.is_nil() || draft.id == document.id() || document.get(draft.id).is_some() {
        return Err(error(
            "Extrusion operation identity must be new and non-nil",
        ));
    }
    if draft.name.trim().is_empty() || draft.name.len() > 1000 {
        return Err(error("Extrusion operation name must contain 1–1000 bytes"));
    }
    let source = ExtrudeGroupDefinition {
        tools: request
            .extrusion
            .outputs
            .iter()
            .map(|o| BodyInput {
                body: o.body,
                producer: o.feature,
            })
            .collect(),
        results: request
            .booleans
            .iter()
            .map(|r| ExtrudeGroupResult {
                body: r.target.body,
                feature: r.feature,
            })
            .collect(),
        captured_scope: draft.captured_scope.clone(),
    };
    let mut entity = Entity::new(
        &draft.name,
        EntityData::Extension {
            definition: source.definition()?,
            inputs: source.inputs()?,
        },
    );
    entity.id = draft.id;
    Ok(entity)
}
fn removal(
    document: &Document,
    group: EntityId,
    body: EntityId,
    feature: EntityId,
) -> Result<(), CommandError> {
    let EntityData::Part { features, .. } = &document
        .get(body)
        .ok_or(crate::document::DocumentError::Missing(body))?
        .data
    else {
        return Err(error("Captured target is no longer a Part"));
    };
    let mut involved = vec![group, body, feature];
    if features.last() != Some(&feature) {
        involved.extend(
            features
                .iter()
                .skip_while(|id| **id != feature)
                .skip(1)
                .copied(),
        );
        return Err(CommandError{code:ErrorCode::InvalidRequest,message:"Cannot remove this captured target while its Boolean has later Part history; edit or remove the dependent history first".into(),involved,parameter:None});
    }
    let dependents: Vec<_> = document
        .entities()
        .filter(|e| {
            e.id != group
                && e.id != body
                && e.id != feature
                && e.references().iter().any(|link| link.target == feature)
        })
        .map(|e| e.id)
        .collect();
    if !dependents.is_empty() {
        involved.extend(dependents);
        return Err(CommandError{code:ErrorCode::InvalidRequest,message:"Cannot remove this captured target while other source depends on its Boolean; repair or remove those dependencies first".into(),involved,parameter:None});
    }
    Ok(())
}
/// Update all shared child definitions and membership in one candidate. No
/// intermediate Session preparation observes disagreeing group settings.
pub fn prepare_extrude_group_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    edit: &ExtrudeGroupEdit,
) -> Result<ExtrudeGroupPrepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    let state = inspect_extrude_group(document, edit.group)?;
    if !matches!(
        edit.operation,
        Operation::Join | Operation::Cut | Operation::Intersect
    ) {
        return Err(error(
            "Captured extrusion operation supports Join, Cut or Intersect",
        ));
    }
    if edit.targets.is_empty() || edit.targets.len() > MAX_BOOLEAN_TARGETS {
        return Err(error(
            "Choose 1–128 captured targets; converting to New body is a separate operation",
        ));
    }
    edit.parameters.resolve_controls(session.parameters())?;
    let existing: BTreeMap<_, _> = state.booleans.iter().map(|r| (r.target.body, r)).collect();
    let tools = state.definition.tools.clone();
    for tool in &tools {
        available(document, tool, false)?;
    }
    let mut edits = vec![];
    for tool in &tools {
        let mut source = model::read_extrude(document, tool.producer)?;
        source.parameters = edit.parameters.clone();
        let faces = match &edit.faces {
            Some(faces) => faces.clone(),
            None => {
                let wanted = source.face_references();
                validation::faces(document, tool.producer)?
                    .into_iter()
                    .filter(|face| wanted.contains(&face.intent.id))
                    .collect()
            }
        };
        let inputs = source.inputs(&faces)?;
        termination_inputs(document, &source, &inputs)?;
        edits.push(validation::replace_feature(
            document,
            tool.producer,
            source.definition()?,
            inputs,
        )?);
    }
    let index = hierarchy::Index::new(document).map_err(|e| error(e.to_string()))?;
    let mut hierarchy = document.hierarchy().clone();
    let mut targets = BTreeSet::new();
    let mut new_ids = BTreeSet::new();
    let mut outputs = vec![];
    let mut results = vec![];
    for selection in &edit.targets {
        let (feature, target, name, is_new) = match selection {
            ExtrudeGroupTarget::Retain { body } => {
                let old=existing.get(body).ok_or_else(||error("Retain must identify a captured target; new targets require explicit current producer bindings"))?;
                (old.feature, old.target.clone(), old.name.clone(), false)
            }
            ExtrudeGroupTarget::Add {
                target,
                feature,
                name,
            } => {
                if existing.contains_key(&target.body) {
                    return Err(error(
                        "A captured target must retain its saved producer and Boolean identity",
                    ));
                }
                if feature.is_nil()
                    || *feature == document.id()
                    || document.get(*feature).is_some()
                    || !new_ids.insert(*feature)
                {
                    return Err(error(
                        "Added target Boolean identity must be new, distinct and non-nil",
                    ));
                }
                if name.trim().is_empty() || name.len() > 1000 {
                    return Err(error("Added target feature name must contain 1–1000 bytes"));
                }
                (*feature, target.clone(), name.clone(), true)
            }
        };
        if !targets.insert(target.body) {
            return Err(error("Select each captured target Part once"));
        }
        available(document, &target, is_new)?;
        let source = BooleanDefinition {
            output: BodyOutput { body: target.body },
            target: target.clone(),
            tools: tools.clone(),
            operation: edit.operation,
            keep_tools: edit.keep_tools,
        };
        let definition = source.definition()?;
        let inputs = source.inputs()?;
        if is_new {
            let mut entity = Entity::new(
                name,
                EntityData::Feature {
                    definition,
                    inputs,
                    suppressed: false,
                },
            );
            entity.id = feature;
            edits.push(Mutation::Insert(entity));
            let mut part = document.get(target.body).expect("validated target").clone();
            let EntityData::Part { features, .. } = &mut part.data else {
                unreachable!()
            };
            features.push(feature);
            edits.push(Mutation::Replace(part));
            if let Some(parent) = index
                .parent(target.body)
                .map_err(|e| error(e.to_string()))?
            {
                hierarchy.members.entry(parent).or_default().push(feature);
            }
        } else {
            edits.push(validation::replace_feature(
                document, feature, definition, inputs,
            )?);
        }
        outputs.push(CreatedBody {
            feature,
            body: target.body,
        });
        results.push(ExtrudeGroupResult {
            body: target.body,
            feature,
        });
    }
    let mut removed = vec![];
    let mut restored_bodies = vec![];
    for old in &state.definition.results {
        if targets.contains(&old.body) {
            continue;
        }
        removal(document, edit.group, old.body, old.feature)?;
        let mut part = document.get(old.body).expect("inspected member").clone();
        let EntityData::Part { features, .. } = &mut part.data else {
            unreachable!()
        };
        features.retain(|id| *id != old.feature);
        edits.extend([Mutation::Replace(part), Mutation::Remove(old.feature)]);
        for members in hierarchy.members.values_mut() {
            members.retain(|id| *id != old.feature);
        }
        hierarchy.hidden.remove(&old.feature);
        removed.push(old.feature);
        restored_bodies.push(old.body);
    }
    let source = ExtrudeGroupDefinition {
        tools,
        results,
        captured_scope: state.definition.captured_scope,
    };
    let mut group = document.get(edit.group).expect("inspected group").clone();
    group.data = EntityData::Extension {
        definition: source.definition()?,
        inputs: source.inputs()?,
    };
    edits.push(Mutation::Replace(group));
    if hierarchy != *document.hierarchy() {
        edits.push(Mutation::SetHierarchy(hierarchy));
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit extrusion operation".into(),
        edits,
    })?;
    let restored = restored_bodies
        .into_iter()
        .map(|body| {
            let EntityData::Part { features, .. } =
                &prepared.document().get(body).expect("retained Part").data
            else {
                unreachable!()
            };
            let producer = features
                .iter()
                .rev()
                .copied()
                .find(|id| {
                    matches!(
                        prepared.document().get(*id).map(|e| &e.data),
                        Some(EntityData::Feature {
                            suppressed: false,
                            ..
                        })
                    )
                })
                .ok_or_else(|| error("Removed target has no restored producer"))?;
            Ok(BodyInput { body, producer })
        })
        .collect::<Result<Vec<_>, CommandError>>()?;
    Ok(ExtrudeGroupPrepared {
        prepared,
        outputs,
        restored,
        removed,
    })
}

use super::super::{self as model, ExtrudeOutput};
use super::*;
use crate::{
    commands::{CommandError, ErrorCode},
    document::{EntityData, Mutation},
};
use std::collections::BTreeMap;

fn metadata(document: &Document, group: EntityId) -> Result<ExtrudeGroupDefinition, DocumentError> {
    let Some(EntityData::Extension { definition, inputs }) = document.get(group).map(|e| &e.data)
    else {
        return Err(invalid("Choose an extrusion operation group"));
    };
    let source = ExtrudeGroupDefinition::from_definition(definition)?;
    let expected = source.inputs()?;
    if inputs.len() != expected.len() || expected.iter().any(|link| !inputs.contains(link)) {
        return Err(invalid(
            "Extrusion operation membership and canonical links disagree",
        ));
    }
    Ok(source)
}
pub(super) fn faces(
    document: &Document,
    feature: EntityId,
) -> Result<Vec<FaceInput>, DocumentError> {
    let Some(EntityData::Feature { inputs, .. }) = document.get(feature).map(|e| &e.data) else {
        return Err(DocumentError::Missing(feature));
    };
    // Termination faces only; a profile face belongs to its own tool output.
    let faces: Vec<_> = inputs
        .iter()
        .filter(|link| link.role != model::PROFILE_FACE_ROLE)
        .filter_map(|link| {
            link.geometry.as_ref().map(|intent| FaceInput {
                source: link.target,
                intent: intent.clone(),
            })
        })
        .collect();
    Ok(faces)
}
/// The profile face input of a face-profile tool, if any.
pub(super) fn profile_face(
    document: &Document,
    feature: EntityId,
) -> Result<Option<FaceInput>, DocumentError> {
    let Some(EntityData::Feature { inputs, .. }) = document.get(feature).map(|e| &e.data) else {
        return Err(DocumentError::Missing(feature));
    };
    Ok(model::RevolveDefinition::stored_faces(inputs)
        .into_iter()
        .next())
}
fn known_part(document: &Document, body: EntityId, feature: EntityId) -> Result<(), DocumentError> {
    if let Some(entity) = document.get(body) {
        let EntityData::Part { features, .. } = &entity.data else {
            return Err(invalid(
                "Extrusion operation member body must identify a Part",
            ));
        };
        if !features.contains(&feature) {
            return Err(invalid(
                "Extrusion operation producer is absent from its Part history",
            ));
        }
    }
    Ok(())
}
pub(super) fn read(
    document: &Document,
    group: EntityId,
) -> Result<ExtrudeGroupDefinition, DocumentError> {
    let source = metadata(document, group)?;
    let mut common = None;
    let mut regions = vec![];
    for tool in &source.tools {
        known_part(document, tool.body, tool.producer)?;
        if document.get(tool.producer).is_some() {
            let tool_source = model::read_extrude(document, tool.producer)?;
            if tool_source.output.body != tool.body
                || tool_source.operation != Operation::NewBody
                || !tool_source.targets.is_empty()
                || tool_source.keep_tools
            {
                return Err(invalid(
                    "Extrusion operation tools must retain their own new-body sources",
                ));
            }
            let mut common_faces = faces(document, tool.producer)?;
            common_faces.sort_by_key(|face| face.intent.id);
            let settings = (
                tool_source.profile.sketch_id(),
                tool_source.parameters.clone(),
                common_faces,
            );
            if common.as_ref().is_some_and(|old| old != &settings) {
                return Err(invalid(
                    "Extrusion operation tools disagree; edit their shared controls through the operation group",
                ));
            }
            common = Some(settings);
            if regions.contains(&tool_source.profile) {
                return Err(invalid(
                    "Extrusion operation contains duplicate tool profile intents",
                ));
            }
            regions.push(tool_source.profile);
        }
    }
    let tools: BTreeSet<_> = source.tools.iter().map(|t| (t.body, t.producer)).collect();
    let mut common = None;
    for result in &source.results {
        known_part(document, result.body, result.feature)?;
        if document.get(result.feature).is_some() {
            let boolean = model::read_boolean(document, result.feature)?;
            if boolean.output.body != result.body
                || boolean.target.body != result.body
                || boolean.tools.len() != tools.len()
                || boolean
                    .tools
                    .iter()
                    .map(|t| (t.body, t.producer))
                    .collect::<BTreeSet<_>>()
                    != tools
            {
                return Err(invalid(
                    "Extrusion operation Boolean membership or tool bindings disagree",
                ));
            }
            let settings = (boolean.operation, boolean.keep_tools);
            if common.is_some_and(|old| old != settings) {
                return Err(invalid(
                    "Extrusion operation Boolean settings disagree; edit the complete operation",
                ));
            }
            common = Some(settings);
        }
    }
    Ok(source)
}
fn group_ids(document: &Document) -> impl Iterator<Item = EntityId> + '_ {
    document.entities().filter_map(|entity|matches!(&entity.data,EntityData::Extension{definition,..} if definition.type_id==EXTRUDE_GROUP_TYPE).then_some(entity.id))
}
pub(in crate::model) fn validate_document(document: &Document) -> Result<(), DocumentError> {
    let mut owners = BTreeMap::new();
    for group in group_ids(document) {
        let source = read(document, group)?;
        for feature in source
            .tools
            .iter()
            .map(|t| t.producer)
            .chain(source.results.iter().map(|r| r.feature))
        {
            if owners.insert(feature, group).is_some() {
                return Err(invalid(format!(
                    "Feature {feature} belongs to more than one extrusion operation"
                )));
            }
        }
    }
    Ok(())
}
pub(super) fn owner(
    document: &Document,
    feature: EntityId,
) -> Result<Option<EntityId>, DocumentError> {
    let mut owner = None;
    for group in group_ids(document) {
        let source = read(document, group)?;
        if (source.tools.iter().any(|t| t.producer == feature)
            || source.results.iter().any(|r| r.feature == feature))
            && owner.replace(group).is_some()
        {
            return Err(invalid(format!(
                "Feature {feature} has conflicting extrusion operation owners"
            )));
        }
    }
    Ok(owner)
}
pub(in crate::model) fn reject_individual_edit(
    document: &Document,
    feature: EntityId,
) -> Result<(), CommandError> {
    if let Some(group) = owner(document, feature)? {
        return Err(CommandError {
            code: ErrorCode::InvalidRequest,
            message: format!(
                "Feature belongs to extrusion operation {group}; edit the complete operation"
            ),
            involved: vec![group, feature],
            parameter: None,
        });
    }
    Ok(())
}
pub(super) fn inspect(
    document: &Document,
    group: EntityId,
) -> Result<ExtrudeGroupState, DocumentError> {
    let definition = read(document, group)?;
    let mut outputs = vec![];
    let mut first = None;
    for tool in &definition.tools {
        tool.resolve_producer(document)?;
        let source = model::read_extrude(document, tool.producer)?;
        if first.is_none() {
            first = Some((
                source.profile.sketch_id().unwrap_or(EntityId::nil()),
                source.parameters.clone(),
                faces(document, tool.producer)?,
            ));
        }
        outputs.push(ExtrudeOutput {
            feature: tool.producer,
            body: tool.body,
            name: document
                .get(tool.body)
                .ok_or(DocumentError::Missing(tool.body))?
                .name
                .clone(),
            region: source
                .profile
                .sketch()
                .map(|profile| profile.region.clone())
                .unwrap_or_else(model::empty_region),
            face: profile_face(document, tool.producer)?,
        });
    }
    let (sketch, parameters, faces) =
        first.ok_or_else(|| invalid("Extrusion operation has no editable tools"))?;
    let mut booleans = vec![];
    for result in &definition.results {
        let source = model::read_boolean(document, result.feature)?;
        source.target.resolve_producer(document)?;
        for tool in &source.tools {
            tool.resolve_producer(document)?;
        }
        booleans.push(BooleanRequest {
            feature: result.feature,
            name: document
                .get(result.feature)
                .ok_or(DocumentError::Missing(result.feature))?
                .name
                .clone(),
            target: source.target,
            tools: source.tools,
            operation: source.operation,
            keep_tools: source.keep_tools,
        });
    }
    Ok(ExtrudeGroupState {
        group,
        schema_version: match &document.get(group).expect("read group").data {
            EntityData::Extension { definition, .. } => definition.schema_version,
            _ => unreachable!(),
        },
        definition,
        extrusion: ExtrudeRequest {
            sketch,
            outputs,
            parameters,
            operation: Operation::NewBody,
            targets: vec![],
            keep_tools: false,
            faces,
        },
        booleans,
    })
}
// Keep pure mutation helpers colocated with validation of their source data.
pub(super) fn replace_feature(
    document: &Document,
    id: EntityId,
    definition: Definition,
    inputs: Vec<Link>,
) -> Result<Mutation, DocumentError> {
    let mut entity = document.get(id).ok_or(DocumentError::Missing(id))?.clone();
    let EntityData::Feature {
        definition: d,
        inputs: i,
        ..
    } = &mut entity.data
    else {
        return Err(invalid("Choose an existing model Feature"));
    };
    *d = definition;
    *i = inputs;
    Ok(Mutation::Replace(entity))
}

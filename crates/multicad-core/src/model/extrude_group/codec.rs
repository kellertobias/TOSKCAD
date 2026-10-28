use super::*;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BooleanV1 {
    tools: Vec<BodyInput>,
    results: Vec<ExtrudeGroupResult>,
    captured_scope: Option<ExtrudeCapturedScope>,
}
pub(super) fn encode(
    source: &ExtrudeGroupDefinition,
    schema: u32,
) -> Result<Definition, DocumentError> {
    source.validate()?;
    if source.source_schema.is_some_and(|old| old > schema) {
        return Err(invalid(
            "Extrusion operation schemas cannot be downgraded implicitly",
        ));
    }
    let data = match schema {
        EXTRUDE_GROUP_SCHEMA if source.kind == ExtrudeGroupKind::Boolean => {
            serde_json::to_value(BooleanV1 {
                tools: source.tools.clone(),
                results: source.results.clone(),
                captured_scope: source.captured_scope.clone(),
            })
        }
        EXTRUDE_GROUP_V2_SCHEMA => serde_json::to_value(source),
        _ => {
            return Err(invalid(
                "Extrusion operation kind cannot be represented in this schema",
            ));
        }
    }
    .map_err(|e| invalid(e.to_string()))?;
    Ok(Definition {
        type_id: EXTRUDE_GROUP_TYPE.into(),
        schema_version: schema,
        data,
    })
}
pub(super) fn decode(definition: &Definition) -> Result<ExtrudeGroupDefinition, DocumentError> {
    if definition.type_id != EXTRUDE_GROUP_TYPE {
        return Err(invalid("Unsupported extrusion operation group type"));
    }
    let mut source = match definition.schema_version {
        EXTRUDE_GROUP_SCHEMA => {
            let old: BooleanV1 = serde_json::from_value(definition.data.clone())
                .map_err(|e| invalid(e.to_string()))?;
            ExtrudeGroupDefinition {
                source_schema: Some(EXTRUDE_GROUP_SCHEMA),
                kind: ExtrudeGroupKind::Boolean,
                tools: old.tools,
                results: old.results,
                captured_scope: old.captured_scope,
            }
        }
        EXTRUDE_GROUP_V2_SCHEMA => {
            serde_json::from_value(definition.data.clone()).map_err(|e| invalid(e.to_string()))?
        }
        _ => return Err(invalid("Unsupported extrusion operation group version")),
    };
    source.source_schema = Some(definition.schema_version);
    source.validate()?;
    Ok(source)
}

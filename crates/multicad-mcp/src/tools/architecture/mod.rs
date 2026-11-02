//! External control uses the editor's architectural source and native transaction gate.
use super::{
    Quantity, Request, ToolResult, compile, fences, id, imports, revision_guard, snapshot,
};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    document::Document,
    model::*,
    parameters::EvaluatedParameters,
    symbols::{self, architecture::*},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

mod args;
mod inspection;
mod library;
mod values;
pub(super) use args::*;
pub(super) use inspection::inspect;
pub(super) use library::{compare, import, update};
fn typed<T: serde::de::DeserializeOwned>(value: impl Serialize) -> Result<T, ToolError> {
    serde_json::from_value(
        serde_json::to_value(value).map_err(|e| ToolError::invalid(e.to_string()))?,
    )
    .map_err(|e| ToolError::invalid(e.to_string()))
}
fn finish(
    host: &dyn Host,
    call: &Request,
    prepared: multicad_core::commands::Prepared,
    revision: u64,
    mut result: serde_json::Value,
) -> ToolResult {
    let model = compile(prepared.document(), revision, call)?;
    result["model_part_count"] = json!(model.snapshot().bodies.len());
    let owned = call.clone();
    result["revision"] = json!(with(host, move |target| super::commit(
        target, &owned, prepared
    ))??);
    result["native_model_preflight"] = json!(true);
    if result.get("native_geometry_checked").is_none() {
        result["native_geometry_checked"] = json!(true);
    }
    Ok(result)
}
pub(super) fn author(host: &dyn Host, call: &Request, args: AuthorArgs) -> ToolResult {
    let revision = args.expected_revision;
    let owned = call.clone();
    let (prepared, result) = with(host, move |target| {
        revision_guard(target, revision)?;
        let s = target.session();
        let construction = id(&args.construction)?;
        let role = match &s
            .document()
            .get(construction)
            .ok_or_else(|| ToolError::invalid("Construction is missing"))?
            .data
        {
            multicad_core::document::EntityData::Extension { definition, .. }
                if definition.type_id == WALL_TYPE_KIND
                    && definition.schema_version == LAYERED_WALL_TYPE_SCHEMA =>
            {
                ArchitectureRole::Wall
            }
            multicad_core::document::EntityData::Extension { definition, .. }
                if definition.type_id == FENCE_TYPE_KIND
                    && definition.schema_version == FENCE_SCHEMA =>
            {
                ArchitectureRole::Fence
            }
            _ => {
                return Err(ToolError::invalid(
                    "Choose a canonical layered-wall or fence construction",
                ));
            }
        };
        let mut interface = ArchitectureBlueprint::new(role, construction);
        let identity = |value: Option<String>| {
            value
                .as_deref()
                .map(id)
                .transpose()
                .map(|v| v.unwrap_or_default())
        };
        for v in args.dimensions {
            interface.dimensions.push(ArchitectureDimensionSlot {
                id: identity(v.id)?,
                name: v.name,
                target: typed(v.target)?,
            });
        }
        for v in args.materials {
            interface.materials.push(ArchitectureMaterialSlot {
                id: identity(v.id)?,
                name: v.name,
                target: typed(v.target)?,
                choices: v.choices.iter().map(|v| id(v)).collect::<Result<_, _>>()?,
                allow_project_materials: v.allow_project_materials,
            });
        }
        for v in args.options {
            interface.options.push(ArchitectureOptionSlot {
                id: identity(v.id)?,
                name: v.name,
                target: typed(v.target)?,
                choices: v.choices.into_iter().map(typed).collect::<Result<_, _>>()?,
            });
        }
        let blueprint = identity(args.blueprint)?;
        let mut symbol = symbols::SymbolAuthorRequest::new(
            args.name,
            blueprint,
            symbols::SymbolKind::Dynamic,
            args.exposed_parameters
                .iter()
                .map(|v| id(v))
                .collect::<Result<_, _>>()?,
            None,
        );
        if let Some(value) = args.symbol {
            symbol.symbol = id(&value)?;
        }
        symbol.revision = args.revision;
        let request = ArchitectureAuthorRequest {
            blueprint,
            interface,
            symbol,
        };
        let prepared = prepare_architecture_author(s, revision, owned.actor.clone(), &request)?;
        let definition =
            symbols::read_definition(prepared.prepared.document(), prepared.definition)?;
        Ok::<_, ToolError>((
            prepared.prepared,
            json!({"definition":prepared.definition,"symbol":definition.symbol,"blueprint":blueprint,"interface":blueprint_for_definition(&definition)?,"native_geometry_checked":false,"geometry_scope":"Construction source only; sample path excluded. Placed geometry is checked when drawn."}),
        ))
    })??;
    finish(host, call, prepared, revision, result)
}
pub(super) fn place(host: &dyn Host, call: &Request, args: PlaceArgs) -> ToolResult {
    let revision = args.expected_revision;
    let owned = call.clone();
    let (prepared, result) = with(host, move |target| {
        revision_guard(target, revision)?;
        let s = target.session();
        let request = ArchitecturePlaceRequest::new(
            s.document(),
            id(&args.definition)?,
            args.name,
            args.inputs.value(s.document(), s.parameters(), None)?,
            args.context.value(s.document(), s.parameters(), None)?,
        )?;
        let result = prepare_architecture_place(s, revision, owned.actor.clone(), &request)?;
        Ok::<_, ToolError>((
            result.prepared,
            json!({"record":result.record,"output":result.output}),
        ))
    })??;
    finish(host, call, prepared, revision, result)
}
pub(super) fn edit(host: &dyn Host, call: &Request, args: EditArgs) -> ToolResult {
    let revision = args.expected_revision;
    let owned = call.clone();
    let (prepared, result) = with(host, move |target| {
        revision_guard(target, revision)?;
        let s = target.session();
        let (record, old) = architecture_instance_for_output(s.document(), id(&args.entity)?)?
            .ok_or_else(|| ToolError::invalid("Choose an architectural symbol output"))?;
        let request = ArchitectureEditRequest {
            record,
            inputs: args
                .inputs
                .value(s.document(), s.parameters(), Some(&old.inputs))?,
            context: args
                .context
                .map(|v| v.value(s.document(), s.parameters(), Some(&old)))
                .transpose()?,
        };
        let result = prepare_architecture_edit(s, revision, owned.actor.clone(), &request)?;
        Ok::<_, ToolError>((
            result.prepared,
            json!({"record":result.record,"output":result.output}),
        ))
    })??;
    finish(host, call, prepared, revision, result)
}

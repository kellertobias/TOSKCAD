//! Full canonical connection requests shared with the native editor.
use super::{Request, ToolResult, commit, compile, id, revision_guard, snapshot};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::model;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScrewConnectionArgs {
    pub expected_revision: u64,
    /// Complete canonical ScrewConnectionRequest, including stable retained IDs,
    /// typed dimensions, hardware sources, axis and ordered stack. This is not
    /// a partial patch. Obtain an editable request with inspect_screw_connection.
    pub request: Map<String, Value>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScrewConnectionInspectArgs {
    pub connection: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScrewConnectionRemoveArgs {
    pub expected_revision: u64,
    pub connection: String,
}
pub(super) fn inspect_screw_connection(
    host: &dyn Host,
    _: &Request,
    args: ScrewConnectionInspectArgs,
) -> ToolResult {
    let (document, revision, _) = snapshot(host)?;
    let connection = id(&args.connection)?;
    Ok(json!({"revision":revision,"connection":connection,
        "request":model::read_screw_connection_request(&document, connection)?,
        "definition":model::read_screw_connection(&document, connection)?}))
}
pub(super) fn set_screw_connection(
    host: &dyn Host,
    call: &Request,
    args: ScrewConnectionArgs,
) -> ToolResult {
    let request: model::ScrewConnectionRequest =
        serde_json::from_value(Value::Object(args.request)).map_err(|error| {
            ToolError::invalid(format!(
                "Invalid canonical screw connection request: {error}"
            ))
        })?;
    let connection = request.connection;
    let expected = args.expected_revision;
    let owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, expected)?;
        Ok::<_, ToolError>(
            model::prepare_screw_connection(
                target.session(),
                expected,
                owned.actor.clone(),
                &request,
            )?
            .prepared,
        )
    })??;
    let native = compile(prepared.document(), expected, call)?;
    let checks = native.snapshot().screw_connection_checks.clone();
    if !checks.iter().any(|check| check.connection == connection) {
        return Err(ToolError {
            code: "model_failed",
            message: "The requested screw connection has no geometry validation receipt".into(),
            involved: vec![connection],
            current_revision: None,
        });
    }
    let owned = call.clone();
    let revision = with(host, move |target| commit(target, &owned, prepared))??;
    Ok(
        json!({"revision":revision,"connection":connection,"native_geometry_checked":true,"checks":checks}),
    )
}
pub(super) fn remove_screw_connection(
    host: &dyn Host,
    call: &Request,
    args: ScrewConnectionRemoveArgs,
) -> ToolResult {
    let request = model::ScrewConnectionRemoveRequest {
        connection: id(&args.connection)?,
    };
    let expected = args.expected_revision;
    let owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, expected)?;
        Ok::<_, ToolError>(model::prepare_remove_screw_connection(
            target.session(),
            expected,
            owned.actor.clone(),
            &request,
        )?)
    })??;
    compile(prepared.document(), expected, call)?;
    let owned = call.clone();
    let revision = with(host, move |target| commit(target, &owned, prepared))??;
    Ok(json!({"revision":revision,"native_geometry_checked":true}))
}

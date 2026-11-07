//! Canonical profile connections shared with the native editor.
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
pub struct ProfileConnectionArgs {
    pub expected_revision: u64,
    /// Complete canonical ProfileConnectionRequest: connection UUID, name, kind,
    /// optional nut choice, and first and second attachments. Each attachment includes member body and
    /// producer IDs, the exact occurrence path, and a groove or end anchor.
    /// Use inspect_profile_connection to retrieve an existing editable request.
    pub request: Map<String, Value>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileConnectionInspectArgs {
    pub connection: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileConnectionRemoveArgs {
    pub expected_revision: u64,
    pub connection: String,
}

pub(super) fn inspect_profile_connection(
    host: &dyn Host,
    _: &Request,
    args: ProfileConnectionInspectArgs,
) -> ToolResult {
    let (document, revision, _) = snapshot(host)?;
    let connection = id(&args.connection)?;
    let request = model::read_profile_connection_request(&document, connection)?;
    Ok(json!({
        "revision": revision,
        "connection": connection,
        "article": request.kind.article(),
        "nut_article": request.nut.article(),
        "request": request,
    }))
}

pub(super) fn set_profile_connection(
    host: &dyn Host,
    call: &Request,
    args: ProfileConnectionArgs,
) -> ToolResult {
    let request: model::ProfileConnectionRequest =
        serde_json::from_value(Value::Object(args.request)).map_err(|error| {
            ToolError::invalid(format!(
                "Invalid canonical profile connection request: {error}"
            ))
        })?;
    let connection = request.connection;
    let expected = args.expected_revision;
    call.check()?;
    let owned = call.clone();
    let model::ModelPrepared { prepared, outputs } = with(host, move |target| {
        revision_guard(target, expected)?;
        Ok::<_, ToolError>(model::prepare_profile_connection(
            target.session(),
            expected,
            owned.actor.clone(),
            &request,
        )?)
    })??;
    compile(prepared.document(), expected, call)?;
    let owned = call.clone();
    let revision = with(host, move |target| commit(target, &owned, prepared))??;
    let parts: Vec<_> = outputs
        .into_iter()
        .map(|part| json!({"part": part.body, "feature": part.feature}))
        .collect();
    Ok(json!({
        "revision": revision,
        "connection": connection,
        "parts": parts,
        "native_geometry_checked": true,
    }))
}

pub(super) fn remove_profile_connection(
    host: &dyn Host,
    call: &Request,
    args: ProfileConnectionRemoveArgs,
) -> ToolResult {
    let connection = id(&args.connection)?;
    let expected = args.expected_revision;
    call.check()?;
    let owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, expected)?;
        Ok::<_, ToolError>(model::prepare_remove_profile_connection(
            target.session(),
            expected,
            owned.actor.clone(),
            connection,
        )?)
    })??;
    compile(prepared.document(), expected, call)?;
    let owned = call.clone();
    let revision = with(host, move |target| commit(target, &owned, prepared))??;
    Ok(json!({
        "revision": revision,
        "connection": connection,
        "native_geometry_checked": true,
    }))
}

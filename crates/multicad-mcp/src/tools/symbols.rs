//! Thin controls over canonical pinned definitions and owned instance overrides.
use super::{Quantity, Request, ToolResult, commit, compile, id, imports, revision_guard};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{commands::Prepared, document::Placement, project_file, symbols};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportSymbolArgs {
    pub expected_revision: u64,
    pub file_name: String,
    /// Required when the file contains more than one reusable definition.
    pub source_definition: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SymbolValueArgs {
    pub value: f64,
    pub unit: String,
}
#[derive(Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SymbolPlacementArgs {
    /// Three typed lengths; omission places the underhead/root datum at zero.
    pub translation: Option<[Quantity; 3]>,
    /// Unit quaternion in x/y/z/w order; omission is identity.
    pub rotation_xyzw: Option<[f64; 4]>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlaceSymbolArgs {
    pub expected_revision: u64,
    pub definition: String,
    pub name: String,
    /// Complete map keyed by the exposed IDs in the pinned template.
    pub overrides: BTreeMap<String, SymbolValueArgs>,
    #[serde(default)]
    pub placement: SymbolPlacementArgs,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditSymbolInstanceArgs {
    pub expected_revision: u64,
    /// Placed instance ID or its symbol ownership record ID.
    pub instance: String,
    /// Replacement map, not a patch. Empty restores all pinned defaults.
    pub overrides: BTreeMap<String, SymbolValueArgs>,
}
fn overrides(
    values: BTreeMap<String, SymbolValueArgs>,
) -> Result<BTreeMap<multicad_core::document::EntityId, symbols::SymbolValue>, ToolError> {
    values
        .into_iter()
        .map(|(key, value)| {
            if !value.value.is_finite() {
                return Err(ToolError::invalid("Symbol override must be finite"));
            }
            Ok((
                id(&key)?,
                symbols::SymbolValue {
                    value: value.value,
                    unit: value.unit,
                },
            ))
        })
        .collect()
}
fn finish(
    host: &dyn Host,
    call: &Request,
    prepared: Prepared,
    revision: u64,
    mut result: Value,
) -> ToolResult {
    compile(prepared.document(), revision, call)?;
    let owned = call.clone();
    result["revision"] = json!(with(host, move |target| {
        // Shared commit validates the prepared owner session and revision
        // before either trusted commit or review queueing.
        commit(target, &owned, prepared)
    })??);
    result["native_model_preflight"] = json!(true);
    Ok(result)
}
pub(super) fn import_symbol(host: &dyn Host, call: &Request, args: ImportSymbolArgs) -> ToolResult {
    let path = imports::scoped(host, &args.file_name, &[".tcd"])?
        .canonicalize()
        .map_err(|e| ToolError::invalid(e.to_string()))?;
    let folder = host
        .scope()
        .export_directory
        .ok_or_else(|| ToolError::new("out_of_scope", "No exchange folder"))?
        .canonicalize()
        .map_err(|e| ToolError::invalid(e.to_string()))?;
    if path.parent() != Some(folder.as_path()) {
        return Err(ToolError::new(
            "out_of_scope",
            "Symbol library must remain inside the exchange folder",
        ));
    }
    let project = project_file::read(
        &mut std::fs::File::open(path).map_err(|e| ToolError::invalid(e.to_string()))?,
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .map_err(|e| ToolError::invalid(e.to_string()))?
    .project;
    let selected = args.source_definition.as_deref().map(id).transpose()?;
    let mut definitions: Vec<_> = symbols::definitions(&project.document)?
        .into_iter()
        .filter(|(key, _)| selected.is_none_or(|id| id == *key))
        .collect();
    if definitions.len() != 1 {
        return Err(ToolError::invalid(
            "Choose exactly one source definition from the library file",
        ));
    }
    let (source_id, definition) = definitions.remove(0);
    let pin = definition.pin()?;
    let symbol = definition.symbol;
    let exposed = definition.exposed.clone();
    let revision = args.expected_revision;
    let owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, revision)?;
        Ok::<_, ToolError>(symbols::prepare_import(
            target.session(),
            revision,
            owned.actor.clone(),
            &definition,
        )?)
    })??;
    let key = prepared.definition;
    finish(
        host,
        call,
        prepared.prepared,
        revision,
        json!({"definition":key,"source_definition":source_id,"symbol":symbol,"pin":pin,"exposed":exposed,"native_geometry_checked":false,"geometry_scope":"Pinned source imported; place an instance to evaluate its geometry"}),
    )
}
pub(super) fn place_symbol(host: &dyn Host, call: &Request, args: PlaceSymbolArgs) -> ToolResult {
    let revision = args.expected_revision;
    let definition = id(&args.definition)?;
    let values = overrides(args.overrides)?;
    let owned = call.clone();
    let (prepared, record, instance, member) = with(host, move |target| {
        revision_guard(target, revision)?;
        let session = target.session();
        let mut placement = Placement::default();
        if let Some(translation) = args.placement.translation {
            placement.translation_mm = [
                translation[0]
                    .dimension(session.document(), session.parameters())?
                    .1,
                translation[1]
                    .dimension(session.document(), session.parameters())?
                    .1,
                translation[2]
                    .dimension(session.document(), session.parameters())?
                    .1,
            ];
        }
        if let Some(rotation) = args.placement.rotation_xyzw {
            placement.rotation_xyzw = rotation;
        }
        let request = symbols::SymbolPlaceRequest::new(
            session.document(),
            definition,
            args.name,
            values,
            placement,
        )?;
        let prepared = symbols::prepare_place(session, revision, owned.actor.clone(), &request)?;
        Ok::<_, ToolError>((
            prepared.prepared,
            request.record,
            request.instance,
            request.member,
        ))
    })??;
    finish(
        host,
        call,
        prepared,
        revision,
        json!({"definition":definition,"record":record,"instance":instance,"member":member,"native_geometry_checked":true}),
    )
}
pub(super) fn edit_symbol_instance(
    host: &dyn Host,
    call: &Request,
    args: EditSymbolInstanceArgs,
) -> ToolResult {
    let revision = args.expected_revision;
    let record = id(&args.instance)?;
    let request = symbols::SymbolEditRequest {
        record,
        overrides: overrides(args.overrides)?,
    };
    let owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, revision)?;
        Ok::<_, ToolError>(symbols::prepare_edit(
            target.session(),
            revision,
            owned.actor.clone(),
            &request,
        )?)
    })??;
    let instances = prepared.instances.clone();
    finish(
        host,
        call,
        prepared.prepared,
        revision,
        json!({"instances":instances,"overrides_semantics":"Replacement map; omitted entries inherit pinned defaults","native_geometry_checked":true}),
    )
}

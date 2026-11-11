//! STEP and KiCad import tools. Files are read only from the exchange
//! folder in scope (`--export-dir`), by bare name; KiCad model lookups are
//! confined to the same folder.
use super::{Request, ToolResult, commit, compile, id, revision_guard};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::model::imports;
use multicad_model::imports::{
    KicadOptions, read_kicad_file_with_backend, read_step_file_with_backend,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StepImportArgs {
    pub expected_revision: u64,
    /// Bare file name in the exchange folder, ending .step or .stp.
    pub file_name: String,
    /// A STEP import to update instead of creating a new one.
    #[serde(default)]
    pub source: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KicadImportArgs {
    pub expected_revision: u64,
    /// Bare file name in the exchange folder, ending .kicad_pcb.
    pub file_name: String,
    /// A board import feature to update instead of creating a new one.
    #[serde(default)]
    pub board: Option<String>,
    /// Footprint references to import as components (default all).
    #[serde(default)]
    pub components: Option<Vec<String>>,
    /// Keep drilled holes at least this wide (default 2 mm: mounting holes).
    #[serde(default)]
    pub min_hole_mm: Option<f64>,
    /// Envelope height for footprints without a model (default 5 mm).
    #[serde(default)]
    pub default_height_mm: Option<f64>,
}

pub(super) fn scoped(
    host: &dyn Host,
    name: &str,
    extensions: &[&str],
) -> Result<PathBuf, ToolError> {
    let directory = host.scope().export_directory.ok_or_else(|| {
        ToolError::new(
            "out_of_scope",
            "No exchange folder is in scope; start the server with --export-dir",
        )
    })?;
    let lower = name.to_ascii_lowercase();
    let valid = !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':', '\0'])
        && extensions.iter().any(|e| lower.ends_with(e));
    if !valid {
        return Err(ToolError::new(
            "out_of_scope",
            format!(
                "file_name must be a bare name ending in {}",
                extensions.join(" or ")
            ),
        ));
    }
    Ok(directory.join(name))
}
fn finish(
    host: &dyn Host,
    call: &Request,
    prepared: multicad_core::commands::Prepared,
    revision: u64,
    mut out: Value,
) -> ToolResult {
    let model = compile(prepared.document(), revision, call)?;
    call.finish(multicad_model::imports::validate_references(
        prepared.document(),
        &model,
    ))?;
    let call = call.clone();
    let committed = with(host, move |target| commit(target, &call, prepared))??;
    out["revision"] = json!(committed);
    Ok(out)
}

pub(super) fn import_step(host: &dyn Host, call: &Request, args: StepImportArgs) -> ToolResult {
    let path = scoped(host, &args.file_name, &[".step", ".stp"])?;
    call.check()?;
    let file = call.finish(read_step_file_with_backend(&path, call.backend))?;
    let source = args.source.as_deref().map(id).transpose()?;
    let revision = args.expected_revision;
    let call_owned = call.clone();
    let (prepared, out) = with(host, move |target| {
        revision_guard(target, revision)?;
        let session = target.session();
        let actor = call_owned.actor.clone();
        Ok::<_, ToolError>(match source {
            Some(id) => {
                let (plan, p) = imports::prepare_step_update(session, revision, actor, id, &file)?;
                let out = json!({
                    "source": id,
                    "matched": plan.matched.iter().map(|(s, i, changed)| json!({ "solid": s, "index": i, "geometry_changed": changed })).collect::<Vec<_>>(),
                    "added": plan.added, "missing": plan.missing,
                });
                (p.model.prepared, out)
            }
            None => {
                let p = imports::prepare_step_import(session, revision, actor, &file)?;
                let out = json!({
                    "source": p.source,
                    "parts": p.model.outputs.iter().map(|o| o.body).collect::<Vec<_>>(),
                });
                (p.model.prepared, out)
            }
        })
    })??;
    finish(host, call, prepared, revision, out)
}

pub(super) fn import_kicad(host: &dyn Host, call: &Request, args: KicadImportArgs) -> ToolResult {
    let path = scoped(host, &args.file_name, &[".kicad_pcb"])?;
    call.check()?;
    let options = KicadOptions {
        min_hole_mm: args.min_hole_mm.unwrap_or(2.),
        default_height_mm: args.default_height_mm.unwrap_or(5.),
        components: args.components.clone(),
        model_root: path.parent().map(PathBuf::from),
    };
    let import = call.finish(read_kicad_file_with_backend(&path, &options, call.backend))?;
    let board = args.board.as_deref().map(id).transpose()?;
    let revision = args.expected_revision;
    let call_owned = call.clone();
    let (prepared, out) = with(host, move |target| {
        revision_guard(target, revision)?;
        let session = target.session();
        let actor = call_owned.actor.clone();
        Ok::<_, ToolError>(match board {
            Some(board) => {
                let (plan, p) =
                    imports::prepare_kicad_update(session, revision, actor, board, &import)?;
                let out = json!({
                    "board": board, "outline_changed": plan.outline_changed,
                    "holes_changed": plan.holes_changed, "moved": plan.moved,
                    "unchanged": plan.unchanged, "added": plan.added, "missing": plan.missing,
                });
                (p.prepared, out)
            }
            None => {
                let p = imports::prepare_kicad_import(session, revision, actor, &import)?;
                let out = json!({
                    "board": p.board.feature, "board_part": p.board.body,
                    "holes": import.board.holes.len(),
                    "components": p.components.iter().map(|(r, o, m)| json!({ "reference": r, "part": o.body, "model": m })).collect::<Vec<_>>(),
                    "missing_models": p.missing_models.iter().map(|(r, path)| json!({ "reference": r, "path": path })).collect::<Vec<_>>(),
                    "unsupported": p.unsupported,
                });
                (p.model.prepared, out)
            }
        })
    })??;
    finish(host, call, prepared, revision, out)
}

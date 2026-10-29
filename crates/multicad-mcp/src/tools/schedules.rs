//! Schedules over MCP: the BOM, panel cut list, machining, member cut list
//! and weld schedule as rows, optionally written as CSV into the export
//! folder in scope.
use super::{Request, ToolResult, compile, snapshot};
use crate::{error::ToolError, host::Host};
use multicad_model::{group_cuts, schedules};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleKindArg {
    Bom,
    PanelCutList,
    Machining,
    MemberCutList,
    WeldSchedule,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduleArgs {
    pub kind: ScheduleKindArg,
    /// BOM only: include FDM aid Parts (default false).
    #[serde(default)]
    pub include_aids: bool,
    /// BOM only: include purchased and imported Parts (default true).
    #[serde(default)]
    pub include_purchased: Option<bool>,
    /// Write the schedule as CSV with this bare file name (ending .csv)
    /// into the export folder in scope.
    #[serde(default)]
    pub file_name: Option<String>,
}

pub(super) fn schedule(host: &dyn Host, call: &Request, args: ScheduleArgs) -> ToolResult {
    let target = match &args.file_name {
        Some(name) => {
            let directory = host.scope().export_directory.ok_or_else(|| {
                ToolError::new(
                    "out_of_scope",
                    "No export folder is in scope; start the server with --export-dir",
                )
            })?;
            let valid = !name.is_empty()
                && name.len() <= 200
                && !name.starts_with('.')
                && !name.contains(['/', '\\', ':', '\0'])
                && name.to_ascii_lowercase().ends_with(".csv");
            if !valid {
                return Err(ToolError::new(
                    "out_of_scope",
                    "file_name must be a bare name ending in .csv",
                ));
            }
            Some(directory.join(name))
        }
        None => None,
    };
    let (document, revision, _) = snapshot(host)?;
    let model = compile(&document, revision, call)?;
    let schedule = match args.kind {
        ScheduleKindArg::Bom => {
            let bom = model.bom(
                &document,
                &schedules::BomOptions {
                    include_aids: args.include_aids,
                    include_purchased: args.include_purchased.unwrap_or(true),
                    roots: None,
                },
            )?;
            schedules::bom_schedule(&bom)
        }
        ScheduleKindArg::PanelCutList => {
            schedules::cut_list_schedule(&document, revision, &model.cut_list(&document)?)?
        }
        ScheduleKindArg::Machining => {
            schedules::machining_schedule(&document, revision, &model.machining(&document)?)?
        }
        ScheduleKindArg::MemberCutList => schedules::member_cut_schedule(
            &document,
            revision,
            &group_cuts(&model.member_cut_list(&document)?),
        )?,
        ScheduleKindArg::WeldSchedule => {
            schedules::weld_schedule(&document, revision, &model.weld_schedule(&document)?)?
        }
    };
    let csv = schedule.csv();
    if let Some(path) = &target {
        let partial = path.with_extension("csv.partial");
        std::fs::write(&partial, &csv)
            .and_then(|_| std::fs::rename(&partial, path))
            .map_err(|e| ToolError::new("write_failed", format!("Cannot write the CSV: {e}")))?;
    }
    Ok(json!({
        "kind": schedule.kind, "title": schedule.title, "revision": schedule.revision,
        "source": schedule.source.to_string(), "notes": schedule.notes,
        "columns": schedule.columns, "rows": schedule.rows,
        "written": target.map(|p| p.display().to_string()),
    }))
}

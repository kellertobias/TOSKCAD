//! Sketch text over MCP: the same typed `TextOperation` and solver
//! transaction as the native Text tool. Refusals leave the document unchanged.
use super::{
    Request, ToolResult, commit, editable, id, revision_guard, sketch_entity, solve_failure,
};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::constraint_sketch::text::{
    SketchText, TextAlignment, TextFont, TextOperation, TextStyle,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

#[derive(Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextAlignmentArg {
    #[default]
    Left,
    Center,
    Right,
}
#[derive(Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextFontArg {
    /// Hack Regular, bundled in the application.
    #[default]
    HackRegular,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextStyleArgs {
    /// One line of printable ASCII or the listed Latin-1 letters (ÄÖÜäöüßéèàç°±µ).
    pub text: String,
    /// Cap height in mm (0.1 to 10000).
    pub height_mm: f64,
    /// Baseline anchor in sketch coordinates, mm.
    pub origin_mm: [f64; 2],
    /// Counterclockwise baseline rotation from sketch U, degrees.
    #[serde(default)]
    pub rotation_deg: f64,
    #[serde(default)]
    pub alignment: TextAlignmentArg,
    #[serde(default)]
    pub font: TextFontArg,
}
impl TextStyleArgs {
    fn style(self) -> TextStyle {
        TextStyle {
            text: self.text,
            font: match self.font {
                TextFontArg::HackRegular => TextFont::HackRegular,
            },
            height_mm: self.height_mm,
            origin_mm: self.origin_mm,
            rotation_deg: self.rotation_deg,
            alignment: match self.alignment {
                TextAlignmentArg::Left => TextAlignment::Left,
                TextAlignmentArg::Center => TextAlignment::Center,
                TextAlignmentArg::Right => TextAlignment::Right,
            },
        }
    }
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddTextArgs {
    pub expected_revision: u64,
    pub sketch: String,
    /// One line of printable ASCII or the listed Latin-1 letters (ÄÖÜäöüßéèàç°±µ).
    pub text: String,
    /// Cap height in mm (0.1 to 10000).
    pub height_mm: f64,
    /// Baseline anchor in sketch coordinates, mm.
    pub origin_mm: [f64; 2],
    /// Counterclockwise baseline rotation from sketch U, degrees.
    #[serde(default)]
    pub rotation_deg: f64,
    #[serde(default)]
    pub alignment: TextAlignmentArg,
    #[serde(default)]
    pub font: TextFontArg,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditTextArgs {
    pub expected_revision: u64,
    pub sketch: String,
    /// Text element ID from sketch_add_text or inspect_sketch.
    pub text_id: String,
    /// Replacement string, font, height and placement.
    pub style: TextStyleArgs,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveTextArgs {
    pub expected_revision: u64,
    pub sketch: String,
    pub text_id: String,
}

fn run(
    host: &dyn Host,
    call: &Request,
    expected: u64,
    sketch: &str,
    operation: TextOperation,
) -> ToolResult {
    let call = call.clone();
    let sketch = id(sketch)?;
    with(host, move |target| {
        revision_guard(target, expected)?;
        editable(target)?;
        sketch_entity(target, sketch)?;
        let prepared = multicad_solver::prepare_text(
            target.session(),
            expected,
            call.actor.clone(),
            sketch,
            None,
            &operation,
        )
        .map_err(|e| ToolError::from(e).involving(sketch))?;
        let Some(preview) = prepared.edit.preview else {
            return Err(solve_failure(sketch, &prepared.edit.evaluation.diagnostics));
        };
        let contours = match &operation {
            TextOperation::Remove { .. } => vec![],
            TextOperation::Add { style } | TextOperation::Edit { style, .. } => SketchText {
                id: prepared.text,
                style: style.clone(),
            }
            .contours()
            .map(|c| c.into_iter().map(|c| c.id).collect())
            .unwrap_or_default(),
        };
        let revision = commit(target, &call, preview)?;
        Ok(json!({
            "revision": revision,
            "sketch": sketch,
            "text": prepared.text,
            "contour_curves": contours,
        }))
    })?
}

pub(super) fn sketch_add_text(host: &dyn Host, call: &Request, args: AddTextArgs) -> ToolResult {
    run(
        host,
        call,
        args.expected_revision,
        &args.sketch,
        TextOperation::Add {
            style: TextStyleArgs {
                text: args.text,
                height_mm: args.height_mm,
                origin_mm: args.origin_mm,
                rotation_deg: args.rotation_deg,
                alignment: args.alignment,
                font: args.font,
            }
            .style(),
        },
    )
}
pub(super) fn sketch_edit_text(host: &dyn Host, call: &Request, args: EditTextArgs) -> ToolResult {
    let text = id(&args.text_id)?;
    run(
        host,
        call,
        args.expected_revision,
        &args.sketch,
        TextOperation::Edit {
            text,
            style: args.style.style(),
        },
    )
}
pub(super) fn sketch_remove_text(
    host: &dyn Host,
    call: &Request,
    args: RemoveTextArgs,
) -> ToolResult {
    let text = id(&args.text_id)?;
    run(
        host,
        call,
        args.expected_revision,
        &args.sketch,
        TextOperation::Remove { text },
    )
}

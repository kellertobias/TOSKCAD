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
    SketchText, TextAlignment, TextAnchor, TextFont, TextOperation, TextStyle,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

#[derive(Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextAlignmentArg {
    /// Lines start at the left edge of the text block.
    #[default]
    Left,
    /// Lines are centered in the text block.
    Center,
    /// Lines end at the right edge of the text block.
    Right,
}
#[derive(Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextFontArg {
    /// Cantarell Regular, a proportional sans-serif outline font (OFL-1.1).
    #[default]
    CantarellRegular,
    /// Hack Regular, a monospaced outline font (MIT / Bitstream Vera).
    HackRegular,
}
/// The point of the text block box that sits at `position_mm`. The box spans
/// the widest line's advance width, and from the first line's cap height to
/// the last line's baseline.
#[derive(Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextAnchorArg {
    TopLeft,
    TopCenter,
    TopRight,
    LeftCenter,
    Center,
    RightCenter,
    #[default]
    BottomLeft,
    BottomCenter,
    BottomRight,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextStyleArgs {
    /// Printable ASCII or the listed Latin-1 letters (ÄÖÜäöüßéèàç°±µ); "\n"
    /// starts a new line. Cantarell refuses | and ç (use Hack Regular).
    pub text: String,
    /// Size: the cap height in mm (0.1 to 10000).
    pub height_mm: f64,
    /// Sketch position of the anchor point, mm. `origin_mm` is accepted as
    /// an alias.
    #[serde(alias = "origin_mm")]
    pub position_mm: [f64; 2],
    #[serde(default)]
    pub anchor: TextAnchorArg,
    /// Counterclockwise rotation about the anchor, from sketch U, degrees.
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
                TextFontArg::CantarellRegular => TextFont::CantarellRegular,
                TextFontArg::HackRegular => TextFont::HackRegular,
            },
            height_mm: self.height_mm,
            position_mm: self.position_mm,
            anchor: match self.anchor {
                TextAnchorArg::TopLeft => TextAnchor::TopLeft,
                TextAnchorArg::TopCenter => TextAnchor::TopCenter,
                TextAnchorArg::TopRight => TextAnchor::TopRight,
                TextAnchorArg::LeftCenter => TextAnchor::LeftCenter,
                TextAnchorArg::Center => TextAnchor::Center,
                TextAnchorArg::RightCenter => TextAnchor::RightCenter,
                TextAnchorArg::BottomLeft => TextAnchor::BottomLeft,
                TextAnchorArg::BottomCenter => TextAnchor::BottomCenter,
                TextAnchorArg::BottomRight => TextAnchor::BottomRight,
            },
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
    /// Printable ASCII or the listed Latin-1 letters (ÄÖÜäöüßéèàç°±µ); "\n"
    /// starts a new line. Cantarell refuses | and ç (use Hack Regular).
    pub text: String,
    /// Size: the cap height in mm (0.1 to 10000).
    pub height_mm: f64,
    /// Sketch position of the anchor point, mm. `origin_mm` is accepted as
    /// an alias.
    #[serde(alias = "origin_mm")]
    pub position_mm: [f64; 2],
    #[serde(default)]
    pub anchor: TextAnchorArg,
    /// Counterclockwise rotation about the anchor, from sketch U, degrees.
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
                position_mm: args.position_mm,
                anchor: args.anchor,
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

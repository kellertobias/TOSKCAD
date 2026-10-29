//! Drawing guide tools: author floor plan, section and elevation intent in
//! the model and generate or update the linked drawing views.
use super::{Quantity, Request, ToolResult, commit, compile, id, revision_guard, snapshot};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::drawings::{self, GuideDefinition, GuideKind, Scale};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GuideKindArg {
    /// Horizontal cut on every listed level, looking down.
    Plan {
        /// Cut plane height above each level's floor.
        cut_height: Quantity,
        /// Depth below the cut shown as uncut geometry.
        view_depth: Quantity,
    },
    /// Vertical cut along a plan line, looking toward the line's left side.
    Section {
        start_mm: [f64; 2],
        end_mm: [f64; 2],
        #[serde(default)]
        depth: Option<Quantity>,
    },
    /// Uncut exterior view looking in a plan direction (degrees from +X).
    Elevation { direction_deg: f64 },
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GuideArgs {
    pub expected_revision: u64,
    /// Name for a new guide; omit when editing with `guide`.
    #[serde(default)]
    pub name: Option<String>,
    /// Existing guide to edit.
    #[serde(default)]
    pub guide: Option<String>,
    pub view: GuideKindArg,
    /// Levels: plans repeat on each; sections are limited to their extent.
    #[serde(default)]
    pub levels: Vec<String>,
    /// Parts, Instances or assemblies shown.
    pub sources: Vec<String>,
    /// Drawing scale 1:N.
    pub scale_denominator: u32,
    /// Generate or update the linked drawing views in the same transaction.
    #[serde(default)]
    pub generate_pages: bool,
    /// Drawing book to generate into; omitted reuses the guide's book or
    /// creates one.
    #[serde(default)]
    pub book: Option<String>,
}

pub(super) fn set_drawing_guide(host: &dyn Host, call: &Request, args: GuideArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    if revision != args.expected_revision {
        return Err(ToolError::new(
            "stale_revision",
            format!(
                "Request is based on revision {}, but the document is at {revision}",
                args.expected_revision
            ),
        )
        .at_revision(revision));
    }
    let q = |q: &Quantity| q.dimension(&document, &evaluated).map(|d| d.0);
    let mm = |v: f64| multicad_core::model::length(v);
    let kind = match &args.view {
        GuideKindArg::Plan {
            cut_height,
            view_depth,
        } => GuideKind::Plan {
            cut_height: q(cut_height)?,
            view_depth: q(view_depth)?,
        },
        GuideKindArg::Section {
            start_mm,
            end_mm,
            depth,
        } => GuideKind::Section {
            start: start_mm.map(mm),
            end: end_mm.map(mm),
            depth: depth.as_ref().map(q).transpose()?,
        },
        GuideKindArg::Elevation { direction_deg } => GuideKind::Elevation {
            direction_deg: *direction_deg,
        },
    };
    let guide = GuideDefinition {
        kind,
        levels: args
            .levels
            .iter()
            .map(|l| id(l))
            .collect::<Result<_, _>>()?,
        sources: args
            .sources
            .iter()
            .map(|s| id(s))
            .collect::<Result<_, _>>()?,
        scale: Scale {
            numerator: 1,
            denominator: args.scale_denominator,
        },
    };
    let existing = args.guide.as_deref().map(id).transpose()?;
    let book = args.book.as_deref().map(id).transpose()?;
    let (name, actor, generate) = (
        args.name.unwrap_or_default(),
        call.actor.clone(),
        args.generate_pages,
    );
    let prepared = with(host, move |target| {
        revision_guard(target, revision)?;
        Ok::<_, ToolError>(drawings::prepare_guide(
            target.session(),
            revision,
            actor,
            existing,
            &name,
            &guide,
            generate,
            book,
        )?)
    })??;
    // Generated views are evaluated natively before commit.
    let model = compile(prepared.prepared.document(), revision, call)?;
    let mut unresolved = vec![];
    if let Some(report) = &prepared.generation {
        let evaluation = model
            .drawings(
                prepared.prepared.document(),
                revision,
                &multicad_model::DrawingProviders::default(),
            )
            .ok();
        if let Some(evaluation) = evaluation {
            for view in report.created.iter().chain(&report.updated) {
                if let Some(drawings::ViewEvaluation::Unresolved { message }) =
                    evaluation.views.get(view)
                {
                    unresolved.push(json!({ "view": view, "message": message }));
                }
            }
        }
    }
    let (guide_id, generation) = (prepared.guide, prepared.generation.clone());
    let call_owned = call.clone();
    let committed = with(host, move |target| {
        commit(target, &call_owned, prepared.prepared)
    })??;
    Ok(json!({
        "revision": committed,
        "guide": guide_id,
        "generation": generation.map(|r| json!({
            "book": r.book, "created": r.created, "updated": r.updated,
            "stale": r.stale, "unresolved": unresolved,
        })),
    }))
}

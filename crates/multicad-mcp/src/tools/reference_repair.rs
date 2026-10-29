//! Staged multi-reference repair over the shared repair-set API. Preview and
//! Apply stage the same rows on the current source; Apply commits only when
//! the complete native candidate passes, as one attributable transaction.
use super::{Request, ToolResult, commit, editable, id, revision_guard};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    commands::PreparationSnapshot,
    document::{EntityData, Mutation},
    references::{TopologyKey, TopologyKind},
    sketch_external::{OccurrenceCaptureRequest, RepairSet, RepairStamp, RepairStep},
};
use multicad_model::{Compiler, RepairSetPrepared};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ElementKind {
    Face,
    Edge,
    Vertex,
}
impl From<ElementKind> for TopologyKind {
    fn from(kind: ElementKind) -> Self {
        match kind {
            ElementKind::Face => Self::Face,
            ElementKind::Edge => Self::Edge,
            ElementKind::Vertex => Self::Vertex,
        }
    }
}
fn kind_name(kind: TopologyKind) -> &'static str {
    match kind {
        TopologyKind::Face => "face",
        TopologyKind::Edge => "edge",
        TopologyKind::Vertex => "vertex",
    }
}
/// A current topology element of a source feature, as reported by
/// reference_candidates at the same revision.
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ElementArg {
    pub kind: ElementKind,
    pub ordinal: u32,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateArgs {
    /// Source feature whose current exact topology to list.
    pub source: String,
    /// Optional element kind filter.
    #[serde(default)]
    pub kind: Option<ElementKind>,
}
/// One staged row. Rows apply in order to the cumulative candidate.
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StepArg {
    /// Bind a feature-source sketch reference to an element of a source
    /// feature's current topology (same kind as the old binding).
    Rebind {
        sketch: String,
        reference: String,
        source: String,
        element: ElementArg,
    },
    /// Replace an occurrence reference: the canonical occurrence capture
    /// request (query, selector, behavior and the existing output IDs).
    RebindOccurrence { request: Value },
    /// Remove a reference and its output geometry. Remove its dependent
    /// constraints and geometry in an earlier remove_sketch_items row.
    Remove { sketch: String, reference: String },
    /// Explicitly remove own sketch constraints, curves and points.
    RemoveSketchItems {
        sketch: String,
        #[serde(default)]
        constraints: Vec<String>,
        #[serde(default)]
        curves: Vec<String>,
        #[serde(default)]
        points: Vec<String>,
    },
    /// Canonical entity changes that make a replacement possible, such as
    /// inserting or editing replacement source features. Each edit is
    /// {"operation": "insert" | "replace", "value": entity} or
    /// {"operation": "remove", "value": id}, with entity records in the form
    /// get_entity returns. Reference records are refused.
    SourceEdit { label: String, edits: Vec<Value> },
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepairArgs {
    /// The document revision the rows were chosen at. Stale requests fail.
    pub expected_revision: u64,
    pub steps: Vec<StepArg>,
}

/// Preview rows without committing. Give `at_revision` to refuse a preview
/// once the document has moved; omit it to check the current revision.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewArgs {
    #[serde(default)]
    pub at_revision: Option<u64>,
    pub steps: Vec<StepArg>,
}

fn ids(values: &[String]) -> Result<Vec<multicad_core::document::EntityId>, ToolError> {
    values.iter().map(|v| id(v)).collect()
}
fn step(arg: StepArg) -> Result<RepairStep, ToolError> {
    Ok(match arg {
        StepArg::Rebind {
            sketch,
            reference,
            source,
            element,
        } => RepairStep::Rebind {
            sketch: id(&sketch)?,
            reference: id(&reference)?,
            source: id(&source)?,
            selected: TopologyKey {
                kind: element.kind.into(),
                ordinal: element.ordinal,
            },
        },
        StepArg::RebindOccurrence { request } => RepairStep::RebindOccurrence {
            request: Box::new(
                serde_json::from_value::<OccurrenceCaptureRequest>(request)
                    .map_err(|e| ToolError::invalid(format!("Occurrence request: {e}")))?,
            ),
        },
        StepArg::Remove { sketch, reference } => RepairStep::Remove {
            sketch: id(&sketch)?,
            reference: id(&reference)?,
        },
        StepArg::RemoveSketchItems {
            sketch,
            constraints,
            curves,
            points,
        } => RepairStep::RemoveSketchItems {
            sketch: id(&sketch)?,
            constraints: ids(&constraints)?,
            curves: ids(&curves)?,
            points: ids(&points)?,
        },
        StepArg::SourceEdit { label, edits } => RepairStep::SourceEdit {
            label,
            edits: edits
                .into_iter()
                .map(|e| {
                    serde_json::from_value::<Mutation>(e)
                        .map_err(|e| ToolError::invalid(format!("Source edit: {e}")))
                })
                .collect::<Result<_, _>>()?,
        },
    })
}

/// Stage and check the rows on the current source, outside the owner.
fn check(
    host: &dyn Host,
    call: &Request,
    expected: Option<u64>,
    steps: Vec<StepArg>,
) -> Result<(RepairSetPrepared, u64, String), ToolError> {
    if steps.is_empty() {
        return Err(ToolError::invalid("Stage at least one repair row"));
    }
    let steps = steps.into_iter().map(step).collect::<Result<Vec<_>, _>>()?;
    let (snapshot, resources): (PreparationSnapshot, String) = with(host, move |target| {
        if let Some(expected) = expected {
            revision_guard(target, expected)?;
        }
        Ok::<_, ToolError>((
            target.session().preparation_snapshot(),
            target.resources().fingerprint(),
        ))
    })??;
    let expected = snapshot.session().revision();
    let set = RepairSet {
        stamp: RepairStamp::current(
            snapshot.session().document(),
            expected,
            Some(resources.clone()),
        )?,
        steps,
    };
    let interrupt = || call.cancelled();
    let checked = call.finish(Compiler::default().prepare_repair_set(
        &snapshot,
        call.actor.clone(),
        &set,
        Some(&resources),
        Some(&interrupt),
    ))?;
    Ok((checked, expected, resources))
}
fn report(checked: &RepairSetPrepared, revision: u64) -> Value {
    json!({
        "revision": revision,
        "can_apply": checked.can_apply(),
        "rows": checked.rows,
        "sketches": checked.sketches,
        "blockers": checked.blockers,
        "references_resolved": checked.report.as_ref().map(|r| r.all_resolved()),
    })
}

pub(super) fn reference_candidates(
    host: &dyn Host,
    call: &Request,
    args: CandidateArgs,
) -> ToolResult {
    let source = id(&args.source)?;
    let (document, revision) = with(host, |target| {
        (
            target.session().document().clone(),
            target.session().revision(),
        )
    })?;
    if !matches!(
        document.get(source).map(|e| &e.data),
        Some(EntityData::Feature { .. })
    ) {
        return Err(
            ToolError::invalid(format!("{source} is not a source feature")).involving(source),
        );
    }
    let interrupt = || call.cancelled();
    let model = call.finish(Compiler::default().compile_sources_interruptible(
        &document,
        revision,
        &[source],
        Some(&interrupt),
    ))?;
    let shape = model.snapshot().shapes.get(&source).ok_or_else(|| {
        ToolError::new(
            "model_failed",
            "Source feature has no current native topology",
        )
        .involving(source)
    })?;
    let kind = args.kind.map(TopologyKind::from);
    let elements: Vec<_> = shape
        .topology
        .elements
        .iter()
        .filter(|e| kind.is_none_or(|k| e.key.kind == k))
        .map(|e| {
            json!({
                "element": { "kind": kind_name(e.key.kind), "ordinal": e.key.ordinal },
                "geometry": e.geometry,
            })
        })
        .collect();
    Ok(json!({ "revision": revision, "source": source, "elements": elements }))
}

pub(super) fn preview_reference_repairs(
    host: &dyn Host,
    call: &Request,
    args: PreviewArgs,
) -> ToolResult {
    let (checked, revision, _) = check(host, call, args.at_revision, args.steps)?;
    Ok(report(&checked, revision))
}

pub(super) fn apply_reference_repairs(
    host: &dyn Host,
    call: &Request,
    args: RepairArgs,
) -> ToolResult {
    let (checked, base, resources) = check(host, call, Some(args.expected_revision), args.steps)?;
    if !checked.can_apply() {
        return Err(ToolError::new(
            "repair_blocked",
            format!(
                "The staged repairs cannot apply: {}",
                checked.blockers.join("; ")
            ),
        )
        .at_revision(base));
    }
    let mut summary = report(&checked, base);
    let prepared = checked.prepared.expect("checked can_apply");
    let call = call.clone();
    let revision = with(host, move |target| {
        revision_guard(target, base)?;
        editable(target)?;
        if target.resources().fingerprint() != resources {
            return Err(ToolError::new(
                "stale_revision",
                "Project resources changed after the repairs were checked; check them again",
            )
            .at_revision(base));
        }
        commit(target, &call, prepared)
    })??;
    summary["revision"] = json!(revision);
    Ok(summary)
}

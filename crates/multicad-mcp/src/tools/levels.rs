//! Level, slab, pedestal, stair and service-envelope tools. Mutations are
//! evaluated natively before they commit; stair checks are geometric notes.
use super::{Quantity, Request, ToolResult, commit, compile, id, revision_guard, snapshot};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    commands::Prepared,
    document::EntityId,
    model::{
        self, BodyOutput, EnvelopeDefinition, LevelDefinition, SlabDefinition, SlabKind,
        StairDefinition, StairForm, Turn, length,
    },
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelArgs {
    pub expected_revision: u64,
    /// Name for a new level; omit when editing with `level`.
    #[serde(default)]
    pub name: Option<String>,
    /// Existing level to edit; every hosted slab, stair and wall rebuilds.
    #[serde(default)]
    pub level: Option<String>,
    pub elevation: Quantity,
    /// Floor-to-floor height up to the next level.
    pub height: Quantity,
    pub slab_thickness: Quantity,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SlabKindArg {
    Slab,
    Pedestal,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlabArgs {
    pub expected_revision: u64,
    pub name: String,
    pub level: String,
    pub kind: SlabKindArg,
    /// Closed outline in world X/Y millimeters.
    pub outline_mm: Vec<[f64; 2]>,
    /// Slab thickness override, or the pedestal height (required for pedestals).
    #[serde(default)]
    pub thickness: Option<Quantity>,
}
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StairFormArg {
    Straight,
    QuarterLeft,
    QuarterRight,
    HalfLeft,
    HalfRight,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StairArgs {
    pub expected_revision: u64,
    /// Name for a new stair; omit when editing with `stair`.
    #[serde(default)]
    pub name: Option<String>,
    /// Existing stair feature to edit.
    #[serde(default)]
    pub stair: Option<String>,
    pub level: String,
    /// Upper level; omit to rise by the base level's floor-to-floor height.
    #[serde(default)]
    pub to_level: Option<String>,
    pub form: StairFormArg,
    /// Centerline start of the first riser, world X/Y mm.
    pub start_mm: [f64; 2],
    /// Walking direction of the first flight in degrees from +X.
    #[serde(default)]
    pub direction_deg: f64,
    pub width: Quantity,
    pub risers: u32,
    pub going: Quantity,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvelopeArgs {
    pub expected_revision: u64,
    pub name: String,
    /// The placed Part or Instance to surround.
    pub target: String,
    pub front: Quantity,
    pub back: Quantity,
    pub left: Quantity,
    pub right: Quantity,
    pub top: Quantity,
    #[serde(default)]
    pub label: String,
}

fn guard(current: u64, expected: u64) -> Result<(), ToolError> {
    if current == expected {
        Ok(())
    } else {
        Err(ToolError::new(
            "stale_revision",
            format!("Request is based on revision {expected}, but the document is at {current}"),
        )
        .at_revision(current))
    }
}
/// Evaluate natively, then commit with the owner's review policy.
fn checked(
    host: &dyn Host,
    call: &Request,
    prepared: Prepared,
    revision: u64,
) -> Result<(u64, multicad_model::Model), ToolError> {
    let model = compile(prepared.document(), revision, call)?;
    let call = call.clone();
    let committed = with(host, move |target| commit(target, &call, prepared))??;
    Ok((committed, model))
}

pub(super) fn set_level(host: &dyn Host, call: &Request, args: LevelArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    guard(revision, args.expected_revision)?;
    let q = |q: &Quantity| q.dimension(&document, &evaluated).map(|d| d.0);
    let value = LevelDefinition {
        elevation: q(&args.elevation)?,
        height: q(&args.height)?,
        slab_thickness: q(&args.slab_thickness)?,
    };
    let existing = args.level.as_deref().map(id).transpose()?;
    let name = args.name.clone();
    let actor = call.actor.clone();
    let (level, prepared) = with(host, move |target| {
        revision_guard(target, revision)?;
        let s = target.session();
        Ok::<_, ToolError>(match existing {
            Some(level) => (
                level,
                model::prepare_level_edit(s, revision, actor, level, &value)?,
            ),
            None => model::prepare_level(
                s,
                revision,
                actor,
                name.as_deref()
                    .ok_or_else(|| ToolError::invalid("Name the new level"))?,
                &value,
            )?,
        })
    })??;
    let (committed, _) = checked(host, call, prepared, revision)?;
    Ok(json!({ "revision": committed, "level": level }))
}
pub(super) fn create_slab(host: &dyn Host, call: &Request, args: SlabArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    guard(revision, args.expected_revision)?;
    let slab = SlabDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        level: id(&args.level)?,
        kind: match args.kind {
            SlabKindArg::Slab => SlabKind::Slab,
            SlabKindArg::Pedestal => SlabKind::Pedestal,
        },
        outline: args
            .outline_mm
            .iter()
            .map(|[x, y]| [length(*x), length(*y)])
            .collect(),
        thickness: args
            .thickness
            .as_ref()
            .map(|q| q.dimension(&document, &evaluated).map(|d| d.0))
            .transpose()?,
        holes: vec![],
    };
    let (body, name, actor) = (slab.output.body, args.name, call.actor.clone());
    let prepared = with(host, move |target| {
        revision_guard(target, revision)?;
        Ok::<_, ToolError>(model::prepare_slab(
            target.session(),
            revision,
            actor,
            &name,
            &slab,
            None,
        )?)
    })??;
    let feature = prepared.outputs[0].feature;
    let (committed, model) = checked(host, call, prepared.prepared, revision)?;
    let shape = &model.snapshot().shapes[&feature];
    Ok(
        json!({ "revision": committed, "slab": feature, "part": body, "volume_mm3": shape.volume_mm3,
        "bounds_mm": { "min": shape.bounds.min, "max": shape.bounds.max } }),
    )
}
pub(super) fn set_stair(host: &dyn Host, call: &Request, args: StairArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    guard(revision, args.expected_revision)?;
    let q = |q: &Quantity| q.dimension(&document, &evaluated).map(|d| d.0);
    let existing = args.stair.as_deref().map(id).transpose()?;
    let body = match existing {
        Some(f) => {
            model::read_stair(&document, f)
                .map_err(|e| ToolError::from(e).involving(f))?
                .output
                .body
        }
        None => EntityId::new(),
    };
    let stair = StairDefinition {
        output: BodyOutput { body },
        level: id(&args.level)?,
        to_level: args.to_level.as_deref().map(id).transpose()?,
        form: match args.form {
            StairFormArg::Straight => StairForm::Straight,
            StairFormArg::QuarterLeft => StairForm::QuarterTurn { turn: Turn::Left },
            StairFormArg::QuarterRight => StairForm::QuarterTurn { turn: Turn::Right },
            StairFormArg::HalfLeft => StairForm::HalfTurn { turn: Turn::Left },
            StairFormArg::HalfRight => StairForm::HalfTurn { turn: Turn::Right },
        },
        start: [length(args.start_mm[0]), length(args.start_mm[1])],
        direction_deg: args.direction_deg,
        width: q(&args.width)?,
        risers: args.risers,
        going: q(&args.going)?,
    };
    let check = model::resolve_stair(&document, &evaluated, &stair)?.check;
    let (name, actor) = (args.name, call.actor.clone());
    let stair_owned = stair.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, revision)?;
        let s = target.session();
        Ok::<_, ToolError>(match existing {
            Some(f) => model::prepare_stair_edit(s, revision, actor, f, &stair_owned)?,
            None => model::prepare_stair(
                s,
                revision,
                actor,
                name.as_deref()
                    .ok_or_else(|| ToolError::invalid("Name the new stair"))?,
                &stair_owned,
                None,
            )?,
        })
    })??;
    let feature = prepared.outputs[0].feature;
    let (committed, model) = checked(host, call, prepared.prepared, revision)?;
    let shape = &model.snapshot().shapes[&feature];
    Ok(json!({
        "revision": committed, "stair": feature, "part": body,
        "volume_mm3": shape.volume_mm3, "solid_count": shape.solid_count,
        "geometric_check": {
            "scope": "Geometric measures only; not a building-code, accessibility or structural approval",
            "rise_mm": check.rise_mm, "riser_mm": check.riser_mm, "going_mm": check.going_mm,
            "step_measure_2r_plus_g_mm": check.step_measure_mm, "run_mm": check.run_mm, "notes": check.notes,
        },
    }))
}
pub(super) fn create_envelope(host: &dyn Host, call: &Request, args: EnvelopeArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    guard(revision, args.expected_revision)?;
    let q = |q: &Quantity| q.dimension(&document, &evaluated).map(|d| d.0);
    let value = EnvelopeDefinition {
        target: id(&args.target)?,
        front: q(&args.front)?,
        back: q(&args.back)?,
        left: q(&args.left)?,
        right: q(&args.right)?,
        top: q(&args.top)?,
        label: args.label,
    };
    let (name, actor) = (args.name, call.actor.clone());
    let (envelope, prepared) = with(host, move |target| {
        revision_guard(target, revision)?;
        Ok::<_, ToolError>(model::prepare_envelope(
            target.session(),
            revision,
            actor,
            &name,
            &value,
        )?)
    })??;
    let (committed, _) = checked(host, call, prepared, revision)?;
    Ok(json!({ "revision": committed, "envelope": envelope }))
}
pub(super) fn check_clearances(host: &dyn Host, call: &Request, _: super::NoArgs) -> ToolResult {
    let (document, revision, _) = snapshot(host)?;
    let model = compile(&document, revision, call)?;
    let envelopes = model.envelopes(&document)?;
    Ok(json!({
        "revision": revision,
        "scope": "Geometric overlap of service envelopes with final model solids; not a code or safety approval",
        "envelopes": envelopes.iter().map(|e| json!({
            "envelope": e.envelope, "target": e.target, "volume_mm3": e.volume_mm3,
            "corners_mm": e.corners,
            "conflicts": e.conflicts.iter().map(|(s, v)| json!({ "entity": s, "overlap_mm3": v })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    }))
}

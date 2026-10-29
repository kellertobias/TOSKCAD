//! FDM design-aid tools over the shared aid and process API. Each aid
//! mutation builds the aid natively before it commits one transaction; the
//! report measures the evaluated aid, target and neighbour solids.
use super::{Quantity, Request, ToolResult, commit, compile, id, revision_guard, snapshot};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    commands::Prepared,
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    model::{self, AidDefinition, AidRequest, AidShape, BodyOutput, ProcessDefinition, angle},
    parameters::EvaluatedParameters,
};
use multicad_kernel::SolidPairRelation;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AidKindArg {
    Tab,
    Bridge,
    Support,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AidArgs {
    pub expected_revision: u64,
    /// Aid feature ID to edit; omit to create a new aid Part.
    #[serde(default)]
    pub aid: Option<String>,
    /// Name of a new aid Part.
    #[serde(default)]
    pub name: Option<String>,
    pub kind: AidKindArg,
    /// The finished Part the aid serves; it is never changed.
    pub target: String,
    /// FDM process allowances to inspect the aid against.
    #[serde(default)]
    pub process: Option<String>,
    /// Tab: contact point on the part outline. Bridge: start of the bar's
    /// centreline. World millimeters, +Z is the build direction.
    #[serde(default)]
    pub origin_mm: Option<[f64; 3]>,
    /// Bridge: end of the bar's centreline in world millimeters.
    #[serde(default)]
    pub end_mm: Option<[f64; 3]>,
    /// Bridge: optional second Part the bar ties to.
    #[serde(default)]
    pub other: Option<String>,
    /// Tab and support direction about +Z in degrees (default 0 = +X).
    #[serde(default)]
    pub angle_deg: f64,
    /// Tab: outward length. Support: footprint length along the angle.
    #[serde(default)]
    pub length: Option<Quantity>,
    /// Tab, bridge and support width across.
    pub width: Quantity,
    /// Tab and bridge thickness.
    #[serde(default)]
    pub thickness: Option<Quantity>,
    /// Tab: length back under the part (default 0).
    #[serde(default)]
    pub overlap: Option<Quantity>,
    /// Support: footprint corner in world millimeters (x, y).
    #[serde(default)]
    pub footprint_mm: Option<[f64; 2]>,
    /// Support: base height (default 0, the build plate).
    #[serde(default)]
    pub base: Option<Quantity>,
    /// Support: gap below the lowest target surface over the footprint.
    #[serde(default)]
    pub gap: Option<Quantity>,
    /// New aids: the aid Part's Material (default: the target's Material).
    #[serde(default)]
    pub material: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcessArgs {
    pub expected_revision: u64,
    /// Process ID to edit; omit to create one.
    #[serde(default)]
    pub process: Option<String>,
    pub name: String,
    pub layer_height: Quantity,
    pub line_width: Quantity,
    /// Minimum tab and bridge thickness in layers.
    pub min_layers: u32,
    /// Minimum aid width in extrusion lines.
    pub min_lines: u32,
    /// Smallest Z gap between a support top and the part above it.
    pub support_gap: Quantity,
    /// Smallest clearance between an aid and any Part it does not serve.
    pub fit_clearance: Quantity,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveAidArgs {
    pub expected_revision: u64,
    /// Aid feature ID.
    pub aid: String,
}

fn q(q: &Quantity, d: &Document, e: &EvaluatedParameters) -> Result<Dimension, ToolError> {
    Ok(q.dimension(d, e)?.0)
}
fn required<'a>(v: &'a Option<Quantity>, what: &str) -> Result<&'a Quantity, ToolError> {
    v.as_ref()
        .ok_or_else(|| ToolError::invalid(format!("This aid needs {what}")))
}
fn shape(args: &AidArgs, d: &Document, e: &EvaluatedParameters) -> Result<AidShape, ToolError> {
    let point = |p: Option<[f64; 3]>, what: &str| {
        p.map(|p| p.map(model::length))
            .ok_or_else(|| ToolError::invalid(format!("This aid needs {what}")))
    };
    Ok(match args.kind {
        AidKindArg::Tab => AidShape::Tab {
            origin: point(args.origin_mm, "origin_mm")?,
            angle: angle(args.angle_deg),
            length: q(required(&args.length, "length")?, d, e)?,
            width: q(&args.width, d, e)?,
            thickness: q(required(&args.thickness, "thickness")?, d, e)?,
            overlap: args
                .overlap
                .as_ref()
                .map(|o| q(o, d, e))
                .transpose()?
                .unwrap_or_else(|| model::length(0.)),
        },
        AidKindArg::Bridge => AidShape::Bridge {
            start: point(args.origin_mm, "origin_mm")?,
            end: point(args.end_mm, "end_mm")?,
            width: q(&args.width, d, e)?,
            thickness: q(required(&args.thickness, "thickness")?, d, e)?,
            other: args.other.as_deref().map(id).transpose()?,
        },
        AidKindArg::Support => AidShape::Support {
            origin: args
                .footprint_mm
                .ok_or_else(|| ToolError::invalid("A support needs footprint_mm"))?
                .map(model::length),
            angle: angle(args.angle_deg),
            length: q(required(&args.length, "length")?, d, e)?,
            width: q(&args.width, d, e)?,
            base: args
                .base
                .as_ref()
                .map(|b| q(b, d, e))
                .transpose()?
                .unwrap_or_else(|| model::length(0.)),
            gap: q(required(&args.gap, "gap")?, d, e)?,
        },
    })
}
/// Build the candidate natively, then commit; returns the committed revision.
fn finish(host: &dyn Host, call: &Request, prepared: Prepared, revision: u64) -> ToolResult {
    compile(prepared.document(), revision, call)?;
    let call = call.clone();
    let committed = with(host, move |target| commit(target, &call, prepared))??;
    Ok(json!({ "revision": committed }))
}

pub(super) fn set_fdm_aid(host: &dyn Host, call: &Request, args: AidArgs) -> ToolResult {
    let (document, revision, evaluated) = snapshot(host)?;
    let expected = args.expected_revision;
    let edit = args.aid.as_deref().map(id).transpose()?;
    let body = match edit {
        Some(feature) => model::read_aid(&document, feature)?.output.body,
        None => EntityId::new(),
    };
    let definition = AidDefinition {
        output: BodyOutput { body },
        target: id(&args.target)?,
        shape: shape(&args, &document, &evaluated)?,
        process: args.process.as_deref().map(id).transpose()?,
    };
    let material = args.material.as_deref().map(id).transpose()?;
    let name = args.name.clone();
    let call_owned = call.clone();
    let (prepared, feature, body) = with(host, move |target| {
        revision_guard(target, expected)?;
        let session = target.session();
        let actor = call_owned.actor.clone();
        Ok::<_, ToolError>(match edit {
            Some(feature) => (
                model::prepare_aid_edit(session, expected, actor, feature, &definition)?.prepared,
                feature,
                body,
            ),
            None => {
                let name = name.ok_or_else(|| ToolError::invalid("A new aid needs a name"))?;
                let mut request = AidRequest::new(name, definition);
                request.material = material;
                let (feature, body) = (request.feature, request.body);
                (
                    model::prepare_aid(session, expected, actor, &request)?.prepared,
                    feature,
                    body,
                )
            }
        })
    })??;
    let mut out = finish(host, call, prepared, revision)?;
    out["aid"] = json!(feature);
    out["part"] = json!(body);
    Ok(out)
}

pub(super) fn remove_fdm_aid(host: &dyn Host, call: &Request, args: RemoveAidArgs) -> ToolResult {
    let (_, revision, _) = snapshot(host)?;
    let feature = id(&args.aid)?;
    let call_owned = call.clone();
    let prepared = with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        Ok::<_, ToolError>(model::prepare_aid_remove(
            target.session(),
            args.expected_revision,
            call_owned.actor.clone(),
            feature,
        )?)
    })??;
    finish(host, call, prepared, revision)
}

pub(super) fn set_fdm_process(host: &dyn Host, call: &Request, args: ProcessArgs) -> ToolResult {
    let (document, _, evaluated) = snapshot(host)?;
    let (d, e) = (&document, &evaluated);
    let process = ProcessDefinition {
        layer_height: q(&args.layer_height, d, e)?,
        line_width: q(&args.line_width, d, e)?,
        min_layers: args.min_layers,
        min_lines: args.min_lines,
        support_gap: q(&args.support_gap, d, e)?,
        fit_clearance: q(&args.fit_clearance, d, e)?,
    };
    let edit = args.process.as_deref().map(id).transpose()?;
    let call = call.clone();
    with(host, move |target| {
        revision_guard(target, args.expected_revision)?;
        let (process_id, prepared) = model::prepare_process(
            target.session(),
            args.expected_revision,
            call.actor.clone(),
            edit,
            &args.name,
            &process,
        )?;
        let revision = commit(target, &call, prepared)?;
        Ok(json!({ "revision": revision, "process": process_id }))
    })?
}

fn relation(r: SolidPairRelation) -> &'static str {
    match r {
        SolidPairRelation::Separated => "separated",
        SolidPairRelation::ContactWithinTolerance => "contact",
        SolidPairRelation::Overlapping => "overlapping",
    }
}
pub(super) fn fdm_aid_report(host: &dyn Host, call: &Request, _: super::NoArgs) -> ToolResult {
    let (document, revision, _) = snapshot(host)?;
    let model = compile(&document, revision, call)?;
    let aids = model.fdm_aids(&document)?;
    let contact = |c: &multicad_model::AidContact| {
        json!({
            "part": c.part, "relation": relation(c.relation),
            "distance_mm": c.distance_mm, "common_volume_mm3": c.common_volume_mm3,
        })
    };
    Ok(json!({
        "revision": revision,
        "scope": "Measured from the evaluated aid, target and neighbour solids with +Z as the build direction. Allowances are the explicit process values; this is not slicer output and does not guarantee support removal or print success.",
        "aids": aids.iter().map(|a| json!({
            "aid": a.feature, "part": a.part, "name": a.name, "kind": a.kind.label(),
            "target": contact(&a.target),
            "other": a.other.as_ref().map(contact),
            "bounds_mm": a.bounds_mm, "volume_mm3": a.volume_mm3,
            "process": a.process.map(|p| json!({
                "layer_height_mm": p.layer_height_mm, "line_width_mm": p.line_width_mm,
                "min_thickness_mm": p.min_thickness_mm(), "min_width_mm": p.min_width_mm(),
                "support_gap_mm": p.support_gap_mm, "fit_clearance_mm": p.fit_clearance_mm,
            })),
            "crowded": a.crowded.iter().map(|(p, d)| json!({ "part": p, "distance_mm": d })).collect::<Vec<_>>(),
            "issues": a.issues,
        })).collect::<Vec<Value>>(),
    }))
}

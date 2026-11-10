//! Revision-checked control of the canonical circular helical cut.
use super::{
    Quantity, Request, ToolResult, commit, compile, current_producer, id, revision_guard, snapshot,
};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    constraint_sketch::{Dimension, binding::DimensionInput},
    document::Document,
    model::{self, BodyInput, HelicalTubeCutParameters, HelicalTubeCutRequest},
    parameters::{BoundExpression, Unit},
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct HelicalTubeCutArgs {
    pub expected_revision: u64,
    /// ID of the cylinder Part to cut. Supply only for creation.
    pub target_part: Option<String>,
    /// ID of the helical cut to edit. Edits retain its Part and predecessor.
    pub feature: Option<String>,
    /// Radius of the circular cutter, in length units.
    pub radius: Quantity,
    /// Positive axial advance per turn, in length units.
    pub pitch: Quantity,
    /// Total axial travel of the helical path, in length units.
    pub path_height: Quantity,
    /// Axial start of the helix relative to the stock bottom, in length units.
    pub start_height: Quantity,
    /// Phase relative to the stock seam, in deg or rad.
    pub start_angle: Quantity,
}

fn angle(quantity: &Quantity, document: &Document) -> Result<Dimension, ToolError> {
    if !matches!(
        Unit::parse(&quantity.unit),
        Some(Unit::Degree | Unit::Radian)
    ) {
        return Err(ToolError::invalid("Start angle requires deg or rad"));
    }
    Ok(match quantity.scalar_input()? {
        DimensionInput::Literal { value, unit } => Dimension::Literal { value, unit },
        DimensionInput::Expression { text, unit } => Dimension::Expression {
            expression: BoundExpression::bind(document, &text, &unit, None)?,
        },
    })
}

pub(super) fn set_helical_tube_cut(
    host: &dyn Host,
    call: &Request,
    args: HelicalTubeCutArgs,
) -> ToolResult {
    let expected = args.expected_revision;
    let (document, revision, evaluated) = snapshot(host)?;
    if revision != expected {
        return Err(ToolError::new(
            "stale_revision",
            format!("Request is based on revision {expected}, but the document is at {revision}"),
        )
        .at_revision(revision));
    }
    let parameters = HelicalTubeCutParameters {
        radius: args.radius.dimension(&document, &evaluated)?.0,
        pitch: args.pitch.dimension(&document, &evaluated)?.0,
        path_height: args.path_height.dimension(&document, &evaluated)?.0,
        start_height: args.start_height.dimension(&document, &evaluated)?.0,
        start_angle: angle(&args.start_angle, &document)?,
    };
    let owned = call.clone();
    let (prepared, output) = with(host, move |target| {
        revision_guard(target, expected)?;
        let session = target.session();
        let result = match (args.target_part, args.feature) {
            (Some(part), None) => {
                let body = id(&part)?;
                let producer = current_producer(session.document(), body)?;
                let request = HelicalTubeCutRequest::new(BodyInput { body, producer }, parameters);
                model::prepare_helical_tube_cut(session, expected, owned.actor.clone(), &request)?
            }
            (None, Some(feature)) => model::prepare_helical_tube_cut_edit(
                session,
                expected,
                owned.actor.clone(),
                id(&feature)?,
                &parameters,
            )?,
            _ => {
                return Err(ToolError::invalid(
                    "Give target_part to create or feature to edit",
                ));
            }
        };
        let output = result.outputs[0].clone();
        Ok::<_, ToolError>((result.prepared, output))
    })??;
    let model = compile(prepared.document(), revision, call)?;
    let shape = model
        .snapshot()
        .shapes
        .get(&output.feature)
        .ok_or_else(|| {
            ToolError::new("model_failed", "Helical tube cut produced no solid")
                .involving(output.feature)
        })?;
    let (volume, solids) = (shape.volume_mm3, shape.solid_count);
    let owned = call.clone();
    let revision = with(host, move |target| commit(target, &owned, prepared))??;
    Ok(json!({
        "revision": revision, "part": output.body, "feature": output.feature,
        "volume_mm3": volume, "solid_count": solids,
    }))
}

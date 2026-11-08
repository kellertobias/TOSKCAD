//! Native relief uses the same request, face references and transaction as Model.
use super::{Request, ToolResult, commit, compile, id, revision_guard, snapshot};
use crate::{
    error::ToolError,
    host::{Host, with},
};
use multicad_core::{
    commands::Prepared,
    model::{self, BodyInput, MAX_SURFACE_STRUCTURE_FACES, SurfaceStructureRequest},
    references::{AnalyticGeometry, TopologyKey, TopologyKind},
};
use multicad_model::Compiler;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SurfaceStructureCaptureArgs {
    pub expected_revision: u64,
    pub part: String,
    /// The current source feature, or the frozen predecessor when repairing an edit.
    pub producer: String,
    /// One to four distinct face ordinals from reference_candidates at expected_revision.
    pub faces: Vec<u32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SurfaceStructureArgs {
    pub expected_revision: u64,
    /// Canonical request containing feature UUID, input Part and producer, captured faces and parameters.
    /// Inspect an existing feature to recover its editable request.
    pub request: Map<String, Value>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SurfaceStructureInspectArgs {
    pub feature: String,
}

pub(super) fn capture_surface_structure_faces(
    host: &dyn Host,
    call: &Request,
    args: SurfaceStructureCaptureArgs,
) -> ToolResult {
    let distinct = args.faces.iter().collect::<BTreeSet<_>>();
    if !(1..=MAX_SURFACE_STRUCTURE_FACES).contains(&args.faces.len())
        || distinct.len() != args.faces.len()
    {
        return Err(ToolError::invalid(format!(
            "Choose one to {MAX_SURFACE_STRUCTURE_FACES} distinct planar faces from one Part"
        )));
    }
    let input = BodyInput {
        body: id(&args.part)?,
        producer: id(&args.producer)?,
    };
    let revision = args.expected_revision;
    let document = with(host, move |target| {
        revision_guard(target, revision)?;
        Ok::<_, ToolError>(target.session().document().clone())
    })??;
    input.resolve_producer(&document)?;
    let interrupt = || call.cancelled();
    let native = call.finish(Compiler::default().compile_sources_interruptible(
        &document,
        revision,
        &[input.producer],
        Some(&interrupt),
    ))?;
    let topology = &native
        .snapshot()
        .shapes
        .get(&input.producer)
        .ok_or_else(|| {
            ToolError::invalid("The selected Part producer has no current native faces")
        })?
        .topology;
    let faces = args
        .faces
        .into_iter()
        .map(|ordinal| {
            let key = TopologyKey {
                kind: TopologyKind::Face,
                ordinal,
            };
            if !topology.elements.iter().any(|element| {
                element.key == key
                    && matches!(element.geometry, Some(AnalyticGeometry::Plane { .. }))
            }) {
                return Err(ToolError::invalid(format!(
                    "Face {ordinal} is missing or curved; choose a current planar face"
                )));
            }
            model::capture_profile_face(topology, key, None)
                .map(|capture| capture.intent)
                .map_err(ToolError::invalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    call.check()?;
    Ok(json!({"revision": revision, "input": input, "faces": faces}))
}

pub(super) fn inspect_surface_structure(
    host: &dyn Host,
    _: &Request,
    args: SurfaceStructureInspectArgs,
) -> ToolResult {
    let (document, revision, _) = snapshot(host)?;
    let request = model::read_surface_structure(&document, id(&args.feature)?)?;
    Ok(json!({"revision": revision, "request": request}))
}

fn checked(
    host: &dyn Host,
    call: &Request,
    args: SurfaceStructureArgs,
) -> Result<(Prepared, Value), ToolError> {
    let request: SurfaceStructureRequest = serde_json::from_value(Value::Object(args.request))
        .map_err(|failure| {
            ToolError::invalid(format!("Invalid Surface Structure request: {failure}"))
        })?;
    let feature = request.feature;
    let body = request.input.body;
    let expected = args.expected_revision;
    let actor = call.actor.clone();
    call.check()?;
    let model::ModelPrepared { prepared, .. } = with(host, move |target| {
        revision_guard(target, expected)?;
        Ok::<_, ToolError>(model::prepare_surface_structure(
            target.session(),
            expected,
            actor,
            &request,
        )?)
    })??;
    let native = compile(prepared.document(), expected, call)?;
    let shape =
        native.snapshot().shapes.get(&feature).ok_or_else(|| {
            ToolError::new("model_failed", "Surface Structure has no native result")
        })?;
    call.check()?;
    let result = json!({
        "revision": expected,
        "part": body,
        "feature": feature,
        "native_geometry_checked": true,
        "solid_count": shape.solid_count,
        "volume_mm3": shape.volume_mm3,
        "bounds_mm": { "min": shape.bounds.min, "max": shape.bounds.max },
    });
    Ok((prepared, result))
}

pub(super) fn preview_surface_structure(
    host: &dyn Host,
    call: &Request,
    args: SurfaceStructureArgs,
) -> ToolResult {
    Ok(checked(host, call, args)?.1)
}

pub(super) fn set_surface_structure(
    host: &dyn Host,
    call: &Request,
    args: SurfaceStructureArgs,
) -> ToolResult {
    let (prepared, mut result) = checked(host, call, args)?;
    let owned = call.clone();
    let revision = with(host, move |target| commit(target, &owned, prepared))??;
    result["revision"] = json!(revision);
    Ok(result)
}

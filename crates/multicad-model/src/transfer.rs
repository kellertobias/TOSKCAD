//! Exact associative machining from producer-local sources. Occurrence poses
//! are composed once by the canonical relative-pose helper; world meshes and
//! final aggregate occurrence solids do not participate in this evaluator.
use crate::{ModelError, error};
use multicad_core::{
    document::{Document, EntityId},
    model::{self, AlignmentInput, TransferDefinition, TransferTarget},
    parameters::EvaluatedParameters,
    references::{AnalyticGeometry, TopologyKind, TopologySnapshot},
};
use multicad_kernel::{GeometryPolicy, PlaneFrame, Solid};
use std::collections::BTreeMap;
#[path = "transfer_sources.rs"]
mod sources;
pub(crate) struct Inputs<'a> {
    pub document: &'a Document,
    pub revision: u64,
    pub parameters: &'a EvaluatedParameters,
    pub body_heads: &'a BTreeMap<EntityId, EntityId>,
    pub solids: &'a BTreeMap<EntityId, Solid>,
    pub topologies: &'a BTreeMap<EntityId, TopologySnapshot>,
    pub policy: GeometryPolicy,
}
pub(crate) struct Build {
    pub solid: Solid,
    pub count: usize,
}
pub(crate) fn build(
    inputs: Inputs<'_>,
    feature: EntityId,
    source: &TransferDefinition,
    target: &TransferTarget,
    captures: &[AlignmentInput],
) -> Result<Build, ModelError> {
    source.validate().map_err(|e| error(Some(feature), e))?;
    source
        .inputs(captures)
        .map_err(|e| error(Some(feature), e))?;
    if feature != target.feature || !source.targets.contains(target) {
        return Err(error(
            Some(feature),
            "Transfer output does not match its canonical receiving row",
        ));
    }
    target
        .input
        .resolve_producer(inputs.document)
        .map_err(|e| error(Some(feature), e))?;
    let settings = target
        .parameters
        .resolve(inputs.parameters)
        .map_err(|e| error(Some(feature), e))?;
    let receiver = inputs.solids.get(&target.input.producer).ok_or_else(|| {
        error(
            Some(feature),
            "Frozen receiving producer has no exact current solid; repair the receiving input",
        )
    })?;
    let receiver_topology = inputs
        .topologies
        .get(&target.input.producer)
        .filter(|s| s.owner == target.input.producer && s.revision == inputs.revision)
        .ok_or_else(|| {
            error(
                Some(feature),
                "Frozen receiving topology is missing or stale",
            )
        })?;
    let capture = sources::capture(captures, target.face, target.input.producer, feature)?;
    let (key, geometry) = sources::resolve(capture, receiver_topology, inputs.revision, feature)?;
    let Some(AnalyticGeometry::Plane { frame }) =
        geometry.filter(|_| key.kind == TopologyKind::Face)
    else {
        return Err(error(
            Some(feature),
            "Receiving transfer entry must be an explicitly captured current planar face",
        ));
    };
    frame.validate().map_err(|e| error(Some(feature), e))?;
    let plane = PlaneFrame {
        origin: frame.origin_mm,
        u: frame.u,
        v: frame.v,
    };
    let relative = model::resolve_transfer_relative_pose(
        inputs.document,
        &source.source,
        target,
        inputs.parameters,
        inputs.revision,
        inputs.topologies,
    )
    .map_err(|e| {
        error(
            Some(feature),
            format!("Transfer occurrence binding needs repair: {e}"),
        )
    })?;
    let producer = *inputs
        .body_heads
        .get(&source.source.component)
        .ok_or_else(|| {
            error(
                Some(feature),
                "Source hardware has no current unsuppressed native producer; repair its source",
            )
        })?;
    let hardware = inputs.solids.get(&producer).ok_or_else(|| {
        error(
            Some(feature),
            "Current source hardware native solid is unavailable",
        )
    })?;
    let topology = inputs
        .topologies
        .get(&source.source.component)
        .filter(|s| s.owner == source.source.component && s.revision == inputs.revision)
        .ok_or_else(|| {
            error(
                Some(feature),
                "Current source Part alias topology is missing or stale",
            )
        })?;
    let cuts = sources::profiles(sources::Inputs {
        feature,
        source: &source.source,
        target,
        settings,
        captures,
        hardware,
        topology,
        relative,
        plane,
        revision: inputs.revision,
        policy: inputs.policy,
    })?;
    let count = cuts.len();
    if count > model::MAX_TRANSFER_SOURCES {
        return Err(error(
            Some(feature),
            "Transfer exceeds the 256-source machining budget",
        ));
    }
    if cuts.is_empty() {
        return Ok(Build {
            solid: receiver
                .transformed(Default::default())
                .map_err(|e| error(Some(feature), e))?,
            count: 0,
        });
    }
    let depth = match settings.blind_depth_mm {
        Some(depth) => {
            if settings.reverse {
                -depth
            } else {
                depth
            }
        }
        None => receiver
            .transfer_through_depth(plane, settings.reverse, inputs.policy)
            .map_err(|e| error(Some(feature), e))?,
    };
    let mut result = None;
    for cut in cuts {
        let prior = result.as_ref().unwrap_or(receiver);
        result = Some(
            prior
                .cut_transfer_profile_named(
                    &cut.profile,
                    depth,
                    feature,
                    &cut.origins,
                    inputs.policy,
                )
                .map_err(|e| error(Some(feature), format!("Transfer source {}: {e}", cut.label)))?,
        );
    }
    Ok(Build {
        solid: result.expect("nonempty checked cuts"),
        count,
    })
}

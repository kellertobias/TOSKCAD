//! Owner-thread exact occurrence evaluation; display shares component meshes.
use crate::{ModelError, error};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{RigidPose, resolve_instance_pose_with_snapshots},
    parameters::EvaluatedParameters,
    references::{AnalyticGeometry, ReferenceFrame, TopologySnapshot},
};
use multicad_kernel::{Bounds, GeometryPolicy, RigidTransform, Solid};
use std::collections::BTreeMap;
pub const MAX_MODEL_OCCURRENCES: usize = multicad_core::model::MAX_PLACED_INSTANCES;
#[derive(Clone, Debug)]
pub struct OccurrenceSnapshot {
    pub instance: EntityId,
    pub component: EntityId,
    pub producer: EntityId,
    pub pose: RigidPose,
    pub bounds: Bounds,
    pub topology: TopologySnapshot,
}
pub(crate) struct Inputs<'a> {
    pub document: &'a Document,
    pub parameters: &'a EvaluatedParameters,
    pub body_heads: &'a BTreeMap<EntityId, EntityId>,
    pub solids: &'a BTreeMap<EntityId, Solid>,
    pub topologies: &'a BTreeMap<EntityId, TopologySnapshot>,
    pub policy: GeometryPolicy,
}
fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter()
        .zip(b)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt()
}
fn plane_equivalent(a: ReferenceFrame, b: ReferenceFrame, tolerance: f64) -> bool {
    distance(a.normal(), b.normal()) <= 1e-10
        && a.origin_mm
            .into_iter()
            .zip(b.origin_mm)
            .zip(a.normal())
            .map(|((a, b), n)| (a - b) * n)
            .sum::<f64>()
            .abs()
            <= tolerance
}
fn curve_point(frame: ReferenceFrame, radius: f64, angle: f64) -> [f64; 3] {
    let (sin, cos) = angle.sin_cos();
    std::array::from_fn(|i| frame.origin_mm[i] + radius * (frame.u[i] * cos + frame.v[i] * sin))
}
fn equivalent(a: Option<AnalyticGeometry>, b: Option<AnalyticGeometry>, tolerance: f64) -> bool {
    use AnalyticGeometry::*;
    match (a, b) {
        (None, None) => true,
        (Some(Point { position_mm: a }), Some(Point { position_mm: b })) => {
            distance(a, b) <= tolerance
        }
        (Some(Line { a_mm: a, b_mm: b }), Some(Line { a_mm: c, b_mm: d })) => {
            (distance(a, c) <= tolerance && distance(b, d) <= tolerance)
                || (distance(a, d) <= tolerance && distance(b, c) <= tolerance)
        }
        (Some(Plane { frame: a }), Some(Plane { frame: b })) => plane_equivalent(a, b, tolerance),
        (
            Some(InvoluteArc {
                frame: a,
                base_radius_mm: ra,
                parameters: pa,
            }),
            Some(InvoluteArc {
                frame: b,
                base_radius_mm: rb,
                parameters: pb,
            }),
        ) => {
            pa == pb
                && ra == rb
                && distance(a.origin_mm, b.origin_mm) <= tolerance
                && distance(a.u, b.u) <= 1e-10
                && distance(a.v, b.v) <= 1e-10
        }
        (
            Some(CircularArc {
                frame: a,
                radius_mm: ra,
                start_rad: sa,
                sweep_rad: wa,
            }),
            Some(CircularArc {
                frame: b,
                radius_mm: rb,
                start_rad: sb,
                sweep_rad: wb,
            }),
        ) => {
            if distance(a.origin_mm, b.origin_mm) > tolerance
                || (ra - rb).abs() > tolerance
                || distance(a.normal(), b.normal()) > 1e-10
                || (wa.abs() - wb.abs()).abs() > 1e-10
            {
                return false;
            }
            if (wa.abs() - std::f64::consts::TAU).abs() <= 1e-12 {
                return true;
            }
            let (a0, a1) = (curve_point(a, ra, sa), curve_point(a, ra, sa + wa));
            let (b0, b1) = (curve_point(b, rb, sb), curve_point(b, rb, sb + wb));
            ((distance(a0, b0) <= tolerance && distance(a1, b1) <= tolerance)
                || (distance(a0, b1) <= tolerance && distance(a1, b0) <= tolerance))
                && distance(
                    curve_point(a, ra, sa + wa / 2.),
                    curve_point(b, rb, sb + wb / 2.),
                ) <= tolerance
        }
        _ => false,
    }
}
pub(crate) fn build(
    inputs: Inputs<'_>,
    instance: EntityId,
    revision: u64,
) -> Result<(OccurrenceSnapshot, Solid), ModelError> {
    let Inputs {
        document,
        parameters,
        body_heads,
        solids,
        topologies,
        policy,
    } = inputs;
    let Some(EntityData::Instance { component, .. }) = document.get(instance).map(|e| &e.data)
    else {
        return Err(error(Some(instance), "Missing occurrence source"));
    };
    if !matches!(
        document.get(*component).map(|e| &e.data),
        Some(EntityData::Part { .. } | EntityData::Assembly { .. })
    ) {
        return Err(error(
            Some(instance),
            "Nested subassembly instances require the assembly evaluator; no unplaced fallback is published",
        ));
    }
    let producer = if matches!(
        document.get(*component).map(|e| &e.data),
        Some(EntityData::Assembly { .. })
    ) {
        *component
    } else {
        *body_heads.get(component).ok_or_else(|| {
            error(
                Some(instance),
                "Occurrence component has no current unsuppressed solid producer",
            )
        })?
    };
    let source = solids.get(&producer).ok_or_else(|| {
        error(
            Some(instance),
            "Occurrence source geometry is unavailable before its placement",
        )
    })?;
    let pose =
        resolve_instance_pose_with_snapshots(document, instance, parameters, revision, topologies)
            .map_err(|e| error(Some(instance), e))?;
    let source_topology = topologies.get(&producer).ok_or_else(|| {
        error(
            Some(instance),
            "Occurrence component topology is unavailable",
        )
    })?;
    let exact_identity = pose == RigidPose::default()
        && source.geometry_policy() == policy
        && source_topology.owner == producer
        && source_topology.revision == revision;
    let rebind = || {
        let mut topology = source_topology.clone();
        topology.owner = instance;
        topology.revision = revision;
        topology
    };
    let (solid, topology) = if exact_identity {
        // Sharing the checked producer keeps the exact geometry and ordinal
        // map. Only the occurrence owner of its topology changes.
        (
            source.share().map_err(|e| error(Some(instance), e))?,
            rebind(),
        )
    } else {
        let solid = source
            .transformed(RigidTransform {
                rotation: pose.rotation,
                translation_mm: pose.translation_mm,
            })
            .map_err(|e| error(Some(instance), e))?;
        let actual = solid
            .topology_snapshot(instance, revision)
            .map_err(|e| error(Some(instance), e))?;
        // A rigid transform must preserve the identity of each element.
        if source_topology.elements.len() != actual.elements.len()
            || source_topology
                .elements
                .iter()
                .zip(&actual.elements)
                .any(|(a, b)| a.key != b.key || a.origins != b.origins || a.boundary != b.boundary)
        {
            return Err(error(
                Some(instance),
                "Native occurrence transform changed topology identity unexpectedly",
            ));
        }
        let topology = if source.uses_rust_geometry() {
            // The Rust kernel places its analytic body directly and publishes
            // exact world descriptors, including ellipses. Check that named
            // attachment frames followed the same placement.
            let anchors_changed =
                source_topology
                    .elements
                    .iter()
                    .zip(&actual.elements)
                    .any(|(a, b)| {
                        a.frames.len() != b.frames.len()
                            || a.frames.iter().zip(&b.frames).any(|(a, b)| {
                                let expected = pose.frame(a.frame);
                                a.origin != b.origin
                                    || distance(expected.origin_mm, b.frame.origin_mm)
                                        > policy.linear_tolerance_mm
                                    || distance(expected.u, b.frame.u) > 1e-10
                                    || distance(expected.v, b.frame.v) > 1e-10
                            })
                    });
            if anchors_changed {
                return Err(error(
                    Some(instance),
                    "Rust occurrence transform changed anchored reference frames unexpectedly",
                ));
            }
            actual.validate().map_err(|e| error(Some(instance), e))?;
            actual
        } else {
            // OpenCascade creates a new B-rep during a transform. Retain the
            // independent comparison against the source analytic descriptors.
            let mut topology = rebind();
            for element in &mut topology.elements {
                element.geometry = element
                    .geometry
                    .map(|geometry| geometry.transformed(|p| pose.point(p), |v| pose.vector(v)));
                for anchor in &mut element.frames {
                    anchor.frame = pose.frame(anchor.frame);
                }
            }
            topology.validate().map_err(|e| error(Some(instance), e))?;
            if topology
                .elements
                .iter()
                .zip(&actual.elements)
                .any(|(a, b)| !equivalent(a.geometry, b.geometry, policy.linear_tolerance_mm))
            {
                return Err(error(
                    Some(instance),
                    "Native world geometry disagrees with its rigid occurrence descriptors",
                ));
            }
            topology
        };
        (solid, topology)
    };

    let snapshot = OccurrenceSnapshot {
        instance,
        component: *component,
        producer,
        pose,
        bounds: solid.bounds().map_err(|e| error(Some(instance), e))?,
        topology,
    };
    Ok((snapshot, solid))
}

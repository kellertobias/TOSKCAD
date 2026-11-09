//! Read-only exact inspection of an owned complete native Model. No retained
//! viewport mesh, foreign handle, or source-only repair model is accepted.
use crate::{EvaluationScope, Model};
use multicad_core::{
    inspection::*,
    references::{self, AnalyticGeometry, Resolution, TopologyKey, TopologyKind, TopologySnapshot},
};
use multicad_kernel::{BRepSelection, Solid};
fn fail(
    kind: MeasurementFailureKind,
    reference: Option<multicad_core::document::EntityId>,
    message: impl ToString,
) -> MeasurementFailure {
    MeasurementFailure {
        kind,
        reference,
        message: message.to_string(),
    }
}
pub(crate) fn resolved<'a>(
    model: &'a Model,
    r: &GeometryReference,
) -> Result<(&'a Solid, TopologyKey, Option<AnalyticGeometry>), MeasurementFailure> {
    let snapshot = model.snapshot();
    let reference = Some(r.intent.id);
    let topology: TopologySnapshot = if let Some(occurrence) = snapshot.occurrences.get(&r.source) {
        occurrence.topology.clone()
    } else if let Some(feature) = snapshot.bodies.get(&r.source) {
        let mut topology = snapshot
            .shapes
            .get(feature)
            .ok_or_else(|| {
                fail(
                    MeasurementFailureKind::Missing,
                    reference,
                    "Current Part producer topology is missing",
                )
            })?
            .topology
            .clone();
        topology.owner = r.source;
        topology
    } else if let Some(feature) = snapshot.shapes.get(&r.source) {
        feature.topology.clone()
    } else {
        return Err(fail(
            MeasurementFailureKind::Missing,
            reference,
            if snapshot.consumed_bodies.contains(&r.source) {
                "The referenced Part is consumed and has no independent current solid"
            } else {
                "Measurement source is missing or has no current solid"
            },
        ));
    };
    if topology.owner != r.source {
        return Err(fail(
            MeasurementFailureKind::Invalid,
            reference,
            "Measurement topology belongs to another source",
        ));
    }
    let (key, geometry) = match references::resolve(&r.intent, &topology, snapshot.stamp.revision) {
        Resolution::Resolved { key, geometry, .. } => (key, geometry),
        Resolution::Missing | Resolution::MissingSource => {
            return Err(fail(
                MeasurementFailureKind::Missing,
                reference,
                "Referenced topology is missing; replace or repair the measurement reference",
            ));
        }
        Resolution::Ambiguous { .. } => {
            return Err(fail(
                MeasurementFailureKind::Ambiguous,
                reference,
                "Referenced topology is ambiguous; explicitly replace or repair the reference",
            ));
        }
        Resolution::Stale { .. } => {
            return Err(fail(
                MeasurementFailureKind::Stale,
                reference,
                "Native reference topology is stale",
            ));
        }
        Resolution::UnavailableFrame => {
            return Err(fail(
                MeasurementFailureKind::Unsupported,
                reference,
                "Referenced attachment frame is unavailable",
            ));
        }
        Resolution::Invalid { message } => {
            return Err(fail(MeasurementFailureKind::Invalid, reference, message));
        }
    };
    let solid = model.solid(r.source).ok_or_else(|| {
        fail(
            MeasurementFailureKind::Missing,
            reference,
            "The referenced current native solid is missing",
        )
    })?;
    Ok((solid, key, geometry))
}
fn selector(key: TopologyKey) -> BRepSelection {
    match key.kind {
        TopologyKind::Face => BRepSelection::Face(key.ordinal),
        TopologyKind::Edge => BRepSelection::Edge(key.ordinal),
        TopologyKind::Vertex => BRepSelection::Vertex(key.ordinal),
    }
}
fn directional_delta_is_defined(
    a: Option<AnalyticGeometry>,
    b: Option<AnalyticGeometry>,
    distance: f64,
    tolerance: f64,
) -> bool {
    use AnalyticGeometry::{Line, Plane, Point};
    if distance == 0. {
        return true;
    }
    let dot = |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
    match (a, b) {
        (Some(Point { .. } | Line { .. }), Some(Point { .. } | Line { .. })) => true,
        (Some(Plane { frame: a }), Some(Plane { frame: b }))
            if dot(a.normal(), b.normal()).abs() > 1. - 1e-10 =>
        {
            let separation = dot(
                std::array::from_fn(|i| b.origin_mm[i] - a.origin_mm[i]),
                a.normal(),
            )
            .abs();
            (distance - separation).abs() <= tolerance * 4.
        }
        (Some(Point { position_mm: p }), Some(Plane { frame }))
        | (Some(Plane { frame }), Some(Point { position_mm: p })) => {
            let separation = dot(
                std::array::from_fn(|i| p[i] - frame.origin_mm[i]),
                frame.normal(),
            )
            .abs();
            (distance - separation).abs() <= tolerance * 4.
        }
        _ => false,
    }
}
pub fn measure_model(model: &Model, query: &MeasurementQuery) -> MeasurementEvaluation {
    measure_geometry(model, query, false)
}

/// Drawing page publication proves its source closure before resolving both
/// endpoints. Standalone inspection still requires a complete model.
pub(crate) fn measure_geometry(
    model: &Model,
    query: &MeasurementQuery,
    drawing_page: bool,
) -> MeasurementEvaluation {
    let stamp = model.snapshot().stamp;
    let result = (|| {
        if !drawing_page && model.snapshot().scope != EvaluationScope::Full {
            return Err(fail(
                MeasurementFailureKind::Unsupported,
                None,
                "Measurements require a complete current Model; partial repair sources are display-only",
            ));
        }
        query
            .validate()
            .map_err(|e| fail(MeasurementFailureKind::Invalid, None, e))?;
        let (a, key_a, geometry_a) = resolved(model, &query.a)?;
        let (b, key_b, geometry_b) = resolved(model, &query.b)?;
        let distance = a
            .minimum_distance(selector(key_a), b, selector(key_b))
            .map_err(|e| fail(MeasurementFailureKind::Invalid, None, e))?;
        let witnesses: Vec<_> = distance
            .witnesses
            .into_iter()
            .map(|w| MeasurementWitness {
                a_mm: w.a_mm,
                b_mm: w.b_mm,
            })
            .collect();
        let first = std::array::from_fn::<_, 3, _>(|i| witnesses[0].b_mm[i] - witnesses[0].a_mm[i]);
        let consistent = directional_delta_is_defined(
            geometry_a,
            geometry_b,
            distance.distance_mm,
            model.policy.linear_tolerance_mm,
        ) && distance.witnesses_complete
            && witnesses.iter().all(|w| {
                (0..3).all(|i| {
                    ((w.b_mm[i] - w.a_mm[i]) - first[i]).abs()
                        <= model.policy.linear_tolerance_mm * 4.
                })
            });
        let (angle, parallel_plane_separation) = analytic_metrics(geometry_a, geometry_b)
            .map_err(|e| fail(MeasurementFailureKind::Invalid, None, e))?;
        Ok(MeasurementValues{minimum_mm:distance.distance_mm,witnesses,witnesses_complete:distance.witnesses_complete,total_solutions:distance.total_solutions,inner_solution:distance.inner_solution,axis_components_mm:consistent.then_some(first),axis_warning:(!consistent).then(||"Closest-point directions are nonunique or the witness budget was reached; axis components are not asserted".into()),angle,parallel_plane_separation,tolerance_mm:model.policy.linear_tolerance_mm,boolean_fuzzy_mm:model.policy.boolean_fuzzy_mm,key_a,key_b})
    })();
    MeasurementEvaluation {
        stamp: MeasurementStamp {
            document: stamp.document,
            revision: stamp.revision,
            source_hash: stamp.source_hash,
        },
        query: query.clone(),
        result,
    }
}

mod worker;
pub use worker::MeasurementWorker;

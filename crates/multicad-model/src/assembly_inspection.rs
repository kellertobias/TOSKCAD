//! Exact physical-leaf inspection. All native work runs on an owning model
//! thread; only detached, source-bound DTOs may cross to a UI/MCP caller.
mod worker;
use crate::{Compiler, EvaluationScope, Model};
use multicad_core::{
    assembly_inspection::*,
    document::Document,
    model::{self, JointRequest},
};
use multicad_kernel::{GeometryErrorKind, SolidPairRelation};
pub use worker::AssemblyInspectionWorker;
fn failure(kind: InspectionFailureKind, message: impl Into<String>) -> InspectionFailure {
    InspectionFailure {
        kind,
        message: message.into(),
    }
}
fn check_cancel(interrupt: Option<&dyn Fn() -> bool>) -> Result<(), InspectionFailure> {
    if interrupt.is_some_and(|f| f()) {
        Err(failure(
            InspectionFailureKind::Cancelled,
            "Assembly inspection superseded or cancelled",
        ))
    } else {
        Ok(())
    }
}
fn stamp(model: &Model) -> InspectionStamp {
    let stamp = model.snapshot().stamp;
    InspectionStamp {
        document: stamp.document,
        revision: stamp.revision,
        source_hash: stamp.source_hash,
    }
}
pub fn inspect_assembly(model: &Model, query: &PairQuery) -> Result<PairReport, InspectionFailure> {
    inspect_assembly_interruptible(model, query, None)
}
pub fn inspect_assembly_interruptible(
    model: &Model,
    query: &PairQuery,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<PairReport, InspectionFailure> {
    check_cancel(interrupt)?;
    query
        .validate()
        .map_err(|e| failure(InspectionFailureKind::Invalid, e))?;
    if model.snapshot().scope != EvaluationScope::Full {
        return Err(failure(
            InspectionFailureKind::Unsupported,
            "Assembly inspection requires a complete current model; repair/source-only geometry is not physical evidence",
        ));
    }
    if stamp(model) != query.expected {
        return Err(failure(
            InspectionFailureKind::Stale,
            "Assembly query does not match the complete model source stamp",
        ));
    }
    let mut pairs = Vec::with_capacity(query.pairs.len());
    for pair in &query.pairs {
        check_cancel(interrupt)?;
        let result = (|| {
            let solid = |path: &model::OccurrencePath| {
                if !model.snapshot().leaf_occurrences.contains_key(path) {
                    return Err(failure(
                        InspectionFailureKind::Missing,
                        "Selected occurrence path is not a current physical leaf; definitions and consumed sources are excluded",
                    ));
                }
                model.leaf_solid(path).ok_or_else(|| {
                    failure(
                        InspectionFailureKind::Missing,
                        "The physical leaf has no owned current native solid",
                    )
                })
            };
            let a = solid(&pair.a)?;
            let b = solid(&pair.b)?;
            let result = a
                .inspect_pair(b, query.contact_tolerance_mm, model.policy)
                .map_err(|error| {
                    failure(
                        match error.kind {
                            GeometryErrorKind::Cancelled => InspectionFailureKind::Cancelled,
                            GeometryErrorKind::Limit | GeometryErrorKind::Unsupported => {
                                InspectionFailureKind::Unsupported
                            }
                            _ => InspectionFailureKind::Invalid,
                        },
                        error.to_string(),
                    )
                })?;
            Ok(PairValues {
                relation: match result.relation {
                    SolidPairRelation::Separated => PairRelation::Separated,
                    SolidPairRelation::ContactWithinTolerance => {
                        PairRelation::ContactWithinTolerance
                    }
                    SolidPairRelation::Overlapping => PairRelation::Overlapping,
                },
                overlap_volume_mm3: result.common_volume_mm3,
                minimum_clearance_mm: result.minimum_distance.distance_mm,
                witnesses: result
                    .minimum_distance
                    .witnesses
                    .into_iter()
                    .map(|w| InspectionWitness {
                        a_mm: w.a_mm,
                        b_mm: w.b_mm,
                    })
                    .collect(),
                witnesses_complete: result.minimum_distance.witnesses_complete,
                total_solutions: result.minimum_distance.total_solutions,
                inner_solution: result.minimum_distance.inner_solution,
                tolerance_mm: result.contact_tolerance_mm,
                boolean_fuzzy_mm: 0.,
            })
        })();
        pairs.push(PairEvaluation {
            pair: pair.clone(),
            result,
        });
    }
    check_cancel(interrupt)?;
    let report = PairReport {
        stamp: stamp(model),
        query: query.clone(),
        pairs,
    };
    report
        .validate_against(query)
        .map_err(|e| failure(InspectionFailureKind::Invalid, e))?;
    Ok(report)
}
pub fn sample_assembly_range(
    compiler: &Compiler,
    document: &Document,
    query: &MotionRangeQuery,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<MotionRangeReport, InspectionFailure> {
    check_cancel(interrupt)?;
    query
        .validate()
        .map_err(|e| failure(InspectionFailureKind::Invalid, e))?;
    query
        .expected
        .check(document, query.expected.revision)
        .map_err(|e| failure(InspectionFailureKind::Stale, e))?;
    let request = JointRequest::existing(document, query.joint)
        .map_err(|e| failure(InspectionFailureKind::Missing, e.to_string()))?;
    if !request.parameters.kind.allows(query.coordinate) {
        return Err(failure(
            InspectionFailureKind::Unsupported,
            "The selected joint kind does not drive this coordinate",
        ));
    }
    // Validate the full unmodified source before evaluating derived candidates.
    let baseline = compiler
        .compile_interruptible(document, query.expected.revision, interrupt)
        .map_err(|e| failure(InspectionFailureKind::Invalid, e.to_string()))?;
    inspect_assembly_interruptible(&baseline, &query.pair_query(), interrupt)?;
    drop(baseline);
    let values = query
        .sample_values()
        .map_err(|e| failure(InspectionFailureKind::Invalid, e))?;
    let mut samples = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        check_cancel(interrupt)?;
        let candidate = query.prepare_sample(document, index);
        let sample = match candidate {
            Err(error) => RangeSample {
                value: *value,
                candidate_stamp: None,
                result: Err(failure(InspectionFailureKind::Invalid, error.to_string())),
            },
            Ok(candidate) => {
                let candidate_document = candidate.prepared.document();
                // Derived snapshots use the base revision with their own actual
                // candidate hash. No temporary Session is ever committed.
                let candidate_stamp =
                    InspectionStamp::capture(candidate_document, query.expected.revision)
                        .map_err(|e| failure(InspectionFailureKind::Invalid, e))?;
                let result = compiler
                    .compile_interruptible(candidate_document, query.expected.revision, interrupt)
                    .map_err(|e| failure(InspectionFailureKind::Invalid, e.to_string()))
                    .and_then(|model| {
                        let mut pair = query.pair_query();
                        pair.expected = candidate_stamp;
                        inspect_assembly_interruptible(&model, &pair, interrupt)
                    });
                RangeSample {
                    value: *value,
                    candidate_stamp: Some(candidate_stamp),
                    result,
                }
            }
        };
        samples.push(sample);
    }
    check_cancel(interrupt)?;
    let report = MotionRangeReport {
        stamp: query.expected,
        query: query.clone(),
        unit: query.unit(),
        coverage: RangeCoverage::SamplesOnly,
        actual_intervals: values.windows(2).map(|p| p[1] - p[0]).collect(),
        sample_values: values,
        samples,
    };
    report
        .validate_against(query)
        .map_err(|e| failure(InspectionFailureKind::Invalid, e))?;
    Ok(report)
}

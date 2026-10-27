//! Current evaluated sketch geometry is separate from canonical authored source.
//! Expressions and their bindings never become literal source during solving.
use crate::{Evaluation, InputError, Measurement, evaluate_with_parameters};
use multicad_core::{
    constraint_sketch::{CurveKind, Sketch, source::Source},
    document::{Document, EntityId},
    parameters::{self, EvaluatedParameters, Unit},
    project_file::ContentHash,
    references::ReferenceReport,
    sketch::{Frame, Geometry},
    sketch_regions::{self, Extraction},
};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SketchStamp {
    pub document: EntityId,
    pub revision: u64,
    pub source_hash: ContentHash,
}
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluatedSketch {
    pub stamp: SketchStamp,
    pub sketch: EntityId,
    pub plane: EntityId,
    pub frame: Frame,
    canonical: Source,
    pub evaluation: Evaluation,
    pub resolved_rhs: BTreeMap<EntityId, Measurement>,
}
fn error(sketch: EntityId, message: impl Into<String>) -> InputError {
    InputError {
        source: Some(sketch),
        message: message.into(),
    }
}
impl EvaluatedSketch {
    pub fn canonical(&self) -> &Source {
        &self.canonical
    }
    pub fn solved(&self) -> Option<&Sketch> {
        self.evaluation.candidate.as_ref()
    }
    pub fn is_current(&self, document: &Document, revision: u64) -> bool {
        self.stamp.document == document.id()
            && self.stamp.revision == revision
            && ContentHash::document(document).is_ok_and(|h| h == self.stamp.source_hash)
    }
    pub fn geometry(&self) -> Result<Vec<Geometry>, InputError> {
        let solved = self
            .solved()
            .ok_or_else(|| error(self.sketch, "Sketch has no current valid solved geometry"))?;
        let point = |id| {
            solved
                .points
                .iter()
                .find(|p| p.id == id)
                .map(|p| p.position_mm)
                .ok_or_else(|| error(id, "Solved curve point is missing"))
        };
        solved
            .curves
            .iter()
            .map(|c| {
                Ok(match c.kind {
                    CurveKind::Line { start, end } => Geometry::Line {
                        id: c.id,
                        a: point(start)?,
                        b: point(end)?,
                    },
                    CurveKind::Circle { center, radius_mm } => Geometry::Circle {
                        id: c.id,
                        center: point(center)?,
                        radius_mm,
                    },
                    CurveKind::Arc {
                        center,
                        start,
                        end,
                        clockwise,
                    } => Geometry::Arc {
                        id: c.id,
                        center: point(center)?,
                        a: point(start)?,
                        b: point(end)?,
                        clockwise,
                    },
                })
            })
            .collect()
    }
    pub fn regions(&self) -> Result<Extraction, InputError> {
        let solved = self
            .solved()
            .ok_or_else(|| error(self.sketch, "Sketch has no current valid solved profiles"))?;
        Ok(sketch_regions::extract_geometry(
            &solved.points,
            &solved.curves,
        ))
    }
}
pub fn evaluate_document_sketch(
    document: &Document,
    revision: u64,
    sketch: EntityId,
    report: Option<&ReferenceReport>,
) -> Result<EvaluatedSketch, InputError> {
    let parameters = parameters::evaluate(document).map_err(|e| error(sketch, e.to_string()))?;
    evaluate_document_sketch_with_parameters(
        &parameters.document,
        revision,
        sketch,
        report,
        &parameters,
    )
}
/// A model provider can evaluate parameters once, then reuse that checked
/// context for every affected sketch. Foreign or stale contexts are rejected.
pub fn evaluate_document_sketch_with_parameters(
    document: &Document,
    revision: u64,
    sketch: EntityId,
    report: Option<&ReferenceReport>,
    parameters: &EvaluatedParameters,
) -> Result<EvaluatedSketch, InputError> {
    evaluate_document_sketch_seeded(document, revision, sketch, report, parameters, None)
}
pub(crate) fn evaluate_document_sketch_seeded(
    document: &Document,
    revision: u64,
    sketch: EntityId,
    report: Option<&ReferenceReport>,
    parameters: &EvaluatedParameters,
    seed: Option<&Sketch>,
) -> Result<EvaluatedSketch, InputError> {
    if parameters.document != *document {
        return Err(error(
            sketch,
            "Parameter evaluation belongs to different source",
        ));
    }
    let (source, frame, plane) = Source::read_for_evaluation(document, sketch, revision, report)
        .map_err(|e| error(sketch, e.to_string()))?;
    let mut resolved_rhs = BTreeMap::new();
    for constraint in &source.sketch.constraints {
        if let Some(d) = constraint.kind.dimension() {
            let unit = Unit::parse(d.unit())
                .ok_or_else(|| error(constraint.id, "Unknown dimension unit"))?;
            resolved_rhs.insert(
                constraint.id,
                Measurement {
                    value: d
                        .value_in(unit, Some(parameters))
                        .map_err(|e| error(constraint.id, e))?,
                    unit: d.unit().into(),
                },
            );
        }
    }
    let mut solver_input = source.sketch.clone();
    if let Some(seed) = seed {
        // Seed only numeric unknowns by authored identity. The canonical
        // expressions, constraints and source points remain untouched.
        for point in &mut solver_input.points {
            if let Some(solved) = seed.points.iter().find(|p| p.id == point.id) {
                point.position_mm = solved.position_mm;
            }
        }
        for curve in &mut solver_input.curves {
            if let CurveKind::Circle { radius_mm, .. } = &mut curve.kind
                && let Some(CurveKind::Circle {
                    radius_mm: solved, ..
                }) = seed
                    .curves
                    .iter()
                    .find(|c| c.id == curve.id)
                    .map(|c| &c.kind)
            {
                *radius_mm = *solved;
            }
        }
    }
    let evaluation = evaluate_with_parameters(&solver_input, parameters, None)?;
    Ok(EvaluatedSketch {
        stamp: SketchStamp {
            document: document.id(),
            revision,
            source_hash: ContentHash::document(document).map_err(|e| error(sketch, e))?,
        },
        sketch,
        plane,
        frame,
        canonical: source,
        evaluation,
        resolved_rhs,
    })
}

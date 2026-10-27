//! Current evaluated sketch geometry is separate from canonical authored source.
//! Expressions and their bindings never become literal source during solving.
use crate::{Evaluation, InputError, Measurement, evaluate_with_parameters};
use multicad_core::{
    constraint_sketch::{CurveKind, Sketch, source::Source},
    document::{Document, EntityData, EntityId},
    parameters::{self, EvaluatedParameters, Unit},
    project_file::ContentHash,
    references::ReferenceReport,
    sketch::{Frame, Geometry},
    sketch_regions::{self, Extraction},
};
use std::{borrow::Cow, collections::BTreeMap};
pub const MAX_BATCH_SKETCHES: usize = 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SketchStamp {
    pub document: EntityId,
    pub revision: u64,
    pub source_hash: ContentHash,
}
impl SketchStamp {
    /// A request/publication-boundary guard, not a per-paint hash operation.
    pub fn is_current(self, document: &Document, revision: u64) -> bool {
        self.document == document.id()
            && self.revision == revision
            && ContentHash::document(document).is_ok_and(|h| h == self.source_hash)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct SketchBatch {
    pub stamp: SketchStamp,
    pub sketches: BTreeMap<EntityId, Result<EvaluatedSketch, InputError>>,
}
impl SketchBatch {
    pub fn is_current(&self, document: &Document, revision: u64) -> bool {
        self.stamp.is_current(document, revision)
    }
}
/// One immutable canonical snapshot and parameter solve per worker job. The
/// document borrow prevents source mutation while this context is in use.
pub struct SketchEvaluationContext<'a> {
    document: &'a Document,
    parameters: Cow<'a, EvaluatedParameters>,
    stamp: SketchStamp,
}
fn stamp(document: &Document, revision: u64) -> Result<SketchStamp, InputError> {
    Ok(SketchStamp {
        document: document.id(),
        revision,
        source_hash: ContentHash::document(document).map_err(|message| InputError {
            source: None,
            message,
        })?,
    })
}
fn cancelled(interrupt: Option<&dyn Fn() -> bool>) -> Result<(), InputError> {
    if interrupt.is_some_and(|f| f()) {
        Err(InputError {
            source: None,
            message: "Sketch evaluation cancelled".into(),
        })
    } else {
        Ok(())
    }
}
fn bounded_ids(ids: impl IntoIterator<Item = EntityId>) -> Result<Vec<EntityId>, InputError> {
    let ids: Vec<_> = ids.into_iter().take(MAX_BATCH_SKETCHES + 1).collect();
    if ids.len() > MAX_BATCH_SKETCHES {
        return Err(InputError {
            source: None,
            message: format!("Sketch batch exceeds the {MAX_BATCH_SKETCHES}-sketch job budget"),
        });
    }
    Ok(ids)
}
impl<'a> SketchEvaluationContext<'a> {
    /// Parameter name normalization is an internal derived view only. Canonical
    /// source, output source and the shared hash remain exactly the input doc.
    pub fn new(document: &'a Document, revision: u64) -> Result<Self, InputError> {
        let parameters = parameters::evaluate(document).map_err(|e| InputError {
            source: e.entity,
            message: e.to_string(),
        })?;
        Ok(Self {
            document,
            parameters: Cow::Owned(parameters),
            stamp: stamp(document, revision)?,
        })
    }
    /// Externally supplied contexts must match canonical source exactly. Use
    /// Session.document()/parameters() after checked normalization; a stale or
    /// foreign context is never relabeled as current source.
    pub fn from_parameters(
        document: &'a Document,
        revision: u64,
        parameters: &'a EvaluatedParameters,
    ) -> Result<Self, InputError> {
        if parameters.document != *document {
            return Err(InputError {source:None,message:"Parameter context does not match canonical source; use the matching Session document and parameters, or let SketchEvaluationContext::new evaluate this source".into()});
        }
        Ok(Self {
            document,
            parameters: Cow::Borrowed(parameters),
            stamp: stamp(document, revision)?,
        })
    }
    pub fn stamp(&self) -> SketchStamp {
        self.stamp
    }
    pub fn document(&self) -> &Document {
        self.document
    }
    pub fn evaluate(
        &self,
        sketch: EntityId,
        report: Option<&ReferenceReport>,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<EvaluatedSketch, InputError> {
        evaluate_checked(
            self.document,
            self.stamp,
            sketch,
            report,
            &self.parameters,
            None,
            interrupt,
        )
    }
    pub fn evaluate_requested(
        &self,
        ids: impl IntoIterator<Item = EntityId>,
        report: Option<&ReferenceReport>,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<SketchBatch, InputError> {
        let ids = bounded_ids(ids)?;
        let mut sketches = BTreeMap::new();
        for id in ids {
            cancelled(interrupt)?;
            sketches
                .entry(id)
                .or_insert_with(|| self.evaluate(id, report, interrupt));
            cancelled(interrupt)?;
        }
        Ok(SketchBatch {
            stamp: self.stamp,
            sketches,
        })
    }
}
/// All supported constrained sketches share one parameter evaluation and one
/// canonical hash. Individual input/solve failures remain local to their IDs.
pub fn evaluate_document_sketches(
    document: &Document,
    revision: u64,
    report: Option<&ReferenceReport>,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<SketchBatch, InputError> {
    cancelled(interrupt)?;
    let ids=bounded_ids(document.entities().filter_map(|e|matches!(&e.data,EntityData::Sketch {definition,..} if definition.type_id==multicad_core::constraint_sketch::source::TYPE).then_some(e.id)))?;
    let context = SketchEvaluationContext::new(document, revision)?;
    cancelled(interrupt)?;
    context.evaluate_requested(ids, report, interrupt)
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
        self.stamp.is_current(document, revision)
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
    SketchEvaluationContext::new(document, revision)?.evaluate(sketch, report, None)
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
    let context = SketchEvaluationContext::from_parameters(document, revision, parameters)?;
    evaluate_checked(
        document,
        context.stamp,
        sketch,
        report,
        parameters,
        seed,
        None,
    )
}
fn evaluate_checked(
    document: &Document,
    stamp: SketchStamp,
    sketch: EntityId,
    report: Option<&ReferenceReport>,
    parameters: &EvaluatedParameters,
    seed: Option<&Sketch>,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<EvaluatedSketch, InputError> {
    cancelled(interrupt)?;
    let (source, frame, plane) =
        Source::read_for_evaluation(document, sketch, stamp.revision, report)
            .map_err(|e| error(sketch, e.to_string()))?;
    let mut resolved_rhs = BTreeMap::new();
    for constraint in &source.sketch.constraints {
        cancelled(interrupt)?;
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
    let evaluation = evaluate_with_parameters(&solver_input, parameters, interrupt)?;
    cancelled(interrupt)?;
    Ok(EvaluatedSketch {
        stamp,
        sketch,
        plane,
        frame,
        canonical: source,
        evaluation,
        resolved_rhs,
    })
}

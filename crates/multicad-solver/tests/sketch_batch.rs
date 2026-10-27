use multicad_core::{
    commands::Session,
    constraint_sketch::{ConstraintKind, Curve, CurveKind, Dimension, source::Source},
    document::*,
    parameters::{self, BoundExpression},
    project_file::ContentHash,
    sketch::{self, Datum},
};
use multicad_solver::{
    MAX_BATCH_SKETCHES, SketchEvaluationContext, evaluate_document_sketch,
    evaluate_document_sketches,
};
use std::cell::Cell;
fn literal(doc: &Document, name: &str) -> (Vec<Mutation>, EntityId) {
    let (id, mut edits) = Source::create(Datum::Xy, name).unwrap();
    let temp = doc.edited(&edits).unwrap();
    let (mut source, _, _) = Source::read(&temp, id).unwrap();
    source.add_rectangle([0., 0.], [40., 20.]).unwrap();
    let definition = source.definition().unwrap();
    for edit in &mut edits {
        if let Mutation::Insert(e) = edit
            && e.id == id
            && let EntityData::Sketch { definition: d, .. } = &mut e.data
        {
            *d = definition.clone();
        }
    }
    (edits, id)
}
fn parameter(name: &str, value: &str) -> Entity {
    Entity::new(
        name,
        EntityData::Parameter {
            expression: value.into(),
            unit: "mm".into(),
            exposed: false,
            inputs: vec![],
        },
    )
}
fn semantic(doc: &Document, name: &str, value: &str) -> (Vec<Mutation>, EntityId) {
    let (mut edits, id) = literal(doc, name);
    let temp = doc.edited(&edits).unwrap();
    let (mut source, _, _) = Source::read(&temp, id).unwrap();
    let edge = source.sketch.curves[0].id;
    source.add_constraint(ConstraintKind::Length {
        line: edge,
        dimension: Dimension::Expression {
            expression: BoundExpression::bind(&temp, value, "mm", None).unwrap(),
        },
    });
    let replacement = source.replace(&temp, id).unwrap().pop().unwrap();
    let Mutation::Replace(entity) = replacement else {
        panic!("replacement")
    };
    edits
        .iter_mut()
        .find(|e| matches!(e,Mutation::Insert(e) if e.id==id))
        .map(|e| *e = Mutation::Insert(entity))
        .unwrap();
    (edits, id)
}
#[test]
fn mixed_versions_and_local_failures_share_one_original_canonical_stamp() {
    let mut doc = Document::new("Batch").unwrap();
    let (edits, plain) = literal(&doc, "Plain");
    doc = doc.edited(&edits).unwrap();
    let (edits, bound) = semantic(&doc, "Bound", "=80");
    doc = doc.edited(&edits).unwrap();
    let (edits, missing) = semantic(&doc, "Deleted parameter", "=70");
    doc = doc.edited(&edits).unwrap();
    let (mut source, _, _) = Source::read_for_evaluation(&doc, missing, 4, None).unwrap();
    let fake = EntityId::new();
    let dimension = source
        .sketch
        .constraints
        .last_mut()
        .unwrap()
        .kind
        .dimension_mut()
        .unwrap();
    *dimension = Dimension::Expression {
        expression: BoundExpression {
            text: "=$deleted".into(),
            unit: "mm".into(),
            bindings: vec![parameters::ParameterBinding {
                alias: "deleted".into(),
                target: fake,
            }],
        },
    };
    doc = doc.edited(&source.replace(&doc, missing).unwrap()).unwrap();
    let (legacy, edits) = sketch::create(Datum::Xy, "Legacy").unwrap();
    doc = doc.edited(&edits).unwrap();
    let before = doc.clone();
    let batch = evaluate_document_sketches(&doc, 4, None, None).unwrap();
    assert_eq!(doc, before);
    assert_eq!(batch.sketches.len(), 3);
    assert!(!batch.sketches.contains_key(&legacy));
    assert!(batch.sketches[&plain].as_ref().unwrap().solved().is_some());
    assert!(batch.sketches[&bound].as_ref().unwrap().solved().is_some());
    assert!(batch.sketches[&missing].is_err());
    assert_eq!(
        batch.sketches[&missing].as_ref().unwrap_err().source,
        Some(source.sketch.constraints.last().unwrap().id)
    );
    assert_eq!(
        batch.stamp.source_hash,
        ContentHash::document(&doc).unwrap()
    );
    for eval in batch.sketches.values().filter_map(|r| r.as_ref().ok()) {
        assert_eq!(eval.stamp, batch.stamp);
    }
    assert!(batch.is_current(&doc, 4));
    assert!(!batch.is_current(&doc, 5));
    let mut changed = doc.clone();
    changed
        .rename("Changed without relabeling snapshot")
        .unwrap();
    assert!(!batch.is_current(&changed, 4));
    assert!(!batch.is_current(&Document::new("Foreign").unwrap(), 4));
}
#[test]
fn convenience_entries_evaluate_unbound_names_without_normalizing_the_canonical_input() {
    let base = parameter("base", "40");
    let derived = parameter("width", "=$base * 2");
    let doc = Document::new("Unbound canonical input")
        .unwrap()
        .edited(&[Mutation::Insert(base), Mutation::Insert(derived)])
        .unwrap();
    let (edits, id) = semantic(&doc, "Driven", "=$width");
    let doc = doc.edited(&edits).unwrap();
    let before = doc.clone();
    let canonical = Source::read_for_evaluation(&doc, id, 7, None).unwrap().0;
    let parameters = parameters::evaluate(&doc).unwrap();
    assert_ne!(parameters.document, doc);
    assert!(SketchEvaluationContext::from_parameters(&doc, 7, &parameters).is_err());
    let one = evaluate_document_sketch(&doc, 7, id, None).unwrap();
    let batch = evaluate_document_sketches(&doc, 7, None, None).unwrap();
    let context = SketchEvaluationContext::new(&doc, 7).unwrap();
    assert_eq!(doc, before);
    assert_eq!(context.document(), &doc);
    assert_eq!(one.canonical(), &canonical);
    assert_eq!(one.stamp.source_hash, ContentHash::document(&doc).unwrap());
    assert_eq!(batch.sketches[&id].as_ref().unwrap(), &one);
    assert_eq!(context.stamp(), one.stamp);
    let value = one.resolved_rhs.values().last().unwrap().value;
    assert!((value - 80.).abs() < 1e-7);
}
#[test]
fn requested_context_rejects_foreign_or_stale_parameter_views_and_preserves_id_errors() {
    let doc = Document::new("Requested").unwrap();
    let (edits, id) = literal(&doc, "Sketch");
    let doc = doc.edited(&edits).unwrap();
    let session = Session::new(doc).unwrap();
    let context =
        SketchEvaluationContext::from_parameters(session.document(), 3, session.parameters())
            .unwrap();
    let missing = EntityId::new();
    let result = context
        .evaluate_requested([id, missing], None, None)
        .unwrap();
    assert!(result.sketches[&id].is_ok());
    assert!(result.sketches[&missing].is_err());
    let mut stale = session.document().clone();
    stale.rename("New source").unwrap();
    assert!(SketchEvaluationContext::from_parameters(&stale, 4, session.parameters()).is_err());
    let foreign = Document::new("Foreign").unwrap();
    assert!(SketchEvaluationContext::from_parameters(&foreign, 3, session.parameters()).is_err());
    assert!(
        context
            .evaluate_requested(std::iter::repeat_n(id, MAX_BATCH_SKETCHES + 1), None, None)
            .is_err()
    );
}
#[test]
fn batch_cancellation_never_returns_a_partly_current_receipt() {
    let doc = Document::new("Cancel").unwrap();
    let (edits, id) = literal(&doc, "Sketch");
    let doc = doc.edited(&edits).unwrap();
    let before = doc.clone();
    assert!(evaluate_document_sketches(&doc, 0, None, Some(&|| true)).is_err());
    let context = SketchEvaluationContext::new(&doc, 0).unwrap();
    let calls = Cell::new(0);
    let interrupt = || {
        let next = calls.get() + 1;
        calls.set(next);
        next >= 4
    };
    assert!(
        context
            .evaluate_requested([id], None, Some(&interrupt))
            .is_err()
    );
    assert!(calls.get() >= 4);
    assert_eq!(doc, before);
}
#[test]
fn numerical_conflict_is_a_per_sketch_result_with_no_geometry_candidate() {
    let doc = Document::new("Conflict").unwrap();
    let (edits, id) = semantic(&doc, "Conflicted", "=80");
    let doc = doc.edited(&edits).unwrap();
    let (mut source, _, _) = Source::read_for_evaluation(&doc, id, 0, None).unwrap();
    let edge = source.sketch.curves[0].id;
    source.add_constraint(ConstraintKind::Length {
        line: edge,
        dimension: Dimension::Literal {
            value: 81.,
            unit: "mm".into(),
        },
    });
    let doc = doc.edited(&source.replace(&doc, id).unwrap()).unwrap();
    let (edits, good) = literal(&doc, "Valid");
    let doc = doc.edited(&edits).unwrap();
    let batch = evaluate_document_sketches(&doc, 0, None, None).unwrap();
    assert!(batch.sketches[&good].as_ref().unwrap().solved().is_some());
    let conflict = batch.sketches[&id].as_ref().unwrap();
    assert!(conflict.solved().is_none());
    assert!(!conflict.evaluation.diagnostics.conflicts.is_empty());
    assert!(conflict.regions().is_err());
}
#[test]
fn one_thousand_independent_profiles_share_the_batch_stamp_and_keep_canonical_source() {
    let mut edits = vec![];
    for i in 0..1000 {
        let (id, mut created) = Source::create(Datum::Xy, &format!("Circle {i}")).unwrap();
        let mut source = Source {
            u: [1., 0., 0.],
            sketch: Default::default(),
        };
        let center = source.add_point([i as f64 * 20., 0.]);
        source.sketch.curves.push(Curve {
            id: EntityId::new(),
            construction: false,
            kind: CurveKind::Circle {
                center,
                radius_mm: 5.,
            },
        });
        let definition = source.definition().unwrap();
        for e in &mut created {
            if let Mutation::Insert(e) = e
                && e.id == id
                && let EntityData::Sketch { definition: d, .. } = &mut e.data
            {
                *d = definition.clone();
            }
        }
        edits.extend(created);
    }
    let doc = Document::new("One thousand profiles")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let before = doc.clone();
    let batch = evaluate_document_sketches(&doc, 11, None, None).unwrap();
    assert_eq!(batch.sketches.len(), 1000);
    for (id, result) in &batch.sketches {
        let result = result.as_ref().unwrap();
        assert_eq!(result.sketch, *id);
        assert_eq!(result.stamp, batch.stamp);
        assert!(result.solved().is_some());
    }
    assert_eq!(doc, before);
}

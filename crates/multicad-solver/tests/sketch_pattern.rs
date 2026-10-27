use multicad_core::{
    constraint_sketch::{ConstraintKind, Curve, CurveKind, Dimension, source::Source},
    document::*,
    parameters::BoundExpression,
    sketch::Datum,
    sketch_pattern::{self, Pattern, PatternOptions},
};
use multicad_solver::evaluate_document_sketch;
use std::{
    collections::BTreeSet,
    f64::consts::{FRAC_PI_2, PI},
};
fn fixture() -> (Document, Source, EntityId, EntityId, EntityId) {
    let radius = Entity::new(
        "radius",
        EntityData::Parameter {
            expression: "3".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let (id, mut edits) = Source::create(Datum::Xy, "Repeated holes").unwrap();
    edits.push(Mutation::Insert(radius.clone()));
    let doc = Document::new("Pattern solver")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let (mut source, _, _) = Source::read(&doc, id).unwrap();
    let center = source.add_point([10., 0.]);
    let curve = EntityId::new();
    source.sketch.curves.push(Curve {
        id: curve,
        construction: false,
        kind: CurveKind::Circle {
            center,
            radius_mm: 2.,
        },
    });
    source.add_constraint(ConstraintKind::Fix { point: center });
    let dimension = source.add_constraint(ConstraintKind::Radius {
        curve,
        dimension: Dimension::Expression {
            expression: BoundExpression::bind(&doc, "=$radius", "mm", None).unwrap(),
        },
    });
    (doc, source, id, curve, dimension)
}
#[test]
fn repeated_formula_dimensions_solve_on_new_identities_and_keep_parameter_bindings() {
    for pattern in [
        Pattern::Linear {
            step_mm: [20., 15.],
            count: 4,
        },
        Pattern::Circular {
            center_mm: [0., 0.],
            step_angle_rad: FRAC_PI_2,
            count: 4,
        },
    ] {
        let (doc, source, id, curve, dimension) = fixture();
        let before = source.clone();
        let original_expression = source
            .sketch
            .constraints
            .iter()
            .find(|c| c.id == dimension)
            .unwrap()
            .kind
            .dimension()
            .unwrap()
            .expression()
            .unwrap()
            .clone();
        let draft = sketch_pattern::draft(
            &source,
            &BTreeSet::from([curve]),
            pattern,
            PatternOptions::default(),
        )
        .unwrap();
        assert_eq!(source, before);
        let doc = doc
            .edited(&draft.source.replace(&doc, id).unwrap())
            .unwrap();
        let encoded = serde_json::to_vec(&doc).unwrap();
        let reopened: Document = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(doc, reopened);
        let evaluated = evaluate_document_sketch(&doc, 5, id, None).unwrap();
        let solved = evaluated.solved().unwrap();
        assert_eq!(solved.curves.len(), 4);
        assert_eq!(evaluated.evaluation.diagnostics.degrees_of_freedom, 0);
        for instance in &draft.report.instances {
            let copied = solved
                .constraints
                .iter()
                .find(|c| c.id == instance.constraints[&dimension])
                .unwrap();
            assert_eq!(
                copied.kind.dimension().unwrap().expression(),
                Some(&original_expression)
            );
            assert_ne!(instance.curves[&curve], curve);
            let copied_curve = solved
                .curves
                .iter()
                .find(|c| c.id == instance.curves[&curve])
                .unwrap();
            assert!(
                matches!(copied_curve.kind,CurveKind::Circle {radius_mm,..} if (radius_mm-3.).abs()<1e-8)
            );
        }
        let regions = evaluated.regions().unwrap();
        assert!(regions.diagnostics.is_empty());
        assert_eq!(regions.regions.len(), 4);
        assert!((regions.regions.iter().map(|r| r.area_mm2).sum::<f64>() - 36. * PI).abs() < 1e-6);
        assert_eq!(evaluated.canonical(), &draft.source);
    }
}
#[test]
fn linear_copies_retain_dimension_reference_and_hv_constraints_under_a_genuine_solve() {
    let (id, edits) = Source::create(Datum::Xy, "Panels").unwrap();
    let doc = Document::new("Panels").unwrap().edited(&edits).unwrap();
    let (mut source, _, _) = Source::read(&doc, id).unwrap();
    let edge = source.add_rectangle([0., 0.], [20., 10.]).unwrap();
    source.add_constraint(ConstraintKind::Fix {
        point: source.sketch.points[0].id,
    });
    source.add_constraint(ConstraintKind::Length {
        line: edge,
        dimension: Dimension::Literal {
            value: 20.,
            unit: "mm".into(),
        },
    });
    source.add_constraint(ConstraintKind::Length {
        line: source.sketch.curves[1].id,
        dimension: Dimension::Literal {
            value: 10.,
            unit: "mm".into(),
        },
    });
    let reference = source.add_constraint(ConstraintKind::Distance {
        a: source.sketch.points[0].id,
        b: source.sketch.points[2].id,
        axis: multicad_core::constraint_sketch::Axis::Direct,
        dimension: Dimension::Literal {
            value: 999.,
            unit: "mm".into(),
        },
    });
    source.sketch.constraints.last_mut().unwrap().reference = true;
    let draft = sketch_pattern::draft(
        &source,
        &source.sketch.curves.iter().map(|c| c.id).collect(),
        Pattern::Linear {
            step_mm: [40., 0.],
            count: 3,
        },
        PatternOptions::default(),
    )
    .unwrap();
    let doc = doc
        .edited(&draft.source.replace(&doc, id).unwrap())
        .unwrap();
    let evaluated = evaluate_document_sketch(&doc, 0, id, None).unwrap();
    assert_eq!(evaluated.evaluation.diagnostics.degrees_of_freedom, 0);
    assert_eq!(evaluated.regions().unwrap().regions.len(), 3);
    for instance in &draft.report.instances {
        let measure = &evaluated.evaluation.measurements[&instance.constraints[&reference]];
        assert!((measure.value - 500_f64.sqrt()).abs() < 1e-7);
        let constraint = evaluated
            .canonical()
            .sketch
            .constraints
            .iter()
            .find(|c| c.id == instance.constraints[&reference])
            .unwrap();
        assert!(constraint.reference);
        assert!(matches!(
            constraint.kind.dimension().unwrap(),
            Dimension::Literal { value: 999., .. }
        ));
    }
}

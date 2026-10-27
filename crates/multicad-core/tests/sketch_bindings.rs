use multicad_core::{
    constraint_sketch::{
        ConstraintKind, Dimension,
        binding::{self, DimensionInput},
        source::Source,
    },
    document::*,
    parameters::{self, BoundExpression, ParameterBinding, Unit},
    sketch::Datum,
};
fn parameter(name: &str, value: &str, unit: &str) -> Entity {
    Entity::new(
        name,
        EntityData::Parameter {
            expression: value.into(),
            unit: unit.into(),
            exposed: true,
            inputs: vec![],
        },
    )
}
fn setup() -> (Document, EntityId, Source, EntityId, EntityId) {
    let width = parameter("width", "80", "mm");
    let (id, mut edits) = Source::create(Datum::Xy, "Profile").unwrap();
    edits.push(Mutation::Insert(width.clone()));
    let doc = Document::new("Binding source")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let (mut source, _, _) = Source::read(&doc, id).unwrap();
    let line = source.add_rectangle([0., 0.], [40., 20.]).unwrap();
    let c = source.add_constraint(ConstraintKind::Length {
        line,
        dimension: Dimension::Literal {
            value: 40.,
            unit: "mm".into(),
        },
    });
    let doc = doc.edited(&source.replace(&doc, id).unwrap()).unwrap();
    (doc, id, source, c, width.id)
}
#[test]
fn literals_retain_schema_one_json_and_reading_never_rewrites_source_hash() {
    let (doc, id, source, _, _) = setup();
    let hash = multicad_core::project_file::ContentHash::document(&doc).unwrap();
    assert_eq!(
        serde_json::to_value(Dimension::Literal {
            value: 12.,
            unit: "cm".into()
        })
        .unwrap(),
        serde_json::json!({"value":12.0,"unit":"cm"})
    );
    assert_eq!(source.definition().unwrap().schema_version, 1);
    let (read, _, _) = Source::read(&doc, id).unwrap();
    assert_eq!(read, source);
    assert_eq!(
        multicad_core::project_file::ContentHash::document(&doc).unwrap(),
        hash
    );
}
#[test]
fn checked_bindings_install_dependency_links_gate_raw_geometry_and_keep_v2_sticky() {
    let (doc, id, source, c, width) = setup();
    let bound = binding::draft_dimension(
        &doc,
        &source,
        c,
        &DimensionInput::Expression {
            text: "= $width / 2".into(),
            unit: "mm".into(),
        },
        false,
    )
    .unwrap();
    assert_eq!(source.sketch.points, bound.sketch.points);
    assert_eq!(source.sketch.curves, bound.sketch.curves);
    let next = doc.edited(&bound.replace(&doc, id).unwrap()).unwrap();
    assert!(multicad_core::constraint_sketch::source::requires_v2(&next));
    assert!(
        next.get(id)
            .unwrap()
            .references()
            .iter()
            .any(|l| l.target == width && l.role == format!("dimension/{c}/width"))
    );
    assert!(Source::read(&next, id).is_err());
    assert!(multicad_core::sketch::Sketch::read(&next, id).is_err());
    assert!(bound.display_geometry().is_err());
    assert!(
        multicad_core::sketch_regions::extract(&bound)
            .regions
            .is_empty()
    );
    assert_eq!(
        Source::read_for_evaluation(&next, id, 0, None).unwrap().0,
        bound
    );
    let literal = binding::draft_dimension(
        &next,
        &bound,
        c,
        &DimensionInput::Literal {
            value: 45.,
            unit: "mm".into(),
        },
        false,
    )
    .unwrap();
    let restored = next.edited(&literal.replace(&next, id).unwrap()).unwrap();
    assert!(multicad_core::constraint_sketch::source::requires_v2(
        &restored
    ));
    assert!(literal.dimension_links().is_empty());
    assert!(Source::read(&restored, id).is_err());
}
#[test]
fn source_validation_rejects_hidden_expression_versions_and_tampered_dependency_edges() {
    let (doc, id, source, c, _) = setup();
    let bound = binding::draft_dimension(
        &doc,
        &source,
        c,
        &DimensionInput::Expression {
            text: "= $width".into(),
            unit: "mm".into(),
        },
        false,
    )
    .unwrap();
    let mut definition = bound.definition().unwrap();
    definition.schema_version = 1;
    assert!(Source::from_definition(&definition).is_err());
    let mut edits = bound.replace(&doc, id).unwrap();
    let Mutation::Replace(e) = &mut edits[0] else {
        panic!("replace")
    };
    let EntityData::Sketch { inputs, .. } = &mut e.data else {
        panic!("sketch")
    };
    inputs.clear();
    assert!(doc.edited(&edits).is_err());
    assert_eq!(Source::read(&doc, id).unwrap().0, source);
}
#[test]
fn rename_and_same_name_recreation_never_steal_existing_bindings() {
    let old = parameter("width", "2", "in");
    let mut doc = Document::new("Names")
        .unwrap()
        .edited(&[Mutation::Insert(old.clone())])
        .unwrap();
    let bound = BoundExpression::bind(&doc, "= $width / 2", "mm", None).unwrap();
    assert!(
        (bound
            .evaluate(&parameters::evaluate(&doc).unwrap())
            .unwrap()
            .in_unit(Unit::Millimeter)
            .unwrap()
            - 25.4)
            .abs()
            < 1e-9
    );
    let mut renamed = old.clone();
    renamed.name = "span".into();
    let replacement = parameter("width", "999", "mm");
    doc = doc
        .edited(&[
            Mutation::Replace(renamed),
            Mutation::Insert(replacement.clone()),
        ])
        .unwrap();
    let preserved = BoundExpression::bind(&doc, &bound.text, &bound.unit, Some(&bound)).unwrap();
    assert_eq!(preserved.bindings[0].target, old.id);
    let rebound = BoundExpression::bind(&doc, &bound.text, &bound.unit, None).unwrap();
    assert_eq!(rebound.bindings[0].target, replacement.id);
    doc.remove(old.id).unwrap();
    assert!(
        bound
            .evaluate(&parameters::evaluate(&doc).unwrap())
            .is_err()
    );
    assert!(BoundExpression::bind(&doc, &bound.text, &bound.unit, Some(&bound)).is_err());
}
#[test]
fn shared_expression_grammar_preserves_contextual_units_and_rejects_wrong_dimensions() {
    let height = parameter("height", "2", "cm");
    let angle = parameter("angle", "90", "deg");
    let doc = Document::new("Units")
        .unwrap()
        .edited(&[Mutation::Insert(height), Mutation::Insert(angle)])
        .unwrap();
    let values = parameters::evaluate(&doc).unwrap();
    let e = BoundExpression::bind(&doc, "=600 - $height", "mm", None).unwrap();
    assert!(
        (e.evaluate(&values)
            .unwrap()
            .in_unit(Unit::Millimeter)
            .unwrap()
            - 580.)
            .abs()
            < 1e-9
    );
    let e = BoundExpression::bind(&doc, "= $angle", "rad", None).unwrap();
    assert!(
        (e.evaluate(&values).unwrap().in_unit(Unit::Radian).unwrap() - std::f64::consts::FRAC_PI_2)
            .abs()
            < 1e-9
    );
    for (text, unit) in [
        ("=$height > 1 mm", "mm"),
        ("=$angle", "mm"),
        ("=1 / 0", "mm"),
    ] {
        let e = BoundExpression::bind(&doc, text, unit, None).unwrap();
        assert!(e.evaluate(&values).is_err());
    }
    assert!(BoundExpression::bind(&doc, "height + 1", "mm", None).is_err());
    assert!(BoundExpression::bind(&doc, "=$unknown", "mm", None).is_err());
}
#[test]
fn binding_metadata_is_exact_and_missing_targets_remain_preservable_for_repair() {
    let (doc, id, source, c, width) = setup();
    let mut bound = BoundExpression::bind(&doc, "=$width", "mm", None).unwrap();
    bound.bindings.push(ParameterBinding {
        alias: "width".into(),
        target: width,
    });
    assert!(bound.validate().is_err());
    bound.bindings.pop();
    bound.bindings[0].alias = "unused".into();
    assert!(bound.validate().is_err());
    let s = binding::draft_dimension(
        &doc,
        &source,
        c,
        &DimensionInput::Expression {
            text: "=$width".into(),
            unit: "mm".into(),
        },
        false,
    )
    .unwrap();
    let mut doc = doc.edited(&s.replace(&doc, id).unwrap()).unwrap();
    doc.remove(width).unwrap();
    doc.validate().unwrap();
    assert!(!doc.reference_diagnostics().is_empty());
    let encoded = serde_json::to_vec(&doc).unwrap();
    let reopened: Document = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(doc, reopened);
}
#[test]
fn classifier_distinguishes_nested_areas_and_shared_boundaries_without_sampling() {
    let mut s = Source {
        u: [1., 0., 0.],
        sketch: Default::default(),
    };
    s.add_rectangle([0., 0.], [40., 20.]).unwrap();
    let center = s.add_point([20., 10.]);
    s.sketch
        .curves
        .push(multicad_core::constraint_sketch::Curve {
            id: EntityId::new(),
            construction: false,
            kind: multicad_core::constraint_sketch::CurveKind::Circle {
                center,
                radius_mm: 5.,
            },
        });
    let r = multicad_core::sketch_regions::extract(&s);
    let ring = r.regions.iter().find(|r| !r.holes.is_empty()).unwrap();
    let disk = r.regions.iter().find(|r| r.holes.is_empty()).unwrap();
    use multicad_core::sketch_regions::PointLocation;
    assert_eq!(ring.classify([5., 5.]).unwrap(), PointLocation::Inside);
    assert_eq!(ring.classify([20., 10.]).unwrap(), PointLocation::Outside);
    assert_eq!(ring.classify([25., 10.]).unwrap(), PointLocation::Boundary);
    assert!(disk.contains([20., 10.]).unwrap());
    assert!(!disk.contains([25., 10.]).unwrap());
}

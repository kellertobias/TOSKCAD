use multicad_core::{
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId},
    model::*,
    parameters::{self, BoundExpression},
    references::{GeometryIntent, Origin, TopologyKind},
};
use std::collections::BTreeSet;
#[test]
fn pattern_counts_keep_bound_units_and_reject_fractional_or_unbounded_rebuilds() {
    let mut doc = Document::new("Advanced parameter source").unwrap();
    doc.insert(Entity::new(
        "copies",
        EntityData::Parameter {
            expression: "3".into(),
            unit: "count".into(),
            exposed: true,
            inputs: vec![],
        },
    ))
    .unwrap();
    let expression = BoundExpression::bind(&doc, "=$copies", "count", None).unwrap();
    let mut p = LinearPatternParameters {
        count: Dimension::Expression { expression },
        ..Default::default()
    };
    assert!(matches!(
        AdvancedParameters::LinearPattern(p.clone())
            .resolve(&parameters::evaluate(&doc).unwrap())
            .unwrap(),
        ResolvedAdvanced::LinearPattern { count: 3, .. }
    ));
    for invalid in [0., 1.5, 129., f64::INFINITY] {
        p.count = Dimension::Literal {
            value: invalid,
            unit: "count".into(),
        };
        assert!(
            AdvancedParameters::LinearPattern(p.clone())
                .resolve(&parameters::evaluate(&doc).unwrap())
                .is_err()
        );
    }
    for kind in [
        AdvancedKind::Shell,
        AdvancedKind::Sweep,
        AdvancedKind::Loft,
        AdvancedKind::Draft,
        AdvancedKind::Mirror,
        AdvancedKind::LinearPattern,
        AdvancedKind::CircularPattern,
    ] {
        let p = AdvancedParameters::default_for(kind);
        assert_eq!(p.kind(), kind);
        p.resolve(&parameters::evaluate(&doc).unwrap()).unwrap();
    }
}
#[test]
fn advanced_face_intent_lives_only_in_its_frozen_canonical_link() {
    let body = EntityId::new();
    let producer = EntityId::new();
    let capture = GeometryIntent {
        id: EntityId::new(),
        kind: TopologyKind::Face,
        origins: BTreeSet::from([Origin {
            source: producer,
            role: "cap.end".into(),
        }]),
        boundary: None,
        attachment: None,
    };
    let request = AdvancedRequest::on_body(
        BodyInput { body, producer },
        AdvancedParameters::Shell(Default::default()),
        vec![capture.clone()],
    );
    let source = request.source().unwrap();
    assert_eq!(source.faces, vec![capture.id]);
    let definition = source.definition().unwrap();
    assert!(
        !serde_json::to_string(&definition.data)
            .unwrap()
            .contains("origins")
    );
    assert_eq!(
        AdvancedDefinition::from_definition(&definition).unwrap(),
        source
    );
    let links = source.inputs(std::slice::from_ref(&capture)).unwrap();
    assert_eq!(links.iter().filter(|l| l.geometry.is_some()).count(), 1);
    assert_eq!(
        links.iter().find(|l| l.geometry.is_some()).unwrap().target,
        producer
    );
    let mut invalid = capture;
    invalid.kind = TopologyKind::Edge;
    assert!(source.inputs(&[invalid]).is_err());
}

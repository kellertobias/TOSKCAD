use multicad_core::{
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId},
    model::*,
    parameters::{self, BoundExpression},
    references::{GeometryIntent, Origin, TopologyKind},
};
use std::collections::BTreeSet;
#[test]
fn advanced_edits_retain_identity_and_roundtrip_bound_count_through_undo() {
    use multicad_core::{
        commands::{Actor, Channel, Session},
        project_file::{self, ProjectFile},
    };
    let actor = Actor::local(Channel::Ui);
    let mut session = Session::new(Document::new("Advanced source edits").unwrap()).unwrap();
    let stock = PrimitiveRequest::new(
        "Stock",
        PrimitiveParameters::Box {
            size: [length(10.), length(20.), length(30.)],
            frame: Default::default(),
        },
    );
    session
        .commit(
            prepare_primitive(&session, 0, actor.clone(), &stock)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let before = session.document().clone();
    let request = AdvancedRequest::on_body(
        BodyInput {
            body: stock.body,
            producer: stock.feature,
        },
        AdvancedParameters::LinearPattern(Default::default()),
        vec![],
    );
    let prepared = prepare_advanced(&session, session.revision(), actor.clone(), &request).unwrap();
    assert_eq!(
        prepared.prepared.document(),
        prepare_advanced(&session, session.revision(), actor.clone(), &request)
            .unwrap()
            .prepared
            .document()
    );
    assert_eq!(session.document(), &before);
    session.commit(prepared.prepared).unwrap();
    let initial = session.document().clone();
    let mut edit = AdvancedRequest::existing(session.document(), request.feature).unwrap();
    let AdvancedParameters::LinearPattern(p) = &mut edit.parameters else {
        panic!()
    };
    p.count = Dimension::Literal {
        value: 3.,
        unit: "count".into(),
    };
    session
        .commit(
            prepare_advanced_edit(&session, session.revision(), actor.clone(), &edit)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let edited = session.document().clone();
    session.undo(session.revision(), actor.clone()).unwrap();
    assert_eq!(session.document(), &initial);
    session.redo(session.revision(), actor.clone()).unwrap();
    assert_eq!(session.document(), &edited);
    edit.body = EntityId::new();
    assert!(prepare_advanced_edit(&session, session.revision(), actor.clone(), &edit).is_err());
    assert!(prepare_advanced(&session, session.revision() + 1, actor, &request).is_err());
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(edited.clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(opened.project.document, edited);
    assert!(opened.project.capabilities().contains(ADVANCED_CAPABILITY));
}
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
fn shell_requires_an_opening_and_zero_draft_is_not_a_supported_operation() {
    let doc = Document::new("Explicit supported operations").unwrap();
    let parameters = parameters::evaluate(&doc).unwrap();
    assert!(
        AdvancedParameters::Draft(DraftParameters {
            angle: angle(0.),
            ..Default::default()
        })
        .resolve(&parameters)
        .is_err()
    );
    let request = AdvancedRequest::on_body(
        BodyInput {
            body: EntityId::new(),
            producer: EntityId::new(),
        },
        AdvancedParameters::Shell(Default::default()),
        vec![],
    );
    assert!(
        request
            .source()
            .unwrap_err()
            .to_string()
            .contains("opening face")
    );
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

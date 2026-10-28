//! Canonical transaction tests; native geometry acceptance lives in model tests.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{GeometryIntent, Origin, TopologyKind},
};
use std::collections::BTreeSet;
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn fixture() -> (Session, BodyInput, GeometryIntent) {
    let mut document = Document::new("Edge treatment source transactions").unwrap();
    document
        .insert(Entity::new(
            "radius",
            EntityData::Parameter {
                expression: "2".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        ))
        .unwrap();
    let mut session = Session::new(document).unwrap();
    let box_request = PrimitiveRequest::new(
        "Stock",
        PrimitiveParameters::Box {
            size: [length(30.), length(20.), length(10.)],
            frame: Default::default(),
        },
    );
    session
        .commit(
            prepare_primitive(&session, session.revision(), actor(), &box_request)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let edge = GeometryIntent {
        id: EntityId::new(),
        kind: TopologyKind::Edge,
        origins: BTreeSet::from([Origin {
            source: box_request.feature,
            role: "selected-source-edge".into(),
        }]),
        boundary: None,
        attachment: None,
    };
    (
        session,
        BodyInput {
            body: box_request.body,
            producer: box_request.feature,
        },
        edge,
    )
}
#[test]
fn edge_captures_have_one_canonical_home_and_switching_profile_preserves_history_bindings_and_undo()
{
    let (mut session, input, edge) = fixture();
    let original = session.document().clone();
    let parameters = EdgeTreatmentParameters {
        profile: EdgeTreatmentProfile::Fillet,
        size: Dimension::Expression {
            expression: BoundExpression::bind(session.document(), "=$radius", "mm", None).unwrap(),
        },
    };
    let request = EdgeTreatmentRequest::new(input.clone(), parameters.clone(), vec![edge.clone()]);
    let revision = session.revision();
    let preview = prepare_edge_treatment(&session, revision, actor(), &request).unwrap();
    assert_eq!(session.document(), &original);
    assert_eq!(
        preview.prepared.document(),
        prepare_edge_treatment(&session, revision, actor(), &request)
            .unwrap()
            .prepared
            .document()
    );
    let (source, captures) =
        read_edge_treatment(preview.prepared.document(), request.feature).unwrap();
    assert_eq!(source.edges, vec![edge.id]);
    assert_eq!(captures, vec![edge.clone()]);
    assert_eq!(source.input, input);
    assert_eq!(source.parameters, parameters);
    let feature = preview.prepared.document().get(request.feature).unwrap();
    let EntityData::Feature {
        definition, inputs, ..
    } = &feature.data
    else {
        panic!()
    };
    assert_eq!(inputs.iter().filter(|l| l.geometry.is_some()).count(), 1);
    assert!(
        !serde_json::to_string(&definition.data)
            .unwrap()
            .contains("origins")
    );
    session.commit(preview.prepared).unwrap();
    let applied = session.document().clone();
    let mut changed = parameters;
    changed.profile = EdgeTreatmentProfile::Chamfer;
    let edit = prepare_edge_treatment_edit(
        &session,
        session.revision(),
        actor(),
        request.feature,
        &changed,
        std::slice::from_ref(&edge),
    )
    .unwrap();
    assert_eq!(
        read_edge_treatment(edit.prepared.document(), request.feature)
            .unwrap()
            .0
            .parameters
            .profile,
        EdgeTreatmentProfile::Chamfer
    );
    session.commit(edit.prepared).unwrap();
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &applied);
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &original);
    session.redo(session.revision(), actor()).unwrap();
    session.redo(session.revision(), actor()).unwrap();
    let project = ProjectFile::new(session.document().clone()).unwrap();
    assert!(project.capabilities().contains(EDGE_TREATMENT_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let opened = project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(opened.project.document, *session.document());
    assert_eq!(
        read_edge_treatment(&opened.project.document, request.feature)
            .unwrap()
            .1,
        vec![edge]
    );
}
#[test]
fn invalid_factors_kinds_capture_ids_or_stale_producers_leave_source_unchanged() {
    let (session, input, edge) = fixture();
    let original = session.document().clone();
    for size in [0., -1., f64::NAN, f64::INFINITY] {
        let request = EdgeTreatmentRequest::new(
            input.clone(),
            EdgeTreatmentParameters {
                size: length(size),
                ..Default::default()
            },
            vec![edge.clone()],
        );
        assert!(prepare_edge_treatment(&session, session.revision(), actor(), &request).is_err());
    }
    for edges in [
        vec![],
        vec![edge.clone(), edge.clone()],
        vec![GeometryIntent {
            kind: TopologyKind::Face,
            ..edge.clone()
        }],
    ] {
        let request = EdgeTreatmentRequest::new(input.clone(), Default::default(), edges);
        assert!(prepare_edge_treatment(&session, session.revision(), actor(), &request).is_err());
    }
    let request = EdgeTreatmentRequest::new(input, Default::default(), vec![edge]);
    assert!(prepare_edge_treatment(&session, session.revision() + 1, actor(), &request).is_err());
    let candidate =
        prepare_edge_treatment(&session, session.revision(), actor(), &request).unwrap();
    let mut feature = candidate
        .prepared
        .document()
        .get(request.feature)
        .unwrap()
        .clone();
    let EntityData::Feature { inputs, .. } = &mut feature.data else {
        panic!()
    };
    inputs
        .iter_mut()
        .find(|link| link.geometry.is_some())
        .unwrap()
        .target = EntityId::new();
    let mut invalid = candidate.prepared.document().clone();
    assert!(invalid.replace(feature).is_err());
    assert_eq!(session.document(), &original);
}

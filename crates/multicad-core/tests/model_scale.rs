//! Canonical transaction intent. Native B-rep certification belongs to model tests.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::*,
    parameters::BoundExpression,
    references::{GeometryIntent, Origin, TopologyKind},
};
use std::collections::BTreeSet;
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn fixture() -> (
    Session,
    PrimitiveRequest,
    PrimitiveRequest,
    EntityId,
    EntityId,
) {
    let mut d = Document::new("Scale source transactions").unwrap();
    let factor = Entity::new(
        "factor",
        EntityData::Parameter {
            expression: "2".into(),
            unit: "ratio".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    d.insert(factor).unwrap();
    let mut s = Session::new(d).unwrap();
    let a = PrimitiveRequest::new(
        "A",
        PrimitiveParameters::Box {
            size: [length(10.), length(20.), length(30.)],
            frame: Default::default(),
        },
    );
    let b = PrimitiveRequest::new("B", a.parameters.clone());
    for p in [&a, &b] {
        s.commit(
            prepare_primitive(&s, s.revision(), actor(), p)
                .unwrap()
                .prepared,
        )
        .unwrap();
    }
    let first = InstanceCopyRequest::new(
        InstanceCopySource::Part {
            source: BodyInput {
                body: b.body,
                producer: b.feature,
            },
        },
        NumericTransform {
            translation: [length(80.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    let second = InstanceCopyRequest::new(
        first.source.clone(),
        NumericTransform {
            translation: [length(120.), length(0.), length(0.)],
            ..Default::default()
        },
    );
    s.commit(
        prepare_instance_copies(&s, s.revision(), actor(), &[first.clone(), second.clone()])
            .unwrap()
            .prepared,
    )
    .unwrap();
    (s, a, b, first.instance, second.instance)
}
fn controls(s: &Session) -> ScaleParameters {
    let expression = BoundExpression::bind(s.document(), "=$factor", "ratio", None).unwrap();
    ScaleParameters {
        factors: std::array::from_fn(|_| Dimension::Expression {
            expression: expression.clone(),
        }),
        pivot: [length(4.), length(5.), length(6.)],
        ..Default::default()
    }
}
#[test]
fn mixed_scale_is_one_source_transaction_and_isolates_only_the_selected_occurrence() {
    let (mut s, a, b, instance, other) = fixture();
    let original = s.document().clone();
    let revision = s.revision();
    let request = ScaleBatchRequest::new(
        s.document(),
        vec![
            ScaleTarget::Part {
                input: BodyInput {
                    body: a.body,
                    producer: a.feature,
                },
            },
            ScaleTarget::Instance { instance },
        ],
        controls(&s),
        false,
    )
    .unwrap();
    let result = prepare_scale_batch(&s, revision, actor(), &request).unwrap();
    let repeated = prepare_scale_batch(&s, revision, actor(), &request).unwrap();
    assert_eq!(result.prepared.document(), repeated.prepared.document());
    assert_eq!(s.document(), &original);
    assert_eq!(result.instances, vec![instance]);
    assert_eq!(result.outputs.len(), 1);
    assert_eq!(result.outputs[0].body, a.body);
    let Some(EntityData::Instance {
        component,
        placement,
        placement_source,
    }) = result.prepared.document().get(instance).map(|e| &e.data)
    else {
        panic!()
    };
    assert_ne!(*component, b.body);
    let branch = *component;
    assert_eq!(
        *placement,
        match original.get(instance).unwrap().data {
            EntityData::Instance { placement, .. } => placement,
            _ => panic!(),
        }
    );
    assert_eq!(
        *placement_source,
        match original.get(instance).unwrap().data {
            EntityData::Instance {
                placement_source, ..
            } => placement_source,
            _ => panic!(),
        }
    );
    assert_eq!(
        scale_component_owner(result.prepared.document(), branch).unwrap(),
        Some(instance)
    );
    assert_eq!(
        scale_component_root(result.prepared.document(), branch).unwrap(),
        b.body
    );
    let export =
        multicad_core::exchange::resolve_parts(result.prepared.document(), &Default::default())
            .unwrap();
    assert_eq!(export.len(), 4);
    assert!(
        export
            .iter()
            .any(|p| p.reference.part == branch && p.reference.occurrence_path == [instance])
    );
    assert!(
        !export
            .iter()
            .any(|p| p.reference.part == branch && p.reference.occurrence_path.is_empty())
    );
    assert!(
        multicad_core::exchange::resolve_parts(
            result.prepared.document(),
            &multicad_core::exchange::Scope {
                root: multicad_core::exchange::ScopeRoot::Explicit {
                    entities: vec![branch]
                },
                tags: None
            }
        )
        .is_err()
    );
    for id in [b.body, b.feature, other] {
        assert_eq!(result.prepared.document().get(id), original.get(id));
    }
    let roundtrip: Document =
        serde_json::from_value(serde_json::to_value(result.prepared.document()).unwrap()).unwrap();
    assert_eq!(&roundtrip, result.prepared.document());
    assert!(required_capabilities(&roundtrip).contains(SCALE_CAPABILITY));
    s.commit(result.prepared).unwrap();
    let after = s.document().clone();
    assert_eq!(s.revision(), revision + 1);
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &original);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &after);
}
#[test]
fn scaled_copy_clones_driver_and_geometry_reference_identities_once_and_preserves_alignment_ancestry()
 {
    let (mut s, _, b, instance, _) = fixture();
    let intent = GeometryIntent {
        id: EntityId::new(),
        kind: TopologyKind::Face,
        origins: BTreeSet::from([Origin {
            source: b.feature,
            role: "cap.start".into(),
        }]),
        boundary: None,
        attachment: None,
    };
    let alignment = InstanceAlignmentRequest::new(
        instance,
        AlignmentParameters {
            source: AlignmentAnchor::Geometry {
                reference: intent.id,
            },
            ..Default::default()
        },
        vec![AlignmentInput {
            source: b.body,
            intent: intent.clone(),
        }],
    );
    s.commit(
        prepare_instance_alignments(&s, s.revision(), actor(), std::slice::from_ref(&alignment))
            .unwrap()
            .prepared,
    )
    .unwrap();
    let original = s.document().clone();
    let request = ScaleBatchRequest::new(
        s.document(),
        vec![ScaleTarget::Instance { instance }],
        controls(&s),
        true,
    )
    .unwrap();
    let a = prepare_scale_batch(&s, s.revision(), actor(), &request).unwrap();
    let repeated = prepare_scale_batch(&s, s.revision(), actor(), &request).unwrap();
    assert_eq!(a.prepared.document(), repeated.prepared.document());
    assert_eq!(s.document(), &original);
    let copied = a.instances[0];
    assert_ne!(copied, instance);
    let Some(EntityData::Instance {
        component,
        placement_source: Some(driver),
        ..
    }) = a.prepared.document().get(copied).map(|e| &e.data)
    else {
        panic!()
    };
    let branch = *component;
    let driver = *driver;
    let (captured, refs) = read_instance_alignment(a.prepared.document(), driver).unwrap();
    assert_eq!(captured.owner, copied);
    assert_eq!(refs[0].source, b.body);
    assert_ne!(refs[0].intent.id, intent.id);
    assert_eq!(
        scale_component_root(a.prepared.document(), branch).unwrap(),
        b.body
    );
    assert_eq!(a.prepared.document().get(instance), original.get(instance));
    assert_eq!(a.prepared.document().get(b.body), original.get(b.body));
    s.commit(a.prepared).unwrap();
    let second = ScaleBatchRequest::new(
        s.document(),
        vec![ScaleTarget::Instance { instance: copied }],
        ScaleParameters::default(),
        false,
    )
    .unwrap();
    let stage = prepare_scale_batch(&s, s.revision(), actor(), &second).unwrap();
    let Some(EntityData::Instance { component, .. }) =
        stage.prepared.document().get(copied).map(|e| &e.data)
    else {
        panic!()
    };
    assert_eq!(
        scale_component_ancestry(stage.prepared.document(), *component).unwrap(),
        vec![*component, branch, b.body]
    );
    assert!(read_instance_alignment(stage.prepared.document(), driver).is_ok());
}
#[test]
fn invalid_stale_or_double_effect_requests_leave_source_unchanged_and_feature_edit_keeps_ids() {
    let (mut s, a, b, instance, _) = fixture();
    let original = s.document().clone();
    let mut invalid = ScaleBatchRequest::new(
        s.document(),
        vec![ScaleTarget::Instance { instance }],
        ScaleParameters::default(),
        false,
    )
    .unwrap();
    invalid.parameters.factors[0] = ratio(0.);
    assert!(prepare_scale_batch(&s, s.revision(), actor(), &invalid).is_err());
    assert!(prepare_scale_batch(&s, s.revision() + 1, actor(), &invalid).is_err());
    let double = ScaleBatchRequest::new(
        s.document(),
        vec![
            ScaleTarget::Part {
                input: BodyInput {
                    body: b.body,
                    producer: b.feature,
                },
            },
            ScaleTarget::Instance { instance },
        ],
        ScaleParameters::default(),
        false,
    )
    .unwrap();
    assert!(prepare_scale_batch(&s, s.revision(), actor(), &double).is_err());
    assert_eq!(s.document(), &original);
    let request = ScaleBatchRequest::new(
        s.document(),
        vec![ScaleTarget::Part {
            input: BodyInput {
                body: a.body,
                producer: a.feature,
            },
        }],
        ScaleParameters::default(),
        false,
    )
    .unwrap();
    let result = prepare_scale_batch(&s, s.revision(), actor(), &request).unwrap();
    let feature = result.outputs[0].feature;
    s.commit(result.prepared).unwrap();
    let parameters = controls(&s);
    let edited = prepare_scale_edit(&s, s.revision(), actor(), feature, &parameters).unwrap();
    assert_eq!(edited.outputs[0].feature, feature);
    assert_eq!(
        read_scale(edited.prepared.document(), feature)
            .unwrap()
            .parameters,
        parameters
    );
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Other source edit".into(),
        edits: vec![Mutation::RenameDocument("Changed source".into())],
    })
    .unwrap();
    assert!(s.commit(edited.prepared).is_err());
}

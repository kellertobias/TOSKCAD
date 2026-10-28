use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Request, Session},
    constraint_sketch::{Curve, CurveKind, Dimension, source::Source},
    document::{Definition, Document, Entity, EntityData, EntityId, Link, Mutation},
    hierarchy::{self, Action, ContainerKind},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{GeometryIntent, Origin, TopologyKind},
    sketch::Datum,
    sketch_regions,
};
use std::collections::BTreeSet;
fn actor() -> Actor {
    Actor {
        id: "test-model-client".into(),
        channel: Channel::Mcp,
    }
}
fn fixture() -> (Session, EntityId, ExtrudeRequest) {
    let (sketch, edits) = Source::create(Datum::Xy, "Reusable profile").unwrap();
    let mut doc = Document::new("Canonical model")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&doc, sketch).unwrap().0;
    source.add_rectangle([0., 0.], [80., 60.]).unwrap();
    let center = source.add_point([20., 20.]);
    source.sketch.curves.push(Curve {
        id: EntityId::new(),
        construction: false,
        kind: CurveKind::Circle {
            center,
            radius_mm: 6.,
        },
    });
    let regions = sketch_regions::extract(&source)
        .regions
        .iter()
        .map(|r| r.intent())
        .collect();
    doc = doc.edited(&source.replace(&doc, sketch).unwrap()).unwrap();
    (
        Session::new(doc).unwrap(),
        sketch,
        ExtrudeRequest::new(sketch, regions, Default::default()),
    )
}
fn apply(session: &mut Session, edits: Vec<Mutation>) {
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Test source edit".into(),
            edits,
        })
        .unwrap();
}
fn one_body() -> (Session, EntityId, ExtrudeRequest) {
    let (mut session, sketch, mut request) = fixture();
    request.outputs.truncate(1);
    let draft = prepare_extrude(&session, 0, actor(), &request).unwrap();
    session.commit(draft.prepared).unwrap();
    (session, sketch, request)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}
#[test]
fn two_parts_reuse_one_unchanged_sketch_with_stable_preview_ids_and_one_undo() {
    let (mut session, sketch, request) = fixture();
    let before = session.document().clone();
    let sketch_bytes = serde_json::to_vec(before.get(sketch).unwrap()).unwrap();
    assert_eq!(request.outputs.len(), 2);
    let first = prepare_extrude(&session, 0, actor(), &request).unwrap();
    let second = prepare_extrude(&session, 0, actor(), &request).unwrap();
    assert_eq!(first.outputs, second.outputs);
    assert_eq!(first.prepared.document(), second.prepared.document());
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), 0);
    assert!(!session.can_undo());
    let candidate = first.prepared.document();
    for output in &first.outputs {
        let source = read_extrude(candidate, output.feature).unwrap();
        assert_eq!(source.profile.sketch, sketch);
        assert_eq!(source.output.body, output.body);
        let EntityData::Part { features, .. } = &candidate.get(output.body).unwrap().data else {
            panic!("Part")
        };
        assert_eq!(features, &[output.feature]);
        assert!(
            candidate
                .get(output.feature)
                .unwrap()
                .references()
                .iter()
                .all(|link| link.target != output.body)
        );
        let order = candidate.topological_order().unwrap();
        assert!(
            order.iter().position(|id| *id == sketch)
                < order.iter().position(|id| *id == output.feature)
        );
        assert!(
            order.iter().position(|id| *id == output.feature)
                < order.iter().position(|id| *id == output.body)
        );
    }
    session.commit(first.prepared).unwrap();
    assert_eq!(session.revision(), 1);
    assert_eq!(session.activity().count(), 1);
    assert_eq!(session.activity().last().unwrap().actor, actor());
    assert_eq!(
        serde_json::to_vec(session.document().get(sketch).unwrap()).unwrap(),
        sketch_bytes
    );
    session.undo(1, actor()).unwrap();
    assert_eq!(session.document(), &before);
    session.redo(2, actor()).unwrap();
    for output in &request.outputs {
        assert!(session.document().get(output.feature).is_some());
        assert!(session.document().get(output.body).is_some());
    }
}
#[test]
fn new_bodies_and_producers_are_scoped_to_the_active_component() {
    let (mut session, sketch, request) = fixture();
    let component = hierarchy::prepare(
        &session,
        0,
        actor(),
        Action::Create {
            kind: ContainerKind::Component,
            name: "Cabinet".into(),
            parent: None,
        },
    )
    .unwrap();
    let component_id = component.created.unwrap();
    session.commit(component.prepared).unwrap();
    let active = hierarchy::prepare(
        &session,
        1,
        actor(),
        Action::SetActive {
            component: Some(component_id),
        },
    )
    .unwrap();
    session.commit(active.prepared).unwrap();
    let before = session.document().clone();
    let preview = prepare_extrude(&session, 2, actor(), &request).unwrap();
    let index = hierarchy::Index::new(preview.prepared.document()).unwrap();
    for output in &preview.outputs {
        assert_eq!(
            index.owning_component(output.body).unwrap(),
            Some(component_id)
        );
        assert_eq!(
            index.owning_component(output.feature).unwrap(),
            Some(component_id)
        );
    }
    assert_eq!(session.document(), &before);
    assert_eq!(before.get(sketch), preview.prepared.document().get(sketch));
}
#[test]
fn direction_uses_both_sides_of_normal_and_normal_offset_stays_independent() {
    let (session, _, _) = fixture();
    for elevation in [45., 90., 135.] {
        for reverse in [false, true] {
            let mut parameters = ExtrudeParameters {
                start_offset: length(7.),
                direction: Direction {
                    elevation: angle(elevation),
                    azimuth: angle(0.),
                    reverse,
                },
                ..Default::default()
            };
            let resolved = parameters.resolve(session.parameters()).unwrap();
            let sign = if reverse { -1. } else { 1. };
            near(resolved.direction[0], elevation.to_radians().cos() * sign);
            near(resolved.direction[1], 0.);
            near(resolved.direction[2], elevation.to_radians().sin() * sign);
            near(resolved.start_offset_mm, 7.);
            // Offset moves a datum origin along N, not along the tilted sweep.
            let origin = Datum::Xy.frame().point([0., 0.]);
            let offset_origin = [origin[0], origin[1], origin[2] + resolved.start_offset_mm];
            assert_eq!(offset_origin, [0., 0., 7.]);
            parameters.direction.azimuth = angle(90.);
            let rotated = parameters.resolve(session.parameters()).unwrap();
            near(rotated.direction[0], 0.);
            near(rotated.direction[1], resolved.direction[0]);
            near(rotated.direction[2], resolved.direction[2]);
        }
    }
}
#[test]
fn one_two_and_symmetric_total_distances_have_explicit_units_and_extent() {
    let (session, _, _) = fixture();
    let mut p = ExtrudeParameters {
        extent: Extent::TwoSided {
            forward: Termination::Distance {
                distance: Dimension::Literal {
                    value: 2.,
                    unit: "cm".into(),
                },
            },
            backward: Termination::Distance {
                distance: length(5.),
            },
        },
        ..Default::default()
    };
    let r = p.resolve(session.parameters()).unwrap();
    near(r.forward_mm, 20.);
    near(r.backward_mm, 5.);
    p.extent = Extent::Symmetric { total: length(30.) };
    let r = p.resolve(session.parameters()).unwrap();
    near(r.forward_mm, 15.);
    near(r.backward_mm, 15.);
    p.extent = Extent::OneSided {
        end: Termination::Distance {
            distance: length(9.),
        },
    };
    let r = p.resolve(session.parameters()).unwrap();
    near(r.forward_mm, 9.);
    near(r.backward_mm, 0.);
}
#[test]
fn stable_parameter_bindings_invalidate_producers_and_survive_rename_without_baking() {
    let (mut session, _, mut request) = fixture();
    request.outputs.truncate(1);
    let depth = Entity::new(
        "depth",
        EntityData::Parameter {
            expression: "25".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    apply(&mut session, vec![Mutation::Insert(depth.clone())]);
    let bound = BoundExpression::bind(session.document(), "=$depth", "mm", None).unwrap();
    request.parameters.extent = Extent::OneSided {
        end: Termination::Distance {
            distance: Dimension::Expression {
                expression: bound.clone(),
            },
        },
    };
    let draft = prepare_extrude(&session, 1, actor(), &request).unwrap();
    let output = draft.outputs[0].clone();
    session.commit(draft.prepared).unwrap();
    let source = read_extrude(session.document(), output.feature).unwrap();
    let mut renamed = depth.clone();
    renamed.name = "renamed_depth".into();
    apply(&mut session, vec![Mutation::Replace(renamed.clone())]);
    near(
        source
            .parameters
            .resolve(session.parameters())
            .unwrap()
            .forward_mm,
        25.,
    );
    let EntityData::Parameter { expression, .. } = &mut renamed.data else {
        panic!()
    };
    *expression = "40".into();
    let change = session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Change driving depth".into(),
            edits: vec![Mutation::Replace(renamed)],
        })
        .unwrap();
    assert!(change.invalidated.contains(&output.feature));
    assert!(change.invalidated.contains(&output.body));
    near(
        source
            .parameters
            .resolve(session.parameters())
            .unwrap()
            .forward_mm,
        40.,
    );
    assert_eq!(
        read_extrude(session.document(), output.feature).unwrap(),
        source
    );
    apply(&mut session, vec![Mutation::Remove(depth.id)]);
    assert!(source.parameters.resolve(session.parameters()).is_err());
    apply(
        &mut session,
        vec![Mutation::Insert(Entity::new(
            "depth",
            EntityData::Parameter {
                expression: "90".into(),
                unit: "mm".into(),
                exposed: false,
                inputs: vec![],
            },
        ))],
    );
    assert!(
        source.parameters.resolve(session.parameters()).is_err(),
        "Missing bound identity silently rebound by name"
    );
    let Extent::OneSided {
        end:
            Termination::Distance {
                distance: Dimension::Expression { expression },
            },
    } = &source.parameters.extent
    else {
        panic!()
    };
    assert_eq!(expression, &bound);
}
#[test]
fn editing_one_extrusion_preserves_body_history_and_is_cancelable_and_undoable() {
    let (mut session, sketch, request) = one_body();
    let output = &request.outputs[0];
    let before = session.document().clone();
    let mut p = request.parameters.clone();
    p.extent = Extent::Symmetric { total: length(22.) };
    p.direction.elevation = angle(135.);
    let preview = prepare_extrude_edit(&session, 1, actor(), output.feature, &p).unwrap();
    assert_eq!(session.document(), &before);
    assert_eq!(
        preview.prepared.document().get(output.body),
        before.get(output.body)
    );
    assert_eq!(preview.prepared.document().get(sketch), before.get(sketch));
    drop(preview);
    assert_eq!(session.document(), &before);
    let preview = prepare_extrude_edit(&session, 1, actor(), output.feature, &p).unwrap();
    session.commit(preview.prepared).unwrap();
    assert_eq!(
        read_extrude(session.document(), output.feature)
            .unwrap()
            .parameters,
        p
    );
    session.undo(2, actor()).unwrap();
    assert_eq!(session.document(), &before);
}
#[test]
fn foreign_stale_bad_actor_and_duplicate_ids_never_mutate_source() {
    let (mut session, _, request) = fixture();
    let before = session.document().clone();
    let preview = prepare_extrude(&session, 0, actor(), &request).unwrap();
    let foreign = Session::new(before.clone()).unwrap();
    assert_eq!(
        foreign
            .validate_preview(&preview.prepared)
            .unwrap_err()
            .code,
        ErrorCode::ForeignPreview
    );
    assert_eq!(
        prepare_extrude(&session, 1, actor(), &request)
            .unwrap_err()
            .code,
        ErrorCode::StaleRevision
    );
    assert!(
        prepare_extrude(
            &session,
            0,
            Actor {
                id: String::new(),
                channel: Channel::Ui
            },
            &request
        )
        .is_err()
    );
    let mut duplicate = request.clone();
    duplicate.outputs[1].body = duplicate.outputs[0].feature;
    assert!(prepare_extrude(&session, 0, actor(), &duplicate).is_err());
    session
        .apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Rename".into(),
            edits: vec![Mutation::RenameDocument("Changed".into())],
        })
        .unwrap();
    let changed = session.document().clone();
    assert!(session.commit(preview.prepared).is_err());
    assert_eq!(session.document(), &changed);
    assert_eq!(before.get(request.sketch), changed.get(request.sketch));
}
#[test]
fn unsupported_modes_and_invalid_numeric_inputs_are_explicit_errors_without_previews() {
    let (session, _, request) = fixture();
    let original = session.document().clone();
    let mut cases = vec![];
    for operation in [
        Operation::Join,
        Operation::Cut,
        Operation::Intersect,
        Operation::Split,
    ] {
        let mut r = request.clone();
        r.operation = operation;
        r.targets = vec![BodyInput {
            body: EntityId::new(),
            producer: EntityId::new(),
        }];
        cases.push(r);
    }
    for extent in [
        Extent::OneSided {
            end: Termination::ToPlane {
                plane: EntityId::new(),
                offset: length(0.),
            },
        },
        Extent::OneSided {
            end: Termination::ToFace {
                reference: EntityId::new(),
                offset: length(0.),
            },
        },
        Extent::OneSided {
            end: Termination::Distance {
                distance: length(0.),
            },
        },
        Extent::Symmetric { total: length(-1.) },
    ] {
        let mut r = request.clone();
        r.parameters.extent = extent;
        cases.push(r);
    }
    for elevation in [0., 180., -1., 181.] {
        let mut r = request.clone();
        r.parameters.direction.elevation = angle(elevation);
        cases.push(r);
    }
    let mut taper = request.clone();
    taper.parameters.taper = angle(1.);
    cases.push(taper);
    let mut wrong_unit = request.clone();
    wrong_unit.parameters.start_offset = angle(2.);
    cases.push(wrong_unit);
    let mut infinite = request.clone();
    infinite.parameters.start_offset = length(f64::INFINITY);
    cases.push(infinite);
    let mut keep = request.clone();
    keep.keep_tools = true;
    cases.push(keep);
    for bad in cases {
        assert!(prepare_extrude(&session, 0, actor(), &bad).is_err());
        assert_eq!(session.document(), &original);
    }
}
#[test]
fn native_file_capability_and_typed_request_roundtrip_preserve_canonical_definitions() {
    let (session, _, request) = one_body();
    let wire = serde_json::to_vec(&request).unwrap();
    assert_eq!(
        serde_json::from_slice::<ExtrudeRequest>(&wire).unwrap(),
        request
    );
    let mut unknown = serde_json::to_value(&request).unwrap();
    unknown["ignored_option"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ExtrudeRequest>(unknown).is_err());
    let project = ProjectFile::new(session.document().clone()).unwrap();
    assert!(project.capabilities().contains(CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let opened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(opened.project.document, project.document);
    let mut unsupported = project_file::supported_capabilities();
    unsupported.remove(CAPABILITY);
    assert!(project_file::read(&mut &bytes[..], Default::default(), &unsupported).is_err());
}
#[test]
fn definition_face_references_follow_the_repaired_link_without_duplicated_source() {
    let (session, sketch, request) = fixture();
    let output = &request.outputs[0];
    let producer = Entity::new(
        "First producer",
        EntityData::Feature {
            definition: Definition {
                type_id: "test.solid".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs: vec![],
            suppressed: false,
        },
    );
    let second = Entity::new("Replacement producer", producer.data.clone());
    let intent = GeometryIntent {
        id: EntityId::new(),
        kind: TopologyKind::Face,
        origins: BTreeSet::from([Origin {
            source: producer.id,
            role: "cap.end".into(),
        }]),
        boundary: None,
        attachment: None,
    };
    let source = ExtrudeDefinition {
        profile: ProfileSource {
            sketch,
            region: output.region.clone(),
        },
        output: BodyOutput { body: output.body },
        parameters: ExtrudeParameters {
            extent: Extent::OneSided {
                end: Termination::ToFace {
                    reference: intent.id,
                    offset: length(2.),
                },
            },
            ..Default::default()
        },
        operation: Operation::NewBody,
        targets: vec![],
        keep_tools: false,
    };
    let mut feature = Entity::new(
        "Associative future source",
        EntityData::Feature {
            definition: source.definition().unwrap(),
            inputs: source
                .inputs(&[FaceInput {
                    source: producer.id,
                    intent: intent.clone(),
                }])
                .unwrap(),
            suppressed: false,
        },
    );
    feature.id = output.feature;
    let mut body = Entity::new(
        "Body",
        EntityData::Part {
            features: vec![feature.id],
            material: None,
            tags: BTreeSet::new(),
        },
    );
    body.id = output.body;
    let doc = session
        .document()
        .edited(&[
            Mutation::Insert(producer),
            Mutation::Insert(second.clone()),
            Mutation::Insert(feature.clone()),
            Mutation::Insert(body),
        ])
        .unwrap();
    let EntityData::Feature { inputs, .. } = &mut feature.data else {
        panic!()
    };
    let face = inputs
        .iter_mut()
        .find(|link| link.geometry.is_some())
        .unwrap();
    face.target = second.id;
    face.geometry.as_mut().unwrap().origins = BTreeSet::from([Origin {
        source: second.id,
        role: "cap.end".into(),
    }]);
    let repaired = doc.edited(&[Mutation::Replace(feature)]).unwrap();
    assert_eq!(read_extrude(&repaired, output.feature).unwrap(), source);
    assert!(
        source.parameters.resolve(session.parameters()).is_err(),
        "Unsupported face termination was silently treated as distance"
    );
}
#[test]
fn structural_input_tampering_and_final_body_dependency_cycles_are_rejected() {
    let (session, _, request) = one_body();
    let output = &request.outputs[0];
    let mut feature = session.document().get(output.feature).unwrap().clone();
    let EntityData::Feature { inputs, .. } = &mut feature.data else {
        panic!()
    };
    inputs.clear();
    assert!(
        session
            .document()
            .edited(&[Mutation::Replace(feature)])
            .is_err()
    );
    let mut cyclic = session.document().get(output.feature).unwrap().clone();
    let EntityData::Feature { inputs, .. } = &mut cyclic.data else {
        panic!()
    };
    inputs.push(Link::new(output.body, "incorrect_final_body"));
    assert!(
        session
            .document()
            .edited(&[Mutation::Replace(cyclic)])
            .is_err()
    );
    let mut definition = read_extrude(session.document(), output.feature)
        .unwrap()
        .definition()
        .unwrap();
    definition.schema_version += 1;
    assert!(ExtrudeDefinition::from_definition(&definition).is_err());
}
#[test]
fn body_input_resolves_a_real_upstream_producer_without_a_final_part_dependency() {
    let (session, _, request) = one_body();
    let output = &request.outputs[0];
    let input = BodyInput {
        body: output.body,
        producer: output.feature,
    };
    assert_eq!(
        input.resolve_producer(session.document()).unwrap(),
        output.feature
    );
    assert!(
        BodyInput {
            body: output.body,
            producer: output.body
        }
        .resolve_producer(session.document())
        .is_err()
    );
    assert!(
        BodyInput {
            body: EntityId::new(),
            producer: output.feature
        }
        .resolve_producer(session.document())
        .is_err()
    );
    let mut definition = read_extrude(session.document(), output.feature).unwrap();
    definition.operation = Operation::Cut;
    definition.targets = vec![input];
    definition.keep_tools = true;
    let links = definition.inputs(&[]).unwrap();
    assert!(links.iter().any(|l| l.target == output.feature));
    assert!(links.iter().all(|l| l.target != output.body));
    assert_eq!(
        ExtrudeDefinition::from_definition(&definition.definition().unwrap()).unwrap(),
        definition
    );
}

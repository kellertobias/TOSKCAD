use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Request, Session},
    constraint_sketch::{Dimension, source::Source},
    document::{Document, Entity, EntityData, EntityId, Link, Mutation},
    hierarchy::{self, Action, ContainerKind},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    sketch::Datum,
    sketch_regions,
};
fn actor() -> Actor {
    Actor {
        id: "typed-model-test".into(),
        channel: Channel::Mcp,
    }
}
fn session() -> Session {
    Session::new(Document::new("Typed solids").unwrap()).unwrap()
}
fn box_params() -> PrimitiveParameters {
    PrimitiveParameters::Box {
        size: [length(10.), length(20.), length(30.)],
        frame: Default::default(),
    }
}
fn apply(s: &mut Session, edits: Vec<Mutation>) {
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Test edit".into(),
        edits,
    })
    .unwrap();
}
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
fn bound(s: &Session, text: &str, unit: &str) -> Dimension {
    Dimension::Expression {
        expression: BoundExpression::bind(s.document(), text, unit, None).unwrap(),
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}
fn revolve_fixture() -> (Session, EntityId, RevolveRequest) {
    let (id, edits) = Source::create(Datum::Xz, "Sections").unwrap();
    let mut doc = Document::new("Revolve source")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&doc, id).unwrap().0;
    source.add_rectangle([2., 0.], [5., 6.]).unwrap();
    source.add_rectangle([8., 0.], [10., 4.]).unwrap();
    doc = doc.edited(&source.replace(&doc, id).unwrap()).unwrap();
    let regions = sketch_regions::extract(&source)
        .regions
        .into_iter()
        .map(|r| r.intent())
        .collect();
    (
        Session::new(doc).unwrap(),
        id,
        RevolveRequest::new(id, regions, Default::default()),
    )
}
#[test]
fn primitive_previews_share_ids_are_source_neutral_scoped_and_one_undo() {
    let mut s = session();
    let component = hierarchy::prepare(
        &s,
        0,
        actor(),
        Action::Create {
            kind: ContainerKind::Component,
            name: "Fixture".into(),
            parent: None,
        },
    )
    .unwrap();
    let component_id = component.created.unwrap();
    s.commit(component.prepared).unwrap();
    let active = hierarchy::prepare(
        &s,
        1,
        actor(),
        Action::SetActive {
            component: Some(component_id),
        },
    )
    .unwrap();
    s.commit(active.prepared).unwrap();
    let before = s.document().clone();
    let request = PrimitiveRequest::new("Block", box_params());
    let first = prepare_primitive(&s, 2, actor(), &request).unwrap();
    let second = prepare_primitive(&s, 2, actor(), &request).unwrap();
    assert_eq!(first.prepared.document(), second.prepared.document());
    assert_eq!(s.document(), &before);
    let candidate = first.prepared.document();
    let index = hierarchy::Index::new(candidate).unwrap();
    for id in [request.body, request.feature] {
        assert_eq!(index.owning_component(id).unwrap(), Some(component_id));
    }
    assert_eq!(
        read_primitive(candidate, request.feature)
            .unwrap()
            .parameters,
        request.parameters
    );
    s.commit(first.prepared).unwrap();
    let committed = s.document().clone();
    s.undo(3, actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(4, actor()).unwrap();
    assert_eq!(s.document(), &committed);
    let file = ProjectFile::new(committed.clone()).unwrap();
    assert!(file.capabilities().contains(PRIMITIVE_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    assert_eq!(
        project_file::read(
            &mut &bytes[..],
            Default::default(),
            &project_file::supported_capabilities()
        )
        .unwrap()
        .project
        .document,
        committed
    );
}
#[test]
fn primitive_bindings_survive_parameter_rename_value_edit_and_reopen_without_baking() {
    let mut s = session();
    let width = parameter("width", "2", "cm");
    apply(&mut s, vec![Mutation::Insert(width.clone())]);
    let expr = bound(&s, "=$width * 2", "mm");
    let mut frame = PrimitiveFrame::default();
    frame.origin[2] = bound(&s, "=$width", "cm");
    frame.u = [0., 1., 0.];
    frame.v = [0., 0., 1.];
    let params = PrimitiveParameters::Box {
        size: [expr, length(4.), length(8.)],
        frame,
    };
    let request = PrimitiveRequest::new("Bound block", params);
    let draft = prepare_primitive(&s, 1, actor(), &request).unwrap();
    s.commit(draft.prepared).unwrap();
    let source = read_primitive(s.document(), request.feature).unwrap();
    let source_bytes = serde_json::to_vec(s.document().get(request.feature).unwrap()).unwrap();
    let mut changed = width;
    changed.name = "new_width".into();
    let EntityData::Parameter { expression, .. } = &mut changed.data else {
        panic!()
    };
    *expression = "3".into();
    let change = s
        .apply(Request {
            expected_revision: 2,
            actor: actor(),
            label: "Change width".into(),
            edits: vec![Mutation::Replace(changed)],
        })
        .unwrap();
    assert!(change.invalidated.contains(&request.feature));
    assert!(change.invalidated.contains(&request.body));
    let ResolvedPrimitive::Box { size_mm, frame } =
        source.parameters.resolve(s.parameters()).unwrap()
    else {
        panic!()
    };
    assert_eq!(size_mm, [60., 4., 8.]);
    assert_eq!(frame.origin, [0., 0., 30.]);
    assert_eq!(frame.n, [1., 0., 0.]);
    assert_eq!(
        serde_json::to_vec(s.document().get(request.feature).unwrap()).unwrap(),
        source_bytes
    );
    let reopened = reopened(s.document());
    let reopened = Session::new(reopened).unwrap();
    assert_eq!(
        read_primitive(reopened.document(), request.feature).unwrap(),
        source
    );
    assert_eq!(
        source.parameters.resolve(reopened.parameters()).unwrap(),
        ResolvedPrimitive::Box { size_mm, frame }
    );
}
#[test]
fn primitive_edit_can_switch_shape_preserving_part_and_replacing_dependency_links() {
    let mut s = session();
    let radius = parameter("radius", "5", "mm");
    apply(&mut s, vec![Mutation::Insert(radius.clone())]);
    let request = PrimitiveRequest::new("Original", box_params());
    let draft = prepare_primitive(&s, 1, actor(), &request).unwrap();
    s.commit(draft.prepared).unwrap();
    let before = s.document().clone();
    let p = PrimitiveParameters::Cylinder {
        radius: bound(&s, "=$radius", "mm"),
        height: Dimension::Literal {
            value: 2.,
            unit: "cm".into(),
        },
        frame: Default::default(),
    };
    let draft = prepare_primitive_edit(&s, 2, actor(), request.feature, &p).unwrap();
    assert_eq!(s.document(), &before);
    assert_eq!(
        draft.prepared.document().get(request.body),
        before.get(request.body)
    );
    s.commit(draft.prepared).unwrap();
    let source = read_primitive(s.document(), request.feature).unwrap();
    assert_eq!(source.parameters, p);
    let ResolvedPrimitive::Cylinder {
        radius_mm,
        height_mm,
        ..
    } = source.parameters.resolve(s.parameters()).unwrap()
    else {
        panic!()
    };
    near(radius_mm, 5.);
    near(height_mm, 20.);
    let draft = prepare_primitive_edit(&s, 3, actor(), request.feature, &box_params()).unwrap();
    s.commit(draft.prepared).unwrap();
    let EntityData::Feature { inputs, .. } = &s.document().get(request.feature).unwrap().data
    else {
        panic!()
    };
    assert!(inputs.is_empty());
    s.undo(4, actor()).unwrap();
    assert_eq!(
        read_primitive(s.document(), request.feature).unwrap(),
        source
    );
}
#[test]
fn new_revolved_parts_reuse_one_sketch_and_keep_distinct_stable_output_ids() {
    let (mut s, sketch, request) = revolve_fixture();
    let before = s.document().clone();
    let original = serde_json::to_vec(before.get(sketch).unwrap()).unwrap();
    assert_eq!(request.outputs.len(), 2);
    let first = prepare_revolve(&s, 0, actor(), &request).unwrap();
    let second = prepare_revolve(&s, 0, actor(), &request).unwrap();
    assert_eq!(first.outputs, second.outputs);
    assert_eq!(first.prepared.document(), second.prepared.document());
    assert_eq!(s.document(), &before);
    for output in &request.outputs {
        let source = read_revolve(first.prepared.document(), output.feature).unwrap();
        assert_eq!(source.profile.sketch, sketch);
        assert_eq!(source.output.body, output.body);
        assert_eq!(source.profile.region, output.region);
    }
    s.commit(first.prepared).unwrap();
    assert_eq!(
        serde_json::to_vec(s.document().get(sketch).unwrap()).unwrap(),
        original
    );
    let file = ProjectFile::new(s.document().clone()).unwrap();
    assert!(file.capabilities().contains(REVOLVE_CAPABILITY));
    assert_eq!(reopened(&file.document), *s.document());
    s.undo(1, actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(2, actor()).unwrap();
    assert!(read_revolve(s.document(), request.outputs[1].feature).is_ok());
}
#[test]
fn revolution_axis_and_signed_sweep_support_stable_unit_checked_expressions_and_edit() {
    let (mut s, sketch, mut request) = revolve_fixture();
    request.outputs.truncate(1);
    let sweep = parameter("sweep", "-90", "deg");
    let offset = parameter("offset", "2", "cm");
    apply(
        &mut s,
        vec![Mutation::Insert(sweep.clone()), Mutation::Insert(offset)],
    );
    request.parameters = RevolveParameters {
        axis_origin: [bound(&s, "=$offset", "mm"), length(3.)],
        axis_angle: angle(0.),
        angle: bound(&s, "=$sweep", "deg"),
    };
    let draft = prepare_revolve(&s, 1, actor(), &request).unwrap();
    s.commit(draft.prepared).unwrap();
    let output = &request.outputs[0];
    let before = s.document().clone();
    let mut parameters = request.parameters.clone();
    parameters.axis_angle = Dimension::Literal {
        value: std::f64::consts::PI / 2.,
        unit: "rad".into(),
    };
    let draft = prepare_revolve_edit(&s, 2, actor(), output.feature, &parameters).unwrap();
    assert_eq!(s.document(), &before);
    assert_eq!(draft.prepared.document().get(sketch), before.get(sketch));
    s.commit(draft.prepared).unwrap();
    let source = read_revolve(s.document(), output.feature).unwrap();
    let source_bytes = serde_json::to_vec(&source).unwrap();
    let mut renamed = sweep;
    renamed.name = "sweep_renamed".into();
    let EntityData::Parameter { expression, .. } = &mut renamed.data else {
        panic!()
    };
    *expression = "180".into();
    apply(&mut s, vec![Mutation::Replace(renamed)]);
    let r = source.parameters.resolve(s.parameters()).unwrap();
    assert_eq!(r.axis_origin_mm, [20., 3.]);
    near(r.axis_direction[0], 0.);
    near(r.axis_direction[1], 1.);
    near(r.angle_rad, std::f64::consts::PI);
    assert_eq!(
        serde_json::to_vec(&read_revolve(s.document(), output.feature).unwrap()).unwrap(),
        source_bytes
    );
    s.undo(4, actor()).unwrap();
    near(
        source.parameters.resolve(s.parameters()).unwrap().angle_rad,
        -std::f64::consts::PI / 2.,
    );
}
#[test]
fn invalid_requests_stale_foreign_and_tampered_links_never_mutate_source() {
    let mut s = session();
    let request = PrimitiveRequest::new("Box", box_params());
    let before = s.document().clone();
    let draft = prepare_primitive(&s, 0, actor(), &request).unwrap();
    let foreign = session();
    assert_eq!(
        foreign.validate_preview(&draft.prepared).unwrap_err().code,
        ErrorCode::ForeignPreview
    );
    assert_eq!(
        prepare_primitive(&s, 1, actor(), &request)
            .unwrap_err()
            .code,
        ErrorCode::StaleRevision
    );
    let mut duplicate = request.clone();
    duplicate.body = duplicate.feature;
    assert!(prepare_primitive(&s, 0, actor(), &duplicate).is_err());
    let mut bad = request.clone();
    bad.parameters = PrimitiveParameters::Cylinder {
        radius: length(0.),
        height: length(10.),
        frame: Default::default(),
    };
    assert!(prepare_primitive(&s, 0, actor(), &bad).is_err());
    bad.parameters = PrimitiveParameters::Box {
        size: [angle(10.), length(2.), length(3.)],
        frame: Default::default(),
    };
    assert!(prepare_primitive(&s, 0, actor(), &bad).is_err());
    let badframe = PrimitiveFrame {
        u: [1., 1., 0.],
        ..Default::default()
    };
    bad.parameters = PrimitiveParameters::Box {
        size: [length(1.), length(2.), length(3.)],
        frame: badframe,
    };
    assert!(prepare_primitive(&s, 0, actor(), &bad).is_err());
    assert!(
        prepare_primitive(
            &s,
            0,
            Actor {
                id: "".into(),
                channel: Channel::Ui
            },
            &request
        )
        .is_err()
    );
    assert_eq!(s.document(), &before);
    s.commit(draft.prepared).unwrap();
    let mut feature = s.document().get(request.feature).unwrap().clone();
    let EntityData::Feature { inputs, .. } = &mut feature.data else {
        panic!()
    };
    inputs.push(Link::new(EntityId::new(), "parameter/fake"));
    assert!(s.document().edited(&[Mutation::Replace(feature)]).is_err());
    assert!(prepare_primitive(&s, 0, actor(), &request).is_err());
}
#[test]
fn revolution_rejects_bad_profiles_units_angles_and_ids_without_claiming_native_closure() {
    let (s, sketch, request) = revolve_fixture();
    let before = s.document().clone();
    for angle_value in [0., 361., -361., f64::NAN] {
        let mut bad = request.clone();
        bad.parameters.angle = angle(angle_value);
        assert!(prepare_revolve(&s, 0, actor(), &bad).is_err());
    }
    let mut bad = request.clone();
    bad.parameters.axis_origin[0] = angle(1.);
    assert!(prepare_revolve(&s, 0, actor(), &bad).is_err());
    bad = request.clone();
    bad.outputs[1].region = bad.outputs[0].region.clone();
    assert!(prepare_revolve(&s, 0, actor(), &bad).is_err());
    bad = request.clone();
    bad.outputs[1].feature = bad.outputs[0].feature;
    assert!(prepare_revolve(&s, 0, actor(), &bad).is_err());
    bad = request.clone();
    bad.outputs[0].region.outer.curves.pop_first();
    bad.outputs[0].region.outer.curves.insert(EntityId::new());
    assert!(prepare_revolve(&s, 0, actor(), &bad).is_err());
    let mut source = Source::read(s.document(), sketch).unwrap().0;
    source.sketch.curves[0].construction = true;
    let edited = s
        .document()
        .edited(&source.replace(s.document(), sketch).unwrap())
        .unwrap();
    let construction = Session::new(edited).unwrap();
    assert!(prepare_revolve(&construction, 0, actor(), &request).is_err());
    assert_eq!(s.document(), &before);
    // Structural preparations preserve authored intent; they do not run a solver
    // or certify closure. The native compiler rejects this single-edge boundary.
    let mut open = request.clone();
    open.outputs.truncate(1);
    open.outputs[0].region.outer.curves =
        std::collections::BTreeSet::from([*open.outputs[0].region.outer.curves.first().unwrap()]);
    assert!(prepare_revolve(&s, 0, actor(), &open).is_ok());
}

fn reopened(document: &Document) -> Document {
    let file = ProjectFile::new(document.clone()).unwrap();
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project
    .document
}
#[test]
fn missing_bound_identity_is_preserved_for_repair_and_never_rebound_by_alias() {
    let mut s = session();
    let p = parameter("height", "12", "mm");
    apply(&mut s, vec![Mutation::Insert(p.clone())]);
    let parameters = PrimitiveParameters::Cylinder {
        radius: length(3.),
        height: bound(&s, "=$height", "mm"),
        frame: Default::default(),
    };
    let request = PrimitiveRequest::new("Cylinder", parameters);
    let draft = prepare_primitive(&s, 1, actor(), &request).unwrap();
    s.commit(draft.prepared).unwrap();
    let source = read_primitive(s.document(), request.feature).unwrap();
    apply(&mut s, vec![Mutation::Remove(p.id)]);
    apply(
        &mut s,
        vec![Mutation::Insert(parameter("height", "99", "mm"))],
    );
    assert!(source.parameters.resolve(s.parameters()).is_err());
    assert_eq!(
        read_primitive(s.document(), request.feature).unwrap(),
        source
    );
    assert!(prepare_primitive_edit(&s, 4, actor(), request.feature, &source.parameters).is_err());
}
#[test]
fn provider_capabilities_gate_native_open_and_unknown_versions_or_extra_intent_reject() {
    let (mut s, _, mut request) = revolve_fixture();
    request.outputs.truncate(1);
    let draft = prepare_revolve(&s, 0, actor(), &request).unwrap();
    s.commit(draft.prepared).unwrap();
    let primitive = PrimitiveRequest::new("Block", box_params());
    let draft = prepare_primitive(&s, 1, actor(), &primitive).unwrap();
    s.commit(draft.prepared).unwrap();
    let file = ProjectFile::new(s.document().clone()).unwrap();
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    for capability in [PRIMITIVE_CAPABILITY, REVOLVE_CAPABILITY] {
        let mut supported = project_file::supported_capabilities();
        supported.remove(capability);
        assert!(project_file::read(&mut &bytes[..], Default::default(), &supported).is_err());
    }
    let mut p = read_primitive(s.document(), primitive.feature)
        .unwrap()
        .definition()
        .unwrap();
    p.schema_version = 99;
    assert!(PrimitiveDefinition::from_definition(&p).is_err());
    p.schema_version = 1;
    p.data["operation"] = serde_json::json!("cut");
    assert!(PrimitiveDefinition::from_definition(&p).is_err());
    let feature = request.outputs[0].feature;
    let mut r = read_revolve(s.document(), feature)
        .unwrap()
        .definition()
        .unwrap();
    r.schema_version = 2;
    assert!(RevolveDefinition::from_definition(&r).is_err());
    r.schema_version = 1;
    r.data["taper"] = serde_json::json!(10.);
    assert!(RevolveDefinition::from_definition(&r).is_err());
}

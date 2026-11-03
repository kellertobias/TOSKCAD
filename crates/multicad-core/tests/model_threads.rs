use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, Mutation},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Canonical threads").unwrap()).unwrap()
}
fn apply(session: &mut Session, edits: Vec<Mutation>) {
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Test edit".into(),
            edits,
        })
        .unwrap();
}
fn roundtrip(document: &Document) -> Document {
    let project = ProjectFile::new(document.clone()).unwrap();
    assert!(project.capabilities().contains(THREAD_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
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
fn sourced_metric_presets_and_basic_dimensions_do_not_imply_fit_classes_or_universal_coverage() {
    let s = session();
    let presets = metric_thread_presets();
    for diameter in [2., 2.5, 3., 3.5, 4., 4.5, 5., 6., 7., 8., 9., 10.] {
        assert!(presets.iter().any(|p| p.diameter_mm == diameter));
    }
    for preset in presets {
        let p = ThreadParameters {
            major_diameter: length(preset.diameter_mm),
            pitch: length(preset.pitch_mm),
            ..Default::default()
        };
        let resolved = p.resolve(s.parameters(), ThreadMode::NewExternal).unwrap();
        assert_eq!(resolved.pitch_mm, preset.pitch_mm);
        assert_eq!(resolved.lead_mm, preset.pitch_mm);
    }
    let m6 = ThreadParameters {
        major_diameter: length(6.),
        pitch: length(1.),
        ..Default::default()
    }
    .resolve(s.parameters(), ThreadMode::NewExternal)
    .unwrap();
    assert!((m6.nominal_pitch_diameter_mm.unwrap() - 5.350480947).abs() < 1e-9);
    assert!((2. * (m6.major_radius_mm - m6.profile.depth_mm) - 4.917468245).abs() < 1e-9);
    assert!(ThreadFamily::IsoMetric.source().contains("2001"));
    assert_eq!(
        ThreadFamily::ALL
            .into_iter()
            .filter(|family| family.supported())
            .collect::<Vec<_>>(),
        [
            ThreadFamily::IsoMetric,
            ThreadFamily::Unc,
            ThreadFamily::Unf,
            ThreadFamily::Bsw,
            ThreadFamily::Bsf,
            ThreadFamily::Bspp,
            ThreadFamily::Bspt,
            ThreadFamily::Npt,
            ThreadFamily::Trapezoidal,
            ThreadFamily::Acme,
            ThreadFamily::Custom
        ]
    );
    let unsupported = ThreadParameters {
        major_diameter: length(3.5),
        pitch: length(0.5),
        ..Default::default()
    };
    assert!(
        unsupported
            .resolve(s.parameters(), ThreadMode::NewExternal)
            .is_err()
    );
}
#[test]
fn manufacturing_allowances_are_independent_of_nominal_dimensions_and_representation() {
    let s = session();
    let p = ThreadParameters {
        radial_allowance: length(0.1),
        axial_allowance: length(0.02),
        ..Default::default()
    };
    let male = p.resolve(s.parameters(), ThreadMode::NewExternal).unwrap();
    let female = p.resolve(s.parameters(), ThreadMode::CutInternal).unwrap();
    assert_eq!(male.nominal_major_radius_mm, 4.);
    assert_eq!(female.nominal_major_radius_mm, 4.);
    assert_eq!(
        male.nominal_pitch_diameter_mm,
        female.nominal_pitch_diameter_mm
    );
    assert_eq!(male.major_radius_mm, 3.9);
    assert_eq!(female.major_radius_mm, 4.1);
    assert!((male.profile.crest_width_mm - (1.25 / 8. - 0.02)).abs() < 1e-12);
    assert!((female.profile.crest_width_mm - (1.25 / 8. + 0.02)).abs() < 1e-12);
    let cosmetic = ThreadParameters {
        representation: ThreadRepresentation::CosmeticEnvelope,
        ..p
    };
    let value = serde_json::to_value(&cosmetic).unwrap();
    assert_eq!(value["representation"], "cosmetic_envelope");
    let mut bad = value.clone();
    bad["fit"] = "iso_6g".into();
    assert!(serde_json::from_value::<ThreadParameters>(bad).is_err());
    let mut bad = value;
    bad["runout"] = "automatic_relief".into();
    assert!(serde_json::from_value::<ThreadParameters>(bad).is_err());
}
#[test]
fn preparation_is_source_neutral_and_keeps_bindings_history_identity_and_project_persistence() {
    let mut s = session();
    let parameter = Entity::new(
        "thread_length",
        EntityData::Parameter {
            expression: "10".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let parameter_id = parameter.id;
    apply(&mut s, vec![Mutation::Insert(parameter)]);
    let length = Dimension::Expression {
        expression: BoundExpression::bind(s.document(), "=$thread_length", "mm", None).unwrap(),
    };
    let request = ThreadRequest::new(
        "Threaded shaft",
        ThreadParameters {
            length,
            ..Default::default()
        },
    );
    let before = s.document().clone();
    let candidate = prepare_thread(&s, 1, actor(), &request).unwrap();
    assert_eq!(s.document(), &before);
    let source = read_thread(candidate.prepared.document(), request.feature).unwrap();
    assert!(
        source
            .inputs()
            .unwrap()
            .iter()
            .any(|link| link.target == parameter_id)
    );
    s.commit(candidate.prepared).unwrap();
    let committed = s.document().clone();
    s.undo(2, actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(3, actor()).unwrap();
    assert_eq!(s.document(), &committed);
    let mut changed = s.document().get(parameter_id).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut changed.data else {
        unreachable!()
    };
    *expression = "15".into();
    apply(&mut s, vec![Mutation::Replace(changed)]);
    let source = read_thread(s.document(), request.feature).unwrap();
    assert_eq!(
        source
            .parameters
            .resolve(s.parameters(), source.mode)
            .unwrap()
            .length_mm,
        15.
    );
    assert!(matches!(
        source.parameters.length,
        Dimension::Expression { .. }
    ));
    let reopened = roundtrip(s.document());
    assert_eq!(&reopened, s.document());
    assert_eq!(
        read_thread(&reopened, request.feature).unwrap().output.body,
        request.body
    );
}
#[test]
fn receiving_threads_append_to_body_history_and_historical_edits_keep_the_original_predecessor() {
    let mut s = session();
    let stock = PrimitiveRequest::new(
        "Receiver",
        PrimitiveParameters::Box {
            size: [length(20.), length(20.), length(20.)],
            frame: Default::default(),
        },
    );
    s.commit(prepare_primitive(&s, 0, actor(), &stock).unwrap().prepared)
        .unwrap();
    let params = |x| ThreadParameters {
        frame: PrimitiveFrame {
            origin: [length(x), length(x), length(0.)],
            ..Default::default()
        },
        ..Default::default()
    };
    let first = ThreadRequest::receiving(
        "First hole",
        ThreadMode::CutInternal,
        BodyInput {
            body: stock.body,
            producer: stock.feature,
        },
        params(6.),
    )
    .unwrap();
    s.commit(prepare_thread(&s, 1, actor(), &first).unwrap().prepared)
        .unwrap();
    let second = ThreadRequest::receiving(
        "Second hole",
        ThreadMode::CutInternal,
        BodyInput {
            body: stock.body,
            producer: first.feature,
        },
        params(14.),
    )
    .unwrap();
    s.commit(prepare_thread(&s, 2, actor(), &second).unwrap().prepared)
        .unwrap();
    let edited = ThreadParameters {
        length: length(10.),
        ..first.parameters.clone()
    };
    s.commit(
        prepare_thread_edit(&s, 3, actor(), first.feature, &edited)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let first_source = read_thread(s.document(), first.feature).unwrap();
    assert_eq!(first_source.target.unwrap().producer, stock.feature);
    assert_eq!(
        read_thread(s.document(), second.feature)
            .unwrap()
            .target
            .unwrap()
            .producer,
        first.feature
    );
    let EntityData::Part { features, .. } = &s.document().get(stock.body).unwrap().data else {
        unreachable!()
    };
    assert_eq!(features, &[stock.feature, first.feature, second.feature]);
    let saved = s.document().clone();
    s.undo(4, actor()).unwrap();
    s.redo(5, actor()).unwrap();
    assert_eq!(s.document(), &saved);
    assert_eq!(roundtrip(s.document()), saved);
}
#[test]
fn unsupported_invalid_stale_and_malformed_thread_sources_never_partially_commit() {
    let mut s = session();
    let before = s.document().clone();
    for parameters in [
        ThreadParameters {
            family: ThreadFamily::Npt,
            ..Default::default()
        },
        ThreadParameters {
            starts: 0,
            ..Default::default()
        },
        ThreadParameters {
            length: length(1000.),
            ..Default::default()
        },
        ThreadParameters {
            radial_allowance: length(4.),
            ..Default::default()
        },
        ThreadParameters {
            axial_allowance: length(1.),
            ..Default::default()
        },
        ThreadParameters {
            family: ThreadFamily::Custom,
            custom: None,
            ..Default::default()
        },
    ] {
        assert!(
            prepare_thread(&s, 0, actor(), &ThreadRequest::new("Invalid", parameters)).is_err()
        );
        assert_eq!(s.document(), &before);
    }
    let request = ThreadRequest::new("Valid", Default::default());
    let pending = prepare_thread(&s, 0, actor(), &request).unwrap();
    s.commit(pending.prepared).unwrap();
    assert_eq!(
        prepare_thread(&s, 0, actor(), &request).err().unwrap().code,
        ErrorCode::StaleRevision
    );
    let mut feature = s.document().get(request.feature).unwrap().clone();
    let EntityData::Feature { definition, .. } = &mut feature.data else {
        unreachable!()
    };
    definition.data["parameters"]["unexpected"] = "opaque".into();
    let before = s.document().clone();
    assert!(
        s.apply(Request {
            expected_revision: s.revision(),
            actor: actor(),
            label: "Malformed source".into(),
            edits: vec![Mutation::Replace(feature)]
        })
        .is_err()
    );
    assert_eq!(s.document(), &before);
}

use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, Mutation},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    symbols::{self, SymbolAuthorRequest, SymbolKind},
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Metric engagement").unwrap()).unwrap()
}
fn recipe(d: f64, p: f64) -> ThreadParameters {
    ThreadParameters {
        major_diameter: length(d),
        pitch: length(p),
        length: length(8. * p),
        fit: ThreadFit::Metric6gExternal,
        ..Default::default()
    }
}
fn resolve(
    s: &Session,
    p: &ThreadParameters,
) -> Result<ResolvedThread, multicad_core::document::DocumentError> {
    p.resolve(s.parameters(), ThreadMode::NewExternal)
}
#[test]
fn physical_span_and_fit_intent_are_independent_with_unchanged_limits() {
    let s = session();
    for preset in metric_fit_presets() {
        let mut p = recipe(preset.diameter_mm, preset.pitch_mm);
        p.length = length(16. * preset.pitch_mm);
        assert!(resolve(&s, &p).is_err());
        assert!(
            serde_json::to_value(&p)
                .unwrap()
                .get("metric_fit_engagement")
                .is_none()
        );
        let mut previous = None;
        for e in [preset.engagement_over_mm + 1e-6, preset.engagement_up_to_mm] {
            p.metric_fit_engagement =
                Some(MetricFitEngagement::IntendedLength { length: length(e) });
            let r = resolve(&s, &p).unwrap();
            assert_eq!(r.length_mm, 16. * preset.pitch_mm);
            assert_eq!(
                r.metric_fit_engagement.as_ref().unwrap().intended_length_mm,
                Some(e)
            );
            if let Some(prior) = previous {
                assert_eq!(r.profile, prior);
            }
            previous = Some(r.profile);
        }
        for e in [
            0.,
            -1.,
            preset.engagement_over_mm,
            preset.engagement_up_to_mm + 1e-6,
            f64::INFINITY,
            f64::NAN,
        ] {
            p.metric_fit_engagement =
                Some(MetricFitEngagement::IntendedLength { length: length(e) });
            assert!(resolve(&s, &p).is_err(), "accepted {e}");
        }
        p.metric_fit_engagement = Some(MetricFitEngagement::NormalGroupAssumed {});
        let r = resolve(&s, &p).unwrap();
        let receipt = r.metric_fit_engagement.unwrap();
        assert_eq!(receipt.basis, MetricFitEngagementBasis::NormalGroupAssumed);
        assert_eq!(receipt.intended_length_mm, None);
        p.radial_allowance = length(1.);
        assert!(resolve(&s, &p).is_err());
        p.radial_allowance = length(0.);
        p.length = length(preset.engagement_over_mm);
        assert!(resolve(&s, &p).is_err());
        p.length = length(preset.engagement_over_mm + 1e-6);
        assert!(resolve(&s, &p).is_ok());
        p.metric_fit_engagement = None;
        p.length = length(preset.diameter_mm);
        let r = resolve(&s, &p).unwrap();
        assert_eq!(
            r.metric_fit_engagement.unwrap().basis,
            MetricFitEngagementBasis::LegacyCompleteProfile
        );
    }
}
#[test]
fn intention_must_fit_complete_profile_and_cannot_bypass_family_or_units() {
    let s = session();
    let mut p = recipe(3., 0.5);
    p.length = length(8.);
    p.metric_fit_engagement = Some(MetricFitEngagement::IntendedLength { length: length(3.) });
    p.end_treatments = Some(ThreadEndTreatments {
        start: ThreadStartTreatment::FlatCut,
        end: ThreadEndTreatment::CylindricalRelief {
            length: length(6.),
            radial_depth: length(0.1),
        },
    });
    assert!(resolve(&s, &p).is_err());
    p.metric_fit_engagement = Some(MetricFitEngagement::NormalGroupAssumed {});
    assert!(resolve(&s, &p).is_ok());
    if let Some(t) = &mut p.end_treatments {
        t.end = ThreadEndTreatment::CylindricalRelief {
            length: length(6.5),
            radial_depth: length(0.1),
        };
    }
    assert!(resolve(&s, &p).is_err());
    p.end_treatments = None;
    p.metric_fit_engagement = Some(MetricFitEngagement::IntendedLength { length: angle(3.) });
    assert!(resolve(&s, &p).is_err());
    p.metric_fit_engagement = Some(MetricFitEngagement::NormalGroupAssumed {});
    p.fit = ThreadFit::NominalBasic;
    assert!(p.validate().is_err());
    p.family = ThreadFamily::Custom;
    p.custom = Some(Default::default());
    assert!(p.validate().is_err());
    assert!(
        serde_json::from_value::<MetricFitEngagement>(
            serde_json::json!({"kind":"normal_group_assumed","length":{"value":3,"unit":"mm"}})
        )
        .is_err()
    );
}
fn reader(s: &Session) {
    let project = ProjectFile::new(s.document().clone()).unwrap();
    assert!(
        project
            .capabilities()
            .contains(METRIC_FIT_ENGAGEMENT_CAPABILITY)
    );
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let mut supported = project_file::supported_capabilities();
    supported.remove(METRIC_FIT_ENGAGEMENT_CAPABILITY);
    assert!(
        project_file::read(&mut bytes.as_slice(), Default::default(), &supported)
            .unwrap_err()
            .contains(METRIC_FIT_ENGAGEMENT_CAPABILITY)
    );
    assert_eq!(
        project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities()
        )
        .unwrap()
        .project
        .document,
        *s.document()
    );
}
#[test]
fn bound_intent_survives_history_suppression_and_template_only_readers() {
    let mut s = session();
    let entity = Entity::new(
        "engagement",
        EntityData::Parameter {
            expression: "3".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let parameter = entity.id;
    s.apply(Request {
        expected_revision: 0,
        actor: actor(),
        label: "Parameter".into(),
        edits: vec![Mutation::Insert(entity)],
    })
    .unwrap();
    let mut p = recipe(3., 0.5);
    p.length = length(8.);
    p.metric_fit_engagement = Some(MetricFitEngagement::IntendedLength {
        length: Dimension::Expression {
            expression: BoundExpression::bind(s.document(), "=$engagement", "mm", None).unwrap(),
        },
    });
    let request = ThreadRequest::new("Long screw", p);
    let before = s.document().clone();
    let q = prepare_thread(&s, s.revision(), actor(), &request).unwrap();
    assert_eq!(*s.document(), before);
    s.commit(q.prepared).unwrap();
    let source = read_thread(s.document(), request.feature).unwrap();
    assert!(
        source
            .inputs()
            .unwrap()
            .iter()
            .any(|l| l.target == parameter)
    );
    reader(&s);
    let authored = s.document().clone();
    s.undo(s.revision(), actor()).unwrap();
    assert!(!required_capabilities(s.document()).contains(METRIC_FIT_ENGAGEMENT_CAPABILITY));
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(*s.document(), authored);
    let mut feature = s.document().get(request.feature).unwrap().clone();
    if let EntityData::Feature { suppressed, .. } = &mut feature.data {
        *suppressed = true;
    }
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Suppress".into(),
        edits: vec![Mutation::Replace(feature)],
    })
    .unwrap();
    reader(&s);
    s.undo(s.revision(), actor()).unwrap();
    let author = SymbolAuthorRequest::new(
        "Engagement template",
        request.body,
        SymbolKind::Dynamic,
        vec![parameter],
        None,
    );
    let q = symbols::prepare_author(&s, s.revision(), actor(), &author).unwrap();
    s.commit(q.prepared).unwrap();
    let definition = symbols::read_definition(s.document(), author.definition).unwrap();
    let mut other = session();
    let q = symbols::prepare_import(&other, 0, actor(), &definition).unwrap();
    let imported = q.definition;
    other.commit(q.prepared).unwrap();
    reader(&other);
    let mut instances = Vec::new();
    for intended in [2., 4.] {
        let place = symbols::SymbolPlaceRequest::new(
            other.document(),
            imported,
            "Configured screw",
            std::collections::BTreeMap::from([(
                parameter,
                symbols::SymbolValue {
                    value: intended,
                    unit: "mm".into(),
                },
            )]),
            Default::default(),
        )
        .unwrap();
        let q = symbols::prepare_place(&other, other.revision(), actor(), &place).unwrap();
        other.commit(q.prepared).unwrap();
        let source = read_thread(other.document(), place.ids[&request.feature]).unwrap();
        assert!(
            source
                .inputs()
                .unwrap()
                .iter()
                .any(|l| l.target == place.ids[&parameter])
        );
        assert!(
            !source
                .inputs()
                .unwrap()
                .iter()
                .any(|l| l.target == parameter)
        );
        assert_eq!(
            source
                .parameters
                .resolve(other.parameters(), ThreadMode::NewExternal)
                .unwrap()
                .metric_fit_engagement
                .unwrap()
                .intended_length_mm,
            Some(intended)
        );
        instances.push(place);
    }
    let before = other.document().clone();
    let invalid = symbols::SymbolEditRequest {
        record: instances[0].instance,
        overrides: std::collections::BTreeMap::from([(
            parameter,
            symbols::SymbolValue {
                value: 5.,
                unit: "mm".into(),
            },
        )]),
    };
    // Core symbol preparation is structural; the model/native gate rejects the
    // evaluated candidate before the application commits it.
    let candidate = symbols::prepare_edit(&other, other.revision(), actor(), &invalid).unwrap();
    let candidate_session = Session::new(candidate.prepared.document().clone()).unwrap();
    let source = read_thread(
        candidate_session.document(),
        instances[0].ids[&request.feature],
    )
    .unwrap();
    assert!(
        source
            .parameters
            .resolve(candidate_session.parameters(), ThreadMode::NewExternal)
            .is_err()
    );
    assert_eq!(*other.document(), before);
    let edited = symbols::SymbolEditRequest {
        record: instances[0].instance,
        overrides: std::collections::BTreeMap::from([(
            parameter,
            symbols::SymbolValue {
                value: 3.5,
                unit: "mm".into(),
            },
        )]),
    };
    let q = symbols::prepare_edit(&other, other.revision(), actor(), &edited).unwrap();
    other.commit(q.prepared).unwrap();
    assert_eq!(
        symbols::read_instance(other.document(), instances[0].instance)
            .unwrap()
            .ids,
        instances[0].ids
    );
    for (place, expected) in instances.iter().zip([3.5, 4.]) {
        let source = read_thread(other.document(), place.ids[&request.feature]).unwrap();
        assert_eq!(
            source
                .parameters
                .resolve(other.parameters(), ThreadMode::NewExternal)
                .unwrap()
                .metric_fit_engagement
                .unwrap()
                .intended_length_mm,
            Some(expected)
        );
    }
    reader(&other);
}
#[test]
fn nested_and_retained_reference_intent_gate_readers_and_attribution() {
    for retained_only in [false, true] {
        let mut s = session();
        let mut fitted = recipe(3., 0.5);
        fitted.fit = ThreadFit::Metric6HInternal;
        fitted.metric_fit_engagement = Some(MetricFitEngagement::NormalGroupAssumed {});
        let reference = MechanicalPartParameters::HexNut {
            across_flats: length(6.),
            thread: Box::new(fitted.clone()),
        };
        let actual = if retained_only {
            fitted.metric_fit_engagement = None;
            MechanicalPartParameters::HexNut {
                across_flats: length(6.),
                thread: Box::new(fitted),
            }
        } else {
            reference.clone()
        };
        let mut request = MechanicalPartRequest::new("Nut", actual);
        request.provenance = Some(MechanicalPartProvenance {
            catalog_id: "engagement-test".into(),
            revision: "1".into(),
            designation: "M3 context".into(),
            source_url: METRIC_ROOT_SOURCE_URL.into(),
            reference_geometry: reference,
            catalog_facts: None,
        });
        let q = prepare_mechanical_part(&s, 0, actor(), &request).unwrap();
        s.commit(q.prepared).unwrap();
        reader(&s);
        assert_eq!(
            read_mechanical_part(s.document(), request.feature)
                .unwrap()
                .matches_reference_dimensions(s.parameters())
                .unwrap(),
            !retained_only
        );
    }
}

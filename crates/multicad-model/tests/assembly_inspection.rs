use multicad_core::{
    assembly_inspection::*,
    commands::{Actor, Channel, Session},
    document::Document,
    model::*,
};
use multicad_model::{Compiler, assembly_inspection::*};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn commit(s: &mut Session, p: AssemblyPrepared) {
    s.commit(p.prepared).unwrap();
}
struct Fixture {
    session: Session,
    joint: JointRequest,
    query: PairQuery,
}
fn fixture(kind: JointKind) -> Fixture {
    let mut s = Session::new(Document::new("Read-only assembly checks").unwrap()).unwrap();
    let mut parts = vec![];
    let mut members = vec![];
    for name in ["Fixed block", "Moving block"] {
        let p = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: [length(10.), length(10.), length(10.)],
                frame: Default::default(),
            },
        );
        s.commit(
            prepare_primitive(&s, s.revision(), actor(), &p)
                .unwrap()
                .prepared,
        )
        .unwrap();
        let member = ComponentPlacementRequest::new(p.body, Default::default());
        let prepared = prepare_place_component(&s, s.revision(), actor(), &member).unwrap();
        commit(&mut s, prepared);
        parts.push(p);
        members.push(member);
    }
    let joint = JointRequest::new(
        members[1].instance,
        JointParameters {
            kind,
            parent: Some(members[0].instance),
            limits: JointLimits {
                translation: [
                    None,
                    None,
                    kind.allows(JointCoordinate::Z).then(|| JointLimit {
                        min: length(0.),
                        max: length(12.),
                    }),
                ],
                rotation: kind.allows(JointCoordinate::Rotation).then(|| JointLimit {
                    min: angle(0.),
                    max: angle(90.),
                }),
            },
            ..Default::default()
        },
        vec![],
    );
    let prepared = prepare_joint(&s, s.revision(), actor(), &joint).unwrap();
    commit(&mut s, prepared);
    let group = AssemblyRequest::new(
        "Two blocks",
        members.iter().map(|m| m.instance).collect(),
        parts.iter().map(|p| p.body).collect(),
    );
    let prepared = prepare_assembly(&s, s.revision(), actor(), &group).unwrap();
    commit(&mut s, prepared);
    let root = ComponentPlacementRequest::new(group.assembly, Default::default());
    let prepared = prepare_place_component(&s, s.revision(), actor(), &root).unwrap();
    commit(&mut s, prepared);
    let query = PairQuery {
        expected: InspectionStamp::capture(s.document(), s.revision()).unwrap(),
        pairs: vec![OccurrencePair {
            a: OccurrencePath {
                root: root.instance,
                members: vec![members[0].instance],
            },
            b: OccurrencePath {
                root: root.instance,
                members: vec![members[1].instance],
            },
        }],
        contact_tolerance_mm: 1e-7,
    };
    Fixture {
        session: s,
        joint,
        query,
    }
}
fn range(
    f: &Fixture,
    coordinate: JointCoordinate,
    start: f64,
    end: f64,
    step: f64,
) -> MotionRangeQuery {
    MotionRangeQuery {
        expected: f.query.expected,
        pairs: f.query.pairs.clone(),
        contact_tolerance_mm: f.query.contact_tolerance_mm,
        joint: f.joint.driver,
        coordinate,
        start,
        end,
        step,
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
#[test]
fn exact_current_pairs_are_source_bound_and_nonphysical_definitions_fail_explicitly() {
    let f = fixture(JointKind::Slider);
    let before = f.session.document().clone();
    let compiler = Compiler::default();
    let model = compiler
        .compile(f.session.document(), f.session.revision())
        .unwrap();
    let report = inspect_assembly(&model, &f.query).unwrap();
    report.validate_against(&f.query).unwrap();
    let values = report.pairs[0].result.as_ref().unwrap();
    near(values.overlap_volume_mm3, 1000.);
    near(values.minimum_clearance_mm, 0.);
    let mut stale = f.query.clone();
    stale.expected.revision += 1;
    assert_eq!(
        inspect_assembly(&model, &stale).unwrap_err().kind,
        InspectionFailureKind::Stale
    );
    let mut missing = f.query.clone();
    missing.pairs[0].a = OccurrencePath {
        root: f.joint.owner,
        members: vec![],
    };
    let report = inspect_assembly(&model, &missing).unwrap();
    assert_eq!(
        report.pairs[0].result.as_ref().unwrap_err().kind,
        InspectionFailureKind::Missing
    );
    let scoped = compiler
        .compile_sources(f.session.document(), f.session.revision(), &[f.joint.owner])
        .unwrap();
    assert_eq!(
        inspect_assembly(&scoped, &f.query).unwrap_err().kind,
        InspectionFailureKind::Unsupported
    );
    assert_eq!(f.session.document(), &before);
}
#[test]
fn sampled_drawer_checks_exact_intervals_limits_and_real_overlap_without_source_commit() {
    let f = fixture(JointKind::Slider);
    let before = f.session.document().clone();
    let revision = f.session.revision();
    let query = range(&f, JointCoordinate::Z, 0., 15., 6.);
    let report =
        sample_assembly_range(&Compiler::default(), f.session.document(), &query, None).unwrap();
    report.validate_against(&query).unwrap();
    assert_eq!(report.sample_values, vec![0., 6., 12., 15.]);
    assert_eq!(report.actual_intervals, vec![6., 6., 3.]);
    assert_eq!(report.coverage, RangeCoverage::SamplesOnly);
    for (i, volume, clearance) in [(0, 1000., 0.), (1, 400., 0.), (2, 0., 2.)] {
        let value = report.samples[i].result.as_ref().unwrap().pairs[0]
            .result
            .as_ref()
            .unwrap();
        near(value.overlap_volume_mm3, volume);
        near(value.minimum_clearance_mm, clearance);
    }
    assert!(report.samples[3].candidate_stamp.is_none());
    assert_eq!(
        report.samples[3].result.as_ref().unwrap_err().kind,
        InspectionFailureKind::Invalid
    );
    assert_ne!(
        report.samples[1].candidate_stamp.unwrap().source_hash,
        query.expected.source_hash
    );
    assert_eq!(f.session.document(), &before);
    assert_eq!(f.session.revision(), revision);
    assert_eq!(
        sample_assembly_range(
            &Compiler::default(),
            f.session.document(),
            &query,
            Some(&|| true)
        )
        .unwrap_err()
        .kind,
        InspectionFailureKind::Cancelled
    );
}
#[test]
fn hinge_range_uses_radians_and_preserves_requested_endpoint() {
    let f = fixture(JointKind::Revolute);
    let query = range(
        &f,
        JointCoordinate::Rotation,
        0.,
        std::f64::consts::FRAC_PI_2,
        1.,
    );
    let report =
        sample_assembly_range(&Compiler::default(), f.session.document(), &query, None).unwrap();
    report.validate_against(&query).unwrap();
    assert_eq!(report.unit, SampleUnit::Radians);
    let last = report
        .samples
        .last()
        .unwrap()
        .result
        .as_ref()
        .unwrap()
        .pairs[0]
        .result
        .as_ref()
        .unwrap();
    near(last.overlap_volume_mm3, 0.);
    assert_eq!(last.relation, PairRelation::ContactWithinTolerance);
    let middle = report.samples[1].result.as_ref().unwrap().pairs[0]
        .result
        .as_ref()
        .unwrap();
    assert!(middle.overlap_volume_mm3 > 0. && middle.overlap_volume_mm3 < 900.);
    let wrong = range(&f, JointCoordinate::Z, 0., 1., 1.);
    assert_eq!(
        sample_assembly_range(&Compiler::default(), f.session.document(), &wrong, None)
            .unwrap_err()
            .kind,
        InspectionFailureKind::Unsupported
    );
}
#[test]
fn real_worker_returns_complete_current_and_range_queries() {
    let f = fixture(JointKind::Slider);
    let mut worker = AssemblyInspectionWorker::new(Compiler::default()).unwrap();
    for query in [
        AssemblyInspectionQuery::Current(f.query.clone()),
        AssemblyInspectionQuery::Range(range(&f, JointCoordinate::Z, 0., 12., 6.)),
    ] {
        let id = worker.submit(f.session.document(), query.clone()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let reply = loop {
            if let Some(reply) = worker.poll() {
                break reply;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(reply.request, id);
        assert_eq!(reply.query, query);
        reply.result.unwrap().validate_against(&query).unwrap();
        assert!(worker.poll().is_none());
    }
}

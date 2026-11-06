use multicad_core::{commands::Session, document::Document, model::*, references::ReferenceFrame};
fn frame(z: f64) -> ReferenceFrame {
    ReferenceFrame {
        origin_mm: [0., 0., z],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
    }
}
fn pair(hand: ThreadHand, nut_length: f64) -> (ResolvedThread, ResolvedThread) {
    let s = Session::new(Document::new("Mating intervals").unwrap()).unwrap();
    let p = ThreadParameters {
        major_diameter: length(3.),
        pitch: length(0.5),
        length: length(8.),
        hand,
        fit: ThreadFit::Metric6gExternal,
        metric_fit_engagement: Some(MetricFitEngagement::NormalGroupAssumed {}),
        ..Default::default()
    };
    let a = p.resolve(s.parameters(), ThreadMode::NewExternal).unwrap();
    let b = ThreadParameters {
        length: length(nut_length),
        fit: ThreadFit::Metric6HInternal,
        ..p
    }
    .resolve(s.parameters(), ThreadMode::CutInternal)
    .unwrap();
    (a, b)
}
#[test]
fn actual_interval_and_phase_include_station_hand_and_opposed_axis() {
    for (hand, roll) in [
        (ThreadHand::Right, std::f64::consts::FRAC_PI_2),
        (ThreadHand::Left, -std::f64::consts::FRAC_PI_2),
    ] {
        let (a, b) = pair(hand, 3.);
        let result =
            resolve_metric_thread_mating(&a, frame(0.), &b, frame(5.125), 2., 1e-7).unwrap();
        assert!((result.overlap_length_mm - 2.875).abs() < 1e-12);
        assert_eq!(result.overlap_start_mm, 5.125);
        assert_eq!(result.overlap_end_mm, 8.);
        assert!((result.phase.roll_rad - roll).abs() < 1e-10);
        let moved = result.phase.correction.frame(frame(5.125));
        assert_eq!(moved.origin_mm, frame(5.125).origin_mm);
        let mut opposed = frame(8.);
        opposed.v = [0., -1., 0.];
        let result = resolve_metric_thread_mating(&a, frame(0.), &b, opposed, 3., 1e-7).unwrap();
        assert_eq!(result.overlap_start_mm, 5.);
        assert_eq!(result.overlap_end_mm, 8.);
        assert_eq!(result.overlap_length_mm, 3.);
        assert_eq!(
            result.phase.correction.frame(opposed).normal(),
            [0., 0., -1.]
        );
    }
}
#[test]
fn missing_short_long_and_false_declared_engagement_are_refused() {
    let (a, b) = pair(ThreadHand::Right, 3.);
    for station in [8.1, 7., 6.5] {
        assert!(
            resolve_metric_thread_mating(&a, frame(0.), &b, frame(station), 0.1, 1e-7).is_err()
        );
    }
    assert!(resolve_metric_thread_mating(&a, frame(0.), &b, frame(6.5 - 1e-6), 0.1, 1e-7).is_ok());
    assert!(resolve_metric_thread_mating(&a, frame(0.), &b, frame(5.), 3.000001, 1e-7).is_err());
    for min in [0., -1., f64::NAN, f64::INFINITY] {
        assert!(resolve_metric_thread_mating(&a, frame(0.), &b, frame(5.), min, 1e-7).is_err());
    }
    let (_, long) = pair(ThreadHand::Right, 8.);
    assert!(resolve_metric_thread_mating(&a, frame(0.), &long, frame(0.), 2., 1e-7).is_err());
    assert!(resolve_metric_thread_mating(&a, frame(0.), &long, frame(3.5), 2., 1e-7).is_ok());
    assert!(
        resolve_metric_thread_mating(&a, frame(0.), &long, frame(3.5 - 1e-6), 2., 1e-7).is_err()
    );
    let mut declared = a.clone();
    let intent = declared.metric_fit_engagement.as_mut().unwrap();
    intent.basis = MetricFitEngagementBasis::IntendedLength;
    intent.intended_length_mm = Some(4.);
    assert!(resolve_metric_thread_mating(&declared, frame(0.), &b, frame(5.), 2., 1e-7).is_err());
    declared
        .metric_fit_engagement
        .as_mut()
        .unwrap()
        .intended_length_mm = Some(3.);
    assert!(resolve_metric_thread_mating(&declared, frame(0.), &b, frame(5.), 2., 1e-7).is_ok());
}
#[test]
fn axes_profiles_and_endpoint_treatments_are_not_replaced_by_fit_intent() {
    let (a, b) = pair(ThreadHand::Right, 3.);
    let mut off = frame(5.);
    off.origin_mm[0] = 0.001;
    assert!(resolve_metric_thread_mating(&a, frame(0.), &b, off, 2., 1e-7).is_err());
    let tilt = 1e-5_f64;
    let mut off = frame(5.);
    off.u = [tilt.cos(), 0., -tilt.sin()];
    assert!(resolve_metric_thread_mating(&a, frame(0.), &b, off, 2., 1e-7).is_err());
    let mut wrong = b.clone();
    wrong.hand = ThreadHand::Left;
    assert!(resolve_metric_thread_mating(&a, frame(0.), &wrong, frame(5.), 2., 1e-7).is_err());
    let mut wrong = b.clone();
    wrong.representation = ThreadRepresentation::CosmeticEnvelope;
    assert!(resolve_metric_thread_mating(&a, frame(0.), &wrong, frame(5.), 2., 1e-7).is_err());
    let mut wrong = b.clone();
    wrong.pitch_mm = 0.6;
    assert!(resolve_metric_thread_mating(&a, frame(0.), &wrong, frame(5.), 2., 1e-7).is_err());
    let s = Session::new(Document::new("Relief").unwrap()).unwrap();
    let treated = ThreadParameters {
        major_diameter: length(3.),
        pitch: length(0.5),
        length: length(8.),
        fit: ThreadFit::Metric6gExternal,
        metric_fit_engagement: Some(MetricFitEngagement::NormalGroupAssumed {}),
        end_treatments: Some(ThreadEndTreatments {
            start: ThreadStartTreatment::FlatCut,
            end: ThreadEndTreatment::CylindricalRelief {
                length: length(6.),
                radial_depth: length(0.1),
            },
        }),
        ..Default::default()
    }
    .resolve(s.parameters(), ThreadMode::NewExternal)
    .unwrap();
    assert!(resolve_metric_thread_mating(&treated, frame(0.), &b, frame(2.), 0.1, 1e-7).is_err());
    let result =
        resolve_metric_thread_mating(&treated, frame(0.), &b, frame(0.), 2., 1e-7).unwrap();
    assert_eq!(result.overlap_length_mm, 2.);
}
#[test]
fn partially_threaded_screw_uses_real_neck_and_occurrence_frame() {
    let s = Session::new(Document::new("Partial screw datum").unwrap()).unwrap();
    let mut p = default_machine_screw();
    if let MechanicalPartParameters::MachineScrew {
        underhead_length,
        thread,
        ..
    } = &mut p
    {
        *underhead_length = Some(length(20.));
        thread.length = length(12.5);
    }
    let r = p.resolve(s.parameters()).unwrap();
    let world = machine_screw_thread_start(
        &r,
        RigidPose {
            translation_mm: [10., 20., 30.],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(world.origin_mm, [10., 20., 37.5]);
}

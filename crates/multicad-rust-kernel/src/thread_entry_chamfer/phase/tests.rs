use super::*;
use crate::thread_entry_chamfer::tests::tool;
#[test]
fn monotone_pieces_keep_actual_circle_intersections_and_dense_phase_coverage() {
    for rounded in [false, true] {
        let source = tool(false, 1, rounded);
        for factor in [0.1, 0.9, 1., 1.2] {
            let body = Chamfer::new(source, source.spec().depth_mm * factor, None).unwrap();
            let parts = body.phase_segments().unwrap();
            assert!(parts.len() <= 10);
            let mut end = 0.;
            for piece in &parts {
                assert_eq!(piece.ends[0].nominal, end);
                assert!(piece.ends[1].nominal > end);
                end = piece.ends[1].nominal;
                for point in piece.ends {
                    assert!(point.phase.lo <= point.nominal && point.phase.hi >= point.nominal);
                    if point.threshold {
                        let value = body.profile(piece.band, point.phase).unwrap().radius;
                        assert!(
                            value.lo <= body.entry_radius_mm()
                                && value.hi >= body.entry_radius_mm()
                        );
                    }
                }
                let mid = piece.ends[0].nominal.midpoint(piece.ends[1].nominal);
                assert_eq!(
                    source.radius_at_phase(mid) > body.entry_radius_mm(),
                    piece.active
                );
            }
            assert_eq!(end, source.spec().pitch_mm);
        }
    }
}
#[test]
fn exact_root_contact_and_neighboring_representable_entries_have_honest_topology() {
    let mut spec = tool(false, 1, true).spec();
    let root = 0.125;
    spec.root_radius_mm = Some(root);
    spec.depth_mm = (spec.pitch_mm - spec.crest_width_mm) / (2. * 30_f64.to_radians().tan()) - root;
    let source = Thread::new(spec).unwrap();
    let circle = source.root_circle().unwrap();
    let minimum = circle.center_radius - circle.radius;
    assert_eq!(
        sum3(circle.center_radius, -circle.radius, -minimum)
            .unwrap()
            .0,
        Ordering::Equal
    );
    for r0 in [minimum.next_down(), minimum, minimum.next_up()] {
        let body = Chamfer::from_material(source, r0, spec.major_radius_mm - r0, None).unwrap();
        let parts = body.phase_segments().unwrap();
        let inactive = parts.iter().any(|p| p.band == 2 && !p.active);
        assert_eq!(inactive, r0 > minimum);
        if r0 == minimum {
            assert!(parts.iter().any(|p| {
                p.ends
                    .iter()
                    .any(|k| k.threshold && k.nominal == circle.phase)
            }));
        }
    }
}
#[test]
fn retained_sum_sign_survives_large_cancellation_and_invalid_arithmetic_refuses() {
    let (sign, bound) = sum3(1., -1., f64::from_bits(1)).unwrap();
    assert_eq!(sign, Ordering::Greater);
    assert_eq!(bound, I::point(f64::from_bits(1)));
    assert_eq!(sum3(1., -1., -f64::from_bits(1)).unwrap().0, Ordering::Less);
    assert_eq!(sum3(1., 2., -3.).unwrap(), (Ordering::Equal, I::point(0.)));
    let (sign, value) = sum3(1e7, 0.001, -1e7).unwrap();
    assert_eq!(sign, Ordering::Greater);
    assert!(value.lo <= 0.001 && value.hi >= 0.001);
    assert_eq!(sum3(f64::NAN, 1., 2.), Err(Error::Unresolved));
    assert_eq!(sum3(f64::MAX, f64::MAX, 0.), Err(Error::Unresolved));
}

#[test]
fn inverse_trim_queries_refuse_noninterior_and_nonunique_intersections() {
    let source = tool(false, 1, false);
    let contact = Chamfer::new(source, source.spec().depth_mm, None).unwrap();
    let knots = source.profile_breaks();
    let inverse = contact.inverse_phase(1, [knots[1], knots[2]]).unwrap();
    assert!(matches!(
        contact.threshold(1, [inverse.lo, inverse.lo.next_up()]),
        Err(Error::Unresolved)
    ));
    assert!(matches!(
        contact.inverse_phase(2, [knots[2], knots[3]]),
        Err(Error::Unresolved)
    ));
    let uncertain =
        Chamfer::from_material(source, source.root_radius_mm().next_up(), 0.3, None).unwrap();
    assert!(matches!(
        uncertain.endpoint_sign(1, knots[2]),
        Err(Error::Unresolved)
    ));
    let mut spec = tool(false, 1, true).spec();
    spec.root_radius_mm = Some(0.125);
    spec.depth_mm =
        (spec.pitch_mm - spec.crest_width_mm) / (2. * 30_f64.to_radians().tan()) - 0.125;
    let source = crate::helical_thread::Thread::new(spec).unwrap();
    let circle = source.root_circle().unwrap();
    let minimum = circle.center_radius - circle.radius;
    let body = Chamfer::from_material(source, minimum, 0.3, None).unwrap();
    assert_eq!(
        body.inverse_phase(2, [circle.phase - circle.radius * 0.5, circle.phase]),
        Ok(I::point(circle.phase))
    );
    let below = Chamfer::from_material(source, minimum.next_down(), 0.3, None).unwrap();
    assert_eq!(
        below.inverse_phase(2, [circle.phase - circle.radius * 0.5, circle.phase]),
        Err(Error::Unresolved)
    );
}

#[test]
fn incompatible_shared_source_endpoint_classifications_are_never_published() {
    let mut found = None;
    'sources: for angle in [1_f64, 30., 45., 60., 80.] {
        for fraction in [0.1, 0.5, 0.9] {
            for scale in [1.01, 1.1, 2., 10., 1000.] {
                let radians = angle.to_radians();
                let radius = 0.875 / (2. * radians.cos()) * fraction;
                let depth = 0.875 / (2. * radians.tan()) - radius * (1. / radians.sin() - 1.);
                let Ok(source) = crate::helical_thread::Thread::new(crate::helical_thread::Spec {
                    major_radius_mm: depth * scale,
                    pitch_mm: 1.,
                    length_mm: 6.6,
                    depth_mm: depth,
                    crest_width_mm: 0.125,
                    lower_flank_deg: angle,
                    upper_flank_deg: angle,
                    starts: 1,
                    left_hand: false,
                    root_radius_mm: Some(radius),
                }) else {
                    continue;
                };
                let knots = source.profile_breaks();
                for band in [2, 3] {
                    let q = knots[band];
                    let target = source.radius_at_phase(q);
                    let Ok(body) = Chamfer::from_material(source, target, 0.3, None) else {
                        continue;
                    };
                    if let (Ok(a), Ok(b)) =
                        (body.endpoint_sign(band - 1, q), body.endpoint_sign(band, q))
                        && a.0 != b.0
                    {
                        found = Some(body);
                        break 'sources;
                    }
                }
            }
        }
    }
    let body=found.expect("the admitted high circle-radius/source-radius chart exposes a real shared-support residual");
    assert!(matches!(body.phase_segments(), Err(Error::Unresolved)));
}

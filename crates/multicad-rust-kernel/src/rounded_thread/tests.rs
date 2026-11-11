use super::*;
fn whitworth() -> Spec {
    let p = 1.27;
    let a = 27.5_f64.to_radians();
    let r = p * a.cos() / (12. * (1. - a.sin()));
    Spec {
        major_radius_mm: 3.175,
        pitch_mm: p,
        length_mm: 3.7 * p,
        depth_mm: p / (3. * a.tan()),
        crest_radius_mm: r,
        root_radius_mm: r,
        flank_angle_deg: 27.5,
        starts: 1,
        left_hand: false,
    }
}
fn rd(internal: bool) -> Spec {
    let p = 3.175;
    let a = 15_f64.to_radians();
    let r = p * (a.cos() - a.sin()) / (4. * (1. - a.sin()));
    let delta = if internal {
        0.05 * p * a.sin() / (1. - a.sin())
    } else {
        0.
    };
    Spec {
        major_radius_mm: 10. + if internal { 0.05 * p } else { 0. },
        pitch_mm: p,
        length_mm: 2.37 * p,
        depth_mm: p / 2.,
        crest_radius_mm: r - delta,
        root_radius_mm: r + delta,
        flank_angle_deg: 15.,
        starts: 1,
        left_hand: false,
    }
}
fn near(a: f64, b: f64, e: f64) {
    assert!((a - b).abs() <= e, "{a} != {b}, tolerance {e}")
}
#[test]
fn circles_flanks_and_analytic_moments_retain_both_literal_radii() {
    for spec in [whitworth(), rd(false), rd(true)] {
        let thread = Thread::new(spec).unwrap();
        assert_eq!(thread.spec(), spec);
        let breaks = thread.profile_breaks();
        assert_eq!(thread.face_index(0), Some(2));
        assert_eq!(thread.face_index(4), Some(6));
        assert_eq!(thread.face_index(5), None);
        assert_eq!(thread.face_index(usize::MAX), None);
        assert_eq!(
            thread.profile_circle(0).unwrap().radius_mm,
            spec.crest_radius_mm
        );
        assert_eq!(
            thread.profile_circle(2).unwrap().radius_mm,
            spec.root_radius_mm
        );
        assert_eq!(
            thread.profile_circle(4).unwrap().center_phase_mm,
            spec.pitch_mm
        );
        assert_eq!(thread.profile_circle(1), None);
        assert_eq!(thread.profile_circle(8), None);
        near(thread.radius_at_phase(0.), spec.major_radius_mm, 1e-14);
        near(
            thread.radius_at_phase(spec.pitch_mm / 2.),
            thread.root_radius_mm(),
            1e-14,
        );
        for &p in &breaks[1..5] {
            let [a, b] = thread.radius_and_slope_at_phase(p);
            let [c, d] = thread.radius_and_slope_at_phase(p - 1e-12 * spec.pitch_mm);
            near(a, c, 2e-8);
            near(b, d, 2e-8);
        }
        let mut integral = [0.; 2];
        let n = 2000;
        for band in breaks.windows(2) {
            for i in 0..=n {
                let x = band[0] + (band[1] - band[0]) * i as f64 / n as f64;
                let r = thread.radius_at_phase(x);
                let w = if i == 0 || i == n {
                    1.
                } else if i % 2 == 0 {
                    2.
                } else {
                    4.
                };
                integral[0] += w * r * r * (band[1] - band[0]) / (3. * n as f64);
                integral[1] += w * r.powi(4) * (band[1] - band[0]) / (3. * n as f64);
            }
        }
        near(
            thread.volume_mm3(),
            PI * spec.length_mm / spec.pitch_mm * integral[0],
            1e-8,
        );
        near(
            thread.polar_second_moment_mm5(),
            PI * spec.length_mm / (2. * spec.pitch_mm) * integral[1],
            1e-6,
        );
        assert_eq!(thread.axial_centroid_mm(), spec.length_mm / 2.);
        near(
            thread.axial_second_moment_mm5(),
            thread.volume_mm3() * spec.length_mm.powi(2) / 3.,
            1e-8,
        );
        let z = spec.pitch_mm * 0.17;
        for theta in [0., 0.3, 2., 5.] {
            let r = thread.radius(theta, z);
            let p = [(r - 1e-5) * theta.cos(), (r - 1e-5) * theta.sin(), z];
            assert!(thread.contains(p, 0.));
            assert!(!thread.contains([(r + 1e-5) * theta.cos(), (r + 1e-5) * theta.sin(), z], 0.));
            let normal = thread.normal(theta, z);
            near(normal.into_iter().map(|v| v * v).sum(), 1., 1e-14);
        }
        assert!(!thread.contains([0., 0., -1.], 0.));
        assert!(!thread.contains([0., 0., 0.], -1.));
        assert!(!thread.contains([f64::NAN, 0., 0.], 0.));
        assert!(!thread.contains([0., 0., 0.], f64::INFINITY));
        assert!(thread.normal(1e308, 1e308).into_iter().all(f64::is_finite));
        assert!(thread.radius_at_phase(f64::INFINITY).is_nan());
        assert!(thread.radius_and_slope_at_phase(f64::NAN)[1].is_nan());
    }
}
#[test]
fn support_encloses_actual_material_for_both_hands_and_fractional_caps() {
    for mut spec in [whitworth(), rd(true)] {
        for left in [false, true] {
            spec.left_hand = left;
            let t = Thread::new(spec).unwrap();
            for direction in [
                [1., 0.2, 0.7],
                [-0.3, 0.9, -0.4],
                [0., 0., 1.],
                [0., 0., -2.],
                [1., 0., 0.],
                [0.; 3],
            ] {
                let bounds = t.support_bounds(direction, 1e-7).unwrap();
                assert!(bounds[1] - bounds[0] <= 1e-7 + 1e-12);
                let mut dense = f64::NEG_INFINITY;
                for i in 0..=240 {
                    let z = spec.length_mm * i as f64 / 240.;
                    for j in 0..=720 {
                        let a = TAU * j as f64 / 720.;
                        let r = t.radius(a, z);
                        dense = dense.max(
                            direction[0] * r * a.cos()
                                + direction[1] * r * a.sin()
                                + direction[2] * z,
                        );
                    }
                }
                assert!(dense <= bounds[1], "{dense} > {bounds:?}");
                assert!(bounds[0] - dense < 0.02, "{bounds:?} vs {dense}");
            }
        }
    }
}
#[test]
fn validation_precision_and_support_budget_fail_without_changing_the_profile() {
    let good = whitworth();
    let t = Thread::new(good).unwrap();
    for field in 0..7 {
        let mut bad = good;
        match field {
            0 => bad.major_radius_mm = f64::NAN,
            1 => bad.pitch_mm = 0.,
            2 => bad.length_mm = f64::INFINITY,
            3 => bad.depth_mm = -1.,
            4 => bad.crest_radius_mm = 0.,
            5 => bad.root_radius_mm = 1e8,
            _ => bad.flank_angle_deg = f64::NAN,
        }
        assert_eq!(Thread::new(bad), Err(Error::Invalid));
    }
    for bad in [
        Spec {
            major_radius_mm: good.depth_mm,
            ..good
        },
        Spec {
            depth_mm: good.depth_mm * 0.99,
            ..good
        },
    ] {
        assert_eq!(Thread::new(bad), Err(Error::Invalid));
    }
    for bad in [
        Spec { starts: 2, ..good },
        Spec {
            pitch_mm: 0.0001,
            ..good
        },
        Spec {
            length_mm: good.pitch_mm / 2.,
            ..good
        },
        Spec {
            length_mm: 128. * good.pitch_mm,
            ..good
        },
    ] {
        assert_eq!(Thread::new(bad), Err(Error::Limit));
    }
    assert_eq!(
        t.support_bounds([f64::NAN, 0., 0.], 1e-7),
        Err(Error::Invalid)
    );
    assert_eq!(t.support_bounds([1., 0., 1.], 0.), Err(Error::Invalid));
    assert_eq!(
        t.support_bounds([1., 0., 1.], f64::INFINITY),
        Err(Error::Invalid)
    );
    assert_eq!(
        t.support_bounds([1., 0., 1.], 1e-30),
        Err(Error::Unresolved)
    );
    assert_eq!(t.support_budget([1., 0., 1.], 1e-7, 2), Err(Error::Limit));
    assert!(t.support_bounds([1., 0., 1.], 1e-7).is_ok());
    assert_eq!(t.spec(), good);
}

#[test]
fn unresolved_tangencies_and_scaled_support_refuse_without_nonfinite_bounds() {
    let a = 30_f64.to_radians();
    let p = 1.;
    let rc = p / (4. * a.cos());
    let mut s = whitworth();
    s.pitch_mm = p;
    s.flank_angle_deg = 30.;
    s.crest_radius_mm = rc;
    s.root_radius_mm = rc;
    s.depth_mm = p / (2. * a.tan()) - 2. * rc * (1. / a.sin() - 1.);
    assert_eq!(Thread::new(s), Err(Error::Unresolved));
    let mut near = s;
    near.crest_radius_mm *= 1. - 1e-12;
    near.root_radius_mm *= 1. - 1e-12;
    near.depth_mm =
        p / (2. * a.tan()) - (near.crest_radius_mm + near.root_radius_mm) * (1. / a.sin() - 1.);
    assert_eq!(Thread::new(near), Err(Error::Unresolved));
    let mut tiny = s;
    tiny.crest_radius_mm = 1e-18;
    tiny.root_radius_mm = 0.1;
    tiny.depth_mm =
        p / (2. * a.tan()) - (tiny.crest_radius_mm + tiny.root_radius_mm) * (1. / a.sin() - 1.);
    assert_eq!(Thread::new(tiny), Err(Error::Unresolved));
    let t = Thread::new(whitworth()).unwrap();
    assert_eq!(
        t.support_bounds([1e308, 0., 0.], 1e-7),
        Err(Error::Unresolved)
    );
    assert_eq!(
        t.support_bounds([1e-310, 0., 1e-310], 1.),
        Err(Error::Unresolved)
    );
    let s = whitworth();
    let scale = 1e6;
    let huge = Thread::new(Spec {
        major_radius_mm: s.major_radius_mm * scale,
        pitch_mm: s.pitch_mm * scale,
        length_mm: s.length_mm * scale,
        depth_mm: s.depth_mm * scale,
        crest_radius_mm: s.crest_radius_mm * scale,
        root_radius_mm: s.root_radius_mm * scale,
        ..s
    })
    .unwrap();
    assert_eq!(
        huge.support_bounds([1e308, 0., 1e308], 1e308),
        Err(Error::Unresolved)
    );
    assert!(huge.support_bounds([1., 0., 1.], 1e-4).is_ok());
}

#[test]
fn radial_shortcut_encloses_the_exact_norm_after_every_operation() {
    let t = Thread::new(Spec {
        major_radius_mm: 4.1,
        ..whitworth()
    })
    .unwrap();
    let b = t
        .support_bounds([7.344269896220141e-05, 0.18247794999103528, 0.], 1e-10)
        .unwrap();
    // 90-digit independent value: .74815965555879115478539665 lies strictly
    // between these two adjacent f64 values (the former shortcut's lower bound
    // was the upper of those values and excluded the exact support).
    assert!(b[0] <= 0.748_159_655_558_791_2f64.next_down());
    assert!(b[1] >= 0.748_159_655_558_791_2);
    assert_eq!(
        t.support_bounds([1., 0., 0.], 1e-30),
        Err(Error::Unresolved)
    );
    for d in [[1e-310, 0., 0.], [0., 1e-310, 0.], [1e-310, 1e-310, 0.]] {
        let b = t.support_bounds(d, 1e-320).unwrap();
        assert!(b[0] >= 0. && b[0] <= b[1]);
        assert!(b[1] > 0.);
    }
    let b = t.support_bounds([0., 0., 0.1], 1e-10).unwrap();
    assert!(b[0] <= t.spec().length_mm * 0.1 && b[1] >= t.spec().length_mm * 0.1);
}

#[test]
fn bisection_requires_a_strictly_representable_interior_point() {
    assert_eq!(support::checked_midpoint(-PI / 2., PI / 2.), Ok(0.));
    assert_eq!(
        support::checked_midpoint(1., 1_f64.next_up()),
        Err(Error::Unresolved)
    );
    assert_eq!(support::checked_midpoint(1., 1.), Err(Error::Unresolved));
    assert_eq!(support::checked_midpoint(2., 1.), Err(Error::Unresolved));
    assert_eq!(
        support::checked_midpoint(0., f64::INFINITY),
        Err(Error::Unresolved)
    );
    let hi = 1_f64.next_up().next_up().next_up();
    let m = support::checked_midpoint(1., hi).unwrap();
    assert!(m > 1. && m < hi);
    let mut s = whitworth();
    s.length_mm = s.pitch_mm;
    let t = Thread::new(s).unwrap();
    for direction in [[1., 0.3, 0.7], [-0.2, 1., -0.3]] {
        assert!(t.support_bounds(direction, 1e-7).is_ok());
    }
}

#[test]
fn reconstructed_period_endpoints_never_escape_the_actual_crest_circle() {
    for degrees in [1_f64, 15., 27.5] {
        let a = degrees.to_radians();
        let crest = 8.5e-16;
        let root = 0.49 / a.cos();
        let pitch = 1.;
        let s = Spec {
            major_radius_mm: 3.,
            pitch_mm: pitch,
            length_mm: 2.,
            depth_mm: pitch / (2. * a.tan()) - (crest + root) * (1. / a.sin() - 1.),
            crest_radius_mm: crest,
            root_radius_mm: root,
            flank_angle_deg: degrees,
            starts: 1,
            left_hand: false,
        };
        assert_eq!(Thread::new(s), Err(Error::Unresolved));
    }
    let t = Thread::new(whitworth()).unwrap();
    for &phase in &t.profile_breaks() {
        assert!(t.radius_and_slope_at_phase(phase)[1].is_finite());
    }
    assert!(t.support_bounds([1., 0.4, -0.6], 1e-7).is_ok());
}

#[test]
fn symbolic_peak_and_center_receipt_survive_large_circle_center_cancellation() {
    let a = 80_f64.to_radians();
    let total = 0.8 / (2. * a.cos());
    let rc = 0.99 * total;
    let rr = total - rc;
    let depth = 1. / (2. * a.tan()) - total * (1. / a.sin() - 1.);
    let mut radius = depth * (1. + 1e-5);
    let mut nonzero = false;
    let mut uncharged_center_overshoot = false;
    for _ in 0..64 {
        let t = Thread::new(Spec {
            major_radius_mm: radius,
            pitch_mm: 1.,
            length_mm: 2.,
            depth_mm: depth,
            crest_radius_mm: rc,
            root_radius_mm: rr,
            flank_angle_deg: 80.,
            starts: 1,
            left_hand: false,
        })
        .unwrap();
        assert_eq!(t.radius_at_phase(0.), radius);
        let c = t.profile_circle(0).unwrap();
        let recovered = c.center_radius_mm + c.radius_mm;
        assert!((recovered - radius).abs() <= c.center_error_mm);
        nonzero |= c.center_error_mm > 0.;
        let support = t.support_bounds([1., 0., 0.], 1e-12).unwrap();
        assert!(support[0] <= radius && support[1] >= radius);
        uncharged_center_overshoot |= recovered > support[1];
        for i in 0..200 {
            assert!(t.radius_at_phase(i as f64 / 200.) <= support[1]);
        }
        radius = radius.next_up();
    }
    assert!(nonzero);
    assert!(uncharged_center_overshoot);
}

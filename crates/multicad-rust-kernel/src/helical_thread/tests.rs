use super::*;

fn metric() -> Spec {
    let pitch = 1.25;
    Spec {
        major_radius_mm: 4.,
        pitch_mm: pitch,
        length_mm: 49.8,
        depth_mm: 5. * 3_f64.sqrt() / 16. * pitch,
        crest_width_mm: pitch / 8.,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        starts: 1,
        left_hand: false,
        root_radius_mm: None,
    }
}

fn close(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} differs from {b}");
}

#[test]
fn flat_metric_profile_retains_the_axial_pitch_and_real_swept_volume() {
    let spec = metric();
    let thread = Thread::new(spec).unwrap();
    let pitch = spec.pitch_mm;
    let major = spec.major_radius_mm;
    let root = major - spec.depth_mm;
    let pitch_radius = major - 3. * 3_f64.sqrt() / 16. * pitch;
    assert_eq!(thread.spec(), spec);
    assert_eq!(thread.lead_mm(), pitch);
    assert_eq!(thread.radius_at_phase(0.), major);
    assert_eq!(thread.radius_at_phase(pitch), major);
    assert_eq!(thread.radius_at_phase(pitch / 2.), root);
    for phase in [pitch / 4., 3. * pitch / 4.] {
        close(thread.radius_at_phase(phase), pitch_radius, 1e-12);
    }
    close(
        thread.radius_at_phase(-pitch / 4.),
        thread.radius_at_phase(3. * pitch / 4.),
        1e-12,
    );
    assert!(thread.radius_at_phase(f64::NAN).is_nan());
    let slope = (30_f64.to_radians().tan()).recip();
    close(thread.slope_at_phase(pitch / 4., 1), -slope, 1e-12);
    close(thread.slope_at_phase(3. * pitch / 4., 3), slope, 1e-12);
    assert_eq!(thread.slope_at_phase(pitch / 2., 2), 0.);
    assert_eq!(thread.slope_at_phase(0., 0), 0.);
    // Integrate the excess area over the base cylinder in radial, rather
    // than phase, coordinates.
    let axial_width = spec.crest_width_mm;
    let weighted_area = axial_width * (major * major - root * root) / 2.
        + 2. * (30_f64.to_radians().tan())
            * (major * (major * major - root * root) / 2. - (major.powi(3) - root.powi(3)) / 3.);
    let volume = spec.length_mm * (PI * root * root + TAU / pitch * weighted_area);
    close(thread.volume_mm3(), volume, 1e-10);
}

#[test]
fn the_fitted_root_is_a_tangent_circle_on_both_flanks() {
    for [lower_deg, upper_deg] in [[30., 30.], [3., 30.], [30., 3.]] {
        let base = metric();
        let rounding = 0.15;
        let [lower, upper] = [lower_deg, upper_deg].map(f64::to_radians);
        let half = (upper + lower) * 0.5;
        let tilt = (upper - lower) * 0.5;
        let intersection = (base.pitch_mm - base.crest_width_mm) / (lower.tan() + upper.tan());
        let depth = intersection - rounding * (tilt.cos() / half.sin() - 1.);
        let spec = Spec {
            depth_mm: depth,
            lower_flank_deg: lower_deg,
            upper_flank_deg: upper_deg,
            root_radius_mm: Some(rounding),
            ..base
        };
        let thread = Thread::new(spec).unwrap();
        let [_, crest_end, a, b, crest_start, _] = thread.profile_breaks();
        let center_phase = a + rounding * upper.cos();
        close(
            thread.radius_at_phase(center_phase),
            thread.root_radius_mm(),
            1e-12,
        );
        close(
            thread.radius_at_phase(a),
            base.major_radius_mm - (a - crest_end) / upper.tan(),
            1e-12,
        );
        close(
            thread.radius_at_phase(b),
            base.major_radius_mm - (crest_start - b) / lower.tan(),
            1e-12,
        );
        close(thread.slope_at_phase(a, 2), -upper.tan().recip(), 1e-10);
        close(thread.slope_at_phase(b, 2), lower.tan().recip(), 1e-10);
        let x = rounding * 0.2;
        close(
            thread.radius_at_phase(center_phase + x) - thread.root_radius_mm(),
            rounding - (rounding * rounding - x * x).sqrt(),
            1e-12,
        );
        let count = 30_000;
        let area: f64 = thread
            .profile_breaks()
            .windows(2)
            .map(|band| {
                let dz = (band[1] - band[0]) / f64::from(count);
                (0..count)
                    .map(|i| {
                        thread
                            .radius_at_phase(band[0] + (f64::from(i) + 0.5) * dz)
                            .powi(2)
                            * dz
                    })
                    .sum::<f64>()
            })
            .sum();
        close(
            thread.volume_mm3(),
            PI * spec.length_mm / spec.pitch_mm * area,
            2e-7,
        );
    }
}

#[test]
fn handed_material_and_face_identities_close_across_the_angular_seam() {
    for left_hand in [false, true] {
        let spec = Spec {
            starts: 3,
            left_hand,
            ..metric()
        };
        let thread = Thread::new(spec).unwrap();
        let hand = if left_hand { -1. } else { 1. };
        close(thread.helical_rate(), hand * thread.lead_mm() / TAU, 1e-12);
        let z = 3.37;
        for start in 0..spec.starts {
            let theta = hand * TAU * z / thread.lead_mm() + TAU * f64::from(start) / 3.;
            let outside = theta + PI / 3.;
            let radius = spec.major_radius_mm - 0.05;
            let point = |angle: f64| [angle.cos() * radius, angle.sin() * radius, z];
            assert!(thread.contains(point(theta), 0.));
            assert!(!thread.contains(point(outside), 0.));
            close(
                thread.radius(theta, z),
                thread.radius(theta + TAU, z),
                1e-12,
            );
            let period = (hand * -f64::from(start)) as i64;
            assert_eq!(thread.face_index(period, 0), 3 + 4 * start);
            assert_eq!(thread.face_index(period - 1, 3), 2 + 4 * start);
            assert_eq!(thread.face_index(period, 1), 4 + 4 * start);
            assert_eq!(thread.face_index(period, 2), 5 + 4 * start);
            assert_eq!(thread.face_index(period - 1, 4), 3 + 4 * start);
        }
        assert!(!thread.contains([0., 0., -1e-5], 0.));
        assert!(thread.contains([0., 0., -1e-5], 1e-5));
        assert!(!thread.contains([0., 0., spec.length_mm + 1.], 0.));
        assert!(!thread.contains([0.; 3], -1e-7));
    }
}

#[test]
fn oblique_support_is_tight_and_uses_actual_crest_placement() {
    let spec = Spec {
        length_mm: 8.125,
        ..metric()
    };
    let thread = Thread::new(spec).unwrap();
    assert_eq!(thread.support([1., 0., 0.]), spec.major_radius_mm);
    assert_eq!(thread.support([0., 0., 1.]), spec.length_mm);
    assert_eq!(thread.support([0., 0., -1.]), 0.);
    assert_eq!(thread.support([0.; 3]), 0.);
    assert!(thread.support([f64::INFINITY, 0., 0.]).is_nan());

    // The upper cap is at valley phase along +X. The axial component
    // dominates the radial slope, so no earlier crest can increase support.
    let expected = 10. * spec.length_mm + thread.root_radius_mm();
    let actual = thread.support([1., 0., 10.]);
    assert!(actual >= expected);
    close(actual, expected, 1e-7);

    let threads = [
        thread,
        Thread::new(Spec {
            left_hand: true,
            starts: 2,
            ..spec
        })
        .unwrap(),
    ];
    for thread in threads {
        for direction in [[0.37, 0.51, -0.63], [-0.4, 0.75, 0.51]] {
            let support = thread.support(direction);
            let mut sampled: f64 = f64::NEG_INFINITY;
            for i in 0..1024 {
                let theta = f64::from(i) / 1024. * TAU;
                let (sin, cos) = theta.sin_cos();
                for j in 0..=768 {
                    let z = f64::from(j) / 768. * spec.length_mm;
                    let radius = thread.radius(theta, z);
                    let projected =
                        radius * (cos * direction[0] + sin * direction[1]) + z * direction[2];
                    sampled = sampled.max(projected);
                }
            }
            assert!(support >= sampled - 1e-12);
            close(support, sampled, 0.02);
        }
    }
}

#[test]
fn invalid_profiles_and_exhausted_turn_budgets_are_distinct() {
    let spec = metric();
    let rounding = 0.15;
    let fitted_depth =
        (spec.pitch_mm - spec.crest_width_mm) / (2. * 30_f64.to_radians().tan()) - rounding;
    for invalid in [
        Spec {
            major_radius_mm: f64::NAN,
            ..spec
        },
        Spec {
            depth_mm: spec.major_radius_mm,
            ..spec
        },
        Spec {
            upper_flank_deg: 0.,
            ..spec
        },
        Spec {
            crest_width_mm: spec.pitch_mm,
            ..spec
        },
        Spec {
            crest_width_mm: spec.pitch_mm * 0.8,
            ..spec
        },
        Spec {
            root_radius_mm: Some(0.15),
            ..spec
        },
        Spec {
            // The supplied depth is within tangent arithmetic tolerance,
            // but the derived circle would make the physical core negative.
            major_radius_mm: fitted_depth * (1. - 1e-13),
            depth_mm: fitted_depth * (1. - 2e-13),
            root_radius_mm: Some(rounding),
            ..spec
        },
    ] {
        assert_eq!(Thread::new(invalid), Err(ThreadError::Invalid));
    }
    for excessive in [
        Spec {
            length_mm: spec.pitch_mm * 127.,
            ..spec
        },
        Spec {
            length_mm: spec.pitch_mm * 0.5,
            ..spec
        },
        Spec { starts: 9, ..spec },
        Spec {
            pitch_mm: 0.0005,
            crest_width_mm: 0.00005,
            ..spec
        },
    ] {
        assert_eq!(Thread::new(excessive), Err(ThreadError::Limit));
    }
}

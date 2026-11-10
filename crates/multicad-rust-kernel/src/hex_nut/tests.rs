use super::*;
use crate::helical_thread::Spec;
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, FRAC_PI_6, PI, TAU};

fn thread(starts: u32, left_hand: bool) -> Thread {
    Thread::new(Spec {
        major_radius_mm: 4.,
        pitch_mm: 1.25,
        length_mm: 6.6,
        depth_mm: 1.25 * 5. * 3_f64.sqrt() / 16.,
        crest_width_mm: 1.25 / 8.,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        starts,
        left_hand,
        root_radius_mm: None,
    })
    .unwrap()
}

#[test]
fn hex_flats_and_fractional_through_thread_enclose_the_expected_material() {
    let profile = thread(1, false);
    let nut = Nut::new(profile, 13.).unwrap();
    assert_eq!(nut.thread(), &profile);
    assert_eq!(nut.across_flats_mm(), 13.);
    assert_eq!(nut.outer_radius(0.), 6.5);
    for corner in [FRAC_PI_6, FRAC_PI_2, -FRAC_PI_2] {
        assert!((nut.outer_radius(corner) - 13. / 3_f64.sqrt()).abs() < 1e-12);
    }

    // Independent area integration for a flat-root trapezoidal profile.
    let spec = profile.spec();
    let major = 4.;
    let minor = major - spec.depth_mm;
    let flank_width = spec.depth_mm / 3_f64.sqrt();
    let root_width = spec.pitch_mm - spec.crest_width_mm - 2. * flank_width;
    let mean_squared_radius = (major * major * spec.crest_width_mm
        + 2. * flank_width * (major * major + major * minor + minor * minor) / 3.
        + minor * minor * root_width)
        / spec.pitch_mm;
    let side = 13. / 3_f64.sqrt();
    let hex_area = 6. * (side * 6.5 / 2.);
    assert!((nut.outer_area_mm2() - hex_area).abs() < 1e-12);
    let expected = (hex_area - PI * mean_squared_radius) * spec.length_mm;
    assert!((nut.volume_mm3() - expected).abs() < 1e-10);
    let crest_at_exit = TAU * spec.length_mm.rem_euclid(spec.pitch_mm) / spec.pitch_mm;
    let (exit_sin, exit_cos) = crest_at_exit.sin_cos();

    for point in [
        [4., 0., 0.],
        [6.5, 0., 0.],
        [0., 7., spec.length_mm],
        [3.8, 0., spec.length_mm],
        [3.8, 0., spec.pitch_mm / 2.],
    ] {
        assert!(nut.contains(point, 0.), "{point:?} is material");
    }
    for point in [
        [3.8, 0., 0.],
        [6.6, 0., 1.],
        [0., 0., spec.length_mm / 2.],
        [0., 0., spec.length_mm],
        [3.8 * exit_cos, 3.8 * exit_sin, spec.length_mm],
        [5., 0., -0.002],
        [5., 0., spec.length_mm + 0.002],
    ] {
        assert!(!nut.contains(point, 0.), "{point:?} is empty");
    }
    assert!(nut.contains([5., 0., -0.002], 0.003));
    assert!(nut.contains([6.5005, 0., 1.], 0.001));
    assert!(nut.contains([3.9995, 0., 0.], 0.001));
}

#[test]
fn opposite_hands_reverse_the_bore_phase_without_changing_the_prism_support() {
    let right = Nut::new(thread(2, false), 13.).unwrap();
    let left = Nut::new(thread(2, true), 13.).unwrap();
    let (sin, cos) = FRAC_PI_4.sin_cos();
    let probe = [3.8 * cos, 3.8 * sin, 1.25 / 4.];
    assert!(!right.contains(probe, 0.));
    assert!(left.contains(probe, 0.));
    assert_eq!(left.volume_mm3(), right.volume_mm3());

    // At this oblique direction, the top-most corner with polar angle PI/6
    // wins when Z is positive. The same corner at Z=0 wins when negative.
    let [x, y] = [0.8, 0.6];
    let expected_xy = x * 6.5 + y * 6.5 / 3_f64.sqrt();
    let top = expected_xy + 0.7 * 6.6;
    assert!((left.support([x, y, 0.7]) - top).abs() < 1e-12);
    assert!((right.support([x, y, -0.7]) - expected_xy).abs() < 1e-12);
    assert!((right.support([10. * x, 10. * y, 7.]) - 10. * top).abs() < 1e-11);
    assert_eq!(left.support([0.; 3]), 0.);
    assert_eq!(left.support([0., 0., 2.]), 2. * 6.6);
}

#[test]
fn thin_clearance_is_valid_and_invalid_geometry_is_rejected() {
    let profile = thread(1, false);
    let nut = Nut::new(profile, 8. + 2e-8).unwrap();
    assert!(nut.contains([4. + 5e-9, 0., 0.], 0.));
    assert!(!nut.contains([4. + 2e-8, 0., 0.], 0.));
    assert!(nut.volume_mm3() > 0.);
    for invalid in [f64::NAN, f64::INFINITY, 8., 7.9, -1.] {
        assert_eq!(Nut::new(profile, invalid), Err(ThreadError::Invalid));
    }
    assert_eq!(Nut::new(profile, 1e8), Err(ThreadError::Limit));
    assert!(nut.support([0., f64::INFINITY, 0.]).is_nan());
    assert!(!nut.contains([0., f64::INFINITY, 0.], 0.));
    assert!(!nut.contains([4., 0., 0.], f64::NAN));
    assert!(!nut.contains([4., 0., 0.], -0.01));
}

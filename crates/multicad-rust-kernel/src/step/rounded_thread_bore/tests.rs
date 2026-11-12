use super::*;
use crate::rounded_thread::{Spec, Thread};
fn bore(left_hand: bool) -> ThroughBore {
    let p = 1.27;
    let a = 27.5_f64.to_radians();
    let r = p * a.cos() / (12. * (1. - a.sin()));
    ThroughBore::new(
        Thread::new(Spec {
            major_radius_mm: 3.175,
            pitch_mm: p,
            length_mm: 3.7 * p,
            depth_mm: p / (3. * a.tan()),
            crest_radius_mm: r,
            root_radius_mm: r,
            flank_angle_deg: 27.5,
            starts: 1,
            left_hand,
        })
        .unwrap(),
        5.,
    )
    .unwrap()
}
#[test]
fn through_receiver_caps_share_all_five_rims_and_analytic_outer_wall() {
    for left in [false, true] {
        let b = bore(left);
        let frame = [[17., -8., 13.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]];
        let file = write_rounded_thread_bore(&b, frame[0], frame[1], frame[2], 1e-7).unwrap();
        let volume = crate::step_read::verify_rounded_thread_bore(
            &file, &b, frame[0], frame[1], frame[2], 1e-7,
        )
        .unwrap();
        assert!((volume - b.volume_mm3()).abs() < 1e-6);
        let (proof, imported) = crate::step_read::read_rounded_thread_bore(&file, 1e-7).unwrap();
        assert_eq!(proof.face_count, 8);
        assert_eq!(proof.solid_count, 1);
        assert!(crate::step_read::rounded_thread_bore_import_matches(
            &imported, &b, frame, 1e-7
        ));
    }
}
fn rd(left: bool) -> ThroughBore {
    let p = 3.175;
    let a = 15_f64.to_radians();
    let base = p * (a.cos() - a.sin()) / (4. * (1. - a.sin()));
    let clearance = 0.05 * p * a.sin() / (1. - a.sin());
    ThroughBore::new(
        Thread::new(Spec {
            major_radius_mm: 10. + 0.05 * p,
            pitch_mm: p,
            length_mm: 2.37 * p,
            depth_mm: p / 2.,
            crest_radius_mm: base - clearance,
            root_radius_mm: base + clearance,
            flank_angle_deg: 15.,
            starts: 1,
            left_hand: left,
        })
        .unwrap(),
        12.5,
    )
    .unwrap()
}
#[test]
fn independent_roundtrips_keep_asymmetric_internal_profiles_hands_and_finite_poses() {
    for left in [false, true] {
        for b in [bore(left), rd(left)] {
            for frame in [
                [[0.; 3], [1., 0., 0.], [0., 1., 0.]],
                [[17., -8., 13.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]],
                [[1e4, -8e3, 6e3], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]],
            ] {
                let file =
                    write_rounded_thread_bore(&b, frame[0], frame[1], frame[2], 1e-7).unwrap();
                let (proof, imported) =
                    crate::step_read::read_rounded_thread_bore(&file, 1e-7).unwrap();
                assert!(crate::step_read::rounded_thread_bore_import_matches(
                    &imported, &b, frame, 1e-7
                ));
                assert_eq!(imported.bore.radius_mm(), b.radius_mm());
                assert_eq!(imported.bore.thread().spec().left_hand, left);
                assert!((proof.volume_mm3 - b.volume_mm3()).abs() < 1e-6);
                let again = write_rounded_thread_bore(
                    &imported.bore,
                    imported.origin_mm,
                    imported.u,
                    imported.v,
                    1e-7,
                )
                .unwrap();
                let (_, twice) = crate::step_read::read_rounded_thread_bore(&again, 1e-7).unwrap();
                assert!(crate::step_read::rounded_thread_bore_import_matches(
                    &twice, &b, frame, 1e-7
                ));
            }
        }
    }
}
#[test]
fn append_refusal_preserves_existing_solids_and_retry_uses_valid_topology() {
    let b = bore(false);
    let sphere = crate::sphere::Sphere::new(1.).unwrap();
    let frame = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
    let mut writer = Writer::new();
    writer
        .append_sphere(sphere, frame[0], frame[1], frame[2])
        .unwrap();
    let thin = ThroughBore::new(*b.thread(), b.thread().spec().major_radius_mm + 1e-8).unwrap();
    assert_eq!(
        writer.append_rounded_thread_bore(&thin, frame[0], frame[1], frame[2], 1e-7),
        Err(StepError::Limit)
    );
    assert_eq!(
        writer.finish().unwrap(),
        write_one(|w| w.append_sphere(sphere, frame[0], frame[1], frame[2])).unwrap()
    );
    let mut writer = Writer::new();
    assert!(
        writer
            .append_rounded_thread_bore(&b, frame[0], [2., 0., 0.], frame[2], 1e-7)
            .is_err()
    );
    assert!(
        writer
            .append_rounded_thread_bore(&b, frame[0], frame[1], frame[2], 0.)
            .is_err()
    );
    writer
        .append_rounded_thread_bore(&b, frame[0], frame[1], frame[2], 1e-7)
        .unwrap();
    writer
        .append_rounded_thread_bore(&bore(true), [20., 0., 0.], frame[1], frame[2], 1e-7)
        .unwrap();
    let file = writer.finish().unwrap();
    assert_eq!(
        crate::step_read::read_rounded_thread_bore_compound(&file, 1e-7)
            .unwrap()
            .len(),
        2
    );
    assert!(crate::step_read::read_rounded_thread_bore(&file, 1e-7).is_err());
}
#[test]
fn complete_outer_domain_controls_representation_and_matching() {
    let source = bore(false);
    let f = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
    let large = ThroughBore::new(*source.thread(), 5e6).unwrap();
    assert!(supports_rounded_thread_bore(&large, f[0], f[1], f[2], 1e-7));
    let noisy = [1. + 2e-15, 0., 0.];
    assert!(supports_rounded_thread_bore(
        &source, f[0], noisy, f[2], 1e-7
    ));
    assert!(!supports_rounded_thread_bore(
        &large, f[0], noisy, f[2], 1e-7
    ));
    let large = ThroughBore::new(*source.thread(), 1e5).unwrap();
    let file = write_rounded_thread_bore(&large, f[0], f[1], f[2], 1e-7).unwrap();
    let (_, mut candidate) = crate::step_read::read_rounded_thread_bore(&file, 1e-7).unwrap();
    let tilt = 5e-11_f64;
    candidate.u = [tilt.cos(), 0., tilt.sin()];
    assert!(crate::step_read::rounded_thread_import_matches(
        &crate::step_read::ImportedRoundedThread {
            thread: *candidate.bore.thread(),
            origin_mm: candidate.origin_mm,
            u: candidate.u,
            v: candidate.v
        },
        large.thread(),
        f,
        1e-7
    ));
    assert!(!crate::step_read::rounded_thread_bore_import_matches(
        &candidate, &large, f, 1e-7
    ));
}
#[test]
fn independent_readback_precision_limit_preserves_the_previous_writer() {
    let b = ThroughBore::new(*bore(false).thread(), 5e6).unwrap();
    let f = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
    let mut w = Writer::new();
    assert_eq!(
        w.append_rounded_thread_bore(&b, f[0], f[1], f[2], 1e-9),
        Err(StepError::Limit)
    );
    w.append_rounded_thread_bore(&bore(false), f[0], f[1], f[2], 1e-7)
        .unwrap();
    assert!(crate::step_read::read_rounded_thread_bore(&w.finish().unwrap(), 1e-7).is_ok());
}

use super::*;
use crate::step_read::{read_rounded_cylinder, read_rounded_cylinder_compound};
fn shape(f: [Option<f64>; 2]) -> RoundedCylinder {
    RoundedCylinder::new(5., 12., f, 1e-6, 1e7).unwrap()
}
#[test]
fn quarter_toroids_share_cap_and_wall_rims_in_each_valid_radius_regime() {
    for f in [
        [Some(1.), None],
        [None, Some(3.)],
        [Some(1.), Some(3.)],
        [Some(4.9), Some(4.8)],
    ] {
        let geometry = shape(f);
        for (origin, u, v) in [
            ([0.; 3], [1., 0., 0.], [0., 1., 0.]),
            ([17., -8., 13.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]),
        ] {
            let text = write_rounded_cylinder(&geometry, origin, u, v, 1e-7).unwrap();
            let (read, body) = read_rounded_cylinder(&text, 1e-7).unwrap();
            assert_eq!(read.face_count, geometry.active_faces().len());
            assert_eq!(read.circles.len(), geometry.rims().len());
            assert_eq!(read.solid_count, 1);
            assert!((read.volume_mm3 - geometry.volume_mm3()).abs() < 1e-7);
            assert_eq!(body.geometry.fillet_radii_mm(), f);
            for (a, b) in body.origin_mm.into_iter().zip(origin) {
                assert!((a - b).abs() < 1e-7);
            }
            let text = write_rounded_cylinder(&body.geometry, body.origin_mm, body.u, body.v, 1e-7)
                .unwrap();
            assert!(
                (read_rounded_cylinder(&text, 1e-7).unwrap().0.volume_mm3 - read.volume_mm3).abs()
                    < 1e-7
            );
        }
    }
}
#[test]
fn rejected_appends_recover_on_the_same_writer_and_preserve_member_order() {
    let g = shape([Some(1.), None]);
    assert_eq!(
        write_rounded_cylinder(&g, [0.; 3], [1., 0., 0.], [0., 1., 0.], 0.).unwrap_err(),
        StepError::InvalidGeometry
    );
    let mut writer = Writer::new();
    writer
        .append_rounded_cylinder(&g, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-7)
        .unwrap();
    for (origin, u, tol, error) in [
        ([0.; 3], [0.; 3], 1e-7, StepError::InvalidPlacement),
        ([0.; 3], [1., 0., 0.], f64::NAN, StepError::InvalidGeometry),
        ([0.; 3], [1., 0., 0.], 0.2, StepError::Unsupported),
        ([1e12; 3], [1., 0., 0.], 1e-7, StepError::InvalidPlacement),
        ([1e30; 3], [1., 0., 0.], 1e-7, StepError::InvalidGeometry),
        (
            [0.; 3],
            [(1_f64 + 8e-11).sqrt(), 0., 0.],
            1e-12,
            StepError::InvalidPlacement,
        ),
    ] {
        assert_eq!(
            writer
                .append_rounded_cylinder(&g, origin, u, [0., 1., 0.], tol)
                .unwrap_err(),
            error
        );
    }
    let narrow = shape([Some(1.), Some(4.9)]);
    assert_eq!(
        writer
            .append_rounded_cylinder(&narrow, [1e16, 0., 0.], [1., 0., 0.], [0., 1., 0.], 1e-7)
            .unwrap_err(),
        StepError::InvalidGeometry
    );
    writer
        .append_rounded_cylinder(&g, [20., 0., 0.], [1., 0., 0.], [0., 1., 0.], 1e-7)
        .unwrap();
    let read = read_rounded_cylinder_compound(&writer.finish().unwrap(), 1e-7).unwrap();
    assert_eq!(read.len(), 2);
    assert_eq!(read[0].1.origin_mm, [0.; 3]);
    assert_eq!(read[1].1.origin_mm, [20., 0., 0.]);
}
#[test]
fn compound_resource_failure_rolls_back_the_last_member() {
    let g = shape([Some(1.), None]);
    let mut writer = Writer::new();
    let mut accepted = 0;
    loop {
        match writer.append_rounded_cylinder(&g, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-7) {
            Ok(()) => accepted += 1,
            Err(error) => {
                assert_eq!(error, StepError::Limit);
                break;
            }
        }
    }
    assert!(accepted > 1 && accepted <= MAX_COMPOUND_SOLIDS);
    assert_eq!(
        read_rounded_cylinder_compound(&writer.finish().unwrap(), 1e-7)
            .unwrap()
            .len(),
        accepted
    );
}

#[test]
fn successful_writes_remain_readable_at_small_feature_and_coordinate_limits() {
    for (radius, height, radii) in [
        (2e-6, 2e-6, [Some(1e-6), None]),
        (3e-6, 3e-6, [Some(1e-6), Some(1e-6)]),
    ] {
        let g = RoundedCylinder::new(radius, height, radii, 1e-6, 1e7).unwrap();
        let mut successful = 0;
        for (origin, u, v) in [
            ([0.; 3], [1., 0., 0.], [0., 1., 0.]),
            ([17., -8., 13.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]),
        ] {
            match write_rounded_cylinder(&g, origin, u, v, 1e-7) {
                Ok(text) => {
                    successful += 1;
                    read_rounded_cylinder(&text, 1e-7).unwrap_or_else(|error| {
                        panic!("small rounded geometry {radius} {height} {origin:?}: {error:?}")
                    });
                }
                Err(error) => assert_eq!(error, StepError::Unsupported),
            }
        }
        assert!(successful > 0);
    }
}

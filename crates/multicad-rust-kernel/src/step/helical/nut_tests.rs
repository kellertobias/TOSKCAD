use super::*;
use crate::{helical_thread::Spec, hex_nut::Nut, step_read::verify_helical_nut};
fn nut(starts: u32, left_hand: bool, rounded: bool) -> Nut {
    let root = rounded.then_some(1. / (4. * 3_f64.sqrt()));
    Nut::new(
        Thread::new(Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 3.7,
            depth_mm: root.map_or(5. * 3_f64.sqrt() / 16., |f| 0.875 * 3_f64.sqrt() / 2. - f),
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts,
            left_hand,
            root_radius_mm: root,
        })
        .unwrap(),
        10.,
    )
    .unwrap()
}
#[test]
fn complete_hex_nut_shells_keep_all_starts_hands_roots_and_closed_planar_exterior() {
    for starts in [1, 2, 3, 8] {
        for left in [false, true] {
            for rounded in [false, true] {
                let nut = nut(starts, left, rounded);
                for [o, u, v] in [
                    [[0.; 3], [1., 0., 0.], [0., 1., 0.]],
                    [[2., -3., 4.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]],
                ] {
                    assert!(supports_helical_nut(&nut, o, 1e-7));
                    let text = write_helical_nut(&nut, o, u, v, 1e-7).unwrap();
                    assert_eq!(
                        text.matches("ADVANCED_FACE").count(),
                        4 * starts as usize + 8
                    );
                    assert_eq!(text.matches("MANIFOLD_SOLID_BREP").count(), 1);
                    let volume = verify_helical_nut(&text, &nut, o, u, v, 1e-7).unwrap();
                    assert!((volume - nut.volume_mm3()).abs() < 1e-6);
                    let (metrics, imported) =
                        crate::step_read::read_import_solid(&text, 1e-7).unwrap();
                    assert_eq!(metrics.solid_count, 1);
                    assert!(crate::step_read::helical_nut_import_matches(
                        &imported,
                        &nut,
                        [o, u, v],
                        1e-7
                    ));
                    assert!((metrics.volume_mm3 - nut.volume_mm3()).abs() < 1e-6);
                    let wrong = Nut::new(*nut.thread(), 11.).unwrap();
                    assert!(verify_helical_nut(&text, &wrong, o, u, v, 1e-7).is_err());
                }
            }
        }
    }
}
#[test]
fn hex_nut_resource_failures_leave_existing_writer_members_intact() {
    let nut = nut(1, false, false);
    let thin = Nut::new(*nut.thread(), 6.00000001).unwrap();
    assert!(!supports_helical_nut(&thin, [0.; 3], 1e-7));
    let mut writer = Writer::new();
    writer
        .append_helical_nut(&nut, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-7)
        .unwrap();
    assert_eq!(
        writer.append_helical_nut(&thin, [0.; 3], [1., 0., 0.], [0., 1., 0.], 1e-7),
        Err(StepError::Limit)
    );
    assert_eq!(
        writer.append_helical_nut(&nut, [f64::NAN, 0., 0.], [1., 0., 0.], [0., 1., 0.], 1e-7),
        Err(StepError::InvalidPlacement)
    );
    writer
        .append_helical_nut(&nut, [20., 0., 0.], [1., 0., 0.], [0., 1., 0.], 1e-7)
        .unwrap();
    assert_eq!(
        writer
            .finish()
            .unwrap()
            .matches("MANIFOLD_SOLID_BREP")
            .count(),
        2
    );
    assert!(matches!(
        prepare(nut.thread(), Closure::HexNut(f64::NAN), [0.; 3], 1e-7),
        Err(StepError::InvalidGeometry)
    ));
}
#[test]
fn malformed_internal_hex_chart_cannot_emit_a_collapsed_nut_exterior() {
    let nut = nut(1, false, false);
    let mut file = StepFile::new();
    assert!(
        encode(
            &mut file,
            nut.thread(),
            Closure::HexNut(10.),
            Placement {
                origin: [0.; 3],
                u: [0.; 3],
                v: [0.; 3],
                normal: [0., 0., 1.]
            },
            1e-7
        )
        .is_err()
    );
}

use super::*;
use crate::rounded_thread::Spec;
pub(super) fn thread(internal: bool, rd: bool, left_hand: bool) -> Thread {
    let (p, a, r, depth, major) = if rd {
        let p = 3.175;
        let a = 15_f64.to_radians();
        (
            p,
            a,
            p * (a.cos() - a.sin()) / (4. * (1. - a.sin())),
            p / 2.,
            10.,
        )
    } else {
        let p = 1.27;
        let a = 27.5_f64.to_radians();
        (
            p,
            a,
            p * a.cos() / (12. * (1. - a.sin())),
            p / (3. * a.tan()),
            3.175,
        )
    };
    let d = if internal && rd {
        0.05 * p * a.sin() / (1. - a.sin())
    } else {
        0.
    };
    Thread::new(Spec {
        major_radius_mm: major + if internal && rd { 0.05 * p } else { 0. },
        pitch_mm: p,
        length_mm: 3.7 * p,
        depth_mm: depth,
        crest_radius_mm: r - d,
        root_radius_mm: r + d,
        flank_angle_deg: a.to_degrees(),
        starts: 1,
        left_hand,
    })
    .unwrap()
}
#[test]
fn five_bands_share_exact_rims_rails_and_placed_caps() {
    for rd in [false, true] {
        for internal in [false, true] {
            for hand in [false, true] {
                let t = thread(internal, rd, hand);
                for [o, u, v] in [
                    [[0.; 3], [1., 0., 0.], [0., 1., 0.]],
                    [[80., 60., 20.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]],
                ] {
                    let file = write_rounded_thread(&t, o, u, v, 1e-7).unwrap();
                    let volume =
                        crate::step_read::verify_rounded_thread(&file, &t, o, u, v, 1e-7).unwrap();
                    assert!((volume - t.volume_mm3()).abs() < 1e-7 * t.volume_mm3().max(1.));
                    let (proof, imported) =
                        crate::step_read::read_rounded_thread(&file, 1e-7).unwrap();
                    assert_eq!(proof.face_count, 7);
                    assert_eq!(imported.thread.spec().left_hand, hand);
                    assert!((proof.volume_mm3 - volume).abs() < 1e-12 * volume);
                    let again = write_rounded_thread(
                        &imported.thread,
                        imported.origin_mm,
                        imported.u,
                        imported.v,
                        1e-7,
                    )
                    .unwrap();
                    crate::step_read::read_rounded_thread(&again, 1e-7).unwrap();
                    assert_eq!(file.matches("B_SPLINE_SURFACE_WITH_KNOTS").count(), 5);
                }
            }
        }
    }
}

fn mutate_entity(input: &str, name: &str, edit: impl FnOnce(&str) -> String) -> String {
    let start = input
        .lines()
        .find(|line| line.contains(&format!("={name}(")))
        .unwrap();
    input.replacen(start, &edit(start), 1)
}
fn reverse_faces(input: &str) -> String {
    mutate_entity(input, "CLOSED_SHELL", |line| {
        let (start, list) = line.split_once("('',(").unwrap();
        let refs = list.strip_suffix("));").unwrap();
        let refs = refs.split(',').rev().collect::<Vec<_>>().join(",");
        format!("{start}('',({refs}));")
    })
}
#[test]
fn reordered_declarations_and_members_preserve_physical_proof_without_header_authority() {
    let t = thread(true, true, true);
    let frame = [[4., 5., 6.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]];
    let original = write_rounded_thread(&t, frame[0], frame[1], frame[2], 1e-7).unwrap();
    let reordered = reverse_faces(&original).replace(
        "MultiCAD analytic solid",
        "Foreign physical geometry without source recipe",
    );
    let entities = reordered
        .lines()
        .filter(|l| l.starts_with('#'))
        .collect::<Vec<_>>();
    let first = reordered.find(entities[0]).unwrap();
    let last =
        reordered.find(entities[entities.len() - 1]).unwrap() + entities.last().unwrap().len();
    let file = format!(
        "{}{}{}",
        &reordered[..first],
        entities
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>()
            .join("\n"),
        &reordered[last..]
    );
    let (proof, imported) = crate::step_read::read_rounded_thread(&file, 1e-7).unwrap();
    assert!(crate::step_read::rounded_thread_import_matches(
        &imported, &t, frame, 1e-7
    ));
    assert_eq!(proof.face_count, 7);
    crate::step_read::verify_rounded_thread(&file, &t, frame[0], frame[1], frame[2], 1e-7).unwrap();
    let mut writer = Writer::new();
    writer
        .append_rounded_thread(&t, frame[0], frame[1], frame[2], 1e-7)
        .unwrap();
    let other = thread(false, false, false);
    let offset = [20., 30., 40.];
    writer
        .append_rounded_thread(&other, offset, [1., 0., 0.], [0., 1., 0.], 1e-7)
        .unwrap();
    let compound = writer.finish().unwrap();
    let members = crate::step_read::read_rounded_thread_compound(&compound, 1e-7).unwrap();
    assert_eq!(members.len(), 2);
    assert!(crate::step_read::read_rounded_thread(&compound, 1e-7).is_err());
    assert!(
        crate::step_read::verify_rounded_thread(&compound, &t, frame[0], frame[1], frame[2], 1e-7)
            .is_err()
    );
}
#[test]
fn malformed_physical_graphs_wrong_sources_and_placement_refuse_atomically() {
    let t = thread(false, false, false);
    let frame = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
    let file = write_rounded_thread(&t, frame[0], frame[1], frame[2], 1e-7).unwrap();
    let wrong = thread(false, false, true);
    assert!(
        crate::step_read::verify_rounded_thread(&file, &wrong, frame[0], frame[1], frame[2], 1e-7)
            .is_err()
    );
    assert!(
        crate::step_read::verify_rounded_thread(
            &file,
            &t,
            [f64::NAN, 0., 0.],
            frame[1],
            frame[2],
            1e-7
        )
        .is_err()
    );
    let malformed = [
        mutate_entity(&file, "B_SPLINE_SURFACE_WITH_KNOTS", |s| {
            s.replacen("('',11,11,", "('',10,11,", 1)
        }),
        mutate_entity(&file, "ADVANCED_FACE", |s| s.replacen(".F.", ".T.", 1)),
        mutate_entity(&file, "B_SPLINE_CURVE_WITH_KNOTS", |s| {
            s.replacen("('',11,", "('',10,", 1)
        }),
        mutate_entity(&file, "EDGE_CURVE", |s| s.replacen(".T.", ".F.", 1)),
        mutate_entity(&file, "FACE_OUTER_BOUND", |s| s.replacen(".T.", ".F.", 1)),
        mutate_entity(&file, "PLANE", |s| {
            s.replace("PLANE", "CYLINDRICAL_SURFACE")
        }),
    ];
    for bad in malformed {
        assert!(
            crate::step_read::verify_rounded_thread(&bad, &t, frame[0], frame[1], frame[2], 1e-7)
                .is_err()
        );
        assert!(crate::step_read::read_rounded_thread(&bad, 1e-7).is_err());
    }
    assert!(!supports_rounded_thread(
        &t, frame[0], frame[1], frame[2], 0.
    ));
    assert!(!supports_rounded_thread(
        &t,
        frame[0],
        [2., 0., 0.],
        frame[2],
        1e-7
    ));
    let mut writer = Writer::new();
    writer
        .append_rounded_thread(&t, frame[0], frame[1], frame[2], 1e-7)
        .unwrap();
    assert!(
        writer
            .append_rounded_thread(&t, [1e7, 0., 0.], frame[1], frame[2], 1e-7)
            .is_err()
    );
    let failed = writer.finish().unwrap();
    assert_eq!(failed, file);
    let before = t;
    for tolerance in [0., f64::NAN, 1e-30] {
        assert!(prepare(&t, frame[0], tolerance).is_err());
    }
    assert_eq!(t, before);
    assert!(supports_rounded_thread(
        &t, frame[0], frame[1], frame[2], 1e-7
    ));
}
#[test]
fn imported_descriptor_matches_all_controls_and_rejects_changed_material() {
    let t = thread(false, false, false);
    let frame = [[80., 60., 20.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]];
    let file = write_rounded_thread(&t, frame[0], frame[1], frame[2], 1e-7).unwrap();
    let (_, imported) = crate::step_read::read_rounded_thread(&file, 1e-7).unwrap();
    assert!(crate::step_read::rounded_thread_import_matches(
        &imported, &t, frame, 1e-7
    ));
    assert!(!crate::step_read::rounded_thread_import_matches(
        &imported,
        &t,
        frame,
        f64::NAN
    ));
    let mut moved = imported.clone();
    moved.origin_mm[0] += 1e-5;
    assert!(!crate::step_read::rounded_thread_import_matches(
        &moved, &t, frame, 1e-7
    ));
    let mut bad = frame;
    bad[1] = [2., 0., 0.];
    assert!(!crate::step_read::rounded_thread_import_matches(
        &imported, &t, bad, 1e-7
    ));
    let changed = Thread::new(Spec {
        length_mm: t.spec().length_mm * 1.01,
        ..t.spec()
    })
    .unwrap();
    assert!(
        crate::step_read::verify_rounded_thread(
            &file, &changed, frame[0], frame[1], frame[2], 1e-7
        )
        .is_err()
    );
    assert!(!crate::step_read::rounded_thread_import_matches(
        &imported, &changed, frame, 1e-7
    ));
    let changed = thread(false, false, true);
    assert!(!crate::step_read::rounded_thread_import_matches(
        &imported, &changed, frame, 1e-7
    ));
}

#[test]
fn source_precision_and_tensor_budgets_preserve_typed_exchange_failures() {
    use crate::step_read::ReadError;
    assert_eq!(read_error(ReadError::Limit), StepError::Limit);
    assert_eq!(read_error(ReadError::Invalid), StepError::InvalidGeometry);
    assert_eq!(
        read_error(ReadError::Unsupported),
        StepError::InvalidGeometry
    );
    let t = thread(false, false, false);
    let s = t.spec();
    let large = Thread::new(Spec {
        major_radius_mm: 10.,
        length_mm: s.pitch_mm * 125.,
        ..s
    })
    .unwrap();
    assert!(matches!(
        prepare(&large, [0.; 3], 1e-7),
        Err(StepError::Limit)
    ));
    let thin = Thread::new(Spec {
        major_radius_mm: s.depth_mm + 1.01e-7,
        ..s
    })
    .unwrap();
    assert!(matches!(
        prepare(&thin, [0.; 3], 1e-7),
        Err(StepError::Limit)
    ));
    assert!(prepare(&t, [0.; 3], 1e-7).is_ok());
    // The circular chart factory also rejects an interval outside the real
    // circle, before any Hermite controls or file topology can be published.
    let circle = t.profile_circle(0).unwrap();
    assert!(matches!(
        circle_spline(
            circle,
            [-2. * circle.radius_mm, 0.],
            [s.major_radius_mm; 2],
            t.helical_rate().abs(),
            -1.,
            1e-7
        ),
        Err(StepError::Limit)
    ));
}

#[test]
fn affine_rotation_factory_proves_shared_endpoints_and_recovers_from_real_numeric_limits() {
    let good = checked_affine([0., 1.], [3., 2.], -1., 1e-7, None).unwrap();
    assert!(matches!(
        checked_affine([0., 1e6], [3., 2.], -1., 1e-7, None),
        Err(StepError::Limit)
    ));
    assert!(matches!(
        checked_affine([0., 1.], [3., 2.], -1., 1e-18, None),
        Err(StepError::Limit)
    ));
    assert!(matches!(
        checked_affine([0., 1.], [3., 2.], -1., 1e-7, Some([9., 9.])),
        Err(StepError::Limit)
    ));
    let again = checked_affine([0., 1.], [3., 2.], -1., 1e-7, Some(good.poles[0])).unwrap();
    assert_eq!(again.poles, good.poles);
    assert_eq!(again.knots, good.knots);
}

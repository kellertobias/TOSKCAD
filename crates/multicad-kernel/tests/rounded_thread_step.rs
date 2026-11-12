use multicad_core::{
    commands::Session,
    document::Document,
    model::{self, ThreadFamily, ThreadFit, ThreadHand, ThreadMode, ThreadParameters, length},
};
use multicad_kernel::rust_kernel::rounded_thread::{Spec, Thread};
fn canonical(family: ThreadFamily, internal: bool, left: bool) -> model::ResolvedThread {
    let params = if family == ThreadFamily::Round {
        let mut p = model::round_thread_presets()[1].parameters();
        p.length = length(2.37 * 3.175);
        p
    } else {
        let row = model::whitworth_thread_presets()
            .into_iter()
            .find(|p| p.family == family)
            .unwrap();
        ThreadParameters {
            family,
            major_diameter: length(row.diameter_mm),
            pitch: length(row.pitch_mm),
            length: length(2.37 * row.pitch_mm),
            fit: if family == ThreadFamily::Bspp {
                if internal {
                    ThreadFit::BsppInternal
                } else {
                    ThreadFit::BsppClassBExternal
                }
            } else if internal {
                ThreadFit::WhitworthMediumInternal
            } else {
                ThreadFit::WhitworthMediumExternal
            },
            ..Default::default()
        }
    };
    let params = ThreadParameters {
        hand: if left {
            ThreadHand::Left
        } else {
            ThreadHand::Right
        },
        ..params
    };
    params
        .resolve(
            Session::new(Document::new("Canonical rounded profile").unwrap())
                .unwrap()
                .parameters(),
            if internal {
                ThreadMode::CutInternal
            } else {
                ThreadMode::NewExternal
            },
        )
        .unwrap()
}
fn descriptor(resolved: &model::ResolvedThread) -> Thread {
    let (crest, root) = if let Some(round) = &resolved.round {
        (round.cutter_crest_radius_mm, round.cutter_root_radius_mm)
    } else {
        let r = resolved.profile.rounding_radius_mm.unwrap();
        (r, r)
    };
    Thread::new(Spec {
        major_radius_mm: resolved.major_radius_mm,
        pitch_mm: resolved.pitch_mm,
        length_mm: resolved.length_mm,
        depth_mm: resolved.profile.depth_mm,
        crest_radius_mm: crest,
        root_radius_mm: root,
        flank_angle_deg: resolved.profile.lower_flank_deg,
        starts: resolved.starts,
        left_hand: resolved.hand == ThreadHand::Left,
    })
    .unwrap()
}

#[test]
fn literal_canonical_rounded_profiles_exchange_as_five_proved_bands_without_native_runtime() {
    use multicad_kernel::rust_kernel::{step, step_read};
    for family in [
        ThreadFamily::Bsw,
        ThreadFamily::Bsf,
        ThreadFamily::Bspp,
        ThreadFamily::Round,
    ] {
        for internal in [false, true] {
            for left in [false, true] {
                let source = descriptor(&canonical(family, internal, left));
                let frame = [[17., -8., 13.], [0.8, 0.6, 0.], [-0.36, 0.48, 0.8]];
                let file = step::write_rounded_thread(&source, frame[0], frame[1], frame[2], 1e-7)
                    .unwrap();
                let (proof, imported) = step_read::read_rounded_thread(&file, 1e-7).unwrap();
                assert!(step_read::rounded_thread_import_matches(
                    &imported, &source, frame, 1e-7
                ));
                assert_eq!(proof.solid_count, 1);
                assert_eq!(proof.face_count, 7);
                assert_eq!(imported.thread.spec().left_hand, left);
                assert!(
                    (proof.volume_mm3 - source.volume_mm3()).abs() < source.volume_mm3() * 1e-7
                );
            }
        }
    }
}

#[cfg(feature = "occt")]
#[test]
fn native_reads_valid_closed_rounded_step_and_agrees_with_independent_canonical_material() {
    use multicad_core::document::EntityId;
    use multicad_kernel::{
        BooleanOperation, GeometryPolicy, KnuckleHelicalThread, RigidTransform,
        RoundedHelicalThread, Shape, Solid, rust_kernel::step,
    };
    let policy = GeometryPolicy::default();
    let dir = tempfile::tempdir().unwrap();
    for (family, internal) in [
        (ThreadFamily::Bsw, false),
        (ThreadFamily::Bsf, true),
        (ThreadFamily::Bspp, false),
        (ThreadFamily::Round, false),
        (ThreadFamily::Round, true),
    ] {
        for left in [false, true] {
            println!("Native STEP case {family:?} internal={internal} left={left}");
            let resolved = canonical(family, internal, left);
            let source = descriptor(&resolved);
            let s = source.spec();
            let native = if family == ThreadFamily::Round {
                Solid::knuckle_helical_thread_named(
                    KnuckleHelicalThread {
                        major_radius_mm: s.major_radius_mm,
                        pitch_mm: s.pitch_mm,
                        length_mm: s.length_mm,
                        crest_radius_mm: s.crest_radius_mm,
                        root_radius_mm: s.root_radius_mm,
                        left_hand: left,
                    },
                    EntityId::new(),
                    policy,
                )
                .unwrap()
            } else {
                Solid::rounded_helical_thread_named(
                    RoundedHelicalThread {
                        major_radius_mm: s.major_radius_mm,
                        pitch_mm: s.pitch_mm,
                        length_mm: s.length_mm,
                        depth_mm: s.depth_mm,
                        rounding_radius_mm: s.crest_radius_mm,
                        flank_angle_deg: s.flank_angle_deg,
                        radial_taper: 0.,
                        starts: 1,
                        left_hand: left,
                    },
                    EntityId::new(),
                    policy,
                )
                .unwrap()
            };
            assert!(!native.uses_rust_geometry());
            let pose = if left {
                RigidTransform {
                    rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
                    translation_mm: [17., -8., 13.],
                }
            } else {
                RigidTransform::default()
            };
            let frame = [
                pose.translation_mm,
                std::array::from_fn(|i| pose.rotation[i][0]),
                std::array::from_fn(|i| pose.rotation[i][1]),
            ];
            let file =
                step::write_rounded_thread(&source, frame[0], frame[1], frame[2], 1e-7).unwrap();
            let path = dir
                .path()
                .join(format!("{family:?}-{internal}-{left}.step"));
            std::fs::write(&path, &file).unwrap();
            assert!(Shape::read_step(&path).unwrap().valid().unwrap());
            let imported = Solid::read_step(&path, policy).unwrap();
            assert!(!imported.uses_rust_geometry());
            assert_eq!(imported.solid_count().unwrap(), 1);
            let expected = native.transformed(pose).unwrap();
            let area = expected
                .faces()
                .unwrap()
                .iter()
                .map(|f| f.area_mm2)
                .sum::<f64>();
            let actual_area = imported
                .faces()
                .unwrap()
                .iter()
                .map(|f| f.area_mm2)
                .sum::<f64>();
            let budget =
                policy.linear_tolerance_mm * (area + actual_area) + 1e-10 * source.volume_mm3();
            assert!(
                (imported.volume_mm3().unwrap() - expected.volume_mm3().unwrap()).abs() <= budget,
                "{family:?}/{internal}/{left}"
            );
            let a = expected.bounds().unwrap();
            let b = imported.bounds().unwrap();
            for i in 0..3 {
                assert!((a.min[i] - b.min[i]).abs() <= 8. * policy.linear_tolerance_mm);
                assert!((a.max[i] - b.max[i]).abs() <= 8. * policy.linear_tolerance_mm);
            }
            let cuts = source.profile_breaks();
            for phase in [
                cuts[1] / 2.,
                cuts[1].midpoint(cuts[2]),
                s.pitch_mm / 2.,
                cuts[3].midpoint(cuts[4]),
                cuts[4].midpoint(cuts[5]),
            ] {
                let theta = 0.8;
                let z = phase + s.pitch_mm + source.helical_rate() * theta;
                let r = source.radius_at_phase(phase);
                for offset in [-0.001, 0.001] {
                    let point = [(r + offset) * theta.cos(), (r + offset) * theta.sin(), z];
                    let world = std::array::from_fn(|i| {
                        pose.translation_mm[i]
                            + (0..3).map(|j| pose.rotation[i][j] * point[j]).sum::<f64>()
                    });
                    let edge = 0.00004;
                    let probe = Solid::box_solid([edge; 3], policy)
                        .unwrap()
                        .transformed(RigidTransform {
                            translation_mm: world.map(|v| v - edge / 2.),
                            ..Default::default()
                        })
                        .unwrap();
                    let occupied = imported
                        .boolean(&probe, BooleanOperation::Intersect, policy)
                        .unwrap()
                        .map_or(0., |s| s.volume_mm3().unwrap())
                        / edge.powi(3);
                    assert!(
                        (occupied - f64::from(offset < 0.)).abs() < 0.01,
                        "{family:?}/{internal}/{left} phase={phase} offset={offset} occupied={occupied}"
                    );
                }
            }
        }
    }
}

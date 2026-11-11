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
fn canonical_whitworth_pipe_and_din405_resolve_without_flattening_or_rewriting() {
    for family in [
        ThreadFamily::Bsw,
        ThreadFamily::Bsf,
        ThreadFamily::Bspp,
        ThreadFamily::Round,
    ] {
        for internal in [false, true] {
            for left in [false, true] {
                let resolved = canonical(family, internal, left);
                let t = descriptor(&resolved);
                let s = t.spec();
                assert_eq!(s.major_radius_mm, resolved.major_radius_mm);
                assert_eq!(s.depth_mm, resolved.profile.depth_mm);
                assert_eq!(s.length_mm, resolved.length_mm);
                assert_eq!(s.left_hand, left);
                assert!(t.volume_mm3() > 0.);
                if family == ThreadFamily::Round && internal {
                    assert_ne!(s.crest_radius_mm, s.root_radius_mm);
                    assert_eq!(
                        s.crest_radius_mm,
                        resolved
                            .round
                            .as_ref()
                            .unwrap()
                            .actual_cutter_crest_circle
                            .radius_mm
                    );
                    assert_eq!(
                        s.root_radius_mm,
                        resolved
                            .round
                            .as_ref()
                            .unwrap()
                            .actual_cutter_root_circle
                            .radius_mm
                    );
                }
            }
        }
    }
}
#[cfg(feature = "occt")]
#[test]
fn independent_native_meridian_material_and_finite_bounds_match_literal_resolved_profiles() {
    use multicad_core::document::EntityId;
    use multicad_kernel::{
        BooleanOperation, GeometryPolicy, KnuckleHelicalThread, RigidTransform,
        RoundedHelicalThread, Solid,
    };
    let policy = GeometryPolicy::default();
    for (family, internal) in [
        (ThreadFamily::Bsw, false),
        (ThreadFamily::Bsf, true),
        (ThreadFamily::Bspp, false),
        (ThreadFamily::Round, false),
        (ThreadFamily::Round, true),
    ] {
        for left in [false, true] {
            let resolved = canonical(family, internal, left);
            let t = descriptor(&resolved);
            let s = t.spec();
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
            assert_eq!(native.solid_count().unwrap(), 1);
            let native_volume = native.volume_mm3().unwrap();
            let native_area = native
                .faces()
                .unwrap()
                .iter()
                .map(|face| face.area_mm2)
                .sum::<f64>();
            let volume_budget = policy.linear_tolerance_mm * native_area + 1e-10 * native_volume;
            println!(
                "{family:?}/{internal}/{left}: native={native_volume}, analytic={}, area={native_area}, budget={volume_budget}",
                t.volume_mm3()
            );
            assert!(
                (native_volume - t.volume_mm3()).abs() <= volume_budget,
                "{family:?} internal={internal} left={left}: native={} pure={} spec={s:?}",
                native.volume_mm3().unwrap(),
                t.volume_mm3()
            );
            let b = t.profile_breaks();
            let phases = [
                b[1] / 2.,
                b[1].midpoint(b[2]),
                s.pitch_mm / 2. - s.root_radius_mm * s.flank_angle_deg.to_radians().cos() / 2.,
                s.pitch_mm / 2.,
                b[3].midpoint(b[4]),
            ];
            for phase in phases {
                for theta in [0., 0.8] {
                    let z = phase + s.pitch_mm + t.helical_rate() * theta;
                    let r = t.radius_at_phase(phase);
                    for offset in [-0.001, 0.001] {
                        let point = [(r + offset) * theta.cos(), (r + offset) * theta.sin(), z];
                        assert_eq!(t.contains(point, 0.), offset < 0.);
                        let edge = 0.00004;
                        let probe = Solid::box_solid([edge; 3], policy)
                            .unwrap()
                            .transformed(RigidTransform {
                                translation_mm: point.map(|v| v - edge / 2.),
                                ..Default::default()
                            })
                            .unwrap();
                        let occupied = native
                            .boolean(&probe, BooleanOperation::Intersect, policy)
                            .unwrap()
                            .map_or(0., |s| s.volume_mm3().unwrap())
                            / edge.powi(3);
                        assert!(
                            (occupied - f64::from(offset < 0.)).abs() < 0.01,
                            "{family:?}/{internal}/{left} phase={phase} theta={theta} offset={offset} occupied={occupied}"
                        );
                    }
                }
            }
            for pose in [
                RigidTransform::default(),
                RigidTransform {
                    rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
                    translation_mm: [17., -8., 13.],
                },
            ] {
                let actual = native.transformed(pose).unwrap().bounds().unwrap();
                for i in 0..3 {
                    let upper = t.support_bounds(pose.rotation[i], 1e-7).unwrap();
                    let lower = t
                        .support_bounds(pose.rotation[i].map(|x| -x), 1e-7)
                        .unwrap();
                    assert!(
                        (actual.max[i] - pose.translation_mm[i] - upper[1]).abs()
                            < 8. * policy.linear_tolerance_mm,
                        "{family:?} max{i} {:?} vs {}",
                        upper,
                        actual.max[i] - pose.translation_mm[i]
                    );
                    assert!(
                        (actual.min[i] - pose.translation_mm[i] + lower[1]).abs()
                            < 8. * policy.linear_tolerance_mm
                    );
                }
            }
        }
    }
}

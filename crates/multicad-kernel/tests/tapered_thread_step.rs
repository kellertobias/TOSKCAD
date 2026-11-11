//! Independent Native readback of the owned actual-transport STEP surfaces.
#![cfg(feature = "occt")]
use multicad_core::document::EntityId;
use multicad_kernel::rust_kernel::{helical_thread::Spec, step, tapered_thread::Thread};
use multicad_kernel::{BooleanOperation, GeometryPolicy, HelicalThread, RigidTransform, Solid};

#[test]
fn actual_taper_step_matches_independent_native_material() {
    let policy = GeometryPolicy::default();
    for (k, left_hand, starts) in [
        (0.1, false, 1),
        (-0.1, true, 1),
        (-0.03125, false, 3),
        (0.03125, true, 3),
    ] {
        let spec = Spec {
            major_radius_mm: 4.,
            pitch_mm: 1.25,
            length_mm: 8.,
            depth_mm: 0.5,
            crest_width_mm: 0.25,
            lower_flank_deg: 12.,
            upper_flank_deg: 18.,
            starts,
            left_hand,
            root_radius_mm: None,
        };
        let owned = Thread::new(
            spec,
            k,
            policy.linear_tolerance_mm,
            policy.minimum_feature_mm,
        )
        .unwrap();
        let text = step::write_tapered_thread(
            &owned,
            [0.; 3],
            [1., 0., 0.],
            [0., 1., 0.],
            policy.linear_tolerance_mm,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transport.step");
        std::fs::write(&path, text).unwrap();
        let imported = Solid::read_step(&path, policy).unwrap();
        assert_eq!(imported.solid_count().unwrap(), 1);
        let native = Solid::helical_thread_named_fresh(
            HelicalThread {
                major_radius_mm: spec.major_radius_mm,
                pitch_mm: spec.pitch_mm,
                length_mm: spec.length_mm,
                depth_mm: spec.depth_mm,
                crest_width_mm: spec.crest_width_mm,
                lower_flank_deg: spec.lower_flank_deg,
                upper_flank_deg: spec.upper_flank_deg,
                radial_taper: k,
                starts,
                left_hand,
            },
            EntityId::new(),
            policy,
        )
        .unwrap();
        let expected = native.volume_mm3().unwrap();
        let actual = imported.volume_mm3().unwrap();
        assert!(
            (actual - expected).abs() < 1e-4,
            "k{k} left{left_hand} starts{starts}: actual{actual} expected{expected}"
        );
        for (z, phase) in [(2., 0.21), (6., 1.04)] {
            let [lo, hi] = owned.radial_bounds(phase, z, 1e-10).unwrap();
            let radius = (lo + hi) * 0.5;
            let rate = if left_hand { -1. } else { 1. } * spec.pitch_mm * starts as f64
                / std::f64::consts::TAU;
            let theta = (z - phase) / rate;
            for (offset, inside) in [(-0.0003, true), (0.0003, false)] {
                let radius = radius + offset;
                let edge: f64 = 0.00005;
                let cube = Solid::box_solid([edge; 3], policy)
                    .unwrap()
                    .transformed(RigidTransform {
                        translation_mm: [
                            radius * theta.cos() - edge / 2.,
                            radius * theta.sin() - edge / 2.,
                            z - edge / 2.,
                        ],
                        ..Default::default()
                    })
                    .unwrap();
                for source in [&native, &imported] {
                    let volume = source
                        .boolean(&cube, BooleanOperation::Intersect, policy)
                        .unwrap()
                        .map_or(0., |s| s.volume_mm3().unwrap());
                    assert!(
                        if inside {
                            volume / edge.powi(3) > 0.99
                        } else {
                            volume == 0.
                        },
                        "k{k} starts{starts} z{z} phase{phase} inside{inside} volume{volume}"
                    );
                }
            }
        }
    }
}

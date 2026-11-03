use multicad_core::{document::EntityId, references::TopologyKind};
use multicad_kernel::{
    BooleanOperation, GeometryErrorKind, GeometryPolicy, HelicalThread, MeshOptions, PlaneFrame,
    RigidTransform, Solid,
};
use std::f64::consts::{PI, TAU};

fn metric() -> HelicalThread {
    HelicalThread {
        major_radius_mm: 4.,
        pitch_mm: 1.25,
        length_mm: 8.,
        depth_mm: 5. * 3_f64.sqrt() / 16. * 1.25,
        crest_width_mm: 1.25 / 8.,
        lower_flank_deg: 30.,
        upper_flank_deg: 30.,
        radial_taper: 0.,
        starts: 1,
        left_hand: false,
    }
}
fn build(spec: HelicalThread) -> Solid {
    Solid::helical_thread_named(spec, EntityId::new(), GeometryPolicy::default()).unwrap()
}
#[test]
fn metric_complete_turn_and_fractional_spines_produce_measurable_accurate_solids() {
    for (diameter, pitch, length) in [(3., 0.5, 6.), (3., 0.5, 10.), (2.5, 0.45, 12.)] {
        let spec = HelicalThread {
            major_radius_mm: diameter / 2.,
            pitch_mm: pitch,
            length_mm: length,
            depth_mm: 5. * 3_f64.sqrt() / 16. * pitch,
            crest_width_mm: pitch / 8.,
            ..metric()
        };
        let solid = build(spec);
        let actual = solid.volume_mm3().unwrap();
        let expected = expected_volume(spec);
        let area: f64 = solid
            .faces()
            .unwrap()
            .iter()
            .map(|face| face.area_mm2)
            .sum();
        // The same linear geometry policy and unchanged GK relative-error gate
        // bound measurement uncertainty; this does not relax legacy thresholds.
        let uncertainty =
            GeometryPolicy::default().linear_tolerance_mm * area + 1e-7 * actual.abs();
        assert!(
            (actual - expected).abs() <= uncertainty,
            "M{diameter} length {length}: native {actual}, analytic {expected}, budget {uncertainty}"
        );
        let bounds = solid.bounds().unwrap();
        assert!(bounds.min[2].abs() < 1e-6);
        assert!((bounds.max[2] - length).abs() < 1e-6);
        let radius = spec.major_radius_mm - spec.depth_mm * 0.05;
        for z in [length * 0.19, length * 0.47, length * 0.83] {
            let angle = TAU * z / pitch;
            assert!(probe(&solid, [radius * angle.cos(), radius * angle.sin(), z]) > 0.999);
            assert!(probe(&solid, [-radius * angle.cos(), -radius * angle.sin(), z]) < 1e-5);
        }
    }
}
#[test]
fn short_unf_internal_tools_retain_helices_at_nonintegral_lengths() {
    let pitch = 25.4 / 28.;
    let h = 3_f64.sqrt() * pitch / 2.;
    let pitch_diameter = (0.2268 + 0.2311) / 2. * 25.4;
    for length in [2.5, 2.4, 2.6, 3., 3.2, pitch, 4. * pitch] {
        let spec = HelicalThread {
            major_radius_mm: (pitch_diameter + 3. * h / 4.) / 2.,
            pitch_mm: pitch,
            length_mm: length,
            depth_mm: 5. * h / 8.,
            crest_width_mm: pitch / 8.,
            ..metric()
        };
        let feature = EntityId::new();
        let solid = Solid::helical_thread_named(spec, feature, Default::default())
            .unwrap_or_else(|e| panic!("UNF internal-tool length {length}: {e}"));
        assert!((solid.volume_mm3().unwrap() - expected_volume(spec)).abs() < 1e-3);
        let bounds = solid.bounds().unwrap();
        assert!(bounds.min[2].abs() < 1e-6);
        assert!((bounds.max[2] - length).abs() < 1e-6);
        let topology = solid.topology_snapshot(feature, 0).unwrap();
        for role in [
            "cap.start",
            "cap.end",
            "thread.root",
            "thread/start/0/lower",
            "thread/start/0/crest",
            "thread/start/0/upper",
        ] {
            assert!(
                topology
                    .elements
                    .iter()
                    .any(|e| e.key.kind == TopologyKind::Face
                        && e.origins
                            .iter()
                            .any(|o| o.source == feature && o.role == role)),
                "missing {role} at length {length}"
            );
        }
        let r = spec.major_radius_mm - 0.05 * spec.depth_mm;
        for z in [length * 0.23, length * 0.47, length * 0.71] {
            let angle = TAU * z / pitch;
            assert!(probe(&solid, [r * angle.cos(), r * angle.sin(), z]) > 0.999);
            assert!(probe(&solid, [-r * angle.cos(), -r * angle.sin(), z]) < 1e-5);
        }
    }
}
fn probe(solid: &Solid, center: [f64; 3]) -> f64 {
    let size = 0.005;
    let cube = Solid::box_solid([size; 3], GeometryPolicy::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: center.map(|v| v - size / 2.),
            ..Default::default()
        })
        .unwrap();
    solid
        .boolean(
            &cube,
            BooleanOperation::Intersect,
            GeometryPolicy::default(),
        )
        .unwrap()
        .map_or(0., |shape| shape.volume_mm3().unwrap())
        / size.powi(3)
}
fn expected_volume(s: HelicalThread) -> f64 {
    let r = s.major_radius_mm;
    let root = r - s.depth_mm;
    let slopes = s.lower_flank_deg.to_radians().tan() + s.upper_flank_deg.to_radians().tan();
    let integral = s.crest_width_mm * (r * r - root * root) / 2.
        + slopes * (r * (r * r - root * root) / 2. - (r.powi(3) - root.powi(3)) / 3.);
    s.length_mm * (PI * root * root + TAU / s.pitch_mm * integral)
}
#[test]
fn metric_thread_retains_actual_axial_profile_dimensions_and_volume() {
    let spec = metric();
    let solid = build(spec);
    assert_eq!(solid.solid_count().unwrap(), 1);
    let bounds = solid.bounds().unwrap();
    for axis in 0..2 {
        assert!((bounds.min[axis] + 4.).abs() < 2e-5);
        assert!((bounds.max[axis] - 4.).abs() < 2e-5);
    }
    assert!(bounds.min[2].abs() < 1e-6);
    assert!((bounds.max[2] - 8.).abs() < 1e-6);
    assert!((solid.volume_mm3().unwrap() - expected_volume(spec)).abs() < 1e-3);
    // At the basic pitch radius, the axial tooth width is exactly P/2.
    let pitch_radius = 4. - 3. * 3_f64.sqrt() / 16. * 1.25;
    assert!(probe(&solid, [pitch_radius, 0., 2. * 1.25 + 0.20 * 1.25]) > 0.999);
    assert!(probe(&solid, [pitch_radius, 0., 2. * 1.25 + 0.30 * 1.25]) < 1e-5);
    assert!(probe(&solid, [4. - spec.depth_mm - 0.02, 0., 3.125]) > 0.999);
    assert!(probe(&solid, [4.02, 0., 2.5]) < 1e-5);
}
#[test]
fn handedness_and_multiple_starts_follow_the_requested_lead_with_complete_face_ancestry() {
    for (starts, left_hand) in [(1, false), (1, true), (3, false), (2, true)] {
        let spec = HelicalThread {
            starts,
            left_hand,
            ..metric()
        };
        let feature = EntityId::new();
        let solid = Solid::helical_thread_named(spec, feature, GeometryPolicy::default()).unwrap();
        assert_eq!(spec.lead_mm(), spec.pitch_mm * f64::from(starts));
        assert!((solid.volume_mm3().unwrap() - expected_volume(spec)).abs() < 1e-3);
        let z = 3.37;
        let sign = if left_hand { -1. } else { 1. };
        for start in 0..starts {
            let theta =
                sign * TAU * z / spec.lead_mm() + TAU * f64::from(start) / f64::from(starts);
            let r = spec.major_radius_mm - spec.depth_mm * 0.1;
            assert!(probe(&solid, [r * theta.cos(), r * theta.sin(), z]) > 0.999);
            let gap = theta + PI / f64::from(starts);
            assert!(probe(&solid, [r * gap.cos(), r * gap.sin(), z]) < 1e-5);
        }
        let topology = solid.topology_snapshot(feature, 0).unwrap();
        let faces: Vec<_> = topology
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
            .collect();
        assert!(faces.len() > 3);
        assert!(faces.iter().all(|face| !face.origins.is_empty()));
        for role in ["cap.start", "cap.end", "thread.root"] {
            assert!(faces.iter().any(|face| {
                face.origins
                    .iter()
                    .any(|o| o.source == feature && o.role == role)
            }));
        }
        for start in 0..starts {
            for side in ["lower", "crest", "upper"] {
                let role = format!("thread/start/{start}/{side}");
                assert!(faces.iter().any(|face| {
                    face.origins
                        .iter()
                        .any(|o| o.source == feature && o.role == role)
                }));
            }
        }
    }
}
#[test]
fn receiving_body_cut_and_mating_clearance_are_real_native_geometry() {
    let male = build(metric());
    let tool = build(HelicalThread {
        major_radius_mm: 4.1,
        ..metric()
    });
    let blank = Solid::cylinder(PlaneFrame::default(), 6., 8., GeometryPolicy::default()).unwrap();
    let receiver = blank
        .boolean(&tool, BooleanOperation::Subtract, GeometryPolicy::default())
        .unwrap()
        .unwrap();
    assert_eq!(receiver.solid_count().unwrap(), 1);
    assert!(receiver.volume_mm3().unwrap() < blank.volume_mm3().unwrap() - 300.);
    assert!(
        male.boolean(
            &receiver,
            BooleanOperation::Intersect,
            GeometryPolicy::default()
        )
        .unwrap()
        .is_none()
    );
    assert!(probe(&receiver, [4.3, 0., 3.]) > 0.999);
    assert!(probe(&receiver, [0., 0., 3.]) < 1e-5);
}
#[test]
fn custom_conical_threads_and_step_mesh_export_retain_physical_helices() {
    for radial_taper in [-0.03125, 0.03125] {
        let spec = HelicalThread {
            radial_taper,
            ..metric()
        };
        let solid = build(spec);
        assert_eq!(solid.solid_count().unwrap(), 1);
        let z = 4.;
        let theta = TAU * z / spec.lead_mm();
        let r = 4. + radial_taper * z - 0.05;
        assert!(probe(&solid, [r * theta.cos(), r * theta.sin(), z]) > 0.999);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("modeled-thread.step");
        solid.write_step(&path).unwrap();
        let reopened = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        assert_eq!(reopened.solid_count().unwrap(), 1);
        assert!((reopened.volume_mm3().unwrap() - solid.volume_mm3().unwrap()).abs() < 1e-3);
        let mesh = reopened
            .mesh(MeshOptions {
                linear_deflection_mm: 0.02,
                ..Default::default()
            })
            .unwrap();
        assert!(mesh.indices.len() > 300);
        assert!(
            reopened
                .topology_snapshot(EntityId::new(), 0)
                .unwrap()
                .elements
                .iter()
                .filter(|e| e.key.kind == TopologyKind::Face)
                .count()
                > 3
        );
    }
}
#[test]
fn invalid_profiles_nonfinite_inputs_and_unbounded_work_are_rejected_before_native_generation() {
    let cases = [
        HelicalThread {
            starts: 0,
            ..metric()
        },
        HelicalThread {
            starts: 9,
            ..metric()
        },
        HelicalThread {
            pitch_mm: f64::NAN,
            ..metric()
        },
        HelicalThread {
            radial_taper: f64::INFINITY,
            ..metric()
        },
        HelicalThread {
            length_mm: 0.5,
            ..metric()
        },
        HelicalThread {
            length_mm: 1000.,
            ..metric()
        },
        HelicalThread {
            depth_mm: 4.,
            ..metric()
        },
        HelicalThread {
            crest_width_mm: 1.2,
            ..metric()
        },
        HelicalThread {
            lower_flank_deg: 0.,
            ..metric()
        },
        HelicalThread {
            upper_flank_deg: 90.,
            ..metric()
        },
        HelicalThread {
            radial_taper: 0.2,
            ..metric()
        },
    ];
    for spec in cases {
        let error = Solid::helical_thread_named(spec, EntityId::new(), GeometryPolicy::default())
            .err()
            .unwrap();
        assert!(
            matches!(
                error.kind,
                GeometryErrorKind::Input | GeometryErrorKind::Limit
            ),
            "{error}"
        );
    }
    assert!(
        Solid::helical_thread_named(metric(), EntityId::nil(), GeometryPolicy::default()).is_err()
    );
}

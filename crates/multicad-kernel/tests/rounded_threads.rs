use multicad_core::{document::EntityId, references::TopologyKind};
use multicad_kernel::{
    BooleanOperation, GeometryPolicy, MeshOptions, RigidTransform, RoundedHelicalThread, Solid,
};

fn spec() -> RoundedHelicalThread {
    let pitch = 25.4 / 20.;
    let angle = 27.5_f64.to_radians();
    RoundedHelicalThread {
        major_radius_mm: 3.175,
        pitch_mm: pitch,
        length_mm: 3.7 * pitch,
        depth_mm: pitch / (3. * angle.tan()),
        rounding_radius_mm: pitch * angle.cos() / (12. * (1. - angle.sin())),
        flank_angle_deg: 27.5,
        starts: 1,
        left_hand: false,
    }
}
fn material(solid: &Solid, point: [f64; 3], size: f64) -> f64 {
    let cube = Solid::box_solid([size; 3], Default::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: point.map(|v| v - size / 2.),
            ..Default::default()
        })
        .unwrap();
    solid
        .boolean(&cube, BooleanOperation::Intersect, Default::default())
        .unwrap()
        .map_or(0., |s| s.volume_mm3().unwrap())
        / size.powi(3)
}
fn expected_volume(s: RoundedHelicalThread, n: usize) -> f64 {
    let angle = 27.5_f64.to_radians();
    let r = s.rounding_radius_mm;
    let a = r * angle.cos();
    let drop = r * (1. - angle.sin());
    let radius = |z: f64| {
        let z = z.abs();
        if z <= a {
            s.major_radius_mm - r + (r * r - z * z).sqrt()
        } else if z >= s.pitch_mm / 2. - a {
            let offset = z - s.pitch_mm / 2.;
            s.major_radius_mm - s.depth_mm + r - (r * r - offset * offset).sqrt()
        } else {
            s.major_radius_mm - drop - (z - a) / angle.tan()
        }
    };
    // Independent quadrature of the analytic circular/linear meridian,
    // never a polygon or display mesh used to manufacture the native solid.
    let sum = (0..=n)
        .map(|i| {
            let weight = if i == 0 || i == n {
                1.
            } else if i % 2 == 0 {
                2.
            } else {
                4.
            };
            weight * radius(s.pitch_mm * (i as f64 / n as f64 - 0.5)).powi(2)
        })
        .sum::<f64>();
    std::f64::consts::PI * s.length_mm * sum / (3. * n as f64)
}
fn check_circles(solid: &Solid, s: RoundedHelicalThread) {
    let r = s.rounding_radius_mm;
    let half = r * 27.5_f64.to_radians().cos();
    let offset = (r * 0.02).min(0.003);
    let cube = offset * 0.2;
    for fraction in [0.4, 0.65, 0.85] {
        for sign in [-1., 1.] {
            let dz = sign * half * fraction;
            let crest = s.major_radius_mm - r + (r * r - dz * dz).sqrt();
            let valley = s.major_radius_mm - s.depth_mm + r - (r * r - dz * dz).sqrt();
            for (z, radius) in [(s.pitch_mm + dz, crest), (1.5 * s.pitch_mm + dz, valley)] {
                assert!(
                    material(solid, [radius - offset, 0., z], cube) > 0.999,
                    "missing material at circle {z}/{radius}"
                );
                assert!(
                    material(solid, [radius + offset, 0., z], cube) < 1e-5,
                    "excess material at circle {z}/{radius}"
                );
            }
        }
    }
}
#[test]
fn rounded_crest_and_root_are_true_axial_circles_with_native_ancestry_and_step_geometry() {
    let dir = tempfile::tempdir().unwrap();
    for left_hand in [false, true] {
        let s = RoundedHelicalThread {
            left_hand,
            ..spec()
        };
        let feature = EntityId::new();
        let solid = Solid::rounded_helical_thread_named(s, feature, Default::default()).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        check_circles(&solid, s);
        let topology = solid.topology_snapshot(feature, 0).unwrap();
        let faces: Vec<_> = topology
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
            .collect();
        assert!(faces.iter().all(|f| !f.origins.is_empty()));
        for role in [
            "cap.start",
            "cap.end",
            "thread.root",
            "thread/start/0/crest",
            "thread/start/0/lower",
            "thread/start/0/upper",
            "thread/start/0/root/lower",
            "thread/start/0/root/upper",
        ] {
            assert!(
                faces.iter().any(|f| f
                    .origins
                    .iter()
                    .any(|o| o.source == feature && o.role == role)),
                "missing {role}"
            );
        }
        let bounds = solid.bounds().unwrap();
        assert!(bounds.min[2].abs() < 1e-6, "bounds={bounds:?}");
        assert!(
            (bounds.max[2] - s.length_mm).abs() < 1e-6,
            "bounds={bounds:?}"
        );
        assert!(
            !solid
                .mesh(MeshOptions::default())
                .unwrap()
                .indices
                .is_empty()
        );
        let path = dir.path().join(format!("rounded-{left_hand}.step"));
        solid.write_step(&path).unwrap();
        let readback = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        check_circles(&readback, s);
        let area = solid
            .faces()
            .unwrap()
            .iter()
            .map(|f| f.area_mm2)
            .sum::<f64>()
            + readback
                .faces()
                .unwrap()
                .iter()
                .map(|f| f.area_mm2)
                .sum::<f64>();
        let before = solid.volume_mm3().unwrap();
        let after = readback.volume_mm3().unwrap();
        assert!(
            (before - after).abs()
                <= GeometryPolicy::default().linear_tolerance_mm * area + 1e-7 * (before + after)
        );
        let expected = expected_volume(s, 16384);
        let quadrature = (expected - expected_volume(s, 8192)).abs();
        for shape in [&solid, &readback] {
            let volume = shape.volume_mm3().unwrap();
            let area = shape
                .faces()
                .unwrap()
                .iter()
                .map(|f| f.area_mm2)
                .sum::<f64>();
            assert!(
                (volume - expected).abs()
                    <= GeometryPolicy::default().linear_tolerance_mm * area
                        + 1e-7 * volume
                        + quadrature
            );
            let bounds = shape.bounds().unwrap();
            for axis in [0, 1] {
                assert!((bounds.min[axis] + s.major_radius_mm).abs() < 1e-6);
                assert!((bounds.max[axis] - s.major_radius_mm).abs() < 1e-6);
            }
            assert!(bounds.min[2].abs() < 1e-6);
            assert!((bounds.max[2] - s.length_mm).abs() < 1e-6);
        }
        assert_eq!(
            solid.share().unwrap().bounds().unwrap(),
            solid.bounds().unwrap()
        );
        let reopened = Solid::from_brep(&solid.to_brep().unwrap(), Default::default()).unwrap();
        assert!((reopened.bounds().unwrap().min[1] + s.major_radius_mm).abs() < 1e-6);
        assert!(reopened.bounds().unwrap().min[2].abs() < 1e-6);
        let moved = solid
            .transformed(RigidTransform {
                rotation: [[0., 0., 1.], [1., 0., 0.], [0., 1., 0.]],
                translation_mm: [10., 20., 30.],
            })
            .unwrap();
        let moved_bounds = moved.bounds().unwrap();
        assert!((moved_bounds.min[0] - 10.).abs() < 1e-6);
        assert!((moved_bounds.max[0] - 10. - s.length_mm).abs() < 1e-6);
        assert!((moved_bounds.min[1] - 20. + s.major_radius_mm).abs() < 1e-6);
        let limit = Solid::box_solid([10., 10., s.length_mm / 2.], Default::default())
            .unwrap()
            .transformed(RigidTransform {
                translation_mm: [-5., -5., 0.],
                ..Default::default()
            })
            .unwrap();
        let clipped = solid
            .boolean(&limit, BooleanOperation::Intersect, Default::default())
            .unwrap()
            .unwrap();
        assert!((clipped.bounds().unwrap().max[2] - s.length_mm / 2.).abs() < 1e-6);
    }
}
#[test]
fn unsupported_or_inconsistent_rounded_profiles_are_refused() {
    for s in [
        RoundedHelicalThread {
            starts: 2,
            ..spec()
        },
        RoundedHelicalThread {
            flank_angle_deg: 30.,
            ..spec()
        },
        RoundedHelicalThread {
            depth_mm: 0.5,
            ..spec()
        },
        RoundedHelicalThread {
            rounding_radius_mm: 0.,
            ..spec()
        },
        RoundedHelicalThread {
            length_mm: 0.1,
            ..spec()
        },
    ] {
        assert!(
            Solid::rounded_helical_thread_named(s, EntityId::new(), Default::default()).is_err()
        );
    }
}

#[test]
fn fitted_bsf_inch_pitch_preserves_the_exact_fractional_endpoint() {
    // Exact resolved native inputs from the typed 1/4 inch, 1/26 inch MCP
    // request. In particular, do not replace the pitch with 25.4 / 26 or
    // normalize the independently authored length to an integral turn count.
    let s = RoundedHelicalThread {
        major_radius_mm: 3.137_712_647_516_242,
        pitch_mm: 0.976_923_076_923_076_8,
        length_mm: 2.930_769_230_769_230_6,
        depth_mm: 0.625_550_590_064_969_3,
        rounding_radius_mm: 0.134_159_950_430_831_56,
        flank_angle_deg: 27.5,
        starts: 1,
        left_hand: false,
    };
    let solid =
        Solid::rounded_helical_thread_named(s, EntityId::new(), Default::default()).unwrap();
    assert_eq!(solid.solid_count().unwrap(), 1);
    check_circles(&solid, s);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bsf-inch.step");
    solid.write_step(&path).unwrap();
    let readback = Solid::read_step(&path, Default::default()).unwrap();
    check_circles(&readback, s);
    let expected = expected_volume(s, 16384);
    for shape in [&solid, &readback] {
        let actual = shape.volume_mm3().unwrap();
        let area = shape
            .faces()
            .unwrap()
            .iter()
            .map(|f| f.area_mm2)
            .sum::<f64>();
        assert!(
            (actual - expected).abs()
                <= 1e-7 * actual + GeometryPolicy::default().linear_tolerance_mm * area
        );
        assert!((shape.bounds().unwrap().max[2] - s.length_mm).abs() < 1e-6);
    }
}

#[test]
fn all_sourced_fits_survive_tpi_units_integer_fractional_lengths_and_allowances() {
    use multicad_core::{
        commands::Session,
        constraint_sketch::Dimension,
        document::Document,
        model::{self, ThreadFamily, ThreadFit, ThreadHand, ThreadMode, ThreadParameters, length},
    };
    let session = Session::new(Document::new("Rounded fit native matrix").unwrap()).unwrap();
    for preset in model::whitworth_thread_presets() {
        for internal in [false, true] {
            let fit = match (preset.family == ThreadFamily::Bspp, internal) {
                (false, false) => ThreadFit::WhitworthMediumExternal,
                (false, true) => ThreadFit::WhitworthMediumInternal,
                (true, false) => ThreadFit::BsppClassBExternal,
                (true, true) => ThreadFit::BsppInternal,
            };
            for (turns, allowance, left_hand) in [(3., 0., false), (3.25, 0.0001, true)] {
                let parameters = ThreadParameters {
                    family: preset.family,
                    fit,
                    major_diameter: preset.nominal_diameter_inch.map_or_else(
                        || length(preset.diameter_mm),
                        |value| Dimension::Literal {
                            value,
                            unit: "in".into(),
                        },
                    ),
                    pitch: Dimension::Literal {
                        value: 1. / f64::from(preset.threads_per_inch),
                        unit: "in".into(),
                    },
                    length: length(turns * preset.pitch_mm),
                    radial_allowance: length(allowance),
                    hand: if left_hand {
                        ThreadHand::Left
                    } else {
                        ThreadHand::Right
                    },
                    ..Default::default()
                };
                let resolved = parameters
                    .resolve(
                        session.parameters(),
                        if internal {
                            ThreadMode::CutInternal
                        } else {
                            ThreadMode::NewExternal
                        },
                    )
                    .unwrap();
                let s = RoundedHelicalThread {
                    major_radius_mm: resolved.major_radius_mm,
                    pitch_mm: resolved.pitch_mm,
                    length_mm: resolved.length_mm,
                    depth_mm: resolved.profile.depth_mm,
                    rounding_radius_mm: resolved.profile.rounding_radius_mm.unwrap(),
                    flank_angle_deg: 27.5,
                    starts: 1,
                    left_hand,
                };
                let solid = Solid::rounded_helical_thread_named(
                    s,
                    EntityId::new(),
                    Default::default(),
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "{} internal={internal} allowance={allowance} spec={s:.17?}: {error}",
                        preset.designation
                    )
                });
                assert_eq!(solid.solid_count().unwrap(), 1);
                let expected = expected_volume(s, 16384);
                let actual = solid.volume_mm3().unwrap();
                let area = solid
                    .faces()
                    .unwrap()
                    .iter()
                    .map(|f| f.area_mm2)
                    .sum::<f64>();
                let budget = GeometryPolicy::default().linear_tolerance_mm * area + 1e-7 * actual;
                assert!(
                    (actual - expected).abs() <= budget,
                    "{} spec={s:?} actual={actual} expected={expected} budget={budget}",
                    preset.designation
                );
                assert!((solid.bounds().unwrap().max[2] - s.length_mm).abs() < 1e-6);
            }
        }
    }
}

#[test]
fn fitted_left_bsw_fractional_preserves_diagonal_material_and_step_geometry() {
    let s = RoundedHelicalThread {
        major_radius_mm: 3.924_969_805_903_716,
        pitch_mm: 1.411_111_111_111_111,
        length_mm: 4.586_111_111_111_11,
        depth_mm: 0.903_573_074_538_289_1,
        rounding_radius_mm: 0.193_786_595_066_756_72,
        flank_angle_deg: 27.5,
        starts: 1,
        left_hand: true,
    };
    let solid =
        Solid::rounded_helical_thread_named(s, EntityId::new(), Default::default()).unwrap();
    check_circles(&solid, s);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bsw-left-fractional.step");
    solid.write_step(&path).unwrap();
    let readback = Solid::read_step(&path, Default::default()).unwrap();
    for shape in [&solid, &readback] {
        check_circles(shape, s);
        for turns in [0.875, 1.875] {
            let z = turns * s.pitch_mm;
            let angle = -2. * std::f64::consts::PI * z / s.pitch_mm;
            let radius =
                s.major_radius_mm - 0.1 * 2. * s.rounding_radius_mm * 27.5_f64.to_radians().cos();
            assert!(
                material(
                    shape,
                    [radius * angle.cos(), radius * angle.sin(), z],
                    0.001
                ) > 0.999
            );
        }
        let bounds = shape.bounds().unwrap();
        for axis in 0..2 {
            assert!((bounds.min[axis] + s.major_radius_mm).abs() < 1e-6);
            assert!((bounds.max[axis] - s.major_radius_mm).abs() < 1e-6);
        }
        assert!(bounds.min[2].abs() < 1e-6);
        assert!((bounds.max[2] - s.length_mm).abs() < 1e-6);
    }
}

#[test]
fn fitted_bspp_internal_preserves_native_circles_and_step_geometry() {
    let s = RoundedHelicalThread {
        major_radius_mm: 4.890_717_101_265_082,
        pitch_mm: 0.907_142_857_142_857,
        length_mm: 2.721_428_571_428_571_5,
        depth_mm: 0.580_868_405_060_328_7,
        rounding_radius_mm: 0.124_577_096_828_629_3,
        flank_angle_deg: 27.5,
        starts: 1,
        left_hand: false,
    };
    let solid =
        Solid::rounded_helical_thread_named(s, EntityId::new(), Default::default()).unwrap();
    check_circles(&solid, s);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bspp-internal.step");
    solid.write_step(&path).unwrap();
    let readback = Solid::read_step(&path, Default::default()).unwrap();
    check_circles(&readback, s);
    let bounds = readback.bounds().unwrap();
    for axis in 0..2 {
        assert!((bounds.min[axis] + s.major_radius_mm).abs() < 1e-6);
        assert!((bounds.max[axis] - s.major_radius_mm).abs() < 1e-6);
    }
    assert!(bounds.min[2].abs() < 1e-6);
    assert!((bounds.max[2] - s.length_mm).abs() < 1e-6);
}

#[test]
fn blind_and_through_rounded_receivers_keep_rigid_geometry_ancestry_and_step_bounds() {
    use multicad_core::{
        commands::Session,
        constraint_sketch::Dimension,
        document::Document,
        model::{ThreadFamily, ThreadFit, ThreadMode, ThreadParameters, length},
    };
    use multicad_kernel::PlaneFrame;
    let session = Session::new(Document::new("Rounded receiver representation").unwrap()).unwrap();
    let parameters = ThreadParameters {
        family: ThreadFamily::Bsw,
        fit: ThreadFit::WhitworthMediumInternal,
        major_diameter: Dimension::Literal {
            value: 0.25,
            unit: "in".into(),
        },
        pitch: Dimension::Literal {
            value: 1. / 20.,
            unit: "in".into(),
        },
        length: length(3. * (25.4 / 20.)),
        ..Default::default()
    };
    let native_spec = |p: multicad_core::model::ResolvedThread| RoundedHelicalThread {
        major_radius_mm: p.major_radius_mm,
        pitch_mm: p.pitch_mm,
        length_mm: p.length_mm,
        depth_mm: p.profile.depth_mm,
        rounding_radius_mm: p.profile.rounding_radius_mm.unwrap(),
        flank_angle_deg: 27.5,
        starts: 1,
        left_hand: false,
    };
    let s = native_spec(
        parameters
            .resolve(session.parameters(), ThreadMode::CutInternal)
            .unwrap(),
    );
    let male_spec = native_spec(
        ThreadParameters {
            fit: ThreadFit::WhitworthMediumExternal,
            ..parameters
        }
        .resolve(session.parameters(), ThreadMode::NewExternal)
        .unwrap(),
    );
    let feature = EntityId::new();
    let local_cutter = Solid::rounded_helical_thread_named(s, feature, Default::default()).unwrap();
    let shared_cutter = local_cutter.share().unwrap();
    let original_bounds = local_cutter.bounds().unwrap();
    let original_volume = local_cutter.volume_mm3().unwrap();
    let original_topology = local_cutter.topology_snapshot(feature, 0).unwrap();
    let male = Solid::rounded_helical_thread_named(male_spec, EntityId::new(), Default::default())
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let point = |pose: RigidTransform, p: [f64; 3]| -> [f64; 3] {
        std::array::from_fn(|i| {
            pose.translation_mm[i] + (0..3).map(|j| pose.rotation[i][j] * p[j]).sum::<f64>()
        })
    };
    for blind in [false, true] {
        let height = if blind { 5.08 } else { 3.81 };
        for (index, pose) in [
            RigidTransform::default(),
            RigidTransform {
                rotation: [[0.36, -0.8, 0.48], [0.48, 0.6, 0.64], [-0.8, 0., 0.6]],
                translation_mm: [9., -3., 7.],
            },
        ]
        .into_iter()
        .enumerate()
        {
            // This identity transform reproduces the real model bridge too.
            // Copying geometry used to produce a different STEP trim domain.
            let cutter = shared_cutter.transformed(pose).unwrap();
            let located_male = male.transformed(pose).unwrap();
            let topology = cutter.topology_snapshot(feature, 0).unwrap();
            assert!(
                topology
                    .elements
                    .iter()
                    .filter(|e| e.key.kind == TopologyKind::Face)
                    .all(|e| e.origins.iter().any(|o| o.source == feature))
            );
            let frame = PlaneFrame {
                origin: pose.translation_mm,
                u: std::array::from_fn(|i| pose.rotation[i][0]),
                v: std::array::from_fn(|i| pose.rotation[i][1]),
            };
            let stock = Solid::cylinder(frame, 6.35, height, Default::default())
                .unwrap()
                .with_member_origin(EntityId::new(), 0)
                .unwrap();
            let shared_stock = stock.share().unwrap();
            let stock_bounds = stock.bounds().unwrap();
            let stock_volume = stock.volume_mm3().unwrap();
            let receiver = stock
                .boolean(&cutter, BooleanOperation::Subtract, Default::default())
                .unwrap()
                .unwrap();
            let names = receiver.topology_snapshot(feature, 0).unwrap();
            assert!(
                names
                    .elements
                    .iter()
                    .filter(|e| e.key.kind == TopologyKind::Face)
                    .all(|e| !e.origins.is_empty())
            );
            assert!(names.elements.iter().any(|e| {
                e.origins
                    .iter()
                    .any(|o| o.source == feature && o.role == "thread.root")
            }));
            let path = dir.path().join(format!("receiver-{blind}-{index}.step"));
            receiver.write_step(&path).unwrap();
            let readback = Solid::read_step(&path, Default::default()).unwrap();
            let expected =
                std::f64::consts::PI * 6.35_f64.powi(2) * height - expected_volume(s, 16384);
            for shape in [&receiver, &readback] {
                assert_eq!(shape.solid_count().unwrap(), 1);
                let bounds = shape.bounds().unwrap();
                for i in 0..3 {
                    let n = pose.rotation[i][2];
                    let center = pose.translation_mm[i] + n * height / 2.;
                    let half = n.abs() * height / 2. + 6.35 * (1. - n * n).sqrt();
                    assert!((bounds.min[i] - (center - half)).abs() < 1e-6, "{bounds:?}");
                    assert!((bounds.max[i] - (center + half)).abs() < 1e-6, "{bounds:?}");
                }
                let volume = shape.volume_mm3().unwrap();
                let area = shape
                    .faces()
                    .unwrap()
                    .iter()
                    .map(|f| f.area_mm2)
                    .sum::<f64>();
                assert!(
                    (volume - expected).abs()
                        <= GeometryPolicy::default().linear_tolerance_mm * area + 1e-7 * volume
                );
                assert!(
                    shape
                        .boolean(
                            &located_male,
                            BooleanOperation::Intersect,
                            Default::default()
                        )
                        .unwrap()
                        .is_none()
                );
                for (p, inside) in [
                    ([0., 0., 1.27], false),
                    ([6.34, 0., 1.27], true),
                    ([6.36, 0., 1.27], false),
                    ([0., 0., 4.445], blind),
                ] {
                    let fraction = material(shape, point(pose, p), 0.001);
                    assert!(
                        if inside {
                            fraction > 0.999
                        } else {
                            fraction < 0.001
                        },
                        "{p:?}: {fraction}"
                    );
                }
                let r = s.rounding_radius_mm;
                let dz = 0.65 * r * 27.5_f64.to_radians().cos();
                for valley in [false, true] {
                    let center = if valley { 1.5 * s.pitch_mm } else { s.pitch_mm };
                    let boundary = if valley {
                        s.major_radius_mm - s.depth_mm + r - (r * r - dz * dz).sqrt()
                    } else {
                        s.major_radius_mm - r + (r * r - dz * dz).sqrt()
                    };
                    for (offset, inside) in [(-0.002, false), (0.002, true)] {
                        let fraction = material(
                            shape,
                            point(pose, [boundary + offset, 0., center + dz]),
                            0.0002,
                        );
                        assert!(if inside {
                            fraction > 0.999
                        } else {
                            fraction < 0.001
                        });
                    }
                }
            }
            assert_eq!(stock.bounds().unwrap(), stock_bounds);
            assert_eq!(shared_stock.bounds().unwrap(), stock_bounds);
            assert!((stock.volume_mm3().unwrap() - stock_volume).abs() < 1e-12);
            assert!((shared_stock.volume_mm3().unwrap() - stock_volume).abs() < 1e-12);
        }
    }
    for shape in [&local_cutter, &shared_cutter] {
        assert_eq!(shape.bounds().unwrap(), original_bounds);
        assert!((shape.volume_mm3().unwrap() - original_volume).abs() < 1e-12);
        assert_eq!(
            shape.topology_snapshot(feature, 0).unwrap(),
            original_topology
        );
        let fresh = Solid::from_brep(&shape.to_brep().unwrap(), Default::default()).unwrap();
        assert_eq!(fresh.solid_count().unwrap(), 1);
        check_circles(&fresh, s);
    }
}

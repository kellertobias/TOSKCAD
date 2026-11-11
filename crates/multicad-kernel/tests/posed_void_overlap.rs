use multicad_core::document::EntityId;
use multicad_kernel::{rust_kernel::Rational, *};
#[test]
fn imported_cavity_probes_remain_empty_in_axis_and_posed_charts() {
    for (posed, count) in [(false, 1), (true, 1), (false, 3), (true, 3)] {
        let frame = if posed {
            PlaneFrame {
                origin: [80., 60., 20.],
                u: [0.8, 0.6, 0.],
                v: [-0.36, 0.48, 0.8],
            }
        } else {
            PlaneFrame::default()
        };
        let point = |p: [f64; 3]| {
            std::array::from_fn(|i| {
                frame.origin[i] + frame.u[i] * p[0] + frame.v[i] * p[1] + frame.normal()[i] * p[2]
            })
        };
        let make = |size: [i64; 3], origin: [f64; 3]| {
            Solid::rust_box(
                EntityId::new().uuid_bits(),
                size.map(|n| Rational::new(n, 1).unwrap()),
                PlaneFrame {
                    origin: point(origin),
                    ..frame
                },
                GeometryPolicy::default(),
            )
            .unwrap()
        };
        let stock = Solid::rust_box(
            EntityId::new().uuid_bits(),
            [15, 10, 8].map(|n| Rational::new(n, 1).unwrap()),
            PlaneFrame::default(),
            GeometryPolicy::default(),
        )
        .unwrap();
        let mut probes = vec![make([5, 4, 3], [2., 3., 1.])];
        if count == 3 {
            probes.extend([
                make([1, 2, 2], [8., 3., 2.]),
                make([1, 2, 2], [11., 3., 2.]),
            ]);
        }
        let mut tools = vec![
            Solid::rust_box(
                EntityId::new().uuid_bits(),
                [5, 4, 3].map(|n| Rational::new(n, 1).unwrap()),
                PlaneFrame {
                    origin: [2., 3., 1.],
                    ..Default::default()
                },
                GeometryPolicy::default(),
            )
            .unwrap(),
        ];
        if count == 3 {
            for x in [8., 11.] {
                tools.push(
                    Solid::rust_box(
                        EntityId::new().uuid_bits(),
                        [1, 2, 2].map(|n| Rational::new(n, 1).unwrap()),
                        PlaneFrame {
                            origin: [x, 3., 2.],
                            ..Default::default()
                        },
                        GeometryPolicy::default(),
                    )
                    .unwrap(),
                );
            }
        }
        let cut = stock
            .rust_subtract_orthogonal_tools(&tools, GeometryPolicy::default())
            .unwrap()
            .unwrap();
        let cut = cut
            .transformed(RigidTransform {
                rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], frame.normal()[i]]),
                translation_mm: frame.origin,
            })
            .unwrap();
        for probe in &probes {
            assert_eq!(
                cut.rust_occupied_overlap_volume(probe, GeometryPolicy::default())
                    .unwrap(),
                0.
            );
            assert_eq!(
                probe
                    .rust_occupied_overlap_volume(&cut, GeometryPolicy::default())
                    .unwrap(),
                0.
            );
        }
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("cavity.step");
        cut.write_step(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();

        let imported =
            Solid::import_rust_step(&text, EntityId::new(), GeometryPolicy::default()).unwrap();
        for probe in &probes {
            assert_eq!(
                imported
                    .rust_occupied_overlap_volume(probe, GeometryPolicy::default())
                    .unwrap(),
                0.
            );
            assert_eq!(
                probe
                    .rust_occupied_overlap_volume(&imported, GeometryPolicy::default())
                    .unwrap(),
                0.
            );
            let placement = RigidTransform {
                rotation: [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
                translation_mm: [-13., 8., 11.],
            };
            assert_eq!(
                imported
                    .transformed(placement)
                    .unwrap()
                    .rust_occupied_overlap_volume(
                        &probe.transformed(placement).unwrap(),
                        GeometryPolicy::default()
                    )
                    .unwrap(),
                0.
            );
        }
        #[cfg(feature = "occt")]
        {
            let native = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
            assert!((native.volume_mm3().unwrap() - imported.volume_mm3().unwrap()).abs() < 1e-6);
            for (index, probe) in probes.iter().enumerate() {
                let probe_path = folder.path().join(format!("probe-{index}.step"));
                probe.write_step(&probe_path).unwrap();
                let native_probe =
                    Solid::read_step(&probe_path, GeometryPolicy::default()).unwrap();
                let common = native
                    .boolean(
                        &native_probe,
                        BooleanOperation::Intersect,
                        GeometryPolicy::default(),
                    )
                    .unwrap();
                assert!(
                    common
                        .map(|solid| solid.volume_mm3().unwrap())
                        .unwrap_or(0.)
                        .abs()
                        < 1e-6
                );
            }
        }
        let volume = imported.volume_mm3().unwrap();
        for source in [&cut, &imported] {
            let material = make([1, 1, 1], [0., 0., 0.]);
            assert!(
                (source
                    .rust_occupied_overlap_volume(&material, GeometryPolicy::default())
                    .unwrap()
                    - 1.)
                    .abs()
                    < 1e-7
            );
            let partial = make([5, 4, 3], [1., 3., 1.]);
            assert!(
                (source
                    .rust_occupied_overlap_volume(&partial, GeometryPolicy::default())
                    .unwrap()
                    - 12.)
                    .abs()
                    < 1e-7
            );
            let interior = make([1, 1, 1], [3., 4., 2.]);
            assert_eq!(
                source
                    .rust_occupied_overlap_volume(&interior, GeometryPolicy::default())
                    .unwrap(),
                0.
            );
            let factor = std::f64::consts::FRAC_1_SQRT_2;
            let angled = Solid::rust_box(
                EntityId::new().uuid_bits(),
                [5, 4, 3].map(|n| Rational::new(n, 1).unwrap()),
                PlaneFrame {
                    origin: point([2., 3., 1.]),
                    u: std::array::from_fn(|i| (frame.u[i] + frame.v[i]) * factor),
                    v: std::array::from_fn(|i| (-frame.u[i] + frame.v[i]) * factor),
                },
                GeometryPolicy::default(),
            )
            .unwrap();
            match source.rust_occupied_overlap_volume(&angled, GeometryPolicy::default()) {
                Ok(volume) => assert!(volume > 0.),
                Err(error) => assert_eq!(error.kind, GeometryErrorKind::InvalidResult),
            }
            for penetration in [1e-11, 1e-8, 1e-4] {
                let penetrating = make([5, 4, 3], [2. - penetration, 3., 1.]);
                match source.rust_occupied_overlap_volume(&penetrating, GeometryPolicy::default()) {
                    Ok(volume) => assert!(
                        volume > 0.,
                        "positive penetration {penetration} was lost (posed={posed})"
                    ),
                    Err(error) => assert_eq!(error.kind, GeometryErrorKind::InvalidResult),
                }
            }
        }
        assert_eq!(imported.volume_mm3().unwrap(), volume);
    }
}

#[test]
fn retained_endpoint_precision_is_not_lost_at_large_grid_coordinates() {
    let policy = GeometryPolicy::default();
    let make = |width: Rational, origin: [f64; 3]| {
        Solid::rust_box(
            EntityId::new().uuid_bits(),
            [
                width,
                Rational::new(2, 1).unwrap(),
                Rational::new(2, 1).unwrap(),
            ],
            PlaneFrame {
                origin,
                ..Default::default()
            },
            policy,
        )
        .unwrap()
    };
    let stock = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [15, 6, 6].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame {
            origin: [9_999_985., 0., 0.],
            ..Default::default()
        },
        policy,
    )
    .unwrap();
    let cavity = make(Rational::new(1, 1).unwrap(), [9_999_990., 2., 2.]);
    let cut = stock
        .rust_subtract_orthogonal_tools(&[cavity], policy)
        .unwrap()
        .unwrap();
    let penetrating = make(
        Rational::new(10_000_000_001, 10_000_000_000).unwrap(),
        [9_999_990., 2., 2.],
    );
    assert_eq!(9_999_990. + (1. + 1e-10), 9_999_991.);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large-cavity.step");
    cut.write_step(&path).unwrap();
    let imported = Solid::import_rust_step(
        &std::fs::read_to_string(path).unwrap(),
        EntityId::new(),
        policy,
    )
    .unwrap();
    for source in [&cut, &imported] {
        let error = source
            .rust_occupied_overlap_volume(&penetrating, policy)
            .unwrap_err();
        assert_eq!(error.kind, GeometryErrorKind::InvalidResult);
        assert!(error.message.contains("endpoint addition"));
    }
}

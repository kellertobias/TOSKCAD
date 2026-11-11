use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, Origin, TopologyKind},
};
use multicad_kernel::{
    CurvedEdge2d, CurvedProfile, GeometryError, GeometryErrorKind, GeometryPolicy, MeshOptions,
    PlaneFrame, RevolutionAxis, RigidTransform, Solid,
    rust_kernel::{Rational, polygon::PolygonPrism, prism::Prism},
};

#[derive(Clone, Copy, Debug)]
enum Kind {
    Sphere,
    Box,
    Polygon,
    Annulus,
    Torus,
    RoundedPrism,
}
const KINDS: [Kind; 6] = [
    Kind::Sphere,
    Kind::Box,
    Kind::Polygon,
    Kind::Annulus,
    Kind::Torus,
    Kind::RoundedPrism,
];
fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}
fn noisy() -> PlaneFrame {
    PlaneFrame {
        u: [(1. + 8e-11_f64).sqrt(), 0., 0.],
        ..Default::default()
    }
}
fn pose(frame: PlaneFrame) -> RigidTransform {
    let normal = frame.normal();
    RigidTransform {
        rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], normal[i]]),
        translation_mm: frame.origin,
    }
}
fn make(kind: Kind, size: i64, frame: PlaneFrame) -> Result<Solid, GeometryError> {
    let policy = GeometryPolicy::default();
    let id = EntityId::new();
    let r = size as f64;
    match kind {
        Kind::Sphere => Solid::rust_sphere_named(r, frame, id, policy),
        Kind::Box => Solid::rust_box(id.uuid_bits(), [n(size), n(2), n(3)], frame, policy),
        Kind::Polygon => {
            // The retained polygon chart is deliberately offset and reversed.
            let ring = [[size, 0], [size + 10, 0], [size + 10, 3], [size, 3]]
                .map(|p| p.map(n))
                .to_vec();
            let prism = PolygonPrism::new(vec![ring], n(3)).unwrap();
            let origins: Vec<_> = (0..4)
                .map(|i| Origin {
                    source: id,
                    role: format!("edge.{i}"),
                })
                .collect();
            Solid::from_polygon_prism_sweep(prism, frame, true, id, &origins, &[false; 4], policy)
        }
        Kind::Annulus => Solid::rust_annulus(r / 2., r, 3., id, frame, policy),
        Kind::RoundedPrism => Solid::from_rust_prism(
            Prism::extrude(id.uuid_bits(), n(1), n(size), n(3)).unwrap(),
            policy,
        )?
        .transformed(pose(frame)),
        Kind::Torus => {
            let profile = CurvedProfile {
                frame: PlaneFrame {
                    v: [0., 0., 1.],
                    ..Default::default()
                },
                outer: vec![CurvedEdge2d::Circle {
                    center: [2. * r, 0.],
                    radius_mm: r / 2.,
                }],
                holes: vec![],
            };
            let origin = Origin {
                source: id,
                role: "circle".into(),
            };
            Solid::rust_revolve_curved_profile_named(
                &profile,
                RevolutionAxis {
                    origin_mm: [0.; 3],
                    direction: [0., 0., 1.],
                },
                std::f64::consts::TAU,
                id,
                &[origin],
                policy,
            )?
            .transformed(pose(frame))
        }
    }
}

#[test]
fn large_analytic_frames_refuse_size_amplified_metric_error() {
    for kind in KINDS {
        let error = make(kind, 1_000_000, noisy())
            .err()
            .unwrap_or_else(|| panic!("accepted distorted {kind:?}"));
        assert_eq!(
            error.kind,
            GeometryErrorKind::Unsupported,
            "{kind:?}: {error}"
        );
        assert!(
            error.message.contains("metric residual"),
            "{kind:?}: {error}"
        );
    }
    let sphere = make(Kind::Sphere, 1_000_000, PlaneFrame::default()).unwrap();
    let bounds = sphere.bounds().unwrap();
    let topology = sphere.topology_snapshot(EntityId::new(), 0).unwrap();
    for element in topology
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Vertex)
    {
        let Some(AnalyticGeometry::Point { position_mm }) = element.geometry else {
            panic!("sphere pole is not analytic")
        };
        assert!((0..3).all(|i| position_mm[i] >= bounds.min[i] && position_mm[i] <= bounds.max[i]));
    }
}

#[test]
fn rigid_transform_checks_composed_metric_and_keeps_the_source_immutable() {
    let transform = RigidTransform {
        rotation: [
            [(1. + 8e-11_f64).sqrt(), 0., 0.],
            [0., 1., 0.],
            [0., 0., 1.],
        ],
        ..Default::default()
    };
    for kind in KINDS {
        let source = make(kind, 1_000_000, PlaneFrame::default()).unwrap();
        let owner = EntityId::new();
        let topology = source.topology_snapshot(owner, 0).unwrap();
        let volume = source.volume_mm3().unwrap();
        let bounds = source.bounds().unwrap();
        let error = source
            .transformed(transform)
            .err()
            .unwrap_or_else(|| panic!("transformed distorted {kind:?}"));
        assert_eq!(
            error.kind,
            GeometryErrorKind::Unsupported,
            "{kind:?}: {error}"
        );
        assert_eq!(source.topology_snapshot(owner, 0).unwrap(), topology);
        assert_eq!(source.volume_mm3().unwrap(), volume);
        assert_eq!(source.bounds().unwrap(), bounds);
        let proper = RigidTransform {
            rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
            translation_mm: [17., -8., 13.],
        };
        let placed = source.transformed(proper).unwrap();
        assert_eq!(placed.volume_mm3().unwrap(), volume);
    }
    let small_error = 2e-11;
    let source = Solid::rust_sphere_named(
        500.,
        PlaneFrame {
            u: [1. + small_error, 0., 0.],
            ..Default::default()
        },
        EntityId::new(),
        GeometryPolicy::default(),
    )
    .unwrap();
    let error = source
        .transformed(RigidTransform {
            rotation: [[1. + small_error, 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            ..Default::default()
        })
        .err()
        .expect("composed metric error must be checked");
    assert_eq!(error.kind, GeometryErrorKind::Unsupported);
    assert!(error.message.contains("metric residual"));
}

#[test]
fn small_admitted_frames_keep_finite_mesh_and_real_step_geometry() {
    let policy = GeometryPolicy::default();
    let directory = tempfile::tempdir().unwrap();
    for kind in KINDS {
        let source = make(kind, 10, noisy()).unwrap();
        let bounds = source.bounds().unwrap();
        let receipt = source
            .manufacturing_mesh_with_receipt(MeshOptions {
                linear_deflection_mm: 0.1,
                angular_deflection_rad: 0.4,
                maximum_triangles: 100_000,
            })
            .unwrap();
        let (vertices, remainder) = receipt.mesh.vertices.as_chunks::<3>();
        assert!(remainder.is_empty());
        for point in vertices {
            for i in 0..3 {
                assert!(
                    point[i] >= bounds.min[i] - policy.linear_tolerance_mm
                        && point[i] <= bounds.max[i] + policy.linear_tolerance_mm,
                    "{kind:?}: {point:?} beyond {bounds:?}"
                );
            }
        }
        let path = directory.path().join(format!("{kind:?}.step"));
        if matches!(kind, Kind::RoundedPrism) {
            assert!(!source.supports_step_export());
            assert_eq!(
                source.write_step(&path).unwrap_err().kind,
                GeometryErrorKind::Unsupported
            );
            assert!(!path.exists());
            continue;
        }
        source
            .write_step(&path)
            .unwrap_or_else(|error| panic!("{kind:?}: {error}"));
        let text = std::fs::read_to_string(&path).unwrap();
        let imported = Solid::import_rust_step(&text, EntityId::new(), policy).unwrap();
        assert!(
            (imported.volume_mm3().unwrap() - source.volume_mm3().unwrap()).abs()
                < 1e-7 * source.volume_mm3().unwrap()
        );
        #[cfg(feature = "occt")]
        {
            let native = Solid::read_step(&path, policy).unwrap();
            assert!(
                (native.volume_mm3().unwrap() - source.volume_mm3().unwrap()).abs()
                    < 1e-7 * source.volume_mm3().unwrap()
            );
            let native_bounds = native.bounds().unwrap();
            for (a, b) in bounds
                .min
                .into_iter()
                .chain(bounds.max)
                .zip(native_bounds.min.into_iter().chain(native_bounds.max))
            {
                assert!((a - b).abs() < 5e-6, "{kind:?}: {a} vs {b}");
            }
        }
    }
}

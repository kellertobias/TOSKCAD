use multicad_core::{
    document::EntityId,
    references::{Origin, TopologyKey, TopologyKind},
};
use multicad_kernel::{
    CurvedEdge2d, CurvedProfile, EdgeTreatment, GeometryError, GeometryErrorKind, GeometryPolicy,
    MeshOptions, PlaneFrame, RigidTransform, Solid, TreatmentEdge,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
};
fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}
fn noisy(error: f64) -> PlaneFrame {
    PlaneFrame {
        u: [1. + error, 0., 0.],
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
fn cylinder(radius: i64, height: i64, id: EntityId) -> Solid {
    Solid::from_axial_cylinder(
        AxialCylinder::new(
            id.uuid_bits(),
            Circle::new([n(0); 2], n(radius)).unwrap(),
            n(0),
            n(height),
        )
        .unwrap(),
        PlaneFrame::default(),
        GeometryPolicy::default(),
    )
    .unwrap()
}
#[derive(Clone, Copy, Debug)]
enum Kind {
    Linear,
    Normal,
    Rounded,
    Stack,
}
const KINDS: [Kind; 4] = [Kind::Linear, Kind::Normal, Kind::Rounded, Kind::Stack];
fn make(kind: Kind, size: i64, frame: PlaneFrame) -> Result<Solid, GeometryError> {
    let id = EntityId::new();
    let policy = GeometryPolicy::default();
    match kind {
        Kind::Linear => {
            let x = size as f64;
            let points = [[x, 0.], [x + 10., 0.], [x + 10., 3.], [x, 3.]];
            let profile = CurvedProfile {
                frame,
                outer: points
                    .into_iter()
                    .zip(points.into_iter().cycle().skip(1))
                    .map(|(a, b)| CurvedEdge2d::Line { a, b })
                    .collect(),
                holes: vec![],
            };
            let origins: Vec<_> = (0..4)
                .map(|i| Origin {
                    source: id,
                    role: format!("line.{i}"),
                })
                .collect();
            Solid::rust_extrude_linear_named(
                &profile,
                frame.normal().map(|x| -3. * x),
                0.,
                id,
                &origins,
                policy,
            )
        }
        Kind::Normal => {
            let profile = CurvedProfile {
                frame,
                outer: vec![
                    CurvedEdge2d::Line {
                        a: [0., 0.],
                        b: [2., 0.],
                    },
                    CurvedEdge2d::Arc {
                        center: [2., 1.],
                        a: [2., 0.],
                        b: [2., 2.],
                        clockwise: false,
                    },
                    CurvedEdge2d::Line {
                        a: [2., 2.],
                        b: [0., 2.],
                    },
                    CurvedEdge2d::Line {
                        a: [0., 2.],
                        b: [0., 0.],
                    },
                ],
                holes: vec![],
            };
            let origins: Vec<_> = (0..4)
                .map(|i| Origin {
                    source: id,
                    role: format!("normal.{i}"),
                })
                .collect();
            Solid::rust_extrude_normal_profile_named(&profile, -(size as f64), id, &origins, policy)
        }
        Kind::Rounded => {
            // Tiny radius, very long Z: a radius-only guard misses this case.
            let stock = cylinder(1, size, id);
            let feature = EntityId::new();
            let result = stock.treat_edges_named(
                &[TreatmentEdge {
                    key: TopologyKey {
                        kind: TopologyKind::Edge,
                        ordinal: 0,
                    },
                    origin: Origin {
                        source: feature,
                        role: "round.rim".into(),
                    },
                }],
                EdgeTreatment::Fillet { radius_mm: 0.25 },
                feature,
                policy,
            )?;
            result.transformed(pose(frame))
        }
        Kind::Stack => {
            // Its intrinsic circular layer is small, while a rectangular layer
            // extends far behind the retained axial origin.
            let round = cylinder(1, 2, id);
            let rectangle = Solid::rust_box(
                EntityId::new().uuid_bits(),
                [n(2), n(2), n(size + 1)],
                PlaneFrame {
                    origin: [-1., -1., -(size as f64)],
                    ..Default::default()
                },
                policy,
            )
            .unwrap();
            Solid::union_round_rectangular_stack(&[&round, &rectangle], EntityId::new(), policy)
                .expect("supported axial stack")?
                .transformed(pose(frame))
        }
    }
}
#[test]
fn sweep_constructor_and_transform_guards_include_authored_uv_and_negative_z() {
    for kind in KINDS {
        let error = make(kind, 1_000_000, noisy(4e-11))
            .err()
            .unwrap_or_else(|| panic!("accepted {kind:?} with unbounded metric error"));
        assert_eq!(
            error.kind,
            GeometryErrorKind::Unsupported,
            "{kind:?}: {error}"
        );
        assert!(
            error.message.contains("metric residual"),
            "{kind:?}: {error}"
        );
        let source = make(kind, 1_000_000, PlaneFrame::default()).unwrap();
        let owner = EntityId::new();
        let topology = source.topology_snapshot(owner, 0).unwrap();
        let bounds = source.bounds().unwrap();
        let volume = source.volume_mm3().unwrap();
        let transform = RigidTransform {
            rotation: [[1. + 4e-11, 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            ..Default::default()
        };
        assert_eq!(
            source
                .transformed(transform)
                .err()
                .expect("large composed placement must fail")
                .kind,
            GeometryErrorKind::Unsupported
        );
        assert_eq!(source.topology_snapshot(owner, 0).unwrap(), topology);
        assert_eq!(source.bounds().unwrap(), bounds);
        assert_eq!(source.volume_mm3().unwrap(), volume);
        let proper = RigidTransform {
            rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
            translation_mm: [17., -8., 13.],
        };
        assert_eq!(
            source.transformed(proper).unwrap().volume_mm3().unwrap(),
            volume
        );
    }
    let source = make(Kind::Normal, 800, noisy(2e-11)).unwrap();
    let error = source
        .transformed(RigidTransform {
            rotation: [[1. + 2e-11, 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            ..Default::default()
        })
        .err()
        .expect("accumulated admitted Gram residual must still respect actual sweep size");
    assert_eq!(error.kind, GeometryErrorKind::Unsupported);
    assert!(error.message.contains("metric residual"));
}
#[test]
fn small_metric_frames_preserve_mesh_and_proper_frames_exchange() {
    let directory = tempfile::tempdir().unwrap();
    let policy = GeometryPolicy::default();
    for kind in KINDS {
        let source = make(kind, 10, noisy(4e-11)).unwrap();
        let bounds = source.bounds().unwrap();
        let receipt = source
            .manufacturing_mesh_with_receipt(MeshOptions {
                linear_deflection_mm: 0.1,
                angular_deflection_rad: 0.5,
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
                    "{kind:?}: {point:?} outside {bounds:?}"
                );
            }
        }
        let proper = make(kind, 10, PlaneFrame::default()).unwrap();
        // The noisy axial-stack tangency roundtrip is tracked separately:
        // these guards preserve admission, without changing STEP incidence.
        let candidates = if matches!(kind, Kind::Stack) {
            vec![&proper]
        } else {
            vec![&source, &proper]
        };
        for geometry in candidates {
            let bounds = geometry.bounds().unwrap();
            let path = directory.path().join(format!("{kind:?}.step"));
            geometry
                .write_step(&path)
                .unwrap_or_else(|e| panic!("{kind:?}: {e}"));
            let text = std::fs::read_to_string(&path).unwrap();
            let read = Solid::import_rust_step(&text, EntityId::new(), policy)
                .unwrap_or_else(|error| panic!("{kind:?}: {error}"));
            assert!(
                (read.volume_mm3().unwrap() - geometry.volume_mm3().unwrap()).abs()
                    < 1e-7 * geometry.volume_mm3().unwrap()
            );
            #[cfg(feature = "occt")]
            {
                let native = Solid::read_step(&path, policy).unwrap();
                assert!(
                    (native.volume_mm3().unwrap() - geometry.volume_mm3().unwrap()).abs()
                        < 1e-7 * geometry.volume_mm3().unwrap()
                );
                let expected = native.bounds().unwrap();
                for (a, b) in bounds
                    .min
                    .into_iter()
                    .chain(bounds.max)
                    .zip(expected.min.into_iter().chain(expected.max))
                {
                    assert!((a - b).abs() < 5e-6, "{kind:?}: {a} vs {b}");
                }
            }
        }
    }
}

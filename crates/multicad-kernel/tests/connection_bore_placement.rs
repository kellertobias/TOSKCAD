use multicad_core::{document::EntityId, references::Origin};
use multicad_kernel::{
    CircularPocketTool, GeometryErrorKind, GeometryPolicy, MeshOptions, PlaneFrame, RigidTransform,
    Solid,
};
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{a} != {b}");
}
fn vector(pose: RigidTransform, p: [f64; 3]) -> [f64; 3] {
    pose.rotation
        .map(|row| row.into_iter().zip(p).map(|(a, b)| a * b).sum())
}
fn point(pose: RigidTransform, p: [f64; 3]) -> [f64; 3] {
    let q = vector(pose, p);
    std::array::from_fn(|i| q[i] + pose.translation_mm[i])
}
fn tool(pose: RigidTransform, roll: f64, tilt: f64) -> CircularPocketTool {
    let (s, c) = roll.sin_cos();
    let (st, ct) = tilt.sin_cos();
    let frame = PlaneFrame {
        origin: point(pose, [10., 10., 4.10001]),
        u: vector(pose, [c * ct, s * ct, st]),
        v: vector(pose, [s, -c, 0.]),
    };
    CircularPocketTool {
        frame,
        center_uv: [0.; 2],
        radius_mm: 1.7,
        displacement_mm: frame.normal().map(|v| v * 4.10002),
        origin: Origin {
            source: EntityId::new(),
            role: "receiving.round".into(),
        },
    }
}
#[test]
fn full_round_tools_preserve_actual_bores_after_proper_and_far_placement() {
    let policy = GeometryPolicy::default();
    for translation_mm in [[1000., -2000., 3000.], [1e6, -2e6, 3e6]] {
        let pose = RigidTransform {
            rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
            translation_mm,
        };
        let stock = Solid::box_solid([20., 20., 4.1], policy)
            .unwrap()
            .transformed(pose)
            .unwrap();
        let owner = EntityId::new();
        let before = stock.topology_snapshot(owner, 0).unwrap();
        for roll in [0., 0.37, 1.4] {
            let (cut, mouths) = stock
                .rust_connection_through_bore(&tool(pose, roll, 0.), 1., policy)
                .unwrap();
            for (actual, local) in mouths.into_iter().zip([[10., 10., 4.1], [10., 10., 0.]]) {
                for (a, b) in actual.into_iter().zip(point(pose, local)) {
                    near(a, b);
                }
            }
            let volume = 1640. - std::f64::consts::PI * 1.7_f64.powi(2) * 4.1;
            near(cut.volume_mm3().unwrap(), volume);
            assert_eq!(cut.bounds().unwrap(), stock.bounds().unwrap());
            let mesh = cut
                .manufacturing_mesh_with_receipt(MeshOptions {
                    linear_deflection_mm: 0.001,
                    angular_deflection_rad: 0.05,
                    maximum_triangles: 10000,
                })
                .unwrap();
            assert_eq!(mesh.edge_pairs as usize * 2, mesh.mesh.indices.len());
            let folder = tempfile::tempdir().unwrap();
            let path = folder.path().join("posed-bore.step");
            cut.write_step(&path).unwrap();
            let loaded = Solid::read_step(&path, policy).unwrap();
            assert!((loaded.volume_mm3().unwrap() - volume).abs() < 1e-5);
            assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
        }
        let failure = stock
            .rust_connection_through_bore(&tool(pose, 0.37, 1e-4), 1., policy)
            .err()
            .unwrap();
        assert_eq!(failure.kind, GeometryErrorKind::Unsupported);
        assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
    }
}

#[test]
fn actual_receiving_seats_keep_stock_caps_and_closed_material_after_placement() {
    let policy = GeometryPolicy::default();
    for pose in [
        RigidTransform::default(),
        RigidTransform {
            rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
            translation_mm: [1e6, -2e6, 3e6],
        },
    ] {
        let stock = Solid::box_solid([20., 20., 4.1], policy)
            .unwrap()
            .transformed(pose)
            .unwrap();
        let owner = EntityId::new();
        let before = stock.topology_snapshot(owner, 0).unwrap();
        for roll in [0., 0.37, 1.4] {
            let (cut, mouths) = stock
                .rust_connection_counterbore(
                    &tool(pose, roll, 0.),
                    3.5,
                    1.,
                    1.,
                    Origin {
                        source: EntityId::new(),
                        role: "counterbore".into(),
                    },
                    policy,
                )
                .unwrap();
            let volume = 1640.
                - std::f64::consts::PI
                    * (1.7_f64.powi(2) * 4.1 + (3.5_f64.powi(2) - 1.7_f64.powi(2)));
            near(cut.volume_mm3().unwrap(), volume);
            for (mouth, local) in mouths.into_iter().zip([[10., 10., 4.1], [10., 10., 0.]]) {
                for (a, b) in mouth.into_iter().zip(point(pose, local)) {
                    near(a, b);
                }
            }
            let receipt = cut
                .manufacturing_mesh_with_receipt(MeshOptions {
                    linear_deflection_mm: 0.001,
                    angular_deflection_rad: 0.08,
                    maximum_triangles: 100000,
                })
                .unwrap();
            assert_eq!(receipt.edge_pairs as usize * 2, receipt.mesh.indices.len());
            let folder = tempfile::tempdir().unwrap();
            let path = folder.path().join("seated-receiver.step");
            cut.write_step(&path).unwrap();
            let read = Solid::read_step(&path, policy).unwrap();
            assert!((read.volume_mm3().unwrap() - volume).abs() < 1e-5);
        }
        assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
        for (radius, depth) in [(20., 1.), (3.5, 4.1), (1.7, 1.), (3.5, 0.)] {
            assert!(
                stock
                    .rust_connection_counterbore(
                        &tool(pose, 0., 0.),
                        radius,
                        depth,
                        1.,
                        Origin {
                            source: EntityId::new(),
                            role: "invalid-seat".into()
                        },
                        policy
                    )
                    .is_err()
            );
        }
        assert_eq!(stock.topology_snapshot(owner, 0).unwrap(), before);
    }
}

#[test]
fn counterbore_sections_preserve_real_polygon_holes_and_refuse_axial_steps() {
    use multicad_kernel::BooleanOperation;
    let policy = GeometryPolicy::default();
    let full = Solid::box_solid([40., 20., 4.1], policy).unwrap();
    let slot = Solid::box_solid([8., 12., 6.1], policy)
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: [28., 4., -1.],
            ..Default::default()
        })
        .unwrap();
    let hollow = full
        .boolean(&slot, BooleanOperation::Subtract, policy)
        .unwrap()
        .unwrap();
    let (cut, _) = hollow
        .rust_connection_counterbore(
            &tool(Default::default(), 0., 0.),
            3.5,
            1.,
            1.,
            Origin {
                source: EntityId::new(),
                role: "seat".into(),
            },
            policy,
        )
        .unwrap();
    let loss = std::f64::consts::PI * (1.7_f64.powi(2) * 4.1 + 3.5_f64.powi(2) - 1.7_f64.powi(2));
    near(cut.volume_mm3().unwrap(), (800. - 96.) * 4.1 - loss);
    cut.manufacturing_mesh_with_receipt(Default::default())
        .unwrap();
    let notch = Solid::box_solid([2., 2., 2.], policy)
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: [0., 0., 3.1],
            ..Default::default()
        })
        .unwrap();
    let stepped = full
        .boolean(&notch, BooleanOperation::Subtract, policy)
        .unwrap()
        .unwrap();
    let before = stepped.volume_mm3().unwrap();
    let error = stepped
        .rust_connection_counterbore(
            &tool(Default::default(), 0., 0.),
            3.5,
            1.,
            1.,
            Origin {
                source: EntityId::new(),
                role: "refused-seat".into(),
            },
            policy,
        )
        .err()
        .unwrap();
    assert_eq!(error.kind, GeometryErrorKind::Unsupported);
    assert_eq!(stepped.volume_mm3().unwrap(), before);
}

#[test]
fn receiving_seats_follow_each_actual_stock_axis_and_entry_side() {
    let policy = GeometryPolicy::default();
    let pose = RigidTransform {
        rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
        translation_mm: [1000., -2000., 3000.],
    };
    let stock = Solid::box_solid([20.; 3], policy)
        .unwrap()
        .transformed(pose)
        .unwrap();
    for axis in 0..3 {
        for sign in [-1., 1.] {
            let mut local = [10.; 3];
            local[axis] = if sign < 0. { 20.00001 } else { -0.00001 };
            let mut u = [0.; 3];
            u[(axis + 1) % 3] = 1.;
            let mut v = [0.; 3];
            v[(axis + 2) % 3] = sign;
            let frame = PlaneFrame {
                origin: point(pose, local),
                u: vector(pose, u),
                v: vector(pose, v),
            };
            let tool = CircularPocketTool {
                frame,
                center_uv: [0.; 2],
                radius_mm: 1.7,
                displacement_mm: frame.normal().map(|v| v * 20.00002),
                origin: Origin {
                    source: EntityId::new(),
                    role: "bore".into(),
                },
            };
            let (cut, _) = stock
                .rust_connection_counterbore(
                    &tool,
                    3.5,
                    1.,
                    1.,
                    Origin {
                        source: EntityId::new(),
                        role: "seat".into(),
                    },
                    policy,
                )
                .unwrap();
            near(
                cut.volume_mm3().unwrap(),
                8000.
                    - std::f64::consts::PI
                        * (1.7_f64.powi(2) * 20. + 3.5_f64.powi(2) - 1.7_f64.powi(2)),
            );
            cut.manufacturing_mesh_with_receipt(Default::default())
                .unwrap();
        }
    }
}

#[test]
fn counterbore_reconstruction_keeps_authored_cap_names_on_their_actual_planes() {
    use multicad_core::references::{
        GeometryIntent, Resolution, TopologyKey, TopologyKind, resolve,
    };
    use multicad_kernel::rust_kernel::Rational;
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let stock = Solid::rust_box(
        feature.uuid_bits(),
        [
            Rational::new(20, 1).unwrap(),
            Rational::new(20, 1).unwrap(),
            Rational::new(41, 10).unwrap(),
        ],
        Default::default(),
        policy,
    )
    .unwrap();
    let before = stock.topology_snapshot(feature, 0).unwrap();
    let (cut, _) = stock
        .rust_connection_counterbore(
            &tool(Default::default(), 0., 0.),
            3.5,
            1.,
            1.,
            Origin {
                source: EntityId::new(),
                role: "seat".into(),
            },
            policy,
        )
        .unwrap();
    let after = cut.topology_snapshot(feature, 1).unwrap();
    for element in &before.elements {
        let intent = GeometryIntent::capture(&before, element.key).unwrap();
        assert!(
            matches!(resolve(&intent, &after, 1), Resolution::Resolved { .. }),
            "lost {:?}",
            element.key
        );
    }
    for (ordinal, role, z) in [(0, "cap.start", 0.), (1, "cap.end", 4.1)] {
        let intent = GeometryIntent::capture(
            &before,
            TopologyKey {
                kind: TopologyKind::Face,
                ordinal,
            },
        )
        .unwrap();
        let Resolution::Resolved { key, .. } = resolve(&intent, &after, 1) else {
            panic!("lost {role}");
        };
        let element = after.elements.iter().find(|e| e.key == key).unwrap();
        let anchor = element
            .frames
            .iter()
            .find(|f| {
                f.origin
                    == Origin {
                        source: feature,
                        role: role.into(),
                    }
            })
            .unwrap();
        near(anchor.frame.origin_mm[2], z);
        let original = before
            .elements
            .iter()
            .flat_map(|e| &e.frames)
            .find(|f| {
                f.origin
                    == Origin {
                        source: feature,
                        role: role.into(),
                    }
            })
            .unwrap();
        assert_eq!(anchor.frame.u, original.frame.u);
        assert_eq!(anchor.frame.v, original.frame.v);
    }
}

use multicad_core::{document::EntityId, references::Origin};
use multicad_kernel::{
    BRepSelection, PlaneFrame, RigidTransform, Solid, SolidPairRelation,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
};
fn q(n: i64, d: i64) -> Rational {
    Rational::new(n, d).unwrap()
}
fn cylinder(radius: Rational, height: Rational, frame: PlaneFrame) -> Solid {
    let id = EntityId::new();
    Solid::from_axial_cylinder_sweep(
        AxialCylinder::new(
            id.uuid_bits(),
            Circle::new([q(0, 1); 2], radius).unwrap(),
            q(0, 1),
            height,
        )
        .unwrap(),
        frame,
        [0.; 2],
        false,
        Origin {
            source: id,
            role: "query.circle".into(),
        },
        Default::default(),
    )
    .unwrap()
}
fn base(offset: [f64; 3]) -> Solid {
    cylinder(
        q(5, 1),
        q(10, 1),
        PlaneFrame {
            origin: offset,
            ..Default::default()
        },
    )
}
fn near(a: f64, b: f64) {
    assert!(
        (a - b).abs() < 1e-8 * a.abs().max(b.abs()).max(1.),
        "{a} != {b}"
    );
}
#[test]
fn finite_parallel_pairs_keep_real_witnesses_and_honest_solution_counts_after_poses() {
    let pose = RigidTransform {
        translation_mm: [1e5, -2e5, 3e5],
        rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
    };
    for placed in [false, true] {
        for (offset, d, count, relation) in [
            ([14., 0., 0.], 4., None, SolidPairRelation::Separated),
            ([14., 0., 13.], 5., Some(1), SolidPairRelation::Separated),
            (
                [10., 0., 10.],
                0.,
                Some(1),
                SolidPairRelation::ContactWithinTolerance,
            ),
            ([0.; 3], 0., None, SolidPairRelation::Overlapping),
        ] {
            let mut a = base([0.; 3]);
            let mut b = base(offset);
            if placed {
                a = a.transformed(pose).unwrap();
                b = b.transformed(pose).unwrap();
            }
            let before = (a.volume_mm3().unwrap(), b.volume_mm3().unwrap());
            if placed && relation == SolidPairRelation::ContactWithinTolerance {
                // Rounded world origins produce a real tiny separating gap
                // here. Prove it independently with an analytic support plane.
                let u = pose.rotation.map(|row| row[0]);
                let v = pose.rotation.map(|row| row[1]);
                let cross = |a: [f64; 3], b: [f64; 3]| {
                    [
                        a[1] * b[2] - a[2] * b[1],
                        a[2] * b[0] - a[0] * b[2],
                        a[0] * b[1] - a[1] * b[0],
                    ]
                };
                let dot = |a: [f64; 3], b: [f64; 3]| {
                    a.into_iter().zip(b).map(|(x, y)| x * y).sum::<f64>()
                };
                let n = cross(u, v);
                let direction = cross(v, n);
                let delta = std::array::from_fn(|i| {
                    let rotated = dot(pose.rotation[i], offset);
                    (rotated + pose.translation_mm[i]) - pose.translation_mm[i]
                });
                let support = 10. * dot(direction, u).hypot(dot(direction, v))
                    + 10. * dot(direction, n).abs();
                let gap = dot(direction, delta) - support;
                let scale = direction
                    .into_iter()
                    .zip(delta)
                    .map(|(a, b)| (a * b).abs())
                    .sum::<f64>()
                    + support;
                assert!(gap > 128. * f64::EPSILON * scale);
                let result = a.inspect_pair(&b, 1e-7, Default::default()).unwrap();
                assert_eq!(result.common_volume_mm3, 0.);
                assert!(result.minimum_distance.distance_mm > 0.);
                assert_eq!(result.relation, SolidPairRelation::ContactWithinTolerance);
                assert_eq!(before, (a.volume_mm3().unwrap(), b.volume_mm3().unwrap()));
                continue;
            }
            let inspection = a.inspect_pair(&b, 1e-7, Default::default()).unwrap();
            near(inspection.minimum_distance.distance_mm, d);
            assert_eq!(inspection.relation, relation);
            if !placed {
                assert_eq!(inspection.minimum_distance.total_solutions, count);
                assert_eq!(
                    inspection.minimum_distance.witnesses_complete,
                    count.is_some()
                );
            } else {
                assert!(inspection.minimum_distance.total_solutions.is_none());
            }
            for witness in &inspection.minimum_distance.witnesses {
                near(
                    (witness.a_mm[0] - witness.b_mm[0])
                        .hypot(witness.a_mm[1] - witness.b_mm[1])
                        .hypot(witness.a_mm[2] - witness.b_mm[2]),
                    d,
                );
            }
            let reversed = b
                .minimum_distance(BRepSelection::Whole, &a, BRepSelection::Whole)
                .unwrap();
            near(reversed.distance_mm, d);
            near(
                a.rust_occupied_overlap_volume(&b, Default::default())
                    .unwrap(),
                inspection.common_volume_mm3,
            );
            assert_eq!(before, (a.volume_mm3().unwrap(), b.volume_mm3().unwrap()));
        }
    }
}
#[test]
fn tiny_positive_radial_and_exact_axial_material_survive_contact_tolerance() {
    let a = cylinder(q(1, 1), q(1, 1), Default::default());
    for x in [2. - 1e-11, 2., 2. + 1e-11] {
        let b = cylinder(
            q(1, 1),
            q(1, 1),
            PlaneFrame {
                origin: [x, 0., 0.],
                ..Default::default()
            },
        );
        let result = a.inspect_pair(&b, 1e-7, Default::default()).unwrap();
        if x < 2. {
            assert!(result.common_volume_mm3 > 0.);
            assert_eq!(result.relation, SolidPairRelation::Overlapping);
        } else {
            assert_eq!(result.common_volume_mm3, 0.);
            assert_eq!(result.relation, SolidPairRelation::ContactWithinTolerance);
        }
    }
    let b = cylinder(
        q(1, 1),
        q(99_999_990_000_000_001, 10_000_000_000),
        PlaneFrame {
            origin: [0., 0., -9_999_999.],
            ..Default::default()
        },
    );
    let result = a.inspect_pair(&b, 1e-7, Default::default()).unwrap();
    assert!(result.common_volume_mm3 > 0.);
    assert_eq!(result.relation, SolidPairRelation::Overlapping);
    near(result.common_volume_mm3, std::f64::consts::PI * 1e-10);
}
#[test]
fn reversed_axes_direct_placements_and_crossing_refusal_preserve_geometry() {
    let a = base([0.; 3]);
    let b = cylinder(
        q(5, 1),
        q(10, 1),
        PlaneFrame {
            origin: [0., 0., 10.],
            v: [0., -1., 0.],
            ..Default::default()
        },
    );
    near(
        a.rust_occupied_overlap_volume(&b, Default::default())
            .unwrap(),
        a.volume_mm3().unwrap(),
    );
    let crossed = cylinder(
        q(5, 1),
        q(10, 1),
        PlaneFrame {
            u: [0., 0., 1.],
            v: [0., 1., 0.],
            ..Default::default()
        },
    );
    assert!(
        a.minimum_distance(BRepSelection::Whole, &crossed, BRepSelection::Whole)
            .is_err()
    );
    let f = PlaneFrame {
        origin: [1e5, 3e5, -2e5],
        u: [0.8, 0.6, 0.],
        v: [-0.36, 0.48, 0.8],
    };
    let a = cylinder(q(5, 1), q(10, 1), f);
    let b = cylinder(
        q(5, 1),
        q(10, 1),
        PlaneFrame {
            origin: std::array::from_fn(|i| f.origin[i] + 14. * f.u[i]),
            ..f
        },
    );
    near(
        a.minimum_distance(BRepSelection::Whole, &b, BRepSelection::Whole)
            .unwrap()
            .distance_mm,
        4.,
    );
    assert!(
        a.minimum_distance(BRepSelection::Face(u32::MAX), &b, BRepSelection::Whole)
            .is_err()
    );
    near(a.volume_mm3().unwrap(), 250. * std::f64::consts::PI);
}

#[test]
fn descriptor_center_and_bottom_are_not_applied_twice_after_translation_and_reverse_axis() {
    let source = |center: [i64; 2], bottom: i64, frame: PlaneFrame| {
        let id = EntityId::new();
        Solid::from_axial_cylinder_sweep(
            AxialCylinder::new(
                id.uuid_bits(),
                Circle::new(center.map(|v| q(v, 1)), q(5, 1)).unwrap(),
                q(bottom, 1),
                q(10, 1),
            )
            .unwrap(),
            frame,
            [0.; 2],
            false,
            Origin {
                source: id,
                role: "offset.circle".into(),
            },
            Default::default(),
        )
        .unwrap()
    };
    let a = source(
        [20, -7],
        9,
        PlaneFrame {
            origin: [20., -7., 9.],
            ..Default::default()
        },
    );
    let b = source(
        [34, -7],
        19,
        PlaneFrame {
            origin: [34., -7., 19.],
            v: [0., -1., 0.],
            ..Default::default()
        },
    );
    let transform = RigidTransform {
        translation_mm: [80., -13., 21.],
        ..Default::default()
    };
    let a = a.transformed(transform).unwrap();
    let b = b.transformed(transform).unwrap();
    near(a.bounds().unwrap().min[0], 95.);
    near(b.bounds().unwrap().min[0], 109.);
    let result = a.inspect_pair(&b, 1e-7, Default::default()).unwrap();
    near(result.minimum_distance.distance_mm, 4.);
    assert_eq!(result.common_volume_mm3, 0.);
    assert_eq!(result.relation, SolidPairRelation::Separated);
}

#[test]
fn admitted_near_rigid_placement_cannot_erase_a_real_axial_material_band() {
    let source = base([0.; 3]);
    let second = base([0., 0., 10.]);
    let epsilon = 2e-12;
    let transform = RigidTransform {
        rotation: [[1. + epsilon, 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        ..Default::default()
    };
    let a = source.transformed(transform).unwrap();
    let b = second.transformed(transform).unwrap();
    // Actual stored U/V re-crossing lengthens each axial domain while the
    // second origin is still z=10. This is positive material, not exact contact.
    let z = 10. + 5. * epsilon;
    let local_a = z / (1. + epsilon);
    let local_b = (z - 10.) / (1. + epsilon);
    assert!(local_a < 10. && local_b > 0.);
    assert!(
        a.axial_cylinder()
            .unwrap()
            .closest_point_in_frame([0., 0., local_a])
            .unwrap()
            .strictly_inside
    );
    assert!(
        a.axial_cylinder()
            .unwrap()
            .closest_point_in_frame([0., 0., local_b])
            .unwrap()
            .strictly_inside
    );
    let error = a.inspect_pair(&b, 1e-7, Default::default()).unwrap_err();
    assert_eq!(error.kind, multicad_kernel::GeometryErrorKind::Unsupported);
    assert!(error.message.contains("unresolved positive material"));
    assert!(
        a.rust_occupied_overlap_volume(&b, Default::default())
            .is_err()
    );
    near(source.volume_mm3().unwrap(), 250. * std::f64::consts::PI);
}

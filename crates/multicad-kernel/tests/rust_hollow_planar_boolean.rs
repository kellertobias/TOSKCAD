use multicad_core::{
    document::EntityId,
    references::{GeometryIntent, Origin, Resolution, TopologyKind, resolve},
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, RigidTransform,
    Solid, rust_kernel::Rational,
};

fn profile() -> CurvedProfile {
    let wire = |p: &[[f64; 2]]| {
        (0..p.len())
            .map(|i| CurvedEdge2d::Line {
                a: p[i],
                b: p[(i + 1) % p.len()],
            })
            .collect()
    };
    CurvedProfile {
        frame: PlaneFrame::default(),
        outer: wire(&[[0., 0.], [12., 0.], [12., 10.], [0., 10.]]),
        holes: vec![wire(&[[4., 3.], [4., 7.], [8., 7.], [8., 3.]])],
    }
}
fn box_at(size: [i64; 3], origin: [f64; 3]) -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        size.map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame {
            origin,
            ..Default::default()
        },
        GeometryPolicy::default(),
    )
    .unwrap()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

#[test]
fn rounded_travel_coefficients_cannot_use_nominal_caps_as_a_contact_proof() {
    let profile = CurvedProfile {
        frame: PlaneFrame::default(),
        outer: [
            ([-2., -1.], [2., -1.]),
            ([2., -1.], [2., 1.]),
            ([2., 1.], [-2., 1.]),
            ([-2., 1.], [-2., -1.]),
        ]
        .into_iter()
        .map(|(a, b)| CurvedEdge2d::Line { a, b })
        .collect(),
        holes: vec![],
    };
    let join = PlaneFrame {
        origin: [0., 0., 1.],
        u: [0.8, 0., -0.6],
        v: [0., 1., 0.],
    };
    let stock = |direction, planes| {
        let owner = EntityId::new();
        let origins: Vec<_> = (0..4)
            .map(|i| Origin {
                source: owner,
                role: format!("side.{i}"),
            })
            .collect();
        Solid::rust_extrude_between_planes_named(
            &profile,
            direction,
            planes,
            owner,
            &origins,
            GeometryPolicy::default(),
        )
        .unwrap()
    };
    let first = stock(
        [0.6, 0., 0.8],
        [
            PlaneFrame {
                origin: [0., 0., -10.],
                ..PlaneFrame::default()
            },
            join,
        ],
    );
    let second = stock(
        [-0.6, 0., 0.8],
        [
            join,
            PlaneFrame {
                origin: [0., 0., 10.],
                ..PlaneFrame::default()
            },
        ],
    );
    // The divisions defining the two travel laws round differently. Their
    // exact coefficient surfaces have a tiny positive common band at X=0,
    // despite the matching nominal cap origins and opposite normals.
    assert!(!matches!(
        first.boolean(
            &second,
            BooleanOperation::Intersect,
            GeometryPolicy::default()
        ),
        Ok(None)
    ));
}

#[test]
fn hollow_sloping_caps_prove_contact_without_losing_positive_common_material() {
    let policy = GeometryPolicy::default();
    let source = profile();
    let slope = 0.25_f64;
    let length = slope.hypot(1.);
    let join = PlaneFrame {
        origin: [0., 0., 10.],
        u: [1. / length, 0., slope / length],
        v: [0., 1., 0.],
    };
    let stock = |start, end| {
        let owner = EntityId::new();
        let origins: Vec<_> = (0..8)
            .map(|i| Origin {
                source: owner,
                role: format!("wall.{i}"),
            })
            .collect();
        Solid::rust_extrude_between_planes_named(
            &source,
            [0., 0., 1.],
            [start, end],
            owner,
            &origins,
            policy,
        )
        .unwrap()
    };
    let first = stock(PlaneFrame::default(), join);
    let end = PlaneFrame {
        origin: [0., 0., 25.],
        ..PlaneFrame::default()
    };
    let poses = [
        RigidTransform::default(),
        RigidTransform {
            rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
            translation_mm: [11., -7., 23.],
        },
    ];
    for pose in poses {
        let first = first.transformed(pose).unwrap();
        for penetration in [0., 1e-8, 0.25] {
            let second = stock(
                PlaneFrame {
                    origin: [0., 0., 10. - penetration],
                    ..join
                },
                end,
            )
            .transformed(pose)
            .unwrap();
            let result = first.boolean(&second, BooleanOperation::Intersect, policy);
            if penetration == 0. {
                assert!(result.unwrap().is_none());
            } else if penetration < policy.minimum_feature_mm {
                assert!(!matches!(result, Ok(None)), "penetration was lost");
            } else {
                let common = result.unwrap().unwrap();
                near(common.volume_mm3().unwrap(), 104. * penetration);
                assert_eq!(common.solid_count().unwrap(), 1);
                common
                    .manufacturing_mesh_with_receipt(Default::default())
                    .unwrap();
                let folder = tempfile::tempdir().unwrap();
                let path = folder.path().join("hollow-miter-common.step");
                common.write_step(&path).unwrap();
                near(
                    Solid::import_rust_step(
                        &std::fs::read_to_string(path).unwrap(),
                        EntityId::new(),
                        policy,
                    )
                    .unwrap()
                    .volume_mm3()
                    .unwrap(),
                    104. * penetration,
                );
            }
        }
    }
    // A large translation can round distinct cap positions to the same world
    // value. Repeating an identity placement also removes the shared immediate
    // placement chart: that must not turn a real overlap into proven contact.
    let far = RigidTransform {
        translation_mm: [0., 0., 1e6],
        ..RigidTransform::default()
    };
    let first = first.transformed(far).unwrap();
    let second = stock(
        PlaneFrame {
            origin: [0., 0., 10. - 1e-12],
            ..join
        },
        end,
    )
    .transformed(far)
    .unwrap()
    .transformed(RigidTransform::default())
    .unwrap();
    assert!(!matches!(
        first.boolean(&second, BooleanOperation::Intersect, policy),
        Ok(None)
    ));
}

#[test]
fn posed_hollow_stock_cuts_keep_material_and_untouched_wall_intents() {
    let policy = GeometryPolicy::default();
    for reverse in [false, true] {
        let height = if reverse { -10. } else { 10. };
        let owner = EntityId::new();
        let origins: Vec<_> = (0..8)
            .map(|i| Origin {
                source: owner,
                role: format!("wall.{i}"),
            })
            .collect();
        let stock = Solid::rust_extrude_linear_named(
            &profile(),
            [0., 0., height],
            0.,
            owner,
            &origins,
            policy,
        )
        .unwrap();
        let tool = box_at([7, 12, 8], [6., -1., if reverse { -13. } else { 5. }]);
        for pose in [
            RigidTransform::default(),
            RigidTransform {
                rotation: [[0.8, 0., 0.6], [0., 1., 0.], [-0.6, 0., 0.8]],
                translation_mm: [80., -35., 12.],
            },
        ] {
            let stock = stock.transformed(pose).unwrap();
            let tool = tool.transformed(pose).unwrap();
            let before = stock.topology_snapshot(owner, 0).unwrap();
            let left = before
                .elements
                .iter()
                .find(|e| {
                    e.key.kind == TopologyKind::Face && e.origins.iter().any(|o| o.role == "wall.3")
                })
                .unwrap();
            let intent = GeometryIntent::capture(&before, left.key).unwrap();
            let cut = stock
                .boolean(&tool, BooleanOperation::Subtract, policy)
                .unwrap()
                .unwrap();
            assert!(cut.uses_rust_geometry());
            assert_eq!(cut.solid_count().unwrap(), 1);
            near(stock.volume_mm3().unwrap(), 1040.);
            near(cut.volume_mm3().unwrap(), 780.);
            let after = cut.topology_snapshot(owner, 1).unwrap();
            let resolution = resolve(&intent, &after, 1);
            assert!(
                matches!(resolution, Resolution::Resolved { .. }),
                "{reverse} {pose:?}: {resolution:?}\nbefore: {left:?}\nafter: {:?}",
                after
                    .elements
                    .iter()
                    .filter(|e| e.key.kind == TopologyKind::Face
                        && e.origins.iter().any(|o| o.role == "wall.3"))
                    .collect::<Vec<_>>()
            );
            for (p, occupied) in [
                ([2., 2., 2.], true),
                ([5., 5., 2.], false),
                ([9., 5., 2.], true),
                ([9., 5., 8.], false),
                ([2., 5., 8.], true),
                ([13., 5., 2.], false),
            ] {
                let [x, y, z] = p;
                let z = if reverse { -z } else { z };
                let probe = Solid::rust_box(
                    EntityId::new().uuid_bits(),
                    [Rational::new(1, 10).unwrap(); 3],
                    PlaneFrame {
                        origin: [x - 0.05, y - 0.05, z - 0.05],
                        ..Default::default()
                    },
                    policy,
                )
                .unwrap()
                .transformed(pose)
                .unwrap();
                let volume = cut
                    .boolean(&probe, BooleanOperation::Intersect, policy)
                    .unwrap()
                    .map_or(0., |s| s.volume_mm3().unwrap());
                near(volume, if occupied { 0.001 } else { 0. });
            }
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("cut.step");
            cut.write_step(&path).unwrap();
            if multicad_kernel::OCCT_LINKED {
                let native = Solid::extrude_curved_profile_vector_named(
                    &profile(),
                    [0., 0., height],
                    owner,
                    &origins,
                    policy,
                )
                .unwrap()
                .transformed(pose)
                .unwrap();
                let native_tool = Solid::box_solid([7., 12., 8.], policy)
                    .unwrap()
                    .transformed(RigidTransform {
                        translation_mm: [6., -1., if reverse { -13. } else { 5. }],
                        ..Default::default()
                    })
                    .unwrap()
                    .transformed(pose)
                    .unwrap();
                let expected = native
                    .boolean(&native_tool, BooleanOperation::Subtract, policy)
                    .unwrap()
                    .unwrap();
                near(cut.volume_mm3().unwrap(), expected.volume_mm3().unwrap());
                let imported = Solid::read_step(&path, policy).unwrap();
                assert_eq!(imported.solid_count().unwrap(), 1);
                near(imported.volume_mm3().unwrap(), 780.);
                let [a, b] = [cut.bounds().unwrap(), imported.bounds().unwrap()];
                for i in 0..3 {
                    near(a.min[i], b.min[i]);
                    near(a.max[i], b.max[i]);
                }
            }
        }
    }
}

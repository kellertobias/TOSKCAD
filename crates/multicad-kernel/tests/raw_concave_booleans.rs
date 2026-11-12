use multicad_core::{document::EntityId, references::*};
use multicad_kernel::{
    rust_kernel::{Rational, polygon::PolygonPrism},
    *,
};
const POINTS: [[i64; 2]; 8] = [
    [0, 0],
    [6, 0],
    [6, 5],
    [4, 5],
    [4, 2],
    [2, 2],
    [2, 5],
    [0, 5],
];
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 3e-6, "{a} != {b}");
}
fn source(linear: bool, reverse: bool) -> Solid {
    let owner = EntityId::new();
    let policy = GeometryPolicy::default();
    let origins: Vec<_> = (0..8)
        .map(|i| Origin {
            source: owner,
            role: format!("source.edge.{i}"),
        })
        .collect();
    if linear {
        let p = POINTS.map(|p| p.map(|n| n as f64));
        let profile = CurvedProfile {
            frame: PlaneFrame::default(),
            outer: p
                .into_iter()
                .zip(p.into_iter().cycle().skip(1))
                .map(|(a, b)| CurvedEdge2d::Line { a, b })
                .collect(),
            holes: vec![],
        };
        Solid::rust_extrude_linear_named(
            &profile,
            [1.5, -0.75, if reverse { -6. } else { 6. }],
            0.,
            owner,
            &origins,
            policy,
        )
        .unwrap()
    } else {
        let n = |n| Rational::new(n, 1).unwrap();
        Solid::from_polygon_prism_sweep(
            PolygonPrism::new(vec![POINTS.map(|p| p.map(n)).to_vec()], n(4)).unwrap(),
            PlaneFrame::default(),
            reverse,
            owner,
            &origins,
            &[false; 8],
            policy,
        )
        .unwrap()
    }
}
fn box_at(origin: [f64; 3], size: [i64; 3], angled: bool) -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        size.map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame {
            origin,
            u: if angled { [0.8, 0.6, 0.] } else { [1., 0., 0.] },
            v: if angled {
                [-0.6, 0.8, 0.]
            } else {
                [0., 1., 0.]
            },
        },
        GeometryPolicy::default(),
    )
    .unwrap()
}
fn compare(a: &Solid, b: &Solid) {
    near(a.volume_mm3().unwrap(), b.volume_mm3().unwrap());
    let (x, y) = (a.bounds().unwrap(), b.bounds().unwrap());
    for (a, b) in x
        .min
        .into_iter()
        .chain(x.max)
        .zip(y.min.into_iter().chain(y.max))
    {
        near(a, b);
    }
    near(
        a.faces().unwrap().iter().map(|f| f.area_mm2).sum(),
        b.faces().unwrap().iter().map(|f| f.area_mm2).sum(),
    );
}
#[test]
fn raw_concave_cut_join_and_intersection_queries_use_the_authored_material_in_every_pose() {
    let policy = GeometryPolicy::default();
    let posed = RigidTransform {
        rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
        translation_mm: [11., -7., 23.],
    };
    for linear in [false, true] {
        for reverse in [false, true] {
            for pose in [RigidTransform::default(), posed] {
                let source = source(linear, reverse).transformed(pose).unwrap();
                let tool = box_at([-0.2, 0.5, if reverse { -1.5 } else { 0.5 }], [1; 3], true)
                    .transformed(pose)
                    .unwrap();
                let owner = EntityId::new();
                let before = source.topology_snapshot(owner, 0).unwrap();
                let tool_before = tool.topology_snapshot(owner, 0).unwrap();
                let removed = if linear {
                    25. / 24. * ((0.6_f64 - 0.25).powi(2) + 0.25_f64.powi(2) / 12.)
                } else {
                    0.375
                };
                let volume = source.volume_mm3().unwrap();
                let common = source
                    .rust_planar_intersection_metrics(&tool, policy)
                    .unwrap()
                    .unwrap();
                near(common.0, removed);
                let cut = source
                    .rust_subtract_planar_tools(&[&tool], EntityId::new(), policy)
                    .unwrap()
                    .unwrap();
                near(cut.volume_mm3().unwrap(), volume - removed);
                let joined = Solid::union_planar_pieces(&[&source, &tool], EntityId::new(), policy)
                    .unwrap()
                    .unwrap();
                near(joined.volume_mm3().unwrap(), volume + 1. - removed);
                assert_eq!(joined.solid_count().unwrap(), 1);
                let default_cut = source
                    .boolean(&tool, BooleanOperation::Subtract, policy)
                    .expect("public cut uses certified planar material")
                    .unwrap();
                let default_join = source
                    .boolean(&tool, BooleanOperation::Union, policy)
                    .expect("public join uses certified planar material")
                    .unwrap();
                let default_common = source
                    .boolean(&tool, BooleanOperation::Intersect, policy)
                    .unwrap()
                    .unwrap();
                near(default_cut.volume_mm3().unwrap(), volume - removed);
                near(default_join.volume_mm3().unwrap(), volume + 1. - removed);
                near(default_common.volume_mm3().unwrap(), removed);
                // The right exterior wall is unaffected and retains its authored frame.
                let right = before
                    .elements
                    .iter()
                    .find(|e| {
                        e.frames
                            .iter()
                            .any(|a| a.origin.role.contains("source.edge.1"))
                    })
                    .unwrap();
                let intent = GeometryIntent::capture(&before, right.key).unwrap();
                for result in [&cut, &joined, &default_cut, &default_join] {
                    let topology = result.topology_snapshot(owner, 1).unwrap();
                    assert!(matches!(
                        resolve(&intent, &topology, 1),
                        Resolution::Resolved { .. }
                    ));
                    result
                        .manufacturing_mesh_with_receipt(MeshOptions::default())
                        .unwrap();
                    let folder = tempfile::tempdir().unwrap();
                    let path = folder.path().join("raw-concave.step");
                    result.write_step(&path).unwrap();
                    let read = Solid::import_rust_step(
                        &std::fs::read_to_string(&path).unwrap(),
                        EntityId::new(),
                        policy,
                    )
                    .unwrap();
                    compare(result, &read);
                    if OCCT_LINKED {
                        compare(result, &Solid::read_step(&path, policy).unwrap());
                    }
                    let later_tool =
                        box_at([5.5, 0.5, if reverse { -1.5 } else { 0.5 }], [1; 3], false)
                            .transformed(pose)
                            .unwrap();
                    let later = result
                        .rust_subtract_planar_tools(&[&later_tool], EntityId::new(), policy)
                        .unwrap()
                        .unwrap();
                    near(
                        later.volume_mm3().unwrap(),
                        result.volume_mm3().unwrap() - if linear { 0.75 } else { 0.5 },
                    );
                    later
                        .manufacturing_mesh_with_receipt(MeshOptions::default())
                        .unwrap();
                    let later_path = folder.path().join("later-cut.step");
                    later.write_step(&later_path).unwrap();
                    let read = Solid::import_rust_step(
                        &std::fs::read_to_string(&later_path).unwrap(),
                        EntityId::new(),
                        policy,
                    )
                    .unwrap();
                    compare(&later, &read);
                }
                let void = box_at([2.5, 3., if reverse { -1.5 } else { 0.5 }], [1; 3], false)
                    .transformed(pose)
                    .unwrap();
                assert!(
                    source
                        .rust_planar_intersection_metrics(&void, policy)
                        .unwrap_or_else(|e| panic!(
                            "linear={linear} reverse={reverse} pose={pose:?}: {e}"
                        ))
                        .is_none()
                );
                if OCCT_LINKED {
                    let folder = tempfile::tempdir().unwrap();
                    let a = folder.path().join("source.step");
                    let b = folder.path().join("tool.step");
                    source.write_step(&a).unwrap();
                    tool.write_step(&b).unwrap();
                    let a = Solid::read_step(&a, policy).unwrap();
                    let b = Solid::read_step(&b, policy).unwrap();
                    compare(
                        &cut,
                        &a.boolean(&b, BooleanOperation::Subtract, policy)
                            .unwrap()
                            .unwrap(),
                    );
                    compare(
                        &joined,
                        &a.boolean(&b, BooleanOperation::Union, policy)
                            .unwrap()
                            .unwrap(),
                    );
                    let actual = a
                        .boolean(&b, BooleanOperation::Intersect, policy)
                        .unwrap()
                        .unwrap();
                    near(common.0, actual.volume_mm3().unwrap());
                    let native = actual.bounds().unwrap();
                    for (a, b) in common
                        .1
                        .min
                        .into_iter()
                        .chain(common.1.max)
                        .zip(native.min.into_iter().chain(native.max))
                    {
                        near(a, b);
                    }
                }
                assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
                assert_eq!(tool.topology_snapshot(owner, 0).unwrap(), tool_before);
            }
        }
    }
}
#[test]
fn selected_raw_concave_proof_limits_and_invalid_sources_cannot_fall_back_or_mutate_stock() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let integer = |n| Rational::new(n, 1).unwrap();
    let tool = box_at([-0.2, 0.5, 0.5], [1; 3], true);
    let mut ring: Vec<_> = (0..=120)
        .map(|i| [Rational::new(i, 20).unwrap(), integer(0)])
        .collect();
    ring.extend(POINTS[2..].iter().map(|p| p.map(integer)));
    let origins: Vec<_> = (0..ring.len())
        .map(|i| Origin {
            source: owner,
            role: format!("limit.edge.{i}"),
        })
        .collect();
    let source = Solid::from_polygon_prism(
        PolygonPrism::new(vec![ring], integer(4)).unwrap(),
        PlaneFrame::default(),
        owner,
        &origins,
        &[false; 127],
        policy,
    )
    .unwrap();
    let before = source.topology_snapshot(owner, 0).unwrap();
    assert_eq!(
        source
            .rust_subtract_planar_tools(&[&tool], EntityId::new(), policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Limit
    );
    assert_eq!(
        Solid::union_planar_pieces(&[&source, &tool], EntityId::new(), policy)
            .unwrap()
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Limit
    );
    assert_eq!(
        source
            .rust_planar_intersection_metrics(&tool, policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Limit
    );
    assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
    let ring = vec![
        [integer(0), integer(0)],
        [integer(6), integer(0)],
        [integer(6), integer(6)],
        [integer(4), integer(6)],
        [Rational::new(1, 2_000_000).unwrap(); 2],
        [integer(2), integer(6)],
        [integer(0), integer(6)],
    ];
    let origins: Vec<_> = (0..7)
        .map(|i| Origin {
            source: owner,
            role: format!("near.edge.{i}"),
        })
        .collect();
    let merge_policy = GeometryPolicy {
        linear_tolerance_mm: 1e-3,
        minimum_feature_mm: 0.01,
        ..policy
    };
    let source = Solid::from_polygon_prism(
        PolygonPrism::new(vec![ring], integer(4)).unwrap(),
        PlaneFrame::default(),
        owner,
        &origins,
        &[false; 7],
        merge_policy,
    )
    .unwrap();
    let before = source.topology_snapshot(owner, 0).unwrap();
    let tool = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [integer(1); 3],
        PlaneFrame::default(),
        merge_policy,
    )
    .unwrap();
    assert_eq!(
        source
            .rust_subtract_planar_tools(&[&tool], EntityId::new(), merge_policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::InvalidResult
    );
    assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
}
#[test]
fn raw_concave_admission_keeps_multiwire_sources_outside_its_scope() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let n = |n| Rational::new(n, 1).unwrap();
    let prism = PolygonPrism::new(
        vec![
            [[0, 0], [10, 0], [10, 10], [0, 10]]
                .map(|p| p.map(n))
                .to_vec(),
            [[2, 2], [2, 4], [4, 2]].map(|p| p.map(n)).to_vec(),
        ],
        n(4),
    )
    .unwrap();
    let origins: Vec<_> = (0..7)
        .map(|i| Origin {
            source: owner,
            role: format!("hole.edge.{i}"),
        })
        .collect();
    let source = Solid::from_polygon_prism(
        prism,
        PlaneFrame::default(),
        owner,
        &origins,
        &[false; 7],
        policy,
    )
    .unwrap();
    let before = source.topology_snapshot(owner, 0).unwrap();
    let tool = box_at([-0.2, 0.5, 0.5], [1; 3], true);
    assert_eq!(
        source
            .rust_subtract_planar_tools(&[&tool], EntityId::new(), policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Unsupported
    );
    assert!(Solid::union_planar_pieces(&[&source, &tool], EntityId::new(), policy).is_none());
    assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
}

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
    assert!((a - b).abs() < 2e-6, "{a} != {b}");
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
            if reverse {
                [-1.5, 0.75, -6.]
            } else {
                [1.5, -0.75, 6.]
            },
            0.,
            owner,
            &origins,
            policy,
        )
        .unwrap()
    } else {
        let integer = |n| Rational::new(n, 1).unwrap();
        let prism =
            PolygonPrism::new(vec![POINTS.map(|p| p.map(integer)).to_vec()], integer(4)).unwrap();
        Solid::from_polygon_prism_sweep(
            prism,
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
fn point(transform: AffineTransform, p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| {
        transform.matrix[i]
            .into_iter()
            .zip(p)
            .map(|(a, b)| a * b)
            .sum::<f64>()
            + transform.translation_mm[i]
    })
}
fn close(a: [f64; 3], b: [f64; 3]) {
    for (a, b) in a.into_iter().zip(b) {
        near(a, b);
    }
}
#[test]
fn concave_single_wire_scale_preserves_caps_all_boundaries_material_mesh_and_step() {
    let policy = GeometryPolicy::default();
    let posed = RigidTransform {
        rotation: [[0.8, -0.36, 0.48], [0.6, 0.48, -0.64], [0., 0.8, 0.6]],
        translation_mm: [11., -7., 23.],
    };
    for linear in [false, true] {
        for reverse in [false, true] {
            for pose in [RigidTransform::default(), posed] {
                for matrix in [
                    [[2., 0., 0.], [0., 2., 0.], [0., 0., 2.]],
                    [[2.5, -0.5, 0.], [-0.5, 2.5, 0.], [0., 0., 0.5]],
                ] {
                    let source = source(linear, reverse).transformed(pose).unwrap();
                    let owner = EntityId::new();
                    let before = source.topology_snapshot(owner, 0).unwrap();
                    let transform = AffineTransform {
                        matrix,
                        translation_mm: [-2.5, -4.5, 2.5],
                    };
                    let determinant = if matrix[0][1] == 0. { 8. } else { 3. };
                    let result = source
                        .affine_transformed_named(transform, EntityId::new(), policy)
                        .unwrap();
                    let current = result.topology_snapshot(owner, 1).unwrap();
                    assert_eq!(current.elements.len(), before.elements.len());
                    assert_eq!(result.faces().unwrap().len(), 10);
                    near(
                        result.volume_mm3().unwrap(),
                        source.volume_mm3().unwrap() * determinant,
                    );
                    for element in &before.elements {
                        let intent = GeometryIntent::capture(&before, element.key).unwrap();
                        let Resolution::Resolved {
                            key,
                            geometry: Some(actual),
                            ..
                        } = resolve(&intent, &current, 1)
                        else {
                            panic!("lost concave boundary {intent:?}")
                        };
                        match (element.geometry.unwrap(), actual) {
                            (
                                AnalyticGeometry::Point { position_mm: a },
                                AnalyticGeometry::Point { position_mm: b },
                            ) => close(point(transform, a), b),
                            (
                                AnalyticGeometry::Line { a_mm: a, b_mm: b },
                                AnalyticGeometry::Line { a_mm: c, b_mm: d },
                            ) => {
                                let distance = |a: [f64; 3], b: [f64; 3]| {
                                    a.into_iter()
                                        .zip(b)
                                        .map(|(a, b)| (a - b).abs())
                                        .sum::<f64>()
                                };
                                let a = point(transform, a);
                                let b = point(transform, b);
                                assert!(
                                    (distance(a, c) + distance(b, d))
                                        .min(distance(a, d) + distance(b, c))
                                        < 1e-5
                                );
                            }
                            (AnalyticGeometry::Plane { .. }, AnalyticGeometry::Plane { .. }) => {
                                let old = &source.faces().unwrap()[element.key.ordinal as usize];
                                let new = &result.faces().unwrap()[key.ordinal as usize];
                                close(point(transform, old.center_mm), new.center_mm);
                                let mapped =
                                    current.elements.iter().find(|e| e.key == key).unwrap();
                                for anchor in &element.frames {
                                    let actual = mapped
                                        .frames
                                        .iter()
                                        .find(|a| a.origin == anchor.origin)
                                        .unwrap();
                                    close(
                                        point(transform, anchor.frame.origin_mm),
                                        actual.frame.origin_mm,
                                    );
                                }
                            }
                            pair => panic!("unexpected geometry {pair:?}"),
                        }
                    }
                    result
                        .manufacturing_mesh_with_receipt(MeshOptions::default())
                        .unwrap();
                    let again = result
                        .affine_transformed_named(transform, EntityId::new(), policy)
                        .unwrap();
                    near(
                        again.volume_mm3().unwrap(),
                        source.volume_mm3().unwrap() * determinant * determinant,
                    );
                    let folder = tempfile::tempdir().unwrap();
                    let original = folder.path().join("source.step");
                    source.write_step(&original).unwrap();
                    let path = folder.path().join("scaled.step");
                    result.write_step(&path).unwrap();
                    let imported = Solid::import_rust_step(
                        &std::fs::read_to_string(&path).unwrap(),
                        EntityId::new(),
                        policy,
                    )
                    .unwrap();
                    near(imported.volume_mm3().unwrap(), result.volume_mm3().unwrap());
                    close(imported.bounds().unwrap().min, result.bounds().unwrap().min);
                    close(imported.bounds().unwrap().max, result.bounds().unwrap().max);
                    if OCCT_LINKED {
                        let native = Solid::read_step(&original, policy)
                            .unwrap()
                            .affine_transformed_named(transform, EntityId::new(), policy)
                            .unwrap();
                        near(native.volume_mm3().unwrap(), result.volume_mm3().unwrap());
                        close(native.bounds().unwrap().min, result.bounds().unwrap().min);
                        close(native.bounds().unwrap().max, result.bounds().unwrap().max);
                        near(
                            native.faces().unwrap().iter().map(|f| f.area_mm2).sum(),
                            result.faces().unwrap().iter().map(|f| f.area_mm2).sum(),
                        );
                    }
                    assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
                }
            }
        }
    }
}
#[test]
fn concave_scale_material_remains_exact_for_a_subsequent_cut() {
    let policy = GeometryPolicy::default();
    let transform = AffineTransform {
        matrix: [[2.5, -0.5, 0.], [-0.5, 2.5, 0.], [0., 0., 0.5]],
        translation_mm: [-2.5, -4.5, 2.5],
    };
    let tool = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [1, 1, 1].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame {
            origin: [0.5, 0.5, 0.5],
            ..Default::default()
        },
        policy,
    )
    .unwrap();
    for linear in [false, true] {
        let source = source(linear, false);
        let result = source
            .affine_transformed_named(transform, EntityId::new(), policy)
            .unwrap();
        let mapped_tool = tool
            .affine_transformed_named(transform, EntityId::new(), policy)
            .unwrap();
        let cut = result
            .rust_subtract_planar_tools(&[&mapped_tool], EntityId::new(), policy)
            .unwrap()
            .unwrap();
        near(
            cut.volume_mm3().unwrap(),
            (source.volume_mm3().unwrap() - 1.) * 3.,
        );
        cut.manufacturing_mesh_with_receipt(MeshOptions::default())
            .unwrap();
    }
}

#[test]
fn concave_scale_refuses_multiloop_sources_and_unresolved_placement_without_source_changes() {
    let policy = GeometryPolicy::default();
    let integer = |n| Rational::new(n, 1).unwrap();
    let owner = EntityId::new();
    let prism = PolygonPrism::new(
        vec![
            [[0, 0], [10, 0], [10, 10], [0, 10]]
                .map(|p| p.map(integer))
                .to_vec(),
            [[2, 2], [2, 4], [4, 4], [4, 2]]
                .map(|p| p.map(integer))
                .to_vec(),
        ],
        integer(4),
    )
    .unwrap();
    let origins: Vec<_> = (0..8)
        .map(|i| Origin {
            source: owner,
            role: format!("hole.edge.{i}"),
        })
        .collect();
    let holed = Solid::from_polygon_prism(
        prism,
        PlaneFrame::default(),
        owner,
        &origins,
        &[false; 8],
        policy,
    )
    .unwrap();
    let original = holed.topology_snapshot(owner, 0).unwrap();
    assert_eq!(
        holed
            .affine_transformed_named(
                AffineTransform {
                    matrix: [[2., 0., 0.], [0., 2., 0.], [0., 0., 2.]],
                    ..Default::default()
                },
                EntityId::new(),
                policy
            )
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Unsupported
    );
    assert_eq!(holed.topology_snapshot(owner, 0).unwrap(), original);
    let source = source(false, false);
    let original = source.topology_snapshot(owner, 0).unwrap();
    let outside = AffineTransform {
        matrix: [[1e6, 0., 0.], [0., 1e6, 0.], [0., 0., 1e6]],
        translation_mm: [policy.maximum_coordinate_mm; 3],
    };
    assert!(
        source
            .affine_transformed_named(outside, EntityId::new(), policy)
            .is_err()
    );
    assert_eq!(source.topology_snapshot(owner, 0).unwrap(), original);
    // Nonadjacent authored vertices are distinct even inside the boundary
    // constructor's numerical matching band. Their identities must not merge.
    let tiny = Rational::new(1, 2_000_000).unwrap();
    let merge_policy = GeometryPolicy {
        linear_tolerance_mm: 1e-3,
        minimum_feature_mm: 0.01,
        ..policy
    };
    let zero = integer(0);
    let ring = vec![
        [zero, zero],
        [integer(6), zero],
        [integer(6), integer(6)],
        [integer(4), integer(6)],
        [tiny, tiny],
        [integer(2), integer(6)],
        [zero, integer(6)],
    ];
    let origins: Vec<_> = (0..7)
        .map(|i| Origin {
            source: owner,
            role: format!("near.edge.{i}"),
        })
        .collect();
    let near_source = Solid::from_polygon_prism(
        PolygonPrism::new(vec![ring], integer(4)).unwrap(),
        PlaneFrame::default(),
        owner,
        &origins,
        &[false; 7],
        merge_policy,
    )
    .unwrap();
    let original = near_source.topology_snapshot(owner, 0).unwrap();
    assert_eq!(
        near_source
            .affine_transformed_named(
                AffineTransform {
                    matrix: [[2., 0., 0.], [0., 2., 0.], [0., 0., 2.]],
                    ..Default::default()
                },
                EntityId::new(),
                merge_policy
            )
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::InvalidResult
    );
    assert_eq!(near_source.topology_snapshot(owner, 0).unwrap(), original);
}

#[test]
fn concave_scale_preserves_the_selected_partition_limit_and_source_geometry() {
    let policy = GeometryPolicy::default();
    let owner = EntityId::new();
    let mut ring: Vec<_> = (0..=120)
        .map(|i| [Rational::new(i, 20).unwrap(), Rational::new(0, 1).unwrap()])
        .collect();
    ring.extend(
        POINTS[2..]
            .iter()
            .map(|p| p.map(|n| Rational::new(n, 1).unwrap())),
    );
    assert_eq!(ring.len(), 127);
    let origins: Vec<_> = (0..ring.len())
        .map(|i| Origin {
            source: owner,
            role: format!("limit.edge.{i}"),
        })
        .collect();
    let source = Solid::from_polygon_prism(
        PolygonPrism::new(vec![ring], Rational::new(4, 1).unwrap()).unwrap(),
        PlaneFrame::default(),
        owner,
        &origins,
        &[false; 127],
        policy,
    )
    .unwrap();
    let before = source.topology_snapshot(owner, 0).unwrap();
    let error = source
        .affine_transformed_named(
            AffineTransform {
                matrix: [[2., 0., 0.], [0., 2., 0.], [0., 0., 2.]],
                ..Default::default()
            },
            EntityId::new(),
            policy,
        )
        .err()
        .unwrap();
    assert_eq!(error.kind, GeometryErrorKind::Limit);
    assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
}

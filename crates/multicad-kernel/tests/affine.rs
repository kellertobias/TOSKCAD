use multicad_core::{document::EntityId, references::*};
use multicad_kernel::*;

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}
fn bounds(s: &Solid, min: [f64; 3], max: [f64; 3]) {
    let value = s.bounds().unwrap();
    for (a, b) in value
        .min
        .into_iter()
        .chain(value.max)
        .zip(min.into_iter().chain(max))
    {
        near(a, b);
    }
}
fn counts(snapshot: &TopologySnapshot) -> [usize; 3] {
    [TopologyKind::Face, TopologyKind::Edge, TopologyKind::Vertex].map(|kind| {
        snapshot
            .elements
            .iter()
            .filter(|e| e.key.kind == kind)
            .count()
    })
}
fn frame_for(snapshot: &TopologySnapshot, feature: EntityId, role: &str) -> ReferenceFrame {
    snapshot
        .elements
        .iter()
        .flat_map(|e| &e.frames)
        .find(|a| a.origin.source == feature && a.origin.role == role)
        .unwrap()
        .frame
}

#[test]
fn uniform_scale_about_pivot_preserves_exact_geometry_source_and_all_named_intents() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let source = Solid::box_named([10., 20., 30.], feature, policy).unwrap();
    let before = source.topology_snapshot(feature, 0).unwrap();
    let scaled = source
        .affine_transformed_named(
            AffineTransform {
                matrix: [[2., 0., 0.], [0., 2., 0.], [0., 0., 2.]],
                translation_mm: [-1., -2., -3.],
            },
            EntityId::new(),
            policy,
        )
        .unwrap();
    bounds(&source, [0.; 3], [10., 20., 30.]);
    bounds(&scaled, [-1., -2., -3.], [19., 38., 57.]);
    near(scaled.volume_mm3().unwrap(), 48_000.);
    assert_eq!(source.topology_snapshot(feature, 0).unwrap(), before);
    let after = scaled.topology_snapshot(feature, 1).unwrap();
    assert_eq!(counts(&after), [6, 12, 8]);
    for element in &before.elements {
        let intent = GeometryIntent::capture(&before, element.key).unwrap();
        assert!(matches!(
            resolve(&intent, &after, 1),
            Resolution::Resolved { .. }
        ));
    }
    let cap = frame_for(&after, feature, "cap.end");
    assert_eq!(cap.origin_mm, [-1., -2., 57.]);
    assert_eq!(cap.u, [1., 0., 0.]);
    assert_eq!(cap.v, [0., 1., 0.]);
}

#[test]
fn rotated_frame_per_axis_scale_maps_exact_planes_lines_frames_and_step_without_mutating_source() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let source = Solid::box_named([10., 20., 30.], feature, policy).unwrap();
    let before = source.topology_snapshot(feature, 0).unwrap();
    // R(45deg) * diag(2,3,0.5) * R^-1, around pivot [3,4,5].
    let transform = AffineTransform {
        matrix: [[2.5, -0.5, 0.], [-0.5, 2.5, 0.], [0., 0., 0.5]],
        translation_mm: [-2.5, -4.5, 2.5],
    };
    let scaled = source
        .affine_transformed_named(transform, EntityId::new(), policy)
        .unwrap();
    bounds(&scaled, [-12.5, -9.5, 2.5], [22.5, 45.5, 17.5]);
    near(scaled.volume_mm3().unwrap(), 18_000.);
    assert!(scaled.faces().unwrap().iter().all(|f| f.plane.is_some()));
    assert_eq!(source.topology_snapshot(feature, 0).unwrap(), before);
    let snapshot = scaled.topology_snapshot(feature, 1).unwrap();
    assert_eq!(counts(&snapshot), [6, 12, 8]);
    assert_eq!(
        snapshot
            .elements
            .iter()
            .filter(|e| matches!(e.geometry, Some(AnalyticGeometry::Plane { .. })))
            .count(),
        6
    );
    assert_eq!(
        snapshot
            .elements
            .iter()
            .filter(|e| matches!(e.geometry, Some(AnalyticGeometry::Line { .. })))
            .count(),
        12
    );
    let normal = [2.5 / 6.5_f64.sqrt(), 0.5 / 6.5_f64.sqrt(), 0.];
    let side = before
        .elements
        .iter()
        .find(|e| e.frames.iter().any(|f| f.frame.normal() == [1., 0., 0.]))
        .unwrap();
    let old = &side.frames[0];
    let side = snapshot
        .elements
        .iter()
        .find(|e| e.origins == side.origins)
        .unwrap();
    let frame = side
        .frames
        .iter()
        .find(|f| f.origin == old.origin)
        .unwrap()
        .frame;
    frame.validate().unwrap();
    for (a, b) in frame.normal().into_iter().zip(normal) {
        near(a, b);
    }
    near(frame.origin_mm[0], 22.5);
    near(frame.origin_mm[1], -9.5);
    near(frame.origin_mm[2], 2.5);
    for element in &before.elements {
        let intent = GeometryIntent::capture(&before, element.key).unwrap();
        assert!(matches!(
            resolve(&intent, &snapshot, 1),
            Resolution::Resolved { .. }
        ));
    }
    // A later rigid placement must keep the exact affine-proven planar and
    // linear descriptors when the native surfaces remain NURBS.
    let placed = scaled
        .transformed(RigidTransform {
            translation_mm: [100., 0., 0.],
            ..Default::default()
        })
        .unwrap();
    let placed = placed.topology_snapshot(feature, 2).unwrap();
    assert_eq!(
        placed
            .elements
            .iter()
            .filter(|e| matches!(e.geometry, Some(AnalyticGeometry::Plane { .. })))
            .count(),
        6
    );
    assert_eq!(
        placed
            .elements
            .iter()
            .filter(|e| matches!(e.geometry, Some(AnalyticGeometry::Line { .. })))
            .count(),
        12
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("scaled-box.step");
    scaled.write_step(&path).unwrap();
    let loaded = Solid::read_step(&path, policy).unwrap();
    bounds(&loaded, [-12.5, -9.5, 2.5], [22.5, 45.5, 17.5]);
    near(loaded.volume_mm3().unwrap(), 18_000.);
}

#[test]
fn nonuniform_cylinder_is_exact_brep_and_never_retains_false_circular_metadata() {
    let policy = GeometryPolicy::default();
    let source = Solid::cylinder(PlaneFrame::default(), 5., 10., policy).unwrap();
    let initial = source.topology_snapshot(EntityId::new(), 0).unwrap();
    assert!(
        initial
            .elements
            .iter()
            .any(|e| matches!(e.geometry, Some(AnalyticGeometry::CircularArc { .. })))
    );
    let uniform = source
        .affine_transformed_named(
            AffineTransform {
                matrix: [[2., 0., 0.], [0., 2., 0.], [0., 0., 2.]],
                ..Default::default()
            },
            EntityId::new(),
            policy,
        )
        .unwrap();
    let snapshot = uniform.topology_snapshot(EntityId::new(), 0).unwrap();
    assert!(snapshot.elements.iter().any(|e|matches!(e.geometry,Some(AnalyticGeometry::CircularArc{radius_mm,..}) if (radius_mm-10.).abs()<1e-7)));
    let scaled = source
        .affine_transformed_named(
            AffineTransform {
                matrix: [[2., 0., 0.], [0., 1., 0.], [0., 0., 3.]],
                ..Default::default()
            },
            EntityId::new(),
            policy,
        )
        .unwrap();
    bounds(&source, [-5., -5., 0.], [5., 5., 10.]);
    bounds(&scaled, [-10., -5., 0.], [10., 5., 30.]);
    let expected = 1500. * std::f64::consts::PI;
    assert!((scaled.volume_mm3().unwrap() - expected).abs() / expected < 1e-6);
    let snapshot = scaled.topology_snapshot(EntityId::new(), 0).unwrap();
    assert!(
        !snapshot
            .elements
            .iter()
            .any(|e| matches!(e.geometry, Some(AnalyticGeometry::CircularArc { .. })))
    );
    assert!(
        snapshot
            .elements
            .iter()
            .any(|e| e.key.kind == TopologyKind::Edge && e.geometry.is_none())
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("elliptic-cylinder.step");
    scaled.write_step(&path).unwrap();
    let loaded = Solid::read_step(&path, policy).unwrap();
    bounds(&loaded, [-10., -5., 0.], [10., 5., 30.]);
    assert!((loaded.volume_mm3().unwrap() - expected).abs() / expected < 1e-6);
}

#[test]
fn affine_validation_rejects_nonfinite_singular_reflected_and_unbounded_transforms_without_source_edits()
 {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let source = Solid::box_named([10., 20., 30.], feature, policy).unwrap();
    let before = source.topology_snapshot(feature, 0).unwrap();
    for matrix in [
        [[f64::NAN, 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        [[1., 0., 0.], [0., 0., 0.], [0., 0., 1.]],
        [[-1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        [[1e7, 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        [[1., 0., 0.], [1., 1e-15, 0.], [0., 0., 1.]],
    ] {
        let result = source.affine_transformed_named(
            AffineTransform {
                matrix,
                ..Default::default()
            },
            EntityId::new(),
            policy,
        );
        assert!(matches!(
            result,
            Err(GeometryError {
                kind: GeometryErrorKind::Input,
                ..
            })
        ));
    }
    assert!(
        source
            .affine_transformed_named(
                Default::default(),
                EntityId::parse("00000000-0000-0000-0000-000000000000").unwrap(),
                policy
            )
            .is_err()
    );
    assert!(
        AffineTransform {
            translation_mm: [f64::INFINITY, 0., 0.],
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        source
            .affine_transformed_named(
                AffineTransform {
                    matrix: [[1e6, 0., 0.], [0., 1e6, 0.], [0., 0., 1e6]],
                    ..Default::default()
                },
                EntityId::new(),
                policy
            )
            .is_err()
    );
    assert_eq!(source.topology_snapshot(feature, 0).unwrap(), before);
    bounds(&source, [0.; 3], [10., 20., 30.]);
}

#[test]
fn general_affine_keeps_supplied_small_off_axis_coefficients() {
    let policy = GeometryPolicy::default();
    let source = Solid::box_solid([10., 20., 30.], policy).unwrap();
    let scaled = source
        .affine_transformed_named(
            AffineTransform {
                matrix: [[1., 1e-8, 0.], [0., 1., 0.], [0., 0., 1.]],
                ..Default::default()
            },
            EntityId::new(),
            policy,
        )
        .unwrap();
    let snapshot = scaled.topology_snapshot(EntityId::new(), 0).unwrap();
    let maximum = snapshot
        .elements
        .iter()
        .filter_map(|e| match e.geometry {
            Some(AnalyticGeometry::Point { position_mm }) => Some(position_mm[0]),
            _ => None,
        })
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        (maximum - (10. + 20. * 1e-8)).abs() < 1e-12,
        "native vertex {maximum}"
    );
    assert!(maximum > 10.);
    near(scaled.bounds().unwrap().max[0], maximum);
    near(scaled.volume_mm3().unwrap(), 6000.);
}

#[test]
fn rust_affine_preserves_oblique_convex_sweep_ancestry_and_native_metrics() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let points = [[0., 0.], [4., 0.], [0., 3.]];
    let profile = CurvedProfile {
        frame: PlaneFrame::default(),
        outer: points
            .into_iter()
            .zip(points.into_iter().cycle().skip(1))
            .map(|(a, b)| CurvedEdge2d::Line { a, b })
            .collect(),
        holes: vec![],
    };
    let origins: Vec<_> = (0..3)
        .map(|index| Origin {
            source: feature,
            role: format!("profile/{index}"),
        })
        .collect();
    let sweep = [1., -2., 5.];
    let source =
        Solid::rust_extrude_linear_named(&profile, sweep, 0., feature, &origins, policy).unwrap();
    let authored = source.topology_snapshot(feature, 0).unwrap();
    let transform = AffineTransform {
        matrix: [[2.5, -0.5, 0.], [-0.5, 2.5, 0.], [0., 0., 0.5]],
        translation_mm: [-2.5, -4.5, 2.5],
    };
    let output = EntityId::new();
    let rust = source
        .affine_transformed_named(transform, output, policy)
        .unwrap();
    let snapshot = rust.topology_snapshot(output, 1).unwrap();
    assert!(rust.uses_rust_geometry());
    assert_eq!(counts(&snapshot), [5, 9, 6]);
    near(rust.volume_mm3().unwrap(), 90.);
    for element in &authored.elements {
        let intent = GeometryIntent::capture(&authored, element.key).unwrap();
        assert!(matches!(
            resolve(&intent, &snapshot, 1),
            Resolution::Resolved { .. }
        ));
    }
    let manufacturing = rust
        .manufacturing_mesh_with_receipt(MeshOptions::default())
        .unwrap();
    assert!(!manufacturing.mesh.indices.is_empty());
    assert_eq!(source.topology_snapshot(feature, 0).unwrap(), authored);
    if OCCT_LINKED {
        let native =
            Solid::extrude_curved_profile_vector_named(&profile, sweep, feature, &origins, policy)
                .unwrap()
                .affine_transformed_named(transform, output, policy)
                .unwrap();
        let expected = native.bounds().unwrap();
        bounds(&rust, expected.min, expected.max);
        near(rust.volume_mm3().unwrap(), native.volume_mm3().unwrap());
        let native_topology = native.topology_snapshot(output, 1).unwrap();
        let native_faces = native.faces().unwrap();
        let rust_faces = rust.faces().unwrap();
        for element in authored
            .elements
            .iter()
            .filter(|e| e.key.kind == TopologyKind::Face)
        {
            let intent = GeometryIntent::capture(&authored, element.key).unwrap();
            let resolve_face = |topology| {
                let Resolution::Resolved { key, .. } = resolve(&intent, topology, 1) else {
                    panic!("Scaled source face lost its identity");
                };
                key.ordinal
            };
            let area = |faces: &[Face], index| {
                faces
                    .iter()
                    .find(|face| face.index == index)
                    .unwrap()
                    .area_mm2
            };
            near(
                area(&rust_faces, resolve_face(&snapshot)),
                area(&native_faces, resolve_face(&native_topology)),
            );
        }
    }
    let tiny = AffineTransform {
        matrix: [[1e-6, 0., 0.], [0., 1e-6, 0.], [0., 0., 1e-6]],
        ..Default::default()
    };
    assert_eq!(
        source
            .affine_transformed_named(tiny, output, policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Input
    );
}

#[test]
fn rust_compound_affine_keeps_nested_member_order_and_all_boundary_intents() {
    use multicad_rust_kernel::Rational;
    let policy = GeometryPolicy::default();
    let source_id = EntityId::new();
    let pattern_id = EntityId::new();
    let group_id = EntityId::new();
    let make_box = |size: [i64; 3], origin| {
        Solid::rust_box(
            source_id.uuid_bits(),
            size.map(|n| Rational::new(n, 1).unwrap()),
            PlaneFrame {
                origin,
                ..Default::default()
            },
            policy,
        )
        .unwrap()
    };
    let a = make_box([2, 3, 4], [0.; 3])
        .with_member_origin(pattern_id, 0)
        .unwrap();
    let b = make_box([3, 2, 5], [10., 4., 1.])
        .with_member_origin(pattern_id, 1)
        .unwrap();
    let c = make_box([2, 3, 4], [0., 10., 0.])
        .with_member_origin(pattern_id, 2)
        .unwrap();
    let inner = Solid::compound_named(&[&a, &b], EntityId::new(), policy)
        .unwrap()
        .with_member_origin(group_id, 0)
        .unwrap();
    let source = Solid::compound_named(&[&inner, &c], EntityId::new(), policy)
        .unwrap()
        .with_member_origin(group_id, 1)
        .unwrap();
    let before = source.topology_snapshot(source_id, 0).unwrap();
    let transform = AffineTransform {
        matrix: [[2.5, -0.5, 0.], [-0.5, 2.5, 0.], [0., 0., 0.5]],
        translation_mm: [-2.5, -4.5, 2.5],
    };
    let output_id = EntityId::new();
    let output = source
        .affine_transformed_named(transform, output_id, policy)
        .unwrap();
    near(output.volume_mm3().unwrap(), 234.);
    assert_eq!(output.solid_count().unwrap(), 3);
    let after = output.topology_snapshot(output_id, 1).unwrap();
    assert_eq!(counts(&after), [18, 36, 24]);
    for element in &before.elements {
        let mut intent = GeometryIntent::capture(&before, element.key).unwrap();
        // Explicit boundary discriminators are also a saved-reference contract.
        intent.boundary = Some(element.boundary.clone());
        let Resolution::Resolved { key, .. } = resolve(&intent, &after, 1) else {
            panic!("lost {:?}: {:?}", element.key, intent)
        };
        let current = after.elements.iter().find(|e| e.key == key).unwrap();
        if let (
            Some(AnalyticGeometry::Point {
                position_mm: original,
            }),
            Some(AnalyticGeometry::Point {
                position_mm: actual,
            }),
        ) = (element.geometry, current.geometry)
        {
            for (i, coordinate) in actual.into_iter().enumerate() {
                near(
                    coordinate,
                    transform.translation_mm[i]
                        + (0..3)
                            .map(|j| transform.matrix[i][j] * original[j])
                            .sum::<f64>(),
                );
            }
        }
    }
    let components = output.solid_components(policy).unwrap();
    for (i, (member, volume)) in components.iter().zip([72., 90., 72.]).enumerate() {
        near(member.volume_mm3().unwrap(), volume);
        assert!(
            member
                .topology_snapshot(output_id, 1)
                .unwrap()
                .elements
                .iter()
                .all(|e| e.origins.contains(&Origin {
                    source: pattern_id,
                    role: format!("member/{i}")
                }))
        );
    }
    assert_eq!(source.topology_snapshot(source_id, 0).unwrap(), before);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("scaled-compound.step");
    output.write_step(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let reimported = Solid::import_rust_step(&text, EntityId::new(), policy).unwrap();
    near(reimported.volume_mm3().unwrap(), 234.);
    assert_eq!(reimported.solid_count().unwrap(), 3);
    let expected = output.bounds().unwrap();
    bounds(&reimported, expected.min, expected.max);
    assert!(
        !output
            .manufacturing_mesh_with_receipt(MeshOptions::default())
            .unwrap()
            .mesh
            .indices
            .is_empty()
    );
    if OCCT_LINKED {
        let native_source = Solid::read_step(&path, policy).unwrap();
        near(native_source.volume_mm3().unwrap(), 234.);
        bounds(&native_source, expected.min, expected.max);
        assert_eq!(native_source.solid_count().unwrap(), 3);
    }
}

#[test]
fn rust_compound_affine_refuses_one_unsupported_leaf_atomically() {
    use multicad_rust_kernel::Rational;
    let policy = GeometryPolicy::default();
    let id = EntityId::new();
    let stock = Solid::rust_box(
        id.uuid_bits(),
        [2, 3, 4].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame::default(),
        policy,
    )
    .unwrap();
    let sphere = Solid::rust_sphere_named(
        2.,
        PlaneFrame {
            origin: [10., 0., 0.],
            ..Default::default()
        },
        EntityId::new(),
        policy,
    )
    .unwrap();
    for members in [[&stock, &sphere], [&sphere, &stock]] {
        let source = Solid::compound_named(&members, id, policy).unwrap();
        let before = source.topology_snapshot(id, 0).unwrap();
        let transform = AffineTransform {
            matrix: [[2., 0., 0.], [0., 3., 0.], [0., 0., 0.5]],
            ..Default::default()
        };
        assert_eq!(
            source
                .affine_transformed_named(transform, EntityId::new(), policy)
                .err()
                .unwrap()
                .kind,
            GeometryErrorKind::Unsupported
        );
        assert_eq!(source.topology_snapshot(id, 0).unwrap(), before);
        assert_eq!(source.solid_count().unwrap(), 2);
    }
}

fn indexed_u_loft(origin: [f64; 3]) -> Solid {
    let points = [
        [0., 0.],
        [6., 0.],
        [6., 5.],
        [4., 5.],
        [4., 2.],
        [2., 2.],
        [2., 5.],
        [0., 5.],
    ];
    let profiles: Vec<_> = [0., 4.]
        .map(|z| CurvedProfile {
            frame: PlaneFrame {
                origin: [origin[0], origin[1], origin[2] + z],
                ..Default::default()
            },
            outer: points
                .into_iter()
                .zip(points.into_iter().cycle().skip(1))
                .map(|(a, b)| CurvedEdge2d::Line { a, b })
                .collect(),
            holes: vec![],
        })
        .into();
    let feature = EntityId::new();
    let origins: Vec<Vec<_>> = (0..2)
        .map(|station| {
            (0..points.len())
                .map(|edge| Origin {
                    source: EntityId::new(),
                    role: format!("section.{station}.edge.{edge}"),
                })
                .collect()
        })
        .collect();
    Solid::rust_loft_ruled_polygons_named(&profiles, feature, &origins, GeometryPolicy::default())
        .unwrap()
}

#[test]
fn indexed_affine_scales_concave_disconnected_and_cut_material_with_every_boundary_intent() {
    use multicad_rust_kernel::Rational;
    let policy = GeometryPolicy::default();
    let number = |n| Rational::new(n, 1).unwrap();
    let stock = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [12, 10, 8].map(number),
        PlaneFrame::default(),
        policy,
    )
    .unwrap();
    let cutter = |origin| {
        Solid::rust_box(
            EntityId::new().uuid_bits(),
            [3, 3, 3].map(number),
            PlaneFrame {
                origin,
                ..Default::default()
            },
            policy,
        )
        .unwrap()
    };
    let pocket = stock
        .rust_subtract_planar_tools(&[&cutter([2., 2., 6.])], EntityId::new(), policy)
        .unwrap()
        .unwrap();
    let cavity = stock
        .rust_subtract_planar_tools(&[&cutter([2., 2., 2.])], EntityId::new(), policy)
        .unwrap()
        .unwrap();
    let loft = indexed_u_loft([0.; 3]);
    let other = indexed_u_loft([20., 0., 0.]);
    let disconnected = Solid::union_planar_pieces(&[&loft, &other], EntityId::new(), policy)
        .unwrap()
        .unwrap();
    let profile = CurvedProfile {
        frame: PlaneFrame::default(),
        outer: [[0., 0.], [4., 0.], [4., 1.], [1., 1.], [1., 3.], [0., 3.]]
            .into_iter()
            .zip(
                [[0., 0.], [4., 0.], [4., 1.], [1., 1.], [1., 3.], [0., 3.]]
                    .into_iter()
                    .cycle()
                    .skip(1),
            )
            .map(|(a, b)| CurvedEdge2d::Line { a, b })
            .collect(),
        holes: vec![],
    };
    let origin = || Origin {
        source: EntityId::new(),
        role: "curve".into(),
    };
    let miter = Solid::rust_sweep_polyline_named(
        &profile,
        &[[0.; 3], [0., 0., 10.], [10., 0., 10.]],
        EntityId::new(),
        &(0..6).map(|_| origin()).collect::<Vec<_>>(),
        &[origin(), origin()],
        policy,
    )
    .unwrap();
    for (source, void) in [
        (&loft, false),
        (&disconnected, false),
        (&pocket, false),
        (&cavity, true),
        (&miter, false),
    ] {
        for matrix in [
            [[2., 0., 0.], [0., 3., 0.], [0., 0., 0.5]],
            [[2., 0.5, 0.], [0., 3., 0.25], [0., 0., 0.5]],
        ] {
            let transform = AffineTransform {
                matrix,
                translation_mm: [-3., 5., 11.],
            };
            let feature = EntityId::new();
            let before = source.topology_snapshot(feature, 0).unwrap();
            let point = |p: [f64; 3]| {
                std::array::from_fn(|i| {
                    matrix[i]
                        .into_iter()
                        .zip(p)
                        .map(|(a, b)| a * b)
                        .sum::<f64>()
                        + transform.translation_mm[i]
                })
            };
            let result = source
                .affine_transformed_named(transform, EntityId::new(), policy)
                .unwrap();
            let current = result.topology_snapshot(feature, 1).unwrap();
            assert_eq!(counts(&current), counts(&before));
            near(
                result.volume_mm3().unwrap(),
                source.volume_mm3().unwrap() * 3.,
            );
            assert_eq!(result.solid_count().unwrap(), source.solid_count().unwrap());
            for element in &before.elements {
                let intent = GeometryIntent::capture(&before, element.key).unwrap();
                let Resolution::Resolved {
                    key,
                    geometry: Some(actual),
                    ..
                } = resolve(&intent, &current, 1)
                else {
                    panic!("lost indexed boundary {intent:?}")
                };
                assert_eq!(key, element.key);
                let close = |a: [f64; 3], b: [f64; 3]| {
                    for (a, b) in a.into_iter().zip(b) {
                        near(a, b)
                    }
                };
                match (element.geometry.unwrap(), actual) {
                    (
                        AnalyticGeometry::Point { position_mm: a },
                        AnalyticGeometry::Point { position_mm: b },
                    ) => close(point(a), b),
                    (
                        AnalyticGeometry::Line { a_mm: a, b_mm: b },
                        AnalyticGeometry::Line { a_mm: c, b_mm: d },
                    ) => {
                        close(point(a), c);
                        close(point(b), d);
                    }
                    (AnalyticGeometry::Plane { .. }, AnalyticGeometry::Plane { frame }) => {
                        let old = &source.faces().unwrap()[element.key.ordinal as usize];
                        let new = &result.faces().unwrap()[key.ordinal as usize];
                        close(point(old.center_mm), new.center_mm);
                        assert!(
                            frame
                                .normal()
                                .into_iter()
                                .zip(new.plane.unwrap().1)
                                .map(|(a, b)| a * b)
                                .sum::<f64>()
                                > 1. - 1e-10
                        );
                        let e = current.elements.iter().find(|e| e.key == key).unwrap();
                        for anchor in &element.frames {
                            let mapped =
                                e.frames.iter().find(|a| a.origin == anchor.origin).unwrap();
                            close(point(anchor.frame.origin_mm), mapped.frame.origin_mm);
                        }
                    }
                    pair => panic!("unexpected indexed map {pair:?}"),
                }
            }
            result
                .manufacturing_mesh_with_receipt(MeshOptions::default())
                .unwrap();
            let second = result
                .affine_transformed_named(transform, EntityId::new(), policy)
                .unwrap();
            near(
                second.volume_mm3().unwrap(),
                source.volume_mm3().unwrap() * 9.,
            );
            let folder = tempfile::tempdir().unwrap();
            let source_path = folder.path().join("source.step");
            source.write_step(&source_path).unwrap();
            if OCCT_LINKED {
                let native = Solid::read_step(&source_path, policy)
                    .unwrap()
                    .affine_transformed_named(transform, EntityId::new(), policy)
                    .unwrap();
                let expected = native.bounds().unwrap();
                bounds(&result, expected.min, expected.max);
                near(result.volume_mm3().unwrap(), native.volume_mm3().unwrap());
                near(
                    result.faces().unwrap().iter().map(|f| f.area_mm2).sum(),
                    native.faces().unwrap().iter().map(|f| f.area_mm2).sum(),
                );
            }
            let path = folder.path().join("scaled.step");
            if void && matrix[0][1] != 0. {
                assert_eq!(
                    result.write_step(&path).unwrap_err().kind,
                    GeometryErrorKind::Unsupported
                );
            } else {
                result.write_step(&path).unwrap();
                let read = Solid::import_rust_step(
                    &std::fs::read_to_string(&path).unwrap(),
                    EntityId::new(),
                    policy,
                )
                .unwrap();
                near(read.volume_mm3().unwrap(), result.volume_mm3().unwrap());
            }
            assert_eq!(source.topology_snapshot(feature, 0).unwrap(), before);
        }
    }
    // A new Boolean must consume the transformed material, not stale source cells.
    let transform = AffineTransform {
        matrix: [[2., 0.5, 0.], [0., 3., 0.25], [0., 0., 0.5]],
        translation_mm: [-3., 5., 11.],
    };
    for source in [&loft, &disconnected, &pocket, &cavity, &miter] {
        let scaled = source
            .affine_transformed_named(transform, EntityId::new(), policy)
            .unwrap();
        let tool = cutter([0.5, 0.5, 0.5]);
        let mapped_tool = tool
            .affine_transformed_named(transform, EntityId::new(), policy)
            .unwrap();
        let original = source
            .rust_subtract_planar_tools(&[&tool], EntityId::new(), policy)
            .unwrap()
            .unwrap();
        let actual = scaled
            .rust_subtract_planar_tools(&[&mapped_tool], EntityId::new(), policy)
            .unwrap()
            .unwrap();
        near(
            actual.volume_mm3().unwrap(),
            original.volume_mm3().unwrap() * 3.,
        );
    }
}

#[test]
fn indexed_affine_composes_a_source_pose_and_refuses_uncertified_multiloop_stock_atomically() {
    use multicad_rust_kernel::Rational;
    let policy = GeometryPolicy::default();
    let posed = indexed_u_loft([0.; 3])
        .transformed(RigidTransform {
            rotation: [[0.8, 0., 0.6], [0., 1., 0.], [-0.6, 0., 0.8]],
            translation_mm: [7., -11., 3.],
        })
        .unwrap();
    let feature = EntityId::new();
    let before = posed.topology_snapshot(feature, 0).unwrap();
    let transform = AffineTransform {
        matrix: [[2., 0.5, 0.], [0., 3., 0.25], [0., 0., 0.5]],
        translation_mm: [-3., 5., 11.],
    };
    let mapped = posed
        .affine_transformed_named(transform, EntityId::new(), policy)
        .unwrap();
    near(
        mapped.volume_mm3().unwrap(),
        posed.volume_mm3().unwrap() * 3.,
    );
    let after = mapped.topology_snapshot(feature, 1).unwrap();
    for element in &before.elements {
        let intent = GeometryIntent::capture(&before, element.key).unwrap();
        assert!(matches!(
            resolve(&intent, &after, 1),
            Resolution::Resolved { .. }
        ));
    }
    mapped
        .manufacturing_mesh_with_receipt(MeshOptions::default())
        .unwrap();
    assert_eq!(posed.topology_snapshot(feature, 0).unwrap(), before);
    let stock = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [12, 10, 8].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame::default(),
        policy,
    )
    .unwrap();
    let tool = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [3, 3, 3].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame {
            origin: [2., 2., 6.],
            ..Default::default()
        },
        policy,
    )
    .unwrap();
    let pocket = stock
        .boolean(&tool, BooleanOperation::Subtract, policy)
        .unwrap()
        .unwrap();
    let before = pocket.topology_snapshot(feature, 0).unwrap();
    assert_eq!(
        pocket
            .affine_transformed_named(transform, EntityId::new(), policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Unsupported
    );
    assert_eq!(pocket.topology_snapshot(feature, 0).unwrap(), before);
}

#[test]
fn indexed_affine_does_not_apply_public_bounds_to_a_composed_pose_matrix() {
    let c = 0.5_f64.sqrt();
    let source = indexed_u_loft([0.; 3])
        .transformed(RigidTransform {
            rotation: [[c, -c, 0.], [c, c, 0.], [0., 0., 1.]],
            ..Default::default()
        })
        .unwrap();
    let transform = AffineTransform {
        matrix: [[1e6, 1e6, 0.], [-1e6, 1e6, 0.], [0., 0., 1.]],
        ..Default::default()
    };
    transform.validate().unwrap();
    let mapped = source
        .affine_transformed_named(transform, EntityId::new(), GeometryPolicy::default())
        .unwrap();
    assert!(
        (mapped.volume_mm3().unwrap() / (source.volume_mm3().unwrap() * 2e12) - 1.).abs() < 1e-8
    );
}

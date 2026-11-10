use multicad_core::{
    document::EntityId,
    manufacturing::PlanarEdge,
    references::{AnalyticGeometry, Origin, TopologyKind},
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryErrorKind, GeometryPolicy, MeshOptions,
    PlaneFrame, Solid,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder, step_read},
};

#[test]
fn reversed_annular_extrusion_keeps_both_analytic_sketch_rims_and_the_authored_cap_basis() {
    let feature = EntityId::new();
    let policy = GeometryPolicy::default();
    let frame = PlaneFrame {
        origin: [3., 10., 6.],
        u: [1., 0., 0.],
        v: [0., 0., 1.],
    };
    let origins = ["bore", "outer"].map(|role| Origin {
        source: EntityId::new(),
        role: role.into(),
    });
    let ring = Solid::rust_annulus_from_sketch(
        [2., 5.],
        -3.,
        feature,
        frame,
        [1., 2.],
        origins.clone(),
        policy,
    )
    .unwrap();
    let bounds = ring.bounds().unwrap();
    assert_eq!(bounds.min, [-1., 10., 3.]);
    assert_eq!(bounds.max, [9., 13., 13.]);
    assert_eq!(ring.solid_count().unwrap(), 1);
    assert!((ring.volume_mm3().unwrap() - 63. * std::f64::consts::PI).abs() < 1e-10);
    let topology = ring.topology_snapshot(feature, 4).unwrap();
    let face = |ordinal| {
        topology
            .elements
            .iter()
            .find(|element| {
                element.key.kind == TopologyKind::Face && element.key.ordinal == ordinal
            })
            .unwrap()
    };
    assert_eq!(face(0).frames[0].frame.origin_mm, frame.origin);
    assert_eq!(face(0).frames[0].frame.v, frame.v);
    assert!(face(2).origins.contains(&origins[1]));
    assert!(face(3).origins.contains(&origins[0]));
    let contours = ring.planar_face_contours(0, frame).unwrap();
    assert_eq!(contours.edge_count(), 2);
    assert!(contours.loops[0].outer);
    assert!(!contours.loops[1].outer);
    assert!((contours.area_mm2() - 21. * std::f64::consts::PI).abs() < 1e-10);
    for wire in contours.loops {
        assert!(matches!(wire.edges[0], PlanarEdge::Arc { center, .. } if center == [1., 2.]));
    }
    assert_eq!(ring.display_edges(0.05).unwrap().len(), 4);
    let mesh = ring
        .manufacturing_mesh_with_receipt(MeshOptions::default())
        .unwrap();
    assert_eq!(mesh.edge_pairs as usize * 2, mesh.mesh.indices.len());
    assert_eq!(
        mesh.mesh
            .face_indices
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        (0..4).collect()
    );
}

#[test]
fn subtracting_a_coaxial_cylinder_preserves_the_authored_cap_basis_and_both_sources() {
    let policy = GeometryPolicy::default();
    let target = EntityId::new();
    let tool = EntityId::new();
    let curve = Origin {
        source: EntityId::new(),
        role: "circle".into(),
    };
    let exact = |n| Rational::new(n, 1).unwrap();
    let axial = |feature: EntityId, [x, y, z]: [i64; 3], radius, height| {
        AxialCylinder::new(
            feature.uuid_bits(),
            Circle::new([exact(x), exact(y)], exact(radius)).unwrap(),
            exact(z),
            exact(height),
        )
        .unwrap()
    };
    let frame = PlaneFrame {
        origin: [2., 5., 7.],
        v: [0., -1., 0.],
        ..Default::default()
    };
    let outer = Solid::from_axial_cylinder_sweep(
        axial(target, [0, 0, 0], 5, 3),
        frame,
        [1., 2.],
        true,
        curve.clone(),
        policy,
    )
    .unwrap();
    let vertical = |[x, y, z]: [i64; 3], radius, height| {
        Solid::from_axial_cylinder(
            axial(tool, [x, y, z], radius, height),
            PlaneFrame {
                origin: [x, y, z].map(|v| v as f64),
                ..Default::default()
            },
            policy,
        )
        .unwrap()
    };
    let subtract = |inner: &Solid| outer.boolean(inner, BooleanOperation::Subtract, policy);
    let ring = subtract(&vertical([2, 5, 4], 2, 3)).unwrap().unwrap();
    assert!((ring.volume_mm3().unwrap() - 63. * std::f64::consts::PI).abs() < 1e-10);
    assert_eq!(ring.bounds().unwrap(), outer.bounds().unwrap());
    let topology = ring.topology_snapshot(target, 1).unwrap();
    let face = |ordinal| {
        topology
            .elements
            .iter()
            .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == ordinal)
            .unwrap()
    };
    assert_eq!(face(0).frames[0].frame.origin_mm, [1., 3., 7.]);
    assert_eq!(face(0).frames[0].frame.v, [0., 1., 0.]);
    assert!(face(2).origins.contains(&curve));
    assert!(face(3).origins.iter().any(|origin| origin.source == tool));
    assert!(subtract(&vertical([2, 5, 4], 6, 3)).unwrap().is_none());
    for bad in [vertical([3, 5, 4], 2, 3), vertical([2, 5, 4], 2, 2)] {
        assert_eq!(
            subtract(&bad).err().unwrap().kind,
            GeometryErrorKind::Unsupported
        );
    }
    let plain = vertical([2, 5, 4], 5, 3);
    let concentric = Solid::from_axial_cylinder(
        axial(target, [2, 5, 4], 2, 3),
        PlaneFrame {
            origin: [2., 5., 4.],
            ..Default::default()
        },
        policy,
    )
    .unwrap();
    let axial_ring = plain
        .boolean(&concentric, BooleanOperation::Subtract, policy)
        .unwrap()
        .unwrap();
    assert_eq!(ring.volume_mm3().unwrap(), axial_ring.volume_mm3().unwrap());
}

#[test]
fn annulus_step_exchanges_real_circular_stock_in_both_directions() {
    let feature = EntityId::new();
    let policy = GeometryPolicy::default();
    let (sin, cos) = 0.37_f64.sin_cos();
    let frame = PlaneFrame {
        origin: [5., -9., 21.],
        u: [cos, sin, 0.],
        v: [0., 0., 1.],
    };
    let [inner, outer, height] = [2.75, 6.125, 1.25];
    let ring = Solid::rust_annulus(inner, outer, height, feature, frame, policy).unwrap();
    let native_source = Solid::extrude_curved_profile_vector_named(
        &CurvedProfile {
            frame,
            outer: vec![CurvedEdge2d::Circle {
                center: [0.; 2],
                radius_mm: outer,
            }],
            holes: vec![vec![CurvedEdge2d::Circle {
                center: [0.; 2],
                radius_mm: inner,
            }]],
        },
        frame.normal().map(|x| x * height),
        feature,
        &["outer", "inner"].map(|role| Origin {
            source: feature,
            role: role.into(),
        }),
        policy,
    )
    .unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let rust_path = scratch.path().join("rust.step");
    let native_path = scratch.path().join("native.step");
    assert!(ring.supports_step_export());
    ring.write_step(&rust_path).unwrap();
    native_source.write_step(&native_path).unwrap();
    let rust_source = std::fs::read_to_string(&rust_path).unwrap();
    let native_text = std::fs::read_to_string(&native_path).unwrap();
    let read_native = Solid::read_step(&rust_path, policy).unwrap();
    let read_rust = Solid::import_rust_step(&rust_source, feature, policy).unwrap();
    let imported_native = Solid::import_rust_step(&native_text, feature, policy).unwrap();
    let expected = ring.volume_mm3().unwrap();
    let near = |a: f64, b: f64| assert!((a - b).abs() <= 3e-6, "{a} != {b}");
    let bounds = ring.bounds().unwrap();
    for body in [&read_native, &read_rust, &imported_native] {
        near(body.volume_mm3().unwrap(), expected);
        let current = body.bounds().unwrap();
        for (a, b) in current
            .min
            .into_iter()
            .chain(current.max)
            .zip(bounds.min.into_iter().chain(bounds.max))
        {
            near(a, b);
        }
        assert_eq!(body.solid_count().unwrap(), 1);
        let cap = body
            .faces()
            .unwrap()
            .into_iter()
            .find(|face| {
                face.plane.is_some_and(|(origin, normal)| {
                    let distance = origin
                        .into_iter()
                        .zip(frame.origin)
                        .zip(frame.normal())
                        .map(|((a, b), axis)| (a - b) * axis)
                        .sum::<f64>();
                    distance.abs() < 1e-6
                        && normal
                            .into_iter()
                            .zip(frame.normal())
                            .map(|(a, b)| a * b)
                            .sum::<f64>()
                            < -0.99
                })
            })
            .unwrap();
        let contours = body.planar_face_contours(cap.index, frame).unwrap();
        assert_eq!(contours.edge_count(), 2);
        near(contours.area_mm2(), expected / height);
    }
    for body in [&read_rust, &imported_native] {
        let topology = body.topology_snapshot(feature, 1).unwrap();
        assert_eq!(
            topology
                .elements
                .iter()
                .filter(|element| matches!(
                    element.geometry,
                    Some(AnalyticGeometry::CircularArc { .. })
                ))
                .count(),
            4
        );
    }
    let readback =
        step_read::read_analytic_solid(&rust_source, policy.linear_tolerance_mm).unwrap();
    assert_eq!(readback.face_count, 4);
    assert_eq!(readback.circles.len(), 4);
    near(readback.volume_mm3, expected);
}

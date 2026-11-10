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

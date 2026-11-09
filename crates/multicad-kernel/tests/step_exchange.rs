use multicad_kernel::{GeometryPolicy, RigidTransform, Solid};
use std::sync::{Arc, Barrier};

#[test]
fn rust_step_reopens_a_placed_reversed_concave_prism_with_a_straight_hole() {
    use multicad_core::{
        document::EntityId,
        references::{AnalyticGeometry, Origin, TopologyKind},
    };
    use multicad_kernel::{
        PlaneFrame,
        rust_kernel::{Rational, polygon::PolygonPrism},
    };

    let exact = |n| Rational::new(n, 1).unwrap();
    let ring = |points: &[[i64; 2]]| points.iter().map(|p| p.map(exact)).collect();
    let prism = PolygonPrism::new(
        vec![
            ring(&[[0, 0], [12, 0], [12, 5], [7, 5], [7, 9], [0, 9]]),
            ring(&[[2, 2], [2, 4], [4, 4], [4, 2]]),
        ],
        exact(3),
    )
    .unwrap();
    let (sin, cos) = 0.37_f64.sin_cos();
    let frame = PlaneFrame {
        origin: [3., 5., 11.],
        u: [cos, sin, 0.],
        v: [0., 0., 1.],
    };
    let owner = EntityId::new();
    let origins: Vec<_> = (0..10)
        .map(|_| Origin {
            source: EntityId::new(),
            role: "profile.edge".into(),
        })
        .collect();
    let policy = GeometryPolicy::default();
    let source =
        Solid::from_polygon_prism_sweep(prism, frame, true, owner, &origins, &[false; 10], policy)
            .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("prism.step");
    let second = dir.path().join("same.step");
    assert!(source.supports_step_export());
    source.write_step(&first).unwrap();
    source.write_step(&second).unwrap();
    let bytes = std::fs::read_to_string(&first).unwrap();
    assert_eq!(bytes, std::fs::read_to_string(second).unwrap());
    assert!(bytes.contains("MANIFOLD_SOLID_BREP"));
    assert!(bytes.contains("SI_UNIT(.MILLI.,.METRE.)"));
    assert_eq!(bytes.matches("ADVANCED_FACE(").count(), 12);
    let imported = Solid::read_step(&first, policy).unwrap();
    assert_eq!(imported.solid_count().unwrap(), 1);
    let near = |a: f64, b: f64| assert!((a - b).abs() < 2e-6, "{a} != {b}");
    near(imported.volume_mm3().unwrap(), 252.);
    let expected = source.bounds().unwrap();
    let bounds = imported.bounds().unwrap();
    for (a, b) in bounds
        .min
        .into_iter()
        .chain(bounds.max)
        .zip(expected.min.into_iter().chain(expected.max))
    {
        near(a, b);
    }
    let end_normal = frame.normal().map(|x| -x);
    let cap = imported
        .faces()
        .unwrap()
        .into_iter()
        .find(|face| {
            face.plane.is_some_and(|(_, normal)| {
                normal
                    .into_iter()
                    .zip(end_normal)
                    .map(|(a, b)| a * b)
                    .sum::<f64>()
                    > 1. - 1e-9
            })
        })
        .unwrap();
    let end = PlaneFrame {
        origin: std::array::from_fn(|i| frame.origin[i] + 3. * end_normal[i]),
        ..frame
    };
    let contours = imported.planar_face_contours(cap.index, end).unwrap();
    assert_eq!(contours.loops.len(), 2);
    assert_eq!(contours.edge_count(), 10);
    near(contours.area_mm2(), 84.);
    let topology = imported.topology_snapshot(owner, 0).unwrap();
    let edges: Vec<_> = topology
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Edge)
        .collect();
    assert_eq!(edges.len(), 30);
    assert!(
        edges
            .iter()
            .all(|edge| matches!(edge.geometry, Some(AnalyticGeometry::Line { .. })))
    );
}

#[test]
fn rust_step_writes_placed_boxes_and_preserves_destinations_for_unsupported_solids() {
    use multicad_core::document::EntityId;
    use multicad_kernel::{
        GeometryErrorKind, PlaneFrame,
        rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
    };
    let exact = |n| Rational::new(n, 1).unwrap();
    let policy = GeometryPolicy::default();
    let box_solid = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [exact(3), exact(4), exact(5)],
        PlaneFrame {
            origin: [-6., 10., 20.],
            u: [0., 1., 0.],
            v: [0., 0., 1.],
        },
        policy,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("box.step");
    assert!(box_solid.supports_step_export());
    box_solid.write_step(&path).unwrap();
    let box_read = Solid::read_step(&path, policy).unwrap();
    assert!((box_read.volume_mm3().unwrap() - 60.).abs() < 1e-8);
    assert_eq!(box_read.bounds().unwrap().min, [-6., 10., 20.]);
    assert_eq!(box_read.bounds().unwrap().max, [-1., 13., 24.]);
    let bytes = std::fs::read(&path).unwrap();
    let cylinder = AxialCylinder::new(
        EntityId::new().uuid_bits(),
        Circle::new([exact(0), exact(0)], exact(2)).unwrap(),
        exact(0),
        exact(3),
    )
    .unwrap();
    let curved = Solid::from_axial_cylinder(cylinder, PlaneFrame::default(), policy).unwrap();
    assert!(!curved.supports_step_export());
    assert_eq!(
        curved.write_step(&path).unwrap_err().kind,
        GeometryErrorKind::Unsupported
    );
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn concurrent_step_exchange_retains_independent_owner_geometry_and_unlocks_after_failures() {
    let directory = tempfile::tempdir().unwrap();
    let start = Arc::new(Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|owner| {
            let directory = directory.path().to_path_buf();
            let start = start.clone();
            std::thread::spawn(move || {
                // Only paths, counters and the start barrier cross threads.
                // Every Solid is constructed, used and dropped by this owner.
                start.wait();
                for iteration in 0..12 {
                    let dimensions = [20. + owner as f64, 30. + iteration as f64, 8.];
                    let translation = [owner as f64 * 100., iteration as f64 * 5., 0.];
                    let source = Solid::box_solid(dimensions, GeometryPolicy::default())
                        .unwrap()
                        .transformed(RigidTransform {
                            translation_mm: translation,
                            ..Default::default()
                        })
                        .unwrap();
                    if iteration == 0 {
                        let bad = directory.join(format!("bad-{owner}.step"));
                        std::fs::write(&bad, b"not a STEP file").unwrap();
                        assert!(Solid::read_step(&bad, GeometryPolicy::default()).is_err());
                        assert!(
                            source
                                .write_step(&directory.join(format!("missing/failed-{owner}.step")))
                                .is_err()
                        );
                    }
                    let path = directory.join(format!("solid-{owner}-{iteration}.step"));
                    source.write_step(&path).unwrap();
                    assert!(
                        std::fs::read_to_string(&path)
                            .unwrap()
                            .contains("SI_UNIT(.MILLI.,.METRE.)")
                    );
                    let reopened = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
                    let expected = dimensions.into_iter().product::<f64>();
                    assert!((reopened.volume_mm3().unwrap() - expected).abs() < 1e-6);
                    let bounds = reopened.bounds().unwrap();
                    for axis in 0..3 {
                        assert!((bounds.min[axis] - translation[axis]).abs() < 1e-6);
                        assert!(
                            (bounds.max[axis] - translation[axis] - dimensions[axis]).abs() < 1e-6
                        );
                    }
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
}

#[test]
fn step_solids_split_a_file_into_located_exact_breps_that_round_trip() {
    let directory = tempfile::tempdir().unwrap();
    let policy = GeometryPolicy::default();
    let a = Solid::box_solid([10., 20., 30.], policy).unwrap();
    let b = Solid::box_solid([5., 5., 5.], policy)
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: [100., 0., 0.],
            ..Default::default()
        })
        .unwrap();
    let both = a
        .boolean(&b, multicad_kernel::BooleanOperation::Union, policy)
        .unwrap()
        .unwrap();
    assert_eq!(both.solid_count().unwrap(), 2);
    let path = directory.path().join("two.step");
    both.write_step(&path).unwrap();
    let solids = Solid::step_solids(&path).unwrap();
    assert_eq!(solids.len(), 2);
    let mut volumes: Vec<f64> = solids
        .iter()
        .map(|(_, brep)| {
            Solid::from_brep(brep, policy)
                .unwrap()
                .volume_mm3()
                .unwrap()
        })
        .collect();
    volumes.sort_by(f64::total_cmp);
    assert!((volumes[0] - 125.).abs() < 1e-6 && (volumes[1] - 6000.).abs() < 1e-6);
    // Located solids keep their position, and BRep text round-trips exactly.
    let far = solids
        .iter()
        .map(|(_, brep)| Solid::from_brep(brep, policy).unwrap())
        .find(|s| s.bounds().unwrap().min[0] > 50.)
        .unwrap();
    assert!((far.bounds().unwrap().min[0] - 100.).abs() < 1e-6);
    let again = Solid::from_brep(&far.to_brep().unwrap(), policy).unwrap();
    assert!((again.volume_mm3().unwrap() - 125.).abs() < 1e-9);
    assert!(Solid::from_brep("not brep", policy).is_err());
}

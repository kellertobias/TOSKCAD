use multicad_core::{
    document::EntityId,
    drawings::VectorCurve,
    references::{AnalyticGeometry, Origin, TopologyKind},
};
use multicad_kernel::{
    CurvedEdge2d, CurvedProfile, DrawingProjection, DrawingSolid, GeometryPolicy, MeshOptions,
    PlaneFrame, RevolutionAxis, RigidTransform, Solid, project_drawing,
};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

fn profile(
    frame: PlaneFrame,
    outer: &[[f64; 2]],
    holes: &[&[[f64; 2]]],
) -> (CurvedProfile, Vec<Origin>) {
    let lines = |points: &[[f64; 2]]| {
        points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .map(|(&a, &b)| CurvedEdge2d::Line { a, b })
            .collect()
    };
    let profile = CurvedProfile {
        frame,
        outer: lines(outer),
        holes: holes.iter().map(|points| lines(points)).collect(),
    };
    let origins = (0..outer.len() + holes.iter().map(|points| points.len()).sum::<usize>())
        .map(|i| Origin {
            source: EntityId::new(),
            role: format!("profile/{i}"),
        })
        .collect();
    (profile, origins)
}

#[test]
fn full_cone_has_exact_occupied_volume_axis_poles_and_planar_rim() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let frame = PlaneFrame {
        origin: [4., 6., 8.],
        v: [0., 0., 1.],
        ..PlaneFrame::default()
    };
    let axis = RevolutionAxis {
        origin_mm: frame.origin,
        direction: [0., 0., 1.],
    };
    let (profile, origins) = profile(frame, &[[0., 0.], [3., 0.], [0., 7.]], &[]);
    let make = |angle| {
        Solid::rust_revolve_curved_profile_named(&profile, axis, angle, feature, &origins, policy)
            .unwrap()
    };
    for sweep in [-TAU, TAU] {
        let cone = make(sweep);
        assert!(!cone.supports_step_export());
        let bounds = cone.bounds().unwrap();
        assert_eq!(bounds.min, [1., 3., 8.]);
        assert_eq!(bounds.max, [7., 9., 15.]);
        assert!((cone.volume_mm3().unwrap() - 21. * PI).abs() < 1e-9);
        assert!(cone.rust_revolution_contains([5., 6., 12.]).unwrap());
        assert!(!cone.rust_revolution_contains([6., 6., 12.]).unwrap());
        let faces = cone.faces().unwrap();
        assert_eq!(faces.len(), 2);
        assert_eq!(faces[0].plane.unwrap().1, [0., 0., -1.]);
        let bottom = PlaneFrame {
            origin: frame.origin,
            ..PlaneFrame::default()
        };
        let contours = cone.planar_face_contours(0, bottom).unwrap();
        assert_eq!(contours.edge_count(), 1);
        assert!((contours.area_mm2() - 9. * PI).abs() < 1e-9);
        let snapshot = cone.topology_snapshot(feature, 1).unwrap();
        assert_eq!(
            snapshot
                .elements
                .iter()
                .filter(|e| e.key.kind == TopologyKind::Vertex)
                .count(),
            2
        );
        let mesh = cone
            .manufacturing_mesh_with_receipt(MeshOptions::default())
            .unwrap();
        assert_eq!(mesh.edge_pairs as usize * 2, mesh.mesh.indices.len());
    }
}

#[test]
fn full_circular_revolutions_reuse_exact_step_and_drawing_with_authored_walls() {
    let frame = PlaneFrame {
        origin: [3., 4., 5.],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let axis = RevolutionAxis {
        origin_mm: frame.origin,
        direction: [0., 0., 1.],
    };
    let policy = GeometryPolicy::default();
    let scratch = tempfile::tempdir().unwrap();
    for inner in [0_f64, 2.] {
        let feature = EntityId::new();
        let (profile, origins) =
            profile(frame, &[[inner, 1.], [5., 1.], [5., 4.], [inner, 4.]], &[]);
        let build = |angle| {
            Solid::rust_revolve_curved_profile_named(
                &profile, axis, angle, feature, &origins, policy,
            )
            .unwrap()
        };
        let round = build(-TAU);
        assert!(round.supports_step_export());
        assert!(!build(-FRAC_PI_2).supports_step_export());
        let path = scratch.path().join(format!("ring-{inner}.step"));
        round.write_step(&path).unwrap();
        let occt = Solid::read_step(&path, policy).unwrap();
        let rust = Solid::import_rust_step(
            &std::fs::read_to_string(path).unwrap(),
            EntityId::new(),
            policy,
        )
        .unwrap();
        let volume = PI * (25. - inner * inner) * 3.;
        for result in [&occt, &rust] {
            assert!((result.volume_mm3().unwrap() - volume).abs() < 1e-7);
            assert_eq!(result.solid_count().unwrap(), 1);
        }
        let view = project_drawing(
            &[DrawingSolid {
                solid: &round,
                hatch: None,
            }],
            DrawingProjection::default(),
        )
        .unwrap();
        let mut radii = view
            .into_iter()
            .map(|edge| match edge.curve {
                VectorCurve::Ellipse {
                    center,
                    major_mm,
                    minor_mm,
                    ..
                } => {
                    assert_eq!(center, [3., 4.]);
                    assert_eq!(major_mm, minor_mm);
                    major_mm
                }
                _ => panic!("Full circular revolution should publish an exact circular rim"),
            })
            .collect::<Vec<_>>();
        radii.sort_unstable_by(f64::total_cmp);
        assert_eq!(
            radii,
            if inner == 0. {
                vec![5.]
            } else {
                vec![inner, 5.]
            }
        );
        let topology = round.topology_snapshot(feature, 0).unwrap();
        assert!(topology.elements.iter().any(|element| {
            element.key.kind == TopologyKind::Face && element.origins.contains(&origins[1])
        }));
    }
}

#[test]
fn signed_partial_revolution_has_two_ordered_cap_wires_and_a_real_cavity() {
    let policy = GeometryPolicy::default();
    let feature = EntityId::new();
    let frame = PlaneFrame {
        v: [0., 0., 1.],
        ..PlaneFrame::default()
    };
    let axis = RevolutionAxis {
        origin_mm: [0.; 3],
        direction: [0., 0., 1.],
    };
    // Opposite windings are normalized without changing the source edge names.
    let (profile, origins) = profile(
        frame,
        &[[1., -1.], [1., 9.], [8., 9.], [8., -1.]],
        &[&[[3., 2.], [5., 2.], [5., 5.], [3., 5.]]],
    );
    let sweep = -FRAC_PI_2;
    let rust =
        Solid::rust_revolve_curved_profile_named(&profile, axis, sweep, feature, &origins, policy)
            .unwrap();
    let native =
        Solid::revolve_curved_profile_named(&profile, axis, sweep, feature, &origins, policy)
            .unwrap();
    let expected = FRAC_PI_2 * 291.;
    assert!((rust.volume_mm3().unwrap() - expected).abs() < 1e-8);
    assert!((native.volume_mm3().unwrap() - expected).abs() < 1e-7);
    let angle = PI / 4.;
    let (sin, cos) = angle.sin_cos();
    assert!(
        rust.rust_revolution_contains([2. * cos, -2. * sin, 3.])
            .unwrap()
    );
    assert!(
        !rust
            .rust_revolution_contains([4. * cos, -4. * sin, 3.])
            .unwrap()
    );
    assert!(
        !rust
            .rust_revolution_contains([2. * cos, 2. * sin, 3.])
            .unwrap()
    );
    let wires = rust.planar_face_contours(0, frame).unwrap();
    assert_eq!(wires.loops.len(), 2);
    assert_eq!(wires.edge_count(), 8);
    assert!((wires.area_mm2() - 64.).abs() < 1e-8);
    let snapshot = rust.topology_snapshot(feature, 2).unwrap();
    let faces = snapshot
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
        .collect::<Vec<_>>();
    assert_eq!(faces.len(), 10);
    assert_eq!(faces[0].frames[0].frame.u, frame.u);
    for origin in &origins {
        assert!(faces.iter().any(|e| e.origins.contains(origin)));
    }
    assert!(snapshot.elements.iter().any(|element| matches!(
        element.geometry,
        Some(AnalyticGeometry::CircularArc { radius_mm: 5., sweep_rad, .. }) if sweep_rad == sweep
    )));
    let shifted = rust
        .transformed(RigidTransform {
            translation_mm: [12., 2., -1.],
            ..Default::default()
        })
        .unwrap();
    assert!(
        shifted
            .rust_revolution_contains([12. + 2. * cos, 2. - 2. * sin, 2.])
            .unwrap()
    );
    let receipt = shifted
        .manufacturing_mesh_with_receipt(MeshOptions::default())
        .unwrap();
    assert_eq!(receipt.edge_pairs as usize * 2, receipt.mesh.indices.len());
}

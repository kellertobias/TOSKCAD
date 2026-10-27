use multicad_kernel::*;
fn near(a: f64, b: f64, tolerance: f64) {
    assert!(
        (a - b).abs() <= tolerance,
        "{a} differs from {b} by more than {tolerance}"
    );
}
fn policy() -> GeometryPolicy {
    GeometryPolicy::default()
}
fn moved(s: &Solid, p: [f64; 3]) -> Solid {
    s.transformed(RigidTransform {
        translation_mm: p,
        ..Default::default()
    })
    .unwrap()
}
fn polygon(points: &[[f64; 2]]) -> Vec<ProfileEdge> {
    (0..points.len())
        .map(|i| ProfileEdge::Line {
            a: points[i],
            b: points[(i + 1) % points.len()],
        })
        .collect()
}
#[test]
fn general_profiles_holes_and_reversed_winding_have_expected_volumes() {
    let mut profile = Profile {
        frame: PlaneFrame::default(),
        outer: polygon(&[
            [0., 0.],
            [40., 0.],
            [40., 10.],
            [20., 10.],
            [20., 30.],
            [0., 30.],
        ]),
        holes: vec![],
    };
    let s = Solid::extrude_profile(&profile, 5., policy()).unwrap();
    near(s.volume_mm3().unwrap(), 4000., 1e-7);
    profile.outer = polygon(&[
        [0., 30.],
        [20., 30.],
        [20., 10.],
        [40., 10.],
        [40., 0.],
        [0., 0.],
    ]);
    near(
        Solid::extrude_profile(&profile, 5., policy())
            .unwrap()
            .volume_mm3()
            .unwrap(),
        4000.,
        1e-7,
    );
    let mut panel = Profile::rectangle(40., 30.);
    panel.holes.push(vec![ProfileEdge::Circle {
        center: [20., 15.],
        radius_mm: 5.,
    }]);
    let panel = Solid::extrude_profile(&panel, 5., policy()).unwrap();
    near(
        panel.volume_mm3().unwrap(),
        6000. - 125. * std::f64::consts::PI,
        1e-6,
    );
    assert_eq!(panel.solid_count().unwrap(), 1);
}
#[test]
fn true_circular_arcs_and_profiles_work_on_an_oriented_plane() {
    let profile = Profile {
        frame: PlaneFrame {
            origin: [12., 0., 0.],
            u: [0., 1., 0.],
            v: [0., 0., 1.],
        },
        outer: vec![
            ProfileEdge::Line {
                a: [-10., 0.],
                b: [10., 0.],
            },
            ProfileEdge::Arc {
                center: [0., 0.],
                a: [10., 0.],
                b: [-10., 0.],
                clockwise: false,
            },
        ],
        holes: vec![],
    };
    let solid = Solid::extrude_profile(&profile, -5., policy()).unwrap();
    near(
        solid.volume_mm3().unwrap(),
        250. * std::f64::consts::PI,
        1e-6,
    );
    let bounds = solid.bounds().unwrap();
    near(bounds.min[0], 7., 1e-7);
    near(bounds.max[0], 12., 1e-7);
    near(bounds.max[2], 10., 1e-7);
    let ring = Profile {
        frame: PlaneFrame::default(),
        outer: vec![ProfileEdge::Circle {
            center: [0., 0.],
            radius_mm: 7.,
        }],
        holes: vec![vec![ProfileEdge::Circle {
            center: [0., 0.],
            radius_mm: 2.,
        }]],
    };
    near(
        Solid::extrude_profile(&ring, 3., policy())
            .unwrap()
            .volume_mm3()
            .unwrap(),
        135. * std::f64::consts::PI,
        1e-6,
    );
}
#[test]
fn solid_booleans_report_empty_and_disconnected_results_without_changing_operands() {
    let a = Solid::box_solid([10., 20., 30.], policy()).unwrap();
    let b = moved(&a, [5., 0., 0.]);
    for (op, expected) in [
        (BooleanOperation::Union, 9000.),
        (BooleanOperation::Subtract, 3000.),
        (BooleanOperation::Intersect, 3000.),
    ] {
        let result = a.boolean(&b, op, policy()).unwrap().unwrap();
        near(result.volume_mm3().unwrap(), expected, 1e-6);
        assert_eq!(result.solid_count().unwrap(), 1);
    }
    assert!(
        a.boolean(&a, BooleanOperation::Subtract, policy())
            .unwrap()
            .is_none()
    );
    let touching = moved(&a, [10., 0., 0.]);
    assert!(
        a.boolean(&touching, BooleanOperation::Intersect, policy())
            .unwrap()
            .is_none()
    );
    let disjoint = moved(&a, [10.001, 0., 0.]);
    assert!(
        a.boolean(&disjoint, BooleanOperation::Intersect, policy())
            .unwrap()
            .is_none()
    );
    assert_eq!(
        a.boolean(&disjoint, BooleanOperation::Union, policy())
            .unwrap()
            .unwrap()
            .solid_count()
            .unwrap(),
        2
    );
    near(a.volume_mm3().unwrap(), 6000., 1e-7);
    near(b.volume_mm3().unwrap(), 6000., 1e-7);
}
#[test]
fn near_coplanar_rebuilds_are_valid_or_return_an_explicit_failure() {
    let p = policy();
    for width in [10., 10.1, 25., 100.] {
        for dz in [0., 1e-9, -1e-9, 1e-7, -1e-7, 1e-5, -1e-5] {
            let a = Solid::box_solid([width, 10., 10.], p).unwrap();
            let b = moved(
                &Solid::box_solid([width, 10., 10.], p).unwrap(),
                [width * 0.5, 0., dz],
            );
            match a.boolean(&b, BooleanOperation::Subtract, p) {
                Ok(Some(result)) => {
                    near(
                        result.volume_mm3().unwrap(),
                        width * 50. + width * 5. * dz.abs(),
                        1e-4,
                    );
                    assert_eq!(result.solid_count().unwrap(), 1);
                }
                Ok(None) => panic!("Nonempty material disappeared at width={width} dz={dz}"),
                Err(e) => {
                    assert!(!e.message.is_empty());
                    assert!(matches!(
                        e.kind,
                        GeometryErrorKind::Native | GeometryErrorKind::InvalidResult
                    ));
                    eprintln!("Explicit failure width={width} dz={dz} policy={p:?}: {e}");
                }
            }
            near(a.volume_mm3().unwrap(), width * 100., 1e-6);
        }
    }
}
#[test]
fn rigid_placements_surface_queries_and_bounds_are_in_model_units() {
    let base = Solid::box_solid([10., 20., 30.], policy()).unwrap();
    let moved = base
        .transformed(RigidTransform {
            rotation: [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
            translation_mm: [100., 50., -5.],
        })
        .unwrap();
    let b = moved.bounds().unwrap();
    for (a, e) in b.min.into_iter().zip([80., 50., -5.]) {
        near(a, e, 1e-7);
    }
    for (a, e) in b.max.into_iter().zip([100., 60., 25.]) {
        near(a, e, 1e-7);
    }
    let faces = moved.faces().unwrap();
    assert_eq!(faces.len(), 6);
    near(faces.iter().map(|f| f.area_mm2).sum(), 2200., 1e-6);
    for face in faces {
        let (_, normal) = face.plane.unwrap();
        near(normal.iter().map(|v| v * v).sum(), 1., 1e-12);
        assert!(face.center_mm.iter().all(|v| v.is_finite()));
    }
    let cylinder = Solid::cylinder(PlaneFrame::default(), 5., 10., policy()).unwrap();
    near(
        cylinder.volume_mm3().unwrap(),
        250. * std::f64::consts::PI,
        1e-6,
    );
    assert!(cylinder.faces().unwrap().iter().any(|f| f.plane.is_none()));
}
#[test]
fn tessellation_settings_and_booleans_do_not_mutate_manufacturing_geometry() {
    let s = Solid::cylinder(PlaneFrame::default(), 20., 30., policy()).unwrap();
    let volume = s.volume_mm3().unwrap();
    let bounds = s.bounds().unwrap();
    let faces = s.faces().unwrap();
    let coarse = s
        .mesh(MeshOptions {
            linear_deflection_mm: 1.,
            angular_deflection_rad: 1.,
            ..Default::default()
        })
        .unwrap();
    let fine = s
        .mesh(MeshOptions {
            linear_deflection_mm: 0.01,
            angular_deflection_rad: 0.05,
            ..Default::default()
        })
        .unwrap();
    assert!(fine.indices.len() > coarse.indices.len());
    assert_eq!(s.volume_mm3().unwrap(), volume);
    assert_eq!(s.bounds().unwrap(), bounds);
    assert_eq!(s.faces().unwrap(), faces);
    assert!(
        s.mesh(MeshOptions {
            maximum_triangles: 1,
            ..Default::default()
        })
        .is_err()
    );
    for mesh in [coarse, fine] {
        assert!(
            mesh.indices
                .iter()
                .all(|i| (*i as usize) < mesh.vertices.len() / 3)
        );
    }
}
#[test]
fn malformed_geometry_frames_profiles_and_limits_fail_without_losing_a_valid_solid() {
    for v in [0., -1., f64::NAN, f64::INFINITY, 1e8, 1e-8] {
        assert!(Solid::box_solid([v, 10., 10.], policy()).is_err());
    }
    let s = Solid::box_solid([10., 20., 30.], policy()).unwrap();
    let before = s.volume_mm3().unwrap();
    for rotation in [
        [[2., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        [[-1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
    ] {
        assert!(
            s.transformed(RigidTransform {
                rotation,
                ..Default::default()
            })
            .is_err()
        );
    }
    assert!(
        Solid::cylinder(
            PlaneFrame {
                u: [2., 0., 0.],
                ..Default::default()
            },
            5.,
            10.,
            policy()
        )
        .is_err()
    );
    let mut open = Profile::rectangle(10., 10.);
    open.outer.pop();
    assert!(Solid::extrude_profile(&open, 10., policy()).is_err());
    let crossed = Profile {
        frame: PlaneFrame::default(),
        outer: polygon(&[[0., 0.], [10., 10.], [0., 10.], [10., 0.]]),
        holes: vec![],
    };
    assert!(Solid::extrude_profile(&crossed, 10., policy()).is_err());
    let mut outside = Profile::rectangle(10., 10.);
    outside.holes.push(vec![ProfileEdge::Circle {
        center: [30., 30.],
        radius_mm: 1.,
    }]);
    assert!(Solid::extrude_profile(&outside, 10., policy()).is_err());
    assert!(
        s.mesh(MeshOptions {
            linear_deflection_mm: f64::NAN,
            ..Default::default()
        })
        .is_err()
    );
    assert_eq!(s.volume_mm3().unwrap(), before);
}
#[test]
fn step_exchange_preserves_millimeters_and_invalid_exchange_is_an_error() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("solid.step");
    let source = Solid::box_solid([100., 60., 18.], policy()).unwrap();
    source.write_step(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("SI_UNIT(.MILLI.,.METRE.)"));
    let reopened = Solid::read_step(&path, policy()).unwrap();
    near(reopened.volume_mm3().unwrap(), 108000., 1e-5);
    near(reopened.bounds().unwrap().max[0], 100., 1e-7);
    let bad = directory.path().join("broken.step");
    std::fs::write(&bad, b"not a STEP file").unwrap();
    assert!(Solid::read_step(&bad, policy()).is_err());
    assert!(
        source
            .write_step(&directory.path().join("missing/solid.step"))
            .is_err()
    );
}
#[test]
fn native_handles_are_created_and_destroyed_on_a_background_thread() {
    let result = std::thread::spawn(|| {
        let solid = Solid::box_solid([100., 60., 18.], policy()).unwrap();
        (
            solid.volume_mm3().unwrap(),
            solid.mesh(MeshOptions::default()).unwrap().indices,
        )
    })
    .join()
    .unwrap();
    near(result.0, 108000., 1e-6);
    assert!(!result.1.is_empty());
}

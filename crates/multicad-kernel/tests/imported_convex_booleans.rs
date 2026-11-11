use multicad_core::{document::EntityId, references::*};
use multicad_kernel::{
    rust_kernel::{
        Rational,
        planar_boolean::{ConvexPiece, Union},
        step,
        step_read::{ImportProfile, read_import_solid},
    },
    *,
};

const TOL: f64 = 1e-7;
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 3e-5, "{a} != {b}");
}
fn tetra() -> Union {
    let [a, b, c, d] = [[0., 0., 0.], [4., 0., 0.], [0., 4., 0.], [0., 0., 4.]];
    Union::new(
        &[ConvexPiece::new(
            vec![vec![a, c, b], vec![a, b, d], vec![a, d, c], vec![b, c, d]],
            TOL,
        )
        .unwrap()],
        TOL,
    )
    .unwrap()
}
fn placed_text(geometry: &Union, frame: PlaneFrame) -> String {
    step::write_planar_union(geometry, frame.origin, frame.u, frame.v).unwrap()
}
fn transform(frame: PlaneFrame) -> RigidTransform {
    RigidTransform {
        rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], frame.normal()[i]]),
        translation_mm: frame.origin,
    }
}
fn compare(a: &Solid, b: &Solid) {
    near(a.volume_mm3().unwrap(), b.volume_mm3().unwrap());
    near(
        a.faces().unwrap().iter().map(|f| f.area_mm2).sum(),
        b.faces().unwrap().iter().map(|f| f.area_mm2).sum(),
    );
    let (a, b) = (a.bounds().unwrap(), b.bounds().unwrap());
    for (a, b) in a
        .min
        .into_iter()
        .chain(a.max)
        .zip(b.min.into_iter().chain(b.max))
    {
        near(a, b);
    }
}

#[test]
fn imported_tetrahedron_proves_occupied_convex_material_for_cut_join_and_queries() {
    let policy = GeometryPolicy::default();
    let tilted = PlaneFrame {
        origin: [11., -7., 23.],
        u: [0.8, 0.6, 0.],
        v: [-0.36, 0.48, 0.8],
    };
    for frame in [PlaneFrame::default(), tilted] {
        let text = placed_text(&tetra(), frame);
        assert!(matches!(
            read_import_solid(&text, TOL).unwrap().1.profile,
            ImportProfile::PlanarBoundary { .. }
        ));
        for additional in [
            RigidTransform::default(),
            RigidTransform {
                rotation: [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
                translation_mm: [7., -2., 5.],
            },
        ] {
            let owner = EntityId::new();
            let source = Solid::import_rust_step(&text, owner, policy)
                .unwrap()
                .transformed(additional)
                .unwrap();
            let tool = Solid::rust_box(
                EntityId::new().uuid_bits(),
                [4, 1, 1].map(|n| Rational::new(n, 1).unwrap()),
                PlaneFrame {
                    origin: [1., 0., 0.],
                    ..Default::default()
                },
                policy,
            )
            .unwrap()
            .transformed(transform(frame))
            .unwrap()
            .transformed(additional)
            .unwrap();
            let before = source.topology_snapshot(owner, 0).unwrap();
            let tool_before = tool.topology_snapshot(owner, 0).unwrap();
            let cut = source
                .rust_subtract_planar_tools(&[&tool], EntityId::new(), policy)
                .unwrap()
                .unwrap();
            let join = Solid::union_planar_pieces(&[&source, &tool], EntityId::new(), policy)
                .unwrap()
                .unwrap();
            near(cut.volume_mm3().unwrap(), 26. / 3.);
            near(join.volume_mm3().unwrap(), 38. / 3.);
            let common = source
                .rust_planar_intersection_metrics(&tool, policy)
                .unwrap()
                .unwrap();
            near(common.0, 2.);
            let left_normal = additional.rotation.map(|row| {
                row.into_iter()
                    .zip(frame.u)
                    .map(|(a, b)| -a * b)
                    .sum::<f64>()
            });
            let left=before.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Plane {frame})if frame.normal().into_iter().zip(left_normal).all(|(a,b)|(a-b).abs()<1e-9))).unwrap();
            let Some(AnalyticGeometry::Plane { frame: left_plane }) = left.geometry else {
                unreachable!()
            };
            let on_left = |p: [f64; 3]| {
                (0..3)
                    .map(|i| left_plane.normal()[i] * (p[i] - left_plane.origin_mm[i]))
                    .sum::<f64>()
                    .abs()
                    < 1e-8
            };
            let retained: Vec<_> = before
                .elements
                .iter()
                .filter(|element| match element.geometry {
                    Some(AnalyticGeometry::Plane { .. }) => element.key == left.key,
                    Some(AnalyticGeometry::Line { a_mm, b_mm }) => on_left(a_mm) && on_left(b_mm),
                    Some(AnalyticGeometry::Point { position_mm }) => on_left(position_mm),
                    _ => false,
                })
                .map(|element| {
                    (
                        GeometryIntent::capture(&before, element.key).unwrap(),
                        element.geometry,
                    )
                })
                .collect();
            assert_eq!(retained.len(), 7);
            for result in [&cut, &join] {
                assert_eq!(result.solid_count().unwrap(), 1);
                let output = result.topology_snapshot(owner, 1).unwrap();
                for (reference, original) in &retained {
                    let resolution = resolve(reference, &output, 1);
                    match resolution {
                        Resolution::Resolved { .. } => {}
                        Resolution::Ambiguous { candidates }
                            if reference.kind == TopologyKind::Edge =>
                        {
                            // Existing Boolean clipping can retain collinear
                            // subdivisions. Report every physical fragment;
                            // never silently choose one for the old full edge.
                            assert!(candidates.len() > 1);
                            let Some(AnalyticGeometry::Line { a_mm: a, b_mm: b }) = *original
                            else {
                                unreachable!()
                            };
                            let delta: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
                            let length2 = delta.iter().map(|v| v * v).sum::<f64>();
                            for key in candidates {
                                let current =
                                    output.elements.iter().find(|e| e.key == key).unwrap();
                                let Some(AnalyticGeometry::Line { a_mm, b_mm }) = current.geometry
                                else {
                                    panic!("nonlinear fragment")
                                };
                                for point in [a_mm, b_mm] {
                                    let t =
                                        (0..3).map(|i| (point[i] - a[i]) * delta[i]).sum::<f64>()
                                            / length2;
                                    assert!((-TOL..=1. + TOL).contains(&t));
                                    for i in 0..3 {
                                        near(point[i], a[i] + t * delta[i]);
                                    }
                                }
                            }
                        }
                        other => panic!("lost tetrahedron boundary {reference:?}: {other:?}"),
                    }
                }
                result
                    .manufacturing_mesh_with_receipt(Default::default())
                    .unwrap();
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join("result.step");
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
            }
            if OCCT_LINKED {
                let directory = tempfile::tempdir().unwrap();
                let a = directory.path().join("source.step");
                let b = directory.path().join("tool.step");
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
                    &join,
                    &a.boolean(&b, BooleanOperation::Union, policy)
                        .unwrap()
                        .unwrap(),
                );
                let native = a
                    .boolean(&b, BooleanOperation::Intersect, policy)
                    .unwrap()
                    .unwrap();
                near(common.0, native.volume_mm3().unwrap());
            }
            assert_eq!(source.topology_snapshot(owner, 0).unwrap(), before);
            assert_eq!(tool.topology_snapshot(owner, 0).unwrap(), tool_before);
        }
    }
}

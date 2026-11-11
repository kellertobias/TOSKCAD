use multicad_core::{document::EntityId, references::Origin};
use multicad_kernel::{
    BRepSelection, GeometryErrorKind, GeometryPolicy, PlaneFrame, Solid,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
};

fn cylinder(frame: PlaneFrame) -> Solid {
    let n = |v| Rational::new(v, 1).unwrap();
    let id = EntityId::new();
    Solid::from_axial_cylinder_sweep(
        AxialCylinder::new(
            id.uuid_bits(),
            Circle::new([n(0); 2], n(2)).unwrap(),
            n(0),
            n(5),
        )
        .unwrap(),
        frame,
        [0.; 2],
        false,
        Origin {
            source: id,
            role: "circle".into(),
        },
        GeometryPolicy::default(),
    )
    .unwrap()
}
fn probe(point: [f64; 3]) -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        [Rational::new(1, 1).unwrap(); 3],
        PlaneFrame {
            origin: point,
            ..Default::default()
        },
        GeometryPolicy::default(),
    )
    .unwrap()
}
fn vertex(solid: &Solid, point: [f64; 3]) -> BRepSelection {
    let id = solid
        .display_vertices()
        .unwrap()
        .into_iter()
        .find(|p| {
            p.position_mm
                .into_iter()
                .zip(point)
                .all(|(a, b)| (a - b).abs() < 1e-10)
        })
        .unwrap()
        .index;
    BRepSelection::Vertex(id)
}
fn near(a: [f64; 3], b: [f64; 3]) {
    for (a, b) in a.into_iter().zip(b) {
        assert!((a - b).abs() < 1e-8, "{a} != {b}");
    }
}

#[test]
fn finite_cylinder_point_projection_keeps_world_witnesses_and_selection_order() {
    let (s, c) = 0.4_f64.sin_cos();
    for frame in [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [10., -20., 30.],
            u: [c, 0., -s],
            v: [0., 1., 0.],
        },
    ] {
        let solid = cylinder(frame);
        for (p, q, d, inside) in [
            ([3., 4., 2.], [1.2, 1.6, 2.], 3., false),
            ([0., 1., -3.], [0., 1., 0.], 3., false),
            ([3., 4., 9.], [1.2, 1.6, 5.], 5., false),
            ([1., 0., 2.], [1., 0., 2.], 0., true),
            ([0., 0., 0.], [0., 0., 0.], 0., false),
        ] {
            let world = std::array::from_fn(|i| {
                frame.origin[i] + frame.u[i] * p[0] + frame.v[i] * p[1] + frame.normal()[i] * p[2]
            });
            let expected = std::array::from_fn(|i| {
                frame.origin[i] + frame.u[i] * q[0] + frame.v[i] * q[1] + frame.normal()[i] * q[2]
            });
            let probe = probe(world);
            let selected = vertex(&probe, world);
            let result = solid
                .minimum_distance(BRepSelection::Whole, &probe, selected)
                .unwrap();
            assert!((result.distance_mm - d).abs() < 1e-8);
            assert_eq!(result.inner_solution, inside);
            assert!(result.witnesses_complete);
            assert_eq!(result.total_solutions, Some(1));
            near(result.witnesses[0].a_mm, expected);
            near(result.witnesses[0].b_mm, world);
            let reverse = probe
                .minimum_distance(selected, &solid, BRepSelection::Whole)
                .unwrap();
            assert_eq!(reverse.distance_mm, result.distance_mm);
            assert_eq!(reverse.witnesses[0].a_mm, result.witnesses[0].b_mm);
            assert_eq!(reverse.witnesses[0].b_mm, result.witnesses[0].a_mm);
            #[cfg(feature = "occt")]
            {
                use multicad_kernel::RigidTransform;
                let native = Solid::cylinder(frame, 2., 5., GeometryPolicy::default()).unwrap();
                let point = Solid::box_solid([1.; 3], GeometryPolicy::default())
                    .unwrap()
                    .transformed(RigidTransform {
                        translation_mm: world,
                        ..Default::default()
                    })
                    .unwrap();
                let comparison = native
                    .minimum_distance(BRepSelection::Whole, &point, vertex(&point, world))
                    .unwrap();
                assert!((comparison.distance_mm - result.distance_mm).abs() < 1e-7);
                near(comparison.witnesses[0].a_mm, result.witnesses[0].a_mm);
            }
        }
    }
}

#[test]
fn missing_vertex_and_unresolvable_placement_return_explicit_failures() {
    let solid = cylinder(PlaneFrame::default());
    let probe = probe([4., 0., 2.]);
    assert_eq!(
        solid
            .minimum_distance(
                BRepSelection::Whole,
                &probe,
                BRepSelection::Vertex(u32::MAX)
            )
            .unwrap_err()
            .kind,
        GeometryErrorKind::Input
    );
    let far = cylinder(PlaneFrame {
        origin: [9_000_000., 0., 0.],
        ..Default::default()
    });
    assert_eq!(
        far.minimum_distance(BRepSelection::Whole, &probe, BRepSelection::Vertex(0))
            .unwrap_err()
            .kind,
        GeometryErrorKind::Unsupported
    );
}

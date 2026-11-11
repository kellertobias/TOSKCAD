use multicad_core::{document::EntityId, references::Origin};
use multicad_kernel::{
    BRepSelection, CircularPocketTool, GeometryErrorKind, GeometryPolicy, PlaneFrame,
    RigidTransform, Solid,
    rust_kernel::{Circle, Rational, polygon::PolygonPrism},
};

fn panel() -> Solid {
    let owner = EntityId::new();
    let exact = |n| Rational::new(n, 1).unwrap();
    let prism = PolygonPrism::with_circular_holes(
        vec![
            [[0, 0], [20, 0], [20, 16], [0, 16]]
                .map(|p| p.map(exact))
                .to_vec(),
        ],
        vec![Circle::new([exact(10), exact(8)], exact(2)).unwrap()],
        exact(10),
    )
    .unwrap();
    let origins: Vec<_> = (0..5)
        .map(|i| Origin {
            source: owner,
            role: format!("edge/{i}"),
        })
        .collect();
    Solid::from_polygon_prism_sweep(
        prism,
        PlaneFrame::default(),
        false,
        owner,
        &origins,
        &[false; 5],
        GeometryPolicy::default(),
    )
    .unwrap()
}
fn box_at(size: [i64; 3], origin: [f64; 3]) -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        size.map(|v| Rational::new(v, 1).unwrap()),
        PlaneFrame {
            origin,
            ..Default::default()
        },
        GeometryPolicy::default(),
    )
    .unwrap()
}
fn drilled_stock() -> Solid {
    let stock = box_at([20, 16, 10], [0.; 3]);
    let tools = [
        (
            PlaneFrame {
                origin: [-1., 5., 4.],
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            },
            1.5,
            22.,
        ),
        (
            PlaneFrame {
                origin: [16., 12., 11.],
                v: [0., -1., 0.],
                ..Default::default()
            },
            2.,
            4.,
        ),
    ]
    .map(|(frame, radius_mm, length)| CircularPocketTool {
        frame,
        center_uv: [0.; 2],
        radius_mm,
        displacement_mm: frame.normal().map(|x| x * length),
        origin: Origin {
            source: EntityId::new(),
            role: "cut".into(),
        },
    });
    stock
        .rust_drill_circular_machining(&tools, &[true, false], GeometryPolicy::default())
        .unwrap()
}

#[test]
fn separated_curved_stock_preserves_finite_witnesses_under_both_query_orders_and_pose() {
    let pose = RigidTransform {
        rotation: [[0.36, -0.8, 0.48], [0.48, 0.6, 0.64], [-0.8, 0., 0.6]],
        translation_mm: [19., -23., 41.],
    };
    for stock in [panel(), drilled_stock()] {
        let probe = box_at([20, 16, 2], [0., 0., 14.]);
        for transformed in [false, true] {
            let (a, b) = if transformed {
                (
                    stock.transformed(pose).unwrap(),
                    probe.transformed(pose).unwrap(),
                )
            } else {
                (
                    stock.transformed(RigidTransform::default()).unwrap(),
                    probe.transformed(RigidTransform::default()).unwrap(),
                )
            };
            for (a, b) in [(&a, &b), (&b, &a)] {
                let result = a
                    .minimum_distance(BRepSelection::Whole, b, BRepSelection::Whole)
                    .unwrap();
                assert!((result.distance_mm - 4.).abs() < 1e-7);
                assert!(result.witnesses_complete);
                assert_eq!(result.total_solutions as usize, result.witnesses.len());
                assert!(!result.inner_solution);
                for witness in result.witnesses {
                    let d: [f64; 3] = std::array::from_fn(|i| witness.a_mm[i] - witness.b_mm[i]);
                    assert!((d[0].hypot(d[1]).hypot(d[2]) - 4.).abs() < 1e-7);
                }
            }
        }
    }
}

#[test]
fn hidden_stock_witnesses_are_refused_and_unaffected_contact_is_retained() {
    let stock = panel();
    // Every lower probe corner projects into the bore's removed opening.
    for origin in [[9.75, 7.75, 14.], [11.5, 9.5, 14.]] {
        let probe = Solid::rust_box(
            EntityId::new().uuid_bits(),
            [Rational::new(1, 2).unwrap(); 3],
            PlaneFrame {
                origin,
                ..Default::default()
            },
            GeometryPolicy::default(),
        )
        .unwrap();
        let failure = stock
            .minimum_distance(BRepSelection::Whole, &probe, BRepSelection::Whole)
            .unwrap_err();
        assert_eq!(failure.kind, GeometryErrorKind::Unsupported);
    }
    let contact = box_at([20, 16, 2], [0., 0., 10.]);
    let result = stock
        .minimum_distance(BRepSelection::Whole, &contact, BRepSelection::Whole)
        .unwrap();
    assert_eq!(result.distance_mm, 0.);
    assert!(!result.inner_solution);
}

#[cfg(feature = "occt")]
#[test]
fn independent_native_readback_agrees_on_the_machined_material_distance() {
    let scratch = tempfile::tempdir().unwrap();
    for (i, stock) in [panel(), drilled_stock()].into_iter().enumerate() {
        let probe = box_at([20, 16, 2], [0., 0., 14.]);
        let mut native = Vec::new();
        for (j, shape) in [&stock, &probe].into_iter().enumerate() {
            let path = scratch.path().join(format!("part-{i}-{j}.step"));
            shape.write_step(&path).unwrap();
            native.push(Solid::read_step(&path, GeometryPolicy::default()).unwrap());
        }
        let rust = stock
            .minimum_distance(BRepSelection::Whole, &probe, BRepSelection::Whole)
            .unwrap();
        let actual = native[0]
            .minimum_distance(BRepSelection::Whole, &native[1], BRepSelection::Whole)
            .unwrap();
        assert!((rust.distance_mm - actual.distance_mm).abs() < 1e-7);
    }
}

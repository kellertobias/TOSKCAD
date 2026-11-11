use multicad_core::document::EntityId;
use multicad_kernel::{
    BooleanOperation, GeometryErrorKind, GeometryPolicy, PlaneFrame, Solid,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder, parse_length_mm},
};

fn length(text: &str) -> Rational {
    parse_length_mm(&format!("{text} mm")).unwrap()
}

fn stock(origin: [f64; 3]) -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        [length("10"); 3],
        PlaneFrame {
            origin,
            ..Default::default()
        },
        GeometryPolicy::default(),
    )
    .unwrap()
}

#[test]
fn translated_stock_keeps_its_pose_and_exact_bore_through_the_boolean_dispatch() {
    for origin in [[100., -20., 30.], [0.1, -0.2, 0.3]] {
        let body = stock(origin);
        let feature = EntityId::new();
        let center = origin.map(|v| length(&v.to_string()));
        let five = length("5");
        let circle = Circle::new(
            [
                center[0].checked_add(five).unwrap(),
                center[1].checked_add(five).unwrap(),
            ],
            length("1"),
        )
        .unwrap();
        let tool = Solid::from_axial_cylinder(
            AxialCylinder::new(feature.uuid_bits(), circle, center[2], length("10")).unwrap(),
            PlaneFrame {
                origin: [
                    circle.center()[0].as_f64(),
                    circle.center()[1].as_f64(),
                    origin[2],
                ],
                ..Default::default()
            },
            GeometryPolicy::default(),
        )
        .unwrap();
        let result = body
            .boolean(&tool, BooleanOperation::Subtract, GeometryPolicy::default())
            .unwrap()
            .unwrap();
        assert!(result.uses_rust_geometry());
        assert!((result.volume_mm3().unwrap() - (1000. - 10. * std::f64::consts::PI)).abs() < 1e-8);
        assert_eq!(result.bounds().unwrap(), body.bounds().unwrap());
        let holes = result
            .rust_prism()
            .unwrap()
            .inner_loops()
            .collect::<Vec<_>>();
        assert_eq!(
            holes,
            vec![(
                feature.uuid_bits(),
                Circle::new([five; 2], length("1")).unwrap()
            )]
        );
        assert_eq!(body.rust_prism().unwrap().inner_loops().len(), 0);
        let before = body.topology_snapshot(EntityId::new(), 0).unwrap();
        let after = result.topology_snapshot(before.owner, 1).unwrap();
        for source in before
            .elements
            .iter()
            .filter(|e| e.key.kind == multicad_core::references::TopologyKind::Face)
        {
            assert!(
                after
                    .elements
                    .iter()
                    .any(|e| e.origins == source.origins && e.frames == source.frames)
            );
        }
    }
}

#[test]
fn translated_bore_never_rounds_a_subtolerance_floor_into_an_opening() {
    let body = stock([0.1, -0.2, 0.3]);
    let circle = Circle::new([length("5.1"), length("4.8")], length("1")).unwrap();
    for (bottom, top) in [("0.300000000001", "10.3"), ("0.3", "10.299999999999")] {
        let failure = body
            .subtract_rust_prism_cylinder(
                EntityId::new().uuid_bits(),
                circle,
                length(bottom),
                length(top),
            )
            .err()
            .unwrap();
        assert_eq!(failure.kind, GeometryErrorKind::Input);
        assert_eq!(body.rust_prism().unwrap().inner_loops().len(), 0);
        assert_eq!(body.volume_mm3().unwrap(), 1000.);
    }
}

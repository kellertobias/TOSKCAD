use multicad_core::document::EntityId;
use multicad_kernel::{
    BooleanOperation, GeometryPolicy, PlaneFrame, RigidTransform, Solid,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
};

fn stock() -> Solid {
    Solid::rust_box(
        EntityId::new().uuid_bits(),
        [20, 16, 10].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame::default(),
        GeometryPolicy::default(),
    )
    .unwrap()
}
fn tool(z: f64) -> Solid {
    let n = |v| Rational::new(v, 1).unwrap();
    let cylinder = AxialCylinder::new(
        EntityId::new().uuid_bits(),
        Circle::new([n(0), n(0)], n(2)).unwrap(),
        n(0),
        n(2),
    )
    .unwrap();
    Solid::from_axial_cylinder(cylinder, PlaneFrame::default(), GeometryPolicy::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: [10., 8., z],
            ..Default::default()
        })
        .unwrap()
}

#[test]
fn axially_disjoint_and_end_contact_cylinders_preserve_actual_stock_and_references() {
    let stock = stock();
    let owner = EntityId::new();
    let before = stock.topology_snapshot(owner, 0).unwrap();
    for z in [-5., -2., 10., 14.] {
        let result = stock
            .boolean(
                &tool(z),
                BooleanOperation::Subtract,
                GeometryPolicy::default(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(result.volume_mm3().unwrap(), 3200.);
        assert_eq!(result.solid_count().unwrap(), 1);
        assert!(
            result.rust_prism().is_some(),
            "A no-op must keep the original stock provider"
        );
        assert_eq!(
            before.elements,
            result.topology_snapshot(owner, 0).unwrap().elements
        );
    }
}

#[test]
fn a_real_subtolerance_penetration_is_never_published_as_contact() {
    let stock = stock();
    for z in [10. - 1e-8, -2. + 1e-8] {
        assert!(
            stock
                .boolean(
                    &tool(z),
                    BooleanOperation::Subtract,
                    GeometryPolicy::default()
                )
                .is_err(),
            "A positive cut below the minimum feature size needs an explicit refusal"
        );
    }
    assert_eq!(stock.volume_mm3().unwrap(), 3200.);
}

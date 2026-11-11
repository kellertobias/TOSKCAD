use multicad_core::document::EntityId;
use multicad_kernel::{
    GeometryErrorKind, GeometryPolicy, PlaneFrame, RigidTransform, Solid,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
};
fn descriptor(radius: i64) -> AxialCylinder {
    let n = |n| Rational::new(n, 1).unwrap();
    AxialCylinder::new(
        EntityId::new().uuid_bits(),
        Circle::new([n(0); 2], n(radius)).unwrap(),
        n(0),
        n(10),
    )
    .unwrap()
}
fn frame() -> PlaneFrame {
    PlaneFrame {
        u: [(1. + 8e-11_f64).sqrt(), 0., 0.],
        ..Default::default()
    }
}
#[test]
fn cylinder_refuses_large_metric_distortion_but_accepts_residual_within_budget() {
    let failure =
        Solid::from_axial_cylinder(descriptor(1_000_000), frame(), GeometryPolicy::default())
            .err()
            .unwrap();
    assert_eq!(failure.kind, GeometryErrorKind::Unsupported);
    assert!(failure.message.contains("metric residual"));
    let small =
        Solid::from_axial_cylinder(descriptor(4), frame(), GeometryPolicy::default()).unwrap();
    assert!((small.volume_mm3().unwrap() - std::f64::consts::PI * 160.).abs() < 1e-10);
    let (s, c) = 0.4_f64.sin_cos();
    let proper = PlaneFrame {
        u: [c, s, 0.],
        v: [-s, c, 0.],
        ..Default::default()
    };
    let large =
        Solid::from_axial_cylinder(descriptor(1_000_000), proper, GeometryPolicy::default())
            .unwrap();
    assert!(large.volume_mm3().unwrap().is_finite());
}
#[test]
fn transformed_cylinder_checks_the_composed_metric_before_publication() {
    let original = Solid::from_axial_cylinder(
        descriptor(1_000_000),
        PlaneFrame::default(),
        GeometryPolicy::default(),
    )
    .unwrap();
    let before = original.bounds().unwrap();
    let volume = original.volume_mm3().unwrap();
    let transform = RigidTransform {
        rotation: [[frame().u[0], 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        ..Default::default()
    };
    assert_eq!(
        original.transformed(transform).err().unwrap().kind,
        GeometryErrorKind::Unsupported
    );
    assert_eq!(original.bounds().unwrap(), before);
    assert_eq!(original.volume_mm3().unwrap(), volume);
    let small = Solid::from_axial_cylinder(
        descriptor(4),
        PlaneFrame::default(),
        GeometryPolicy::default(),
    )
    .unwrap();
    assert!(small.transformed(transform).is_ok());
}

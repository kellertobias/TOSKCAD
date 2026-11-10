use multicad_core::{
    document::EntityId,
    references::{Origin, TopologyKind},
};
use multicad_kernel::{
    GeometryErrorKind, GeometryPolicy, PlaneFrame, Solid,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
};
use std::f64::consts::PI;

fn cylinder(radius: i64, height: i64, origin: [f64; 3], positive: bool) -> (Solid, EntityId) {
    let feature = EntityId::new();
    let size = |n| Rational::new(n, 1).unwrap();
    let cylinder = AxialCylinder::new(
        feature.uuid_bits(),
        Circle::new([size(0); 2], size(radius)).unwrap(),
        size(0),
        size(height),
    )
    .unwrap();
    let solid = Solid::from_axial_cylinder_sweep(
        cylinder,
        PlaneFrame {
            origin,
            u: [1., 0., 0.],
            v: [0., 0., if positive { -1. } else { 1. }],
        },
        [0.; 2],
        false,
        Origin {
            source: feature,
            role: "authored circle".into(),
        },
        GeometryPolicy::default(),
    )
    .unwrap();
    (solid, feature)
}

#[test]
fn joins_touching_cylinders_along_both_axial_directions_and_keeps_cap_attachment() {
    let policy = GeometryPolicy::default();
    let (cap, cap_id) = cylinder(4, 3, [1., 2., 3.], false);
    let (shaft, shaft_id) = cylinder(2, 5, [1., -6., 3.], true);
    let feature = EntityId::new();
    let result = Solid::union_coaxial_cylinders(&[&cap, &shaft], feature, policy)
        .unwrap()
        .unwrap();
    assert_eq!(result.solid_count().unwrap(), 1);
    assert!((result.volume_mm3().unwrap() - 68. * PI).abs() < 1e-9);
    assert!(result.rust_revolution_contains([1., -5., 3.]).unwrap());
    assert!(!result.rust_revolution_contains([4., -5., 3.]).unwrap());
    let topology = result.topology_snapshot(feature, 3).unwrap();
    let origin = Origin {
        source: cap_id,
        role: "cap.end".into(),
    };
    let face = topology
        .elements
        .iter()
        .find(|element| element.key.kind == TopologyKind::Face && element.origins.contains(&origin))
        .unwrap();
    let anchor = face
        .frames
        .iter()
        .find(|anchor| anchor.origin == origin)
        .unwrap();
    assert_eq!(anchor.frame.origin_mm, [1., -1., 3.]);
    assert!(
        topology
            .elements
            .iter()
            .any(|element| element.origins.contains(&Origin {
                source: shaft_id,
                role: "authored circle".into(),
            }))
    );
    assert!(result.supports_step_export());
}

#[test]
fn proves_a_single_continuous_span_and_refuses_off_axis_or_fuzzy_union() {
    let policy = GeometryPolicy::default();
    let (a, _) = cylinder(3, 4, [1., 2., 3.], false);
    let (overlap, _) = cylinder(3, 4, [1., -4., 3.], true);
    let (gap, _) = cylinder(3, 4, [1., -3., 3.], false);
    let (off_axis, _) = cylinder(3, 4, [1.000000001, 2., 3.], false);
    let feature = EntityId::new();
    let result = Solid::union_coaxial_cylinders(&[&a, &overlap], feature, policy)
        .unwrap()
        .unwrap();
    assert!((result.volume_mm3().unwrap() - 54. * PI).abs() < 1e-9);
    assert!(result.supports_step_export());
    let disconnected = Solid::union_coaxial_cylinders(&[&a, &gap], feature, policy)
        .unwrap()
        .err()
        .unwrap();
    assert_eq!(disconnected.kind, GeometryErrorKind::InvalidResult);
    assert!(Solid::union_coaxial_cylinders(&[&a, &off_axis], feature, policy).is_none());
    assert!(
        Solid::union_coaxial_cylinders(
            &[&a, &overlap],
            feature,
            GeometryPolicy {
                boolean_fuzzy_mm: 1e-8,
                ..policy
            },
        )
        .is_none()
    );
}

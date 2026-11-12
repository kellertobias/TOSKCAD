use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, Origin, TopologyKind},
};
use multicad_kernel::{
    BRepSelection, GeometryErrorKind, GeometryPolicy, PlaneFrame, Solid,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
};
use std::f64::consts::TAU;

fn cylinder(radius: i64, frame: PlaneFrame) -> Solid {
    let n = |n| Rational::new(n, 1).unwrap();
    let id = EntityId::new();
    Solid::from_axial_cylinder_sweep(
        AxialCylinder::new(
            id.uuid_bits(),
            Circle::new([n(0); 2], n(radius)).unwrap(),
            n(0),
            n(1),
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
fn ring(solid: &Solid, center: [f64; 3]) -> BRepSelection {
    let topology = solid.topology_snapshot(EntityId::new(), 0).unwrap();
    BRepSelection::Edge(topology.elements.into_iter().find(|e|e.key.kind==TopologyKind::Edge && matches!(e.geometry,Some(AnalyticGeometry::CircularArc{frame,sweep_rad,..}) if sweep_rad.abs()==TAU && frame.origin_mm.into_iter().zip(center).all(|(a,b)|(a-b).abs()<1e-8))).unwrap().key.ordinal)
}
fn frames() -> [PlaneFrame; 2] {
    [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [19., -23., 41.],
            u: [0.36, 0.48, -0.8],
            v: [-0.8, 0.6, 0.],
        },
    ]
}

#[test]
fn coaxial_circle_families_preserve_finite_world_witnesses_for_pose_and_axis_reversal() {
    for frame in frames() {
        for (ra, rb, dz, expected) in [
            (5, 10, 0., 5.),
            (2, 5, 4., 5.),
            (5, 2, -4., 5.),
            (3, 3, 7., 7.),
            (3, 3, 0., 0.),
        ] {
            for flip in [false, true] {
                let a = cylinder(ra, frame);
                let center = std::array::from_fn(|i| frame.origin[i] + frame.normal()[i] * dz);
                let other = PlaneFrame {
                    origin: center,
                    u: frame.v,
                    v: frame.u.map(|x| if flip { x } else { -x }),
                };
                let b = cylinder(rb, other);
                let (sa, sb) = (ring(&a, frame.origin), ring(&b, center));
                for reverse in [false, true] {
                    let result = if reverse {
                        b.minimum_distance(sb, &a, sa)
                    } else {
                        a.minimum_distance(sa, &b, sb)
                    }
                    .unwrap();
                    assert!((result.distance_mm - expected).abs() < 1e-7);
                    assert_eq!(result.total_solutions, None);
                    assert!(!result.witnesses_complete && !result.inner_solution);
                    let witness = result.witnesses[0];
                    let (pa, pb) = if reverse {
                        (witness.b_mm, witness.a_mm)
                    } else {
                        (witness.a_mm, witness.b_mm)
                    };
                    let radial = |point: [f64; 3], origin: [f64; 3]| {
                        let d: [f64; 3] = std::array::from_fn(|i| point[i] - origin[i]);
                        d[0].hypot(d[1]).hypot(d[2])
                    };
                    assert!((radial(pa, frame.origin) - ra as f64).abs() < 1e-7);
                    assert!((radial(pb, center) - rb as f64).abs() < 1e-7);
                    assert!((radial(pa, pb) - expected).abs() < 1e-7);
                }
            }
        }
    }
}

#[test]
fn eccentric_tilted_partial_and_missing_edges_are_not_misclassified_as_coaxial() {
    let a = cylinder(3, PlaneFrame::default());
    let sa = ring(&a, [0.; 3]);
    let (s, c) = 0.2_f64.sin_cos();
    for frame in [
        PlaneFrame {
            origin: [1., 0., 0.],
            ..Default::default()
        },
        PlaneFrame {
            u: [c, 0., -s],
            v: [0., 1., 0.],
            ..Default::default()
        },
    ] {
        let b = cylinder(5, frame);
        let sb = ring(&b, frame.origin);
        assert_eq!(
            a.minimum_distance(sa, &b, sb).unwrap_err().kind,
            GeometryErrorKind::Unsupported
        );
    }
    let sphere = Solid::rust_sphere_named(
        3.,
        PlaneFrame::default(),
        EntityId::new(),
        GeometryPolicy::default(),
    )
    .unwrap();
    assert_eq!(
        a.minimum_distance(sa, &sphere, BRepSelection::Edge(0))
            .unwrap_err()
            .kind,
        GeometryErrorKind::Unsupported
    );
    assert_eq!(
        a.minimum_distance(sa, &a, BRepSelection::Edge(u32::MAX))
            .unwrap_err()
            .kind,
        GeometryErrorKind::Input
    );
}

#[cfg(feature = "occt")]
#[test]
fn independent_native_circle_extrema_agree_on_radial_and_axial_clearance() {
    let scratch = tempfile::tempdir().unwrap();
    for frame in frames() {
        let center = std::array::from_fn(|i| frame.origin[i] + frame.normal()[i] * 4.);
        let (a, b) = (
            cylinder(2, frame),
            cylinder(
                5,
                PlaneFrame {
                    origin: center,
                    ..frame
                },
            ),
        );
        let (sa, sb) = (ring(&a, frame.origin), ring(&b, center));
        let mut native = vec![];
        for (i, solid) in [&a, &b].into_iter().enumerate() {
            let path = scratch.path().join(format!("{i}.step"));
            solid.write_step(&path).unwrap();
            native.push(Solid::read_step(&path, GeometryPolicy::default()).unwrap());
        }
        let actual = native[0]
            .minimum_distance(
                ring(&native[0], frame.origin),
                &native[1],
                ring(&native[1], center),
            )
            .unwrap();
        let rust = a.minimum_distance(sa, &b, sb).unwrap();
        assert!((rust.distance_mm - actual.distance_mm).abs() < 1e-7);
    }
}

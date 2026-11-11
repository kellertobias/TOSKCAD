use multicad_core::{document::EntityId, references::Origin};
use multicad_kernel::{
    BRepSelection, CurvedEdge2d, CurvedProfile, GeometryErrorKind, GeometryPolicy, PlaneFrame,
    Solid, SolidPairRelation,
    rust_kernel::{Circle, Rational, cylinder::AxialCylinder},
};
type P = [f64; 2];
fn rectangle(a: P, b: P) -> Vec<P> {
    vec![a, [b[0], a[1]], b, [a[0], b[1]]]
}
fn profile(rings: &[Vec<P>], frame: PlaneFrame) -> CurvedProfile {
    let wire = |ring: &Vec<P>| {
        ring.iter()
            .zip(ring.iter().cycle().skip(1))
            .map(|(&a, &b)| CurvedEdge2d::Line { a, b })
            .collect()
    };
    CurvedProfile {
        frame,
        outer: wire(&rings[0]),
        holes: rings[1..].iter().map(wire).collect(),
    }
}
fn cylinder(frame: PlaneFrame) -> Solid {
    let id = EntityId::new();
    let n = |n| Rational::new(n, 1).unwrap();
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
fn prism(profile: &CurvedProfile, height: f64) -> Solid {
    let id = EntityId::new();
    let count = profile.outer.len() + profile.holes.iter().map(Vec::len).sum::<usize>();
    let origins = (0..count)
        .map(|i| Origin {
            source: id,
            role: format!("edge.{i}"),
        })
        .collect::<Vec<_>>();
    Solid::rust_extrude_linear_named(
        profile,
        profile.frame.normal().map(|n| n * height),
        0.,
        id,
        &origins,
        GeometryPolicy::default(),
    )
    .unwrap()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{a} != {b}");
}
#[test]
fn analytic_parallel_queries_keep_concavity_holes_pose_and_real_native_metrics() {
    let (s, c) = 0.4_f64.sin_cos();
    let mut hole = rectangle([-1., -1.], [1., 1.]);
    hole.reverse();
    let segment = 4. * 0.5_f64.acos() - 3_f64.sqrt();
    for frame in [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [10., -20., 30.],
            u: [c, 0., -s],
            v: [0., 1., 0.],
        },
    ] {
        let cylinder = cylinder(frame);
        for (rings, bottom, height, distance, volume) in [
            (vec![rectangle([-0.5, -0.5], [0.5, 0.5])], 1., 1., 0., 1.),
            (
                vec![rectangle([-3., -3.], [3., 3.])],
                1.,
                1.,
                0.,
                4. * std::f64::consts::PI,
            ),
            (
                vec![rectangle([0., -3.], [3., 3.])],
                1.,
                1.,
                0.,
                2. * std::f64::consts::PI,
            ),
            (vec![rectangle([1., -3.], [3., 3.])], 1., 1., 0., segment),
            (vec![rectangle([3., -1.], [5., 1.])], 1., 1., 1., 0.),
            (
                vec![rectangle([3., -1.], [5., 1.])],
                8.,
                1.,
                10_f64.sqrt(),
                0.,
            ),
            (
                vec![rectangle([-4., -4.], [4., 4.]), hole.clone()],
                1.,
                1.,
                0.,
                4. * std::f64::consts::PI - 4.,
            ),
            (
                vec![vec![
                    [-3., -3.],
                    [3., -3.],
                    [3., 3.],
                    [1., 3.],
                    [1., -2.],
                    [-1., -2.],
                    [-1., 3.],
                    [-3., 3.],
                ]],
                1.,
                1.,
                0.,
                2. * segment,
            ),
        ] {
            let (s, c) = 0.35_f64.sin_cos();
            let pframe = PlaneFrame {
                origin: std::array::from_fn(|i| frame.origin[i] + frame.normal()[i] * bottom),
                u: std::array::from_fn(|i| frame.u[i] * c + frame.v[i] * s),
                v: std::array::from_fn(|i| -frame.u[i] * s + frame.v[i] * c),
            };
            let profile = profile(&rings, pframe);
            let part = prism(&profile, height);
            let result = cylinder
                .inspect_pair(&part, 1e-3, GeometryPolicy::default())
                .unwrap();
            near(result.minimum_distance.distance_mm, distance);
            near(result.common_volume_mm3, volume);
            assert_eq!(
                result.relation,
                if volume > 0. {
                    SolidPairRelation::Overlapping
                } else {
                    SolidPairRelation::Separated
                }
            );
            assert_eq!(
                result.minimum_distance.witnesses_complete,
                result.minimum_distance.witnesses.len()
                    == result.minimum_distance.total_solutions as usize
            );
            let reverse = part
                .inspect_pair(&cylinder, 1e-3, GeometryPolicy::default())
                .unwrap();
            near(reverse.common_volume_mm3, result.common_volume_mm3);
            assert_eq!(
                reverse.minimum_distance.witnesses[0].a_mm,
                result.minimum_distance.witnesses[0].b_mm
            );
            assert_eq!(
                reverse.minimum_distance.witnesses[0].b_mm,
                result.minimum_distance.witnesses[0].a_mm
            );
            near(
                cylinder
                    .minimum_distance(BRepSelection::Whole, &part, BRepSelection::Whole)
                    .unwrap()
                    .distance_mm,
                distance,
            );
            near(
                cylinder
                    .rust_occupied_overlap_volume(&part, GeometryPolicy::default())
                    .unwrap(),
                volume,
            );
            #[cfg(feature = "occt")]
            {
                let native = Solid::cylinder(frame, 2., 5., GeometryPolicy::default()).unwrap();
                let native_part = Solid::extrude_curved_profile_vector_named(
                    &profile,
                    pframe.normal().map(|n| n * height),
                    EntityId::new(),
                    &(0..profile.outer.len() + profile.holes.iter().map(Vec::len).sum::<usize>())
                        .map(|i| Origin {
                            source: EntityId::new(),
                            role: format!("edge.{i}"),
                        })
                        .collect::<Vec<_>>(),
                    GeometryPolicy::default(),
                )
                .unwrap();
                let reference = native
                    .inspect_pair(&native_part, 1e-3, GeometryPolicy::default())
                    .unwrap();
                near(reference.common_volume_mm3, result.common_volume_mm3);
                near(
                    reference.minimum_distance.distance_mm,
                    result.minimum_distance.distance_mm,
                );
            }
        }
    }
}
#[test]
fn uncertain_tangency_and_long_thin_cap_tilt_are_refused_atomically() {
    let c = cylinder(PlaneFrame::default());
    let tangent = prism(
        &profile(&[rectangle([2., -1.], [3., 1.])], PlaneFrame::default()),
        1.,
    );
    let error = c
        .inspect_pair(&tangent, 1e-3, GeometryPolicy::default())
        .unwrap_err();
    assert_eq!(error.kind, GeometryErrorKind::Unsupported);
    let (s, co) = 1e-8_f64.sin_cos();
    let frame = PlaneFrame {
        origin: [0.; 3],
        u: [co, 0., s],
        v: [0., 1., 0.],
    };
    let skew = prism(
        &profile(&[rectangle([-500., -1.], [500., 1.])], frame),
        0.001,
    );
    assert_eq!(
        c.inspect_pair(&skew, 1e-3, GeometryPolicy::default())
            .unwrap_err()
            .kind,
        GeometryErrorKind::Unsupported
    );
    near(c.volume_mm3().unwrap(), 20. * std::f64::consts::PI);
}

#[test]
fn rectangular_and_reversed_polygon_stock_use_the_same_occupied_query() {
    use multicad_kernel::rust_kernel::polygon::PolygonPrism;
    let c = cylinder(PlaneFrame::default());
    let one = Rational::new(1, 1).unwrap();
    let box_ = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [one; 3],
        PlaneFrame {
            origin: [-0.5, -0.5, 1.],
            ..Default::default()
        },
        GeometryPolicy::default(),
    )
    .unwrap();
    near(
        c.inspect_pair(&box_, 1e-3, GeometryPolicy::default())
            .unwrap()
            .common_volume_mm3,
        1.,
    );
    for (reverse, v, z) in [
        (false, [0., 1., 0.], 1.),
        (true, [0., 1., 0.], 2.),
        (false, [0., -1., 0.], 2.),
    ] {
        let id = EntityId::new();
        let origins = (0..4)
            .map(|i| Origin {
                source: id,
                role: format!("edge.{i}"),
            })
            .collect::<Vec<_>>();
        let ring = rectangle([-1., -1.], [1., 1.])
            .into_iter()
            .map(|p| p.map(|v| Rational::parse_decimal(&v.to_string()).unwrap()))
            .collect();
        let stock = Solid::from_polygon_prism_sweep(
            PolygonPrism::new(vec![ring], one).unwrap(),
            PlaneFrame {
                origin: [0., 0., z],
                v,
                ..Default::default()
            },
            reverse,
            id,
            &origins,
            &[false; 4],
            GeometryPolicy::default(),
        )
        .unwrap();
        near(
            c.inspect_pair(&stock, 1e-3, GeometryPolicy::default())
                .unwrap()
                .common_volume_mm3,
            4.,
        );
    }
    let mut hole = rectangle([-3., -3.], [3., 3.]);
    hole.reverse();
    let stock = prism(
        &profile(
            &[rectangle([-4., -4.], [4., 4.]), hole],
            PlaneFrame::default(),
        ),
        1.,
    );
    let distance = c
        .minimum_distance(BRepSelection::Whole, &stock, BRepSelection::Whole)
        .unwrap();
    assert!(distance.witnesses_complete);
    assert_eq!(distance.total_solutions, 4);
    near(distance.distance_mm, 1.);
    assert!(
        distance
            .witnesses
            .iter()
            .any(|w| w.b_mm[0] - w.a_mm[0] > 0.9)
    );
    assert!(
        distance
            .witnesses
            .iter()
            .any(|w| w.b_mm[0] - w.a_mm[0] < -0.9)
    );
}

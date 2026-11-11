use multicad_core::{document::EntityId, references::Origin};
use multicad_kernel::{
    CircularPocketTool, CurvedEdge2d, CurvedProfile, GeometryErrorKind, GeometryPolicy, PlaneFrame,
    RigidTransform, Solid, rust_kernel::Rational,
};
fn rectangle(frame: PlaneFrame, x: [f64; 2], y: [f64; 2]) -> Solid {
    let feature = EntityId::new();
    let corners = [[x[0], y[0]], [x[1], y[0]], [x[1], y[1]], [x[0], y[1]]];
    let profile = CurvedProfile {
        frame,
        outer: (0..4)
            .map(|i| CurvedEdge2d::Line {
                a: corners[i],
                b: corners[(i + 1) % 4],
            })
            .collect(),
        holes: vec![],
    };
    let origins = (0..4)
        .map(|i| Origin {
            source: feature,
            role: format!("edge/{i}"),
        })
        .collect::<Vec<_>>();
    Solid::rust_extrude_between_planes_named(
        &profile,
        [0., 0., 1.],
        [
            frame,
            PlaneFrame {
                origin: [frame.origin[0], frame.origin[1], frame.origin[2] + 3.],
                ..frame
            },
        ],
        feature,
        &origins,
        GeometryPolicy::default(),
    )
    .unwrap()
}

#[test]
fn curved_orthogonal_stock_keeps_its_existing_occupied_cell_clearance_provider() {
    let policy = GeometryPolicy::default();
    let make_box = |size: [i64; 3], origin| {
        Solid::rust_box(
            EntityId::new().uuid_bits(),
            size.map(|v| Rational::new(v, 1).unwrap()),
            PlaneFrame {
                origin,
                ..Default::default()
            },
            policy,
        )
        .unwrap()
    };
    let stock = make_box([30, 20, 4], [0.; 3])
        .rust_subtract_orthogonal_tools(&[make_box([20, 10, 6], [5., 5., -1.])], policy)
        .unwrap()
        .unwrap();
    let drill = CircularPocketTool {
        frame: PlaneFrame {
            origin: [2.5, 10., -1.],
            ..Default::default()
        },
        center_uv: [0.; 2],
        radius_mm: 1.,
        displacement_mm: [0., 0., 6.],
        origin: Origin {
            source: EntityId::new(),
            role: "mounting bore".into(),
        },
    };
    let drilled = stock
        .rust_drill_circular_machining(&[drill], &[true], policy)
        .unwrap();
    for rotation in [
        RigidTransform::default().rotation,
        [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
    ] {
        let pose = RigidTransform {
            rotation,
            translation_mm: [100., 200., 7.],
        };
        let moved = drilled.transformed(pose).unwrap();
        let chart = PlaneFrame {
            origin: pose.translation_mm,
            u: rotation.map(|row| row[0]),
            v: rotation.map(|row| row[1]),
        };
        let probe = rectangle(chart, [8., 13.], [7., 10.]);
        assert_eq!(
            moved.rust_occupied_overlap_volume(&probe, policy).unwrap(),
            0.
        );
        assert_eq!(
            probe.rust_occupied_overlap_volume(&moved, policy).unwrap(),
            0.
        );
        let material = rectangle(chart, [1., 4.], [7., 10.]);
        assert_eq!(
            moved
                .rust_occupied_overlap_volume(&material, policy)
                .unwrap_err()
                .kind,
            GeometryErrorKind::Unsupported
        );
    }
}
#[test]
fn common_chart_empty_certificate_never_hides_a_retained_positive_overlap() {
    let policy = GeometryPolicy::default();
    for angle in [0., std::f64::consts::PI / 6.] {
        let (s, c) = angle.sin_cos();
        for translation in [
            [0.; 3],
            [10_000., 20_000., 7.],
            [9_999_900., 9_999_900., 7.],
        ] {
            let frame = PlaneFrame {
                origin: translation,
                u: [c, s, 0.],
                v: [-s, c, 0.],
            };
            let first = rectangle(frame, [0., 10.], [0., 1.]);
            let second = rectangle(frame, [0., 10.], [2., 3.]);
            let compound =
                Solid::compound_named(&[&first, &second], EntityId::new(), policy).unwrap();
            for epsilon in [-1e-4, -1e-11, 0., 1e-11, 1e-8, 1e-4] {
                let probe = rectangle(frame, [10. - epsilon, 20.], [0.25, 0.75]);
                for stock in [&first, &compound] {
                    let result = stock.rust_planar_intersection_metrics(&probe, policy);
                    let occupied = stock.rust_occupied_overlap_volume(&probe, policy);
                    let inspected = stock.inspect_pair(&probe, policy.linear_tolerance_mm, policy);
                    if epsilon <= 0. {
                        assert!(result.unwrap().is_none());
                        assert_eq!(occupied.unwrap(), 0.);
                        assert_eq!(inspected.unwrap().common_volume_mm3, 0.);
                    } else {
                        assert!(
                            !matches!(result, Ok(None)),
                            "positive source band erased at {translation:?}, epsilon {epsilon}"
                        );
                        assert!(
                            !matches!(occupied,Ok(volume) if volume==0.),
                            "occupied query erased source overlap"
                        );
                        assert!(
                            !matches!(inspected,Ok(ref pair) if pair.common_volume_mm3==0.),
                            "pair inspection erased source overlap"
                        );
                    }
                }
            }
            let overlap = rectangle(frame, [0.5, 2.], [0.25, 2.75]);
            assert!(
                compound
                    .rust_planar_intersection_metrics(&overlap, policy)
                    .unwrap()
                    .unwrap()
                    .0
                    > 0.
            );
        }
    }
}

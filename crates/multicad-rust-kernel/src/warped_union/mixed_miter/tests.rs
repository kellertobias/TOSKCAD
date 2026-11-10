use super::*;
use crate::{display::Tessellation, manufacturing};
use std::f64::consts::PI;

const TOL: f64 = 1e-7;
const CHAIN: [Point; 4] = [[0.; 3], [0., 0., 20.], [20., 0., 20.], [20., 20., 20.]];
const AREA: f64 = 12. + PI;
const PERIMETER: f64 = 12. + 2. * PI;

fn stadium(offset: f64) -> [ProfileEdge; 4] {
    let [a, b, c, d] = [[-3., -1.], [3., -1.], [3., 1.], [-3., 1.]].map(|[x, y]| [x + offset, y]);
    [
        ProfileEdge::Line { a, b },
        ProfileEdge::Arc {
            center: [3. + offset, 0.],
            a: b,
            b: c,
            clockwise: false,
        },
        ProfileEdge::Line { a: c, b: d },
        ProfileEdge::Arc {
            center: [-3. + offset, 0.],
            a: d,
            b: a,
            clockwise: false,
        },
    ]
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

#[test]
fn exact_stadium_follows_three_axes_and_shares_joint_trims() {
    let (shell, end_frame) = mixed_miter_with_end_frame(&stadium(0.), &CHAIN, TOL).unwrap();
    assert_eq!(end_frame, [[0., 0., -1.], [-1., 0., 0.]]);
    assert_eq!((shell.solid_count(), shell.shell_count()), (1, 1));
    assert_eq!((shell.faces().len(), shell.edges().len()), (20, 42));
    assert_eq!(shell.bounds_mm(), [[-4., -1., 0.], [21., 20., 24.]]);
    close(shell.volume_mm3(), 60. * AREA);
    close(
        shell.faces().iter().map(|face| face.area_mm2).sum(),
        2. * AREA + 60. * PERIMETER,
    );
    assert_eq!(
        (shell.faces()[0].source, shell.faces()[0].source_face),
        (0, 0)
    );
    assert_eq!(
        (shell.faces()[19].source, shell.faces()[19].source_face),
        (2, 1)
    );

    for joint in 1..3 {
        for index in 6 * joint..6 * (joint + 1) {
            let uses: Vec<_> = shell
                .faces()
                .iter()
                .flat_map(|face| {
                    face.boundary
                        .iter()
                        .filter(move |edge| edge.edge == index)
                        .map(move |edge| (face.source, edge.forward))
                })
                .collect();
            assert_eq!(uses, [(joint - 1, false), (joint, true)]);
        }
        let Curve::Ellipse(arc) = shell.edges()[6 * joint + 1].curve else {
            panic!("expected an exact ellipse at a miter");
        };
        close(arc.ellipse().semiaxes_mm()[0], std::f64::consts::SQRT_2);
        close(arc.ellipse().semiaxes_mm()[1], 1.);
        close(arc.sweep_rad(), FRAC_PI_2);
    }
    let mesh = shell
        .mesh(Tessellation {
            deflection: 0.002,
            angle: 1.,
            triangle_limit: 10_000,
        })
        .unwrap();
    let metrics = manufacturing::certify(&mesh).unwrap();
    assert!((metrics.signed_volume_mm3 / shell.volume_mm3() - 1.).abs() < 0.003);
}

#[test]
fn centroid_controls_volume_and_clockwise_authored_indices_are_preserved() {
    let mut profile = stadium(1.);
    profile.reverse();
    let profile = profile.map(ProfileEdge::reversed);
    let path = [[0.; 3], [0., 0., 10.], CHAIN[1], CHAIN[2]];
    let shell = mixed_miter(&profile, &path, TOL).unwrap();
    close(shell.volume_mm3(), 38. * AREA);
    close(
        shell.faces().iter().map(|face| face.area_mm2).sum(),
        2. * AREA + 38. * PERIMETER,
    );
    for (i, expected) in [5, 4, 4, 3, 2, 2].into_iter().enumerate() {
        let face = &shell.faces()[i + 1];
        assert_eq!(face.source_face, expected);
        assert_eq!(
            matches!(face.support, Support::Cylinder { outward: true, .. }),
            i % 3 != 0
        );
    }
}

#[test]
fn rejects_invalid_inputs_and_uncertified_sweep_clearance() {
    let profile = stadium(0.);
    assert_eq!(mixed_miter(&profile, &CHAIN[..2], TOL), Err(Error::Invalid));
    assert_eq!(mixed_miter(&profile, &CHAIN, 0.), Err(Error::Invalid));
    assert_eq!(
        mixed_miter(&profile, &[[0.; 3]; MAX_PATH_POINTS + 1], TOL),
        Err(Error::Limit)
    );
    assert_eq!(
        mixed_miter(&[profile[0]; MAX_RIM_EDGES + 1], &CHAIN, TOL),
        Err(Error::Limit)
    );
    assert_eq!(
        mixed_miter(&profile[..1], &CHAIN, TOL),
        Err(Error::Unsupported)
    );
    assert_eq!(mixed_miter(&profile[..2], &CHAIN, TOL), Err(Error::Invalid));
    assert_eq!(
        mixed_miter(&profile, &[[0.; 3], CHAIN[1], CHAIN[1]], TOL),
        Err(Error::Invalid)
    );
    assert_eq!(
        mixed_miter(&profile, &[[0.; 3], CHAIN[1], [0.; 3]], TOL),
        Err(Error::Unsupported)
    );
    assert_eq!(
        mixed_miter(&profile, &[[0.; 3], [0., 0., 2.], [20., 0., 2.]], TOL),
        Err(Error::Unsupported)
    );
    let returns_through_first_leg = [
        [0.; 3],
        [0., 0., 20.],
        [20., 0., 20.],
        [20., 0., 10.],
        [-20., 0., 10.],
    ];
    assert_eq!(
        mixed_miter(&profile, &returns_through_first_leg, TOL),
        Err(Error::Unsupported)
    );
}

#[test]
fn replay_accepts_piecewise_arc_only_profiles() {
    let a = [2., 0.];
    let b = [-2., 0.];
    let profile = [
        ProfileEdge::Arc {
            center: [0., 0.],
            a,
            b,
            clockwise: false,
        },
        ProfileEdge::Arc {
            center: [0., 0.],
            a: b,
            b: a,
            clockwise: false,
        },
    ];
    let shell = mixed_miter(&profile, &CHAIN[..3], TOL).unwrap();
    close(shell.volume_mm3(), 160. * PI);
    let rim = shell.faces()[0].boundary.len();
    assert_eq!(rim, 4);
    assert!(
        shell.edges()[..rim]
            .iter()
            .all(|edge| matches!(edge.curve, Curve::Ellipse(_)))
    );
}

#[test]
fn generator_clearance_checks_the_interior_of_an_arc() {
    let profile = [[1., 0.], [-1., 0.]].map(|a| ProfileEdge::Arc {
        center: [0., 0.],
        a,
        b: a.map(|v| -v),
        clockwise: false,
    });
    // On the first leg, the four quarter-arc vertices each have a positive
    // gap, while the curve point at 45 degrees already crosses the start cap.
    let path = [[0.; 3], [0., 0., 0.9], [20., 20., 0.9]];
    assert_eq!(mixed_miter(&profile, &path, TOL), Err(Error::Unsupported));
}

#[test]
fn refuses_geometry_outside_the_supported_coordinate_and_trim_budgets() {
    let profile = stadium(0.);
    let excessive = MAX_COORDINATE_MM * 2.;
    let path = [[0.; 3], [0., 0., excessive], [20., 0., excessive]];
    assert_eq!(mixed_miter(&profile, &path, TOL), Err(Error::Limit));

    let limit = MAX_COORDINATE_MM;
    let path = [[0.; 3], [0., 0., limit], [20., 0., limit]];
    assert_eq!(mixed_miter(&profile, &path, TOL), Err(Error::Limit));
    assert_eq!(mixed_miter(&stadium(1e8), &CHAIN, TOL), Err(Error::Limit));

    let mut profile = vec![ProfileEdge::Arc {
        center: [0., 0.],
        a: [1., 0.],
        b: [0., -1.],
        clockwise: false,
    }];
    let count = MAX_RIM_EDGES - 2;
    let point = |i| {
        let theta = FRAC_PI_2 * (i as f64 / count as f64 - 1.);
        [theta.cos(), theta.sin()]
    };
    for i in 0..count {
        profile.push(ProfileEdge::Arc {
            center: [0., 0.],
            a: point(i),
            b: point(i + 1),
            clockwise: false,
        });
    }
    assert_eq!(mixed_miter(&profile, &CHAIN, TOL), Err(Error::Limit));
}

#[test]
fn refuses_curves_or_generators_whose_endpoints_disagree() {
    let profile = stadium(0.);
    let rim = Rim::perpendicular(
        [0.; 3],
        CARTESIAN_AXES[0],
        CARTESIAN_AXES[1],
        CARTESIAN_AXES[2],
    );
    assert_eq!(
        rim.curve(profile[1], [[3., -1., 0.], [3., 1., 1.]], TOL),
        Err(Error::Unsupported)
    );
    let mut shell = mixed_miter(&profile, &CHAIN, TOL).unwrap();
    let (_, pieces) = certify_profile(&profile, TOL, true).unwrap();
    // Simulate a trim vertex displaced across the generator by rounding.
    shell.vertices[pieces.len()][0] += 2. * TOL;
    assert_eq!(
        append_leg(
            &mut shell,
            &pieces,
            [rim; 2],
            0,
            CARTESIAN_AXES[2],
            [0., 0., 20.],
            TOL
        ),
        Err(Error::Unsupported)
    );
}

#[test]
fn refuses_a_rim_edge_lost_to_coordinate_roundoff() {
    let profile = stadium(0.).map(|edge| {
        let shrink = |p: [f64; 2]| p.map(|v| v * 1e-6);
        match edge {
            ProfileEdge::Line { a, b } => ProfileEdge::Line {
                a: shrink(a),
                b: shrink(b),
            },
            ProfileEdge::Arc {
                center,
                a,
                b,
                clockwise,
            } => ProfileEdge::Arc {
                center: shrink(center),
                a: shrink(a),
                b: shrink(b),
                clockwise,
            },
            _ => unreachable!(),
        }
    });
    let far = 1e11;
    let path = [[0.; 3], [0., 0., far], [20., 0., far]];
    assert_eq!(mixed_miter(&profile, &path, TOL), Err(Error::Unsupported));
}

#[test]
fn replay_keeps_near_quarter_partitions_through_mesh_and_section() {
    let profile = |overshoot: f64| {
        let points =
            [0., FRAC_PI_2 + overshoot, PI, 3. * FRAC_PI_2].map(|theta| [theta.cos(), theta.sin()]);
        std::array::from_fn::<_, 4, _>(|i| ProfileEdge::Arc {
            center: [0., 0.],
            a: points[i],
            b: points[(i + 1) % 4],
            clockwise: false,
        })
    };
    let path = &CHAIN[..3];
    let shell = mixed_miter_from_rim(&profile(1e-9), path, TOL).unwrap();
    assert_eq!(shell.faces()[0].boundary.len(), 4);
    close(shell.volume_mm3(), 40. * PI);
    assert_eq!(
        mixed_miter(&profile(1e-9), path, TOL).unwrap().faces()[0]
            .boundary
            .len(),
        5
    );
    let mesh = shell
        .mesh(Tessellation {
            deflection: 0.002,
            angle: 1.,
            triangle_limit: 10_000,
        })
        .unwrap();
    assert!(manufacturing::certify(&mesh).is_ok());
    let section = shell
        .section([0., 0., 10.], CARTESIAN_AXES[2], TOL, 32)
        .unwrap();
    assert_eq!(section.len(), 1);
    assert_eq!(section[0].len(), 4);
    assert_eq!(
        mixed_miter_from_rim(&profile(1e-5), path, TOL),
        Err(Error::Unsupported)
    );
}

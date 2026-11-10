use super::*;
use crate::{display::Tessellation, normal_sweep::Piercing};
use std::f64::consts::PI;

const TOL: f64 = 1e-7;
const MIN: f64 = 1e-5;

fn line(a: [f64; 2], b: [f64; 2]) -> ProfileEdge {
    ProfileEdge::Line { a, b }
}

fn rectangle([x, y]: [f64; 2]) -> Vec<ProfileEdge> {
    let corners = [[0., 0.], [x, 0.], [x, y], [0., y]];
    (0..4)
        .map(|i| line(corners[i], corners[(i + 1) % 4]))
        .collect()
}

fn circle(center: [f64; 2], radius_mm: f64, clockwise: bool) -> ProfileEdge {
    ProfileEdge::Circle {
        center,
        radius_mm,
        clockwise,
    }
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() <= TOL, "{actual} != {expected}");
}

#[test]
fn exact_sweep_drawing_shell_keeps_stock_and_piercing_source_faces() {
    let stock = Sweep::new(
        vec![
            vec![
                line([0., 0.], [10., 0.]),
                line([10., 0.], [10., 7.]),
                ProfileEdge::Arc {
                    center: [9., 7.],
                    a: [10., 7.],
                    b: [9., 8.],
                    clockwise: false,
                },
                line([9., 8.], [0., 8.]),
                line([0., 8.], [0., 0.]),
            ],
            vec![circle([5., 4.], 1., true)],
        ],
        12.,
        TOL,
        MIN,
    )
    .unwrap();
    let opening = |x, axial, outline| Piercing {
        sides: [0, 3],
        origin: [x, 0., axial],
        u: [1., 0., 0.],
        v: [0., 0., 1.],
        through: [0., 8., 0.],
        outline,
    };
    let pierced = PiercedSweep::new(
        &stock,
        vec![
            opening(2., 3., rectangle([0.8, 0.6])),
            opening(7.5, 5., vec![circle([0.; 2], 0.4, false)]),
        ],
        TOL,
        MIN,
    )
    .unwrap();

    for reverse in [false, true] {
        let sign: f64 = if reverse { -1. } else { 1. };
        let bare = normal_sweep_shell(&stock, reverse, TOL).unwrap();
        let cut = pierced_sweep_shell(&pierced, reverse, TOL).unwrap();
        let stock_volume = 12. * (80. - (1. - PI / 4.) - PI);
        close(bare.volume_mm3(), stock_volume);
        close(
            cut.volume_mm3(),
            stock_volume - 8. * (0.8 * 0.6 + PI * 0.4_f64.powi(2)),
        );
        assert_eq!(cut.bounds_mm(), bare.bounds_mm());
        assert_eq!(cut.bounds_mm()[0], [0., 0., (sign * 12.).min(0.)]);
        assert_eq!(cut.bounds_mm()[1], [10., 8., (sign * 12.).max(0.)]);
        assert_eq!(cut.solid_count(), 1);
        assert_eq!(cut.shell_count(), 1);

        let face = |source| {
            cut.faces()
                .iter()
                .find(|face| face.source_face == source)
                .unwrap()
        };
        for (cap, normal) in [-sign, sign].into_iter().enumerate() {
            assert_eq!(
                face(cap).support,
                Support::Plane {
                    normal: [0., 0., normal]
                }
            );
            assert_eq!(face(cap).holes.len(), 1);
            assert_eq!(face(cap).holes[0].len(), 4);
        }
        for wall in [2, 5] {
            assert_eq!(face(wall).holes.len(), 2);
            assert!(face(wall).holes.iter().all(|ring| ring.len() == 4));
        }
        for wall in 8..12 {
            assert!(matches!(face(wall).support, Support::Plane { .. }));
        }
        for (wall, radius) in [(7, 1.), (12, 0.4)] {
            let pieces = cut.faces().iter().filter(|face| face.source_face == wall);
            assert_eq!(pieces.clone().count(), 4);
            for piece in pieces {
                let Support::Cylinder {
                    radius_mm,
                    outward: false,
                    ..
                } = piece.support
                else {
                    panic!("opening quarter must be an inward cylinder");
                };
                close(radius_mm, radius);
            }
        }

        let q = std::f64::consts::FRAC_1_SQRT_2;
        let view = [[1., 0., 0.], [0., q, q], [0., -q, q]];
        let elevation = cut.elevation([0.; 3], view, 4, TOL, false).unwrap();
        assert!(!elevation.blockers.is_empty());
        assert!(
            elevation
                .rims
                .iter()
                .any(|rim| (rim.ellipse.major - 0.4).abs() < TOL
                    && (rim.ellipse.minor - 0.4 * q).abs() < TOL
                    && rim.source == 4)
        );
        assert!(
            !cut.mesh(Tessellation {
                deflection: 0.01,
                angle: 0.1,
                triangle_limit: 10000,
            })
            .unwrap()
            .indices
            .is_empty()
        );
    }
}

#[test]
fn refuses_uncertified_arc_charts_invalid_tolerance_and_excess_stitching() {
    let arc = |center, a, b| ProfileEdge::Arc {
        center,
        a,
        b,
        clockwise: false,
    };
    let stadium = Sweep::new(
        vec![vec![
            line([0., 0.], [4., 0.]),
            arc([4., 1.], [4., 0.], [4., 2.]),
            line([4., 2.], [0., 2.]),
            arc([0., 1.], [0., 2.], [0., 0.]),
        ]],
        3.,
        TOL,
        MIN,
    )
    .unwrap();
    assert_eq!(
        normal_sweep_shell(&stadium, false, TOL),
        Err(Error::Unsupported)
    );
    assert_eq!(
        normal_sweep_shell(&stadium, false, f64::NAN),
        Err(Error::Invalid)
    );

    let many = std::iter::once(rectangle([30., 30.]))
        .chain((1..=27).flat_map(|x| {
            (1..=23).map(move |y| vec![circle([f64::from(x), f64::from(y)], 0.2, true)])
        }))
        .collect();
    let stock = Sweep::new(many, 3., TOL, MIN).unwrap();
    assert_eq!(normal_sweep_shell(&stock, false, TOL), Err(Error::Limit));
}

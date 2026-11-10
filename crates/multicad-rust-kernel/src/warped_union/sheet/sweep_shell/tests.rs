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

fn arc(center: [f64; 2], a: [f64; 2], b: [f64; 2]) -> ProfileEdge {
    ProfileEdge::Arc {
        center,
        a,
        b,
        clockwise: false,
    }
}

fn pierced_stock(profile: Vec<ProfileEdge>, sides: [usize; 2], origin: [f64; 3]) -> PiercedSweep {
    let stock = Sweep::new(vec![profile], 3., TOL, MIN).unwrap();
    PiercedSweep::new(
        &stock,
        vec![Piercing {
            sides,
            origin,
            u: [1., 0., 0.],
            v: [0., 0., 1.],
            through: [0., 2., 0.],
            outline: rectangle([0.8, 0.6]),
        }],
        TOL,
        MIN,
    )
    .unwrap()
}

fn verify_host_faces(shell: &Shell, sources: [usize; 2]) {
    for source in sources {
        let [host] = shell
            .faces()
            .iter()
            .filter(|face| face.source_face == source)
            .collect::<Vec<_>>()[..]
        else {
            panic!("expected one authored straight host");
        };
        assert!(matches!(host.support, Support::Plane { .. }));
        assert_eq!(host.holes.len(), 1);
        assert_eq!(host.holes[0].len(), 4);
    }
    for source in 6..10 {
        let [cut] = shell
            .faces()
            .iter()
            .filter(|face| face.source_face == source)
            .collect::<Vec<_>>()[..]
        else {
            panic!("expected one authored aperture wall");
        };
        assert!(matches!(cut.support, Support::Plane { .. }));
        assert!(cut.holes.is_empty());
    }
}

fn verify_arc_faces(shell: &Shell, source: usize, radius: f64, expected: f64, parts: usize) {
    let faces = shell
        .faces()
        .iter()
        .filter(|face| face.source_face == source);
    assert_eq!(faces.clone().count(), parts);
    let mut swept_area = 0.;
    for face in faces {
        let Support::Cylinder {
            radius_mm,
            outward: true,
            ..
        } = face.support
        else {
            panic!("expected outward cylinder");
        };
        close(radius_mm, radius);
        let [arc, _, _, _] = face.boundary[..] else {
            panic!("expected one regular sector");
        };
        let Curve::Ellipse(trim) = shell.edges()[arc.edge].curve else {
            panic!("expected circular cap trim");
        };
        close(trim.ellipse().semiaxes_mm()[0], radius);
        close(trim.ellipse().semiaxes_mm()[1], radius);
        assert!(trim.sweep_rad().abs() <= FRAC_PI_2 + ANGULAR);
        swept_area += face.area_mm2;
    }
    close(swept_area, 3. * radius * expected);
}

fn verify_oblique_rim(shell: &Shell, [cx, cy]: [f64; 2], radius: f64, expected: f64) {
    let q = std::f64::consts::FRAC_1_SQRT_2;
    let view = [[1., 0., 0.], [0., q, q], [0., -q, q]];
    let elevation = shell.elevation([0.; 3], view, 4, TOL, false).unwrap();
    let sweep = elevation
        .rims
        .iter()
        .filter_map(|rim| {
            let [x, y] = rim.ellipse.center;
            if (x - cx).abs() > TOL || (y - cy * q).abs() > TOL || (rim.depth + cy * q).abs() > TOL
            {
                return None;
            }
            close(rim.ellipse.major, radius);
            close(rim.ellipse.minor, radius * q);
            close(rim.depth_sine.abs(), radius * q);
            Some(rim.ellipse.trim[1] - rim.ellipse.trim[0])
        })
        .sum();
    close(sweep, expected);
}

#[test]
fn subdivided_semicircles_preserve_authored_piercings_and_exact_silhouettes() {
    let pierced = pierced_stock(
        vec![
            line([0., 0.], [4., 0.]),
            arc([4., 1.], [4., 0.], [4., 2.]),
            line([4., 2.], [0., 2.]),
            arc([0., 1.], [0., 2.], [0., 0.]),
        ],
        [0, 2],
        [2., 0., 1.],
    );
    for reverse in [false, true] {
        let shell = pierced_sweep_shell(&pierced, reverse, TOL).unwrap();
        close(shell.volume_mm3(), 3. * (8. + PI) - 2. * 0.8 * 0.6);
        verify_host_faces(&shell, [2, 4]);
        verify_arc_faces(&shell, 3, 1., PI, 2);
        verify_arc_faces(&shell, 5, 1., PI, 2);
        verify_oblique_rim(&shell, [4., 1.], 1., PI);

        let z = if reverse { -3. } else { 3. };
        let matching = |axes: [Point; 3], rail: [Point; 2]| {
            let elevation = shell.elevation([0.; 3], axes, 4, TOL, true).unwrap();
            elevation
                .lines
                .iter()
                .filter(|line| [rail, [rail[1], rail[0]]].contains(&line.points))
                .count()
        };
        let x = [1., 0., 0.];
        let y = [0., 1., 0.];
        assert_eq!(
            matching([x, y, [0., 0., 1.]], [[5., 1., 0.], [5., 1., z]]),
            0
        );
        assert_eq!(
            matching([x, [0., 0., -1.], y], [[5., 0., 1.], [5., -z, 1.]]),
            1
        );
    }
}

#[test]
fn a_three_quarter_arc_keeps_both_later_straight_hosts() {
    let pierced = pierced_stock(
        vec![
            arc([0.; 2], [2., 0.], [0., -2.]),
            line([0., -2.], [6., -2.]),
            line([6., -2.], [6., 0.]),
            line([6., 0.], [2., 0.]),
        ],
        [1, 3],
        [3., -2., 1.],
    );
    let shell = pierced_sweep_shell(&pierced, false, TOL).unwrap();
    close(shell.volume_mm3(), 3. * (12. + 3. * PI) - 2. * 0.8 * 0.6);
    verify_host_faces(&shell, [3, 5]);
    verify_arc_faces(&shell, 2, 2., 1.5 * PI, 3);
    verify_oblique_rim(&shell, [0.; 2], 2., 1.5 * PI);
}

#[test]
fn a_complete_outer_circle_has_four_outward_patches_and_two_exact_rims() {
    let center = [2., 5.];
    let stock = Sweep::new(vec![vec![circle(center, 1.5, false)]], 3., TOL, MIN).unwrap();
    let shell = normal_sweep_shell(&stock, false, TOL).unwrap();
    assert_eq!(shell.faces().len(), 6);
    close(shell.volume_mm3(), 3. * PI * 1.5_f64.powi(2));
    verify_arc_faces(&shell, 2, 1.5, 2. * PI, 4);
    verify_oblique_rim(&shell, center, 1.5, 2. * PI);
}

#[test]
fn refuses_invalid_tolerance_and_excess_stitching() {
    let stock = Sweep::new(vec![rectangle([10., 10.])], 3., TOL, MIN).unwrap();
    assert_eq!(
        normal_sweep_shell(&stock, false, f64::NAN),
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

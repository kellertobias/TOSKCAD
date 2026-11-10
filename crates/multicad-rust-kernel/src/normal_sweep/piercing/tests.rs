use super::*;
use crate::{
    display::{MeshError, Tessellation, pierced_normal_sweep},
    manufacturing::certify as certify_mesh,
};
use std::f64::consts::PI;

const TOLERANCE: f64 = 1e-7;
const MINIMUM: f64 = 1e-5;
const OPTIONS: Tessellation = Tessellation {
    deflection: 0.001,
    angle: 0.15,
    triangle_limit: 100_000,
};

fn line(a: Point, b: Point) -> Edge {
    Edge::Line { a, b }
}

fn arc(center: Point, a: Point, b: Point, clockwise: bool) -> Edge {
    Edge::Arc {
        center,
        a,
        b,
        clockwise,
    }
}

fn circle(center: Point, radius_mm: f64, clockwise: bool) -> Vec<Edge> {
    vec![Edge::Circle {
        center,
        radius_mm,
        clockwise,
    }]
}

fn polygon(points: &[Point]) -> Vec<Edge> {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(&a, &b)| line(a, b))
        .collect()
}

fn rectangle([x, y]: Point) -> Vec<Edge> {
    polygon(&[[-x, -y], [x, -y], [x, y], [-x, y]])
}

fn channel() -> Sweep {
    let [a, b, c, d, e, f, g, h, i, j, k, l] = [
        [-1.5, 0.],
        [1.5, -3.],
        [41.5, -3.],
        [41.5, -1.5],
        [1.5, -1.5],
        [0., 0.],
        [0., 80.],
        [1.5, 81.5],
        [41.5, 81.5],
        [41.5, 83.],
        [1.5, 83.],
        [-1.5, 80.],
    ];
    let ring = vec![
        arc([1.5, 0.], a, b, false),
        line(b, c),
        line(c, d),
        line(d, e),
        arc([1.5, 0.], e, f, true),
        line(f, g),
        arc([1.5, 80.], g, h, true),
        line(h, i),
        line(i, j),
        line(j, k),
        arc([1.5, 80.], k, l, false),
        line(l, a),
    ];
    Sweep::new(vec![ring], 260., TOLERANCE, MINIMUM).unwrap()
}

fn web(along: f64, height: f64, outline: Vec<Edge>) -> Piercing {
    Piercing {
        sides: [11, 5],
        origin: [-1.5, height, along],
        u: [0., 0., 1.],
        v: [0., 1., 0.],
        through: [1.5, 0., 0.],
        outline,
    }
}

fn flange(along: f64, upper: bool) -> Piercing {
    Piercing {
        sides: if upper { [9, 7] } else { [1, 3] },
        origin: [21.5, if upper { 83. } else { -3. }, along],
        u: [0., 0., 1.],
        v: [1., 0., 0.],
        through: [0., if upper { -1.5 } else { 1.5 }, 0.],
        outline: circle([0.; 2], 1.7, false),
    }
}

fn pierce(sweep: &Sweep, openings: Vec<Piercing>) -> Result<PiercedSweep, Error> {
    PiercedSweep::new(sweep, openings, TOLERANCE, MINIMUM)
}

fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected} ± {tolerance}"
    );
}

#[test]
fn shipped_channel_positions_keep_one_mesh_with_all_fifteen_openings() {
    let sweep = channel();
    let mut cuts = vec![web(70., 40., rectangle([36., 12.5]))];
    for x in [150., 185., 220.] {
        for y in [25., 55.] {
            cuts.push(web(x, y, circle([0.; 2], 6.25, false)));
        }
    }
    for x in [32.5, 107.5] {
        for y in [24.5, 55.5] {
            cuts.push(web(x, y, circle([0.; 2], 1.7, false)));
        }
    }
    for along in [8., 252.] {
        for upper in [false, true] {
            cuts.push(flange(along, upper));
        }
    }
    let solid = pierce(&sweep, cuts).unwrap();
    assert_eq!(solid.openings().len(), 15);
    let window = &solid.openings()[0];
    near(window.area_mm2(), 1800., 1e-10);
    near(window.thickness_mm(), 1.5, 1e-10);
    assert_eq!(window.centroid(), [0.; 2]);
    assert_eq!(window.point(window.centroid(), true), [0., 40., 70.]);
    assert_eq!(solid.sweep(), &sweep);
    let area = 1800. + PI * (6. * 6.25_f64.powi(2) + 8. * 1.7_f64.powi(2));
    near(solid.volume_mm3(), sweep.volume_mm3() - 1.5 * area, 1e-8);

    let mesh = pierced_normal_sweep(&solid, OPTIONS).unwrap();
    let proof = certify_mesh(&mesh).unwrap();
    near(proof.signed_volume_mm3, solid.volume_mm3(), 3.);
    assert_eq!(mesh.face_indices.iter().copied().max(), Some(31));
    for face in 0..=31 {
        assert!(mesh.face_indices.contains(&face));
    }
}

#[test]
fn overlapping_rims_and_cuts_that_reach_the_bends_or_end_caps_are_refused() {
    let sweep = channel();
    let first = web(40., 40., circle([0.; 2], 1., false));
    let diagonal = web(41.6, 41.6, circle([0.; 2], 1., false));
    assert!(pierce(&sweep, vec![first.clone(), diagonal]).is_ok());
    for other in [
        web(41., 40., circle([0.; 2], 1., false)),
        web(40., 40., circle([0.; 2], 0.5, false)),
        web(1., 40., circle([0.; 2], 1., false)),
        web(40., 1., circle([0.; 2], 1., false)),
    ] {
        assert_eq!(
            pierce(&sweep, vec![first.clone(), other]),
            Err(Error::InvalidProfile)
        );
    }
    let reversed = Piercing {
        sides: [5, 11],
        origin: [0., 40., 42.],
        through: [-1.5, 0., 0.],
        ..first.clone()
    };
    assert_eq!(
        pierce(&sweep, vec![first, reversed]),
        Err(Error::InvalidProfile)
    );
}

#[test]
fn material_continuity_is_proved_across_the_entire_opening_width() {
    let sweep = channel();
    let crosses_void = Piercing {
        sides: [1, 9],
        through: [0., 86., 0.],
        ..flange(10., false)
    };
    assert_eq!(
        pierce(&sweep, vec![crosses_void]),
        Err(Error::InvalidProfile)
    );
    let ring = polygon(&[[0., 0.], [10., 0.], [10., 4.], [0., 4.]]);
    let opening = Piercing {
        sides: [0, 2],
        origin: [5., 0., 5.],
        u: [1., 0., 0.],
        v: [0., 0., 1.],
        through: [0., 4., 0.],
        outline: rectangle([1., 1.]),
    };
    for (center, radius) in [([5., 1.], 0.25), ([6.5, 1.], 0.75)] {
        let sweep = Sweep::new(
            vec![ring.clone(), circle(center, radius, true)],
            10.,
            TOLERANCE,
            MINIMUM,
        )
        .unwrap();
        assert_eq!(
            pierce(&sweep, vec![opening.clone()]),
            Err(Error::InvalidProfile)
        );
    }
    let solid = Sweep::new(vec![ring], 10., TOLERANCE, MINIMUM).unwrap();
    let other_pair = Piercing {
        sides: [3, 1],
        origin: [0., 2., 5.],
        u: [0., 1., 0.],
        through: [10., 0., 0.],
        ..opening.clone()
    };
    assert_eq!(
        pierce(&solid, vec![opening, other_pair]),
        Err(Error::InvalidProfile)
    );
}

#[test]
fn invalid_targets_frames_outline_and_mesh_budgets_are_rejected() {
    let sweep = channel();
    let opening = web(40., 40., rectangle([1., 1.]));
    assert_eq!(pierce(&sweep, vec![]), Err(Error::InvalidProfile));
    assert_eq!(
        pierce(&sweep, vec![opening.clone(); MAX_VERTICES + 1]),
        Err(Error::Limit)
    );
    for invalid in [
        Piercing {
            sides: [0, 5],
            ..opening.clone()
        },
        Piercing {
            sides: [200, 5],
            ..opening.clone()
        },
        Piercing {
            sides: [11, 7],
            ..opening.clone()
        },
        Piercing {
            through: [1.6, 0., 0.],
            ..opening.clone()
        },
        Piercing {
            origin: [-1.4, 40., 40.],
            ..opening.clone()
        },
        Piercing {
            origin: [-1.5, f64::NAN, 40.],
            ..opening.clone()
        },
        Piercing {
            u: [0., 0.1, 1.],
            ..opening.clone()
        },
        Piercing {
            outline: polygon(&[[0., 0.], [4., 0.], [3., 4.], [0., 4.]]),
            ..opening.clone()
        },
    ] {
        assert_eq!(pierce(&sweep, vec![invalid]), Err(Error::InvalidProfile));
    }
    let valid = pierce(&sweep, vec![opening]).unwrap();
    for limit in [16, 20, 100] {
        assert_eq!(
            pierced_normal_sweep(
                &valid,
                Tessellation {
                    triangle_limit: limit,
                    ..OPTIONS
                }
            ),
            Err(MeshError::BudgetExceeded)
        );
    }
    assert_eq!(
        pierced_normal_sweep(
            &valid,
            Tessellation {
                angle: 0.,
                ..OPTIONS
            }
        ),
        Err(MeshError::InvalidOptions)
    );
}

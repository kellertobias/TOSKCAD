use super::*;
use crate::{
    display::{MeshError, Tessellation, normal_sweep},
    manufacturing::certify,
};
use std::f64::consts::PI;

const TOLERANCE: f64 = 1e-7;
const MINIMUM: f64 = 1e-5;
const OPTIONS: Tessellation = Tessellation {
    deflection: 0.001,
    angle: 0.15,
    triangle_limit: 200_000,
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

fn circle(center: Point, radius_mm: f64, clockwise: bool) -> Edge {
    Edge::Circle {
        center,
        radius_mm,
        clockwise,
    }
}

fn polygon(points: &[Point]) -> Vec<Edge> {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(&a, &b)| line(a, b))
        .collect()
}

fn square(side: f64) -> Vec<Edge> {
    polygon(&[[0., 0.], [side, 0.], [side, side], [0., side]])
}

fn sweep(rings: Vec<Vec<Edge>>, height: f64) -> Result<Sweep, Error> {
    Sweep::new(rings, height, TOLERANCE, MINIMUM)
}

fn near(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} != {b} ± {tolerance}");
}

fn watertight(sweep: &Sweep, options: Tessellation) {
    let mesh = normal_sweep(sweep, options).unwrap();
    let proof = certify(&mesh).unwrap();
    let arc_area: f64 = sweep
        .rings()
        .iter()
        .flatten()
        .filter(|edge| edge.radius_mm().is_some())
        .map(|edge| edge.length_mm() * sweep.height_mm())
        .sum();
    near(
        proof.signed_volume_mm3,
        sweep.volume_mm3(),
        arc_area * options.deflection + 1e-8,
    );
    assert_eq!(
        mesh.face_indices.iter().copied().max(),
        Some(1 + sweep.rings().iter().map(Vec::len).sum::<usize>() as u32)
    );
}

#[test]
fn shipped_toolcase_quarter_arc_profiles_keep_thin_lips_and_exact_volumes() {
    for ([x, y], radius, height, native_volume) in [
        ([32., 68.25], 4., 214., 820.931_688_527_134_3),
        ([21., 252.5], 2., 109.5, 116.169_354_431_920_11),
    ] {
        let [a, b, c, d, e] = [
            [x - radius, y],
            [x - radius - 0.05, y],
            [x - radius - 0.05, y - radius - 0.05],
            [x, y - radius - 0.05],
            [x, y - radius],
        ];
        let ring = vec![
            line(a, b),
            line(b, c),
            line(c, d),
            line(d, e),
            arc([x, y], e, a, true),
        ];
        let solid = sweep(vec![ring], height).unwrap();
        near(solid.volume_mm3(), native_volume, 1e-8);
        near(
            solid.area_mm2(),
            (radius + 0.05).powi(2) - PI * radius.powi(2) / 4.,
            1e-10,
        );
        near(solid.support([-1., 0., 0.]), -b[0], 1e-10);
        near(solid.support([0., 0., 2.]), 2. * height, 1e-10);
        assert!(solid.contains([x - radius - 0.025, y - radius, height / 2.], TOLERANCE));
        assert!(!solid.contains([x - 0.01, y - 0.01, height / 2.], TOLERANCE));
        assert_eq!(solid.height_mm(), height);
        watertight(&solid, OPTIONS);
    }
}

#[test]
fn one_folded_channel_sweep_preserves_four_curved_bend_faces() {
    let (height, width, thickness, r, leg) = (80., 260., 1.5, 1.5, 40.);
    let outer = r + thickness;
    let reach = r + leg;
    let [a, b, c, d, e, f, g, h, i, j, k, l] = [
        [-thickness, 0.],
        [r, -outer],
        [reach, -outer],
        [reach, -r],
        [r, -r],
        [0., 0.],
        [0., height],
        [r, height + r],
        [reach, height + r],
        [reach, height + outer],
        [r, height + outer],
        [-thickness, height],
    ];
    let ring = vec![
        arc([r, 0.], a, b, false),
        line(b, c),
        line(c, d),
        line(d, e),
        arc([r, 0.], e, f, true),
        line(f, g),
        arc([r, height], g, h, true),
        line(h, i),
        line(i, j),
        line(j, k),
        arc([r, height], k, l, false),
        line(l, a),
    ];
    let solid = sweep(vec![ring], width).unwrap();
    let section_area =
        thickness * height + 2. * (thickness * leg + PI / 4. * (outer * outer - r * r));
    near(solid.area_mm2(), section_area, 1e-10);
    near(solid.centroid()[1], height / 2., 1e-10);
    near(solid.support([0., 1., 0.]), height + outer, 1e-10);
    near(solid.support([1., 0., 0.]), reach, 1e-10);
    assert!(solid.contains([-0.5, height / 2., 20.], TOLERANCE));
    assert!(solid.contains([r + 10., height + r + 0.5, 20.], TOLERANCE));
    assert!(!solid.contains([r, height / 2., 20.], TOLERANCE));
    watertight(&solid, OPTIONS);
}

#[test]
fn sector_and_circle_hole_keep_winding_area_and_measurement_frames() {
    let [a, b, c] = [[0., 0.], [10., 0.], [0., 10.]];
    let outline = vec![line(a, b), arc(a, b, c, false), line(c, a)];
    let hole = circle([2., 2.], 1., true);
    assert_eq!(signed_area(&[]), 0.);
    near(signed_area(&outline), 25. * PI, 1e-10);
    near(signed_area(&[hole]), -PI, 1e-10);
    let solid = sweep(vec![outline.clone(), vec![hole]], 5.).unwrap();
    near(solid.area_mm2(), 24. * PI, 1e-10);
    let expected = (25. * 40. / (3. * PI) - 2.) / 24.;
    for axis in solid.centroid() {
        near(axis, expected, 1e-10);
    }
    let edge = outline[1];
    assert_eq!(edge.reversed().end(), b);
    assert_eq!(outline[0].reversed().centroid(), [5., 0.]);
    assert_eq!(outline[0].point(0.3), [3., 0.]);
    assert_eq!(hole.reversed().signed_sweep_rad(), Some(TAU));
    assert_eq!(hole.centroid(), [2., 2.]);
    for axis in edge.centroid() {
        near(axis, 20. / PI, 1e-10);
    }
    assert!(solid.contains([3., 2., 0.], TOLERANCE));
    assert!(!solid.contains([2., 2., 1.], TOLERANCE));
    assert!(solid.contains([1.5, 4., 1.], TOLERANCE));
    assert!(!solid.contains([9., 9., 1.], TOLERANCE));
    assert!(!solid.contains([3., 2., 6.], TOLERANCE));
    assert!(!solid.contains([0., 0., 0.], -1.));
    assert!(!solid.contains([f64::INFINITY, 2., 1.], TOLERANCE));
    assert_eq!(solid.support([0.; 3]), 0.);
    assert!(solid.support([f64::INFINITY, 0., 0.]).is_nan());
    watertight(&solid, OPTIONS);
}

#[test]
fn invalid_rings_heights_winding_and_envelopes_are_refused() {
    assert_eq!(sweep(vec![], 2.), Err(Error::InvalidProfile));
    assert_eq!(sweep(vec![vec![]], 2.), Err(Error::InvalidProfile));
    assert_eq!(sweep(vec![square(10.)], 0.), Err(Error::InvalidHeight));
    assert_eq!(
        sweep(vec![square(10.)], MAX_DIMENSION_MM + 1.),
        Err(Error::Limit)
    );
    assert_eq!(
        Sweep::new(vec![square(10.)], 2., TOLERANCE, TOLERANCE),
        Err(Error::InvalidProfile)
    );
    let [a, b, c, d] = [[0., 0.], [4., 0.], [4., 4.], [0., 4.]];
    for ring in [
        polygon(&[a, d, c, b]),
        polygon(&[a, c, d, b]),
        vec![line(a, b), line(c, d), line(d, a)],
        vec![line(a, b), circle(c, 2., false)],
        vec![line(a, a), line(a, b), line(b, a)],
        vec![circle(c, -2., false)],
        vec![
            arc(a, b, d, false),
            line(d, c),
            line(c, [4. + f64::NAN, 0.]),
        ],
        vec![arc(a, b, c, false), line(c, b)],
        vec![arc(a, b, b, false), line(b, b)],
    ] {
        assert_eq!(sweep(vec![ring], 2.), Err(Error::InvalidProfile));
    }
    assert_eq!(
        sweep(vec![square(MAX_DIMENSION_MM + 1.)], 2.),
        Err(Error::Limit)
    );
    assert_eq!(
        sweep(
            vec![vec![circle([0., MAX_DIMENSION_MM - 1.], 2., false)]],
            2.
        ),
        Err(Error::Limit)
    );
    assert_eq!(
        sweep(vec![vec![line(a, b); MAX_VERTICES + 1]], 2.),
        Err(Error::Limit)
    );
    for holes in [
        vec![circle([5., 5.], 2., false)],
        vec![circle([20., 5.], 2., true)],
        vec![circle([2., 2.], 2., true)],
        vec![circle([5., 5.], 3., true), circle([5., 5.], 1., true)],
        vec![circle([4., 5.], 2., true), circle([7., 5.], 2., true)],
    ] {
        let mut rings = vec![square(10.)];
        rings.extend(holes.into_iter().map(|hole| vec![hole]));
        assert_eq!(sweep(rings, 2.), Err(Error::InvalidProfile));
    }
    let tiny = polygon(&[
        [4e-7, 4e-7],
        [4e-7, 1.06e-5],
        [1.06e-5, 1.06e-5],
        [1.06e-5, 4e-7],
    ]);
    assert_eq!(
        sweep(vec![square(1.1e-5), tiny], 2.),
        Err(Error::InvalidProfile)
    );
}

#[test]
fn small_join_roundoff_is_canonical_and_chord_refinement_never_breaks_the_budget() {
    let [a, b, c] = [[0., 0.], [10., 0.], [0., 10.]];
    let outline = vec![
        line(a, [b[0], 1e-10]),
        arc(a, b, c, false),
        line([1e-10, c[1]], a),
    ];
    let sector = sweep(vec![outline], 1.).unwrap();
    assert_eq!(sector.rings()[0][0].end(), b);
    assert_eq!(sector.rings()[0][2].start(), c);
    let ring = sweep(
        vec![
            vec![circle([0., 0.], 10., false)],
            vec![circle([8.95, 0.5], 1., true)],
        ],
        1.,
    )
    .unwrap();
    let coarse = Tessellation {
        deflection: 1.,
        angle: 1.,
        ..OPTIONS
    };
    watertight(&ring, coarse);
    assert_eq!(
        normal_sweep(
            &ring,
            Tessellation {
                deflection: 1e6,
                ..coarse
            }
        ),
        Err(MeshError::UnresolvedBoundary)
    );
    assert_eq!(
        normal_sweep(
            &sector,
            Tessellation {
                triangle_limit: 100,
                ..OPTIONS
            }
        ),
        Err(MeshError::BudgetExceeded)
    );
}

use super::*;

const TOL: f64 = 1e-7;
const RECTANGLE: [[f64; 2]; 4] = [[-2., -1.], [2., -1.], [2., 1.], [-2., 1.]];

fn loft() -> NonplanarLoft {
    let (sin, cos) = std::f64::consts::FRAC_PI_6.sin_cos();
    let top = RECTANGLE.map(|[x, y]| [cos * x - sin * y + 1., sin * x + cos * y + 0.5]);
    NonplanarLoft::new([&RECTANGLE, &top], 6., TOL).unwrap()
}

fn evaluate(curve: LoftSectionCurve, t: f64) -> Point {
    match curve {
        LoftSectionCurve::Line([a, b]) => interpolate(a, b, t),
        LoftSectionCurve::Quadratic { poles, weights } => {
            let weighted = [(1. - t).powi(2), 2. * (1. - t) * t, t * t]
                .into_iter()
                .zip(weights)
                .map(|(b, w)| b * w)
                .collect::<Vec<_>>();
            let sum = weighted.iter().sum::<f64>();
            std::array::from_fn(|axis| {
                (0..3).map(|i| poles[i][axis] * weighted[i]).sum::<f64>() / sum
            })
        }
    }
}

fn near(a: Point, b: Point) {
    assert!(norm(sub(a, b)) < 1e-12, "{a:?} and {b:?}");
}

#[test]
fn oblique_quadratics_evaluate_on_the_exact_walls_and_close_at_shared_rails() {
    let loft = loft();
    let origin = [0., 0., 3.];
    let normal = [0., -0.6, 0.8];
    let curves = loft.section(origin, normal, TOL).unwrap();
    assert_eq!(curves.len(), 4);
    for (i, (wall, curve)) in loft.walls().zip(curves.iter().copied()).enumerate() {
        let LoftSectionCurve::Quadratic { weights, .. } = curve else {
            panic!("expected an exact curved cut on wall {i}");
        };
        assert!(weights.into_iter().all(|w| w > 0. && w <= 1.));
        let next = curves[(i + 1) % 4];
        assert!([0., 1.].into_iter().any(|a| {
            [0., 1.]
                .into_iter()
                .any(|b| norm(sub(evaluate(curve, a), evaluate(next, b))) < 1e-12)
        }));
        for j in 0..=20 {
            let p = evaluate(curve, j as f64 / 20.);
            assert!(dot(sub(p, origin), normal).abs() < 1e-12);
            // Recover the wall parameters independently from its ruling at
            // the resulting height, then test against the authored surface.
            let t = p[2] / 6.;
            let a = wall.point(0., t);
            let edge = sub(wall.point(1., t), a);
            let u = dot(sub(p, a), edge) / dot(edge, edge);
            assert!((-1e-12..=1. + 1e-12).contains(&u));
            near(wall.point(u, t), p);
        }
    }
}

#[test]
fn cap_and_parallel_sections_have_four_exact_lines_without_duplicate_rulings() {
    let loft = loft();
    for (origin, normal) in [([0., 0., 0.], [0., 0., 1.]), ([0., 0., 3.], [0., 0., -1.])] {
        let curves = loft.section(origin, normal, TOL).unwrap();
        assert_eq!(curves.len(), 4);
        for (wall, curve) in loft.walls().zip(curves) {
            let LoftSectionCurve::Line([a, b]) = curve else {
                panic!("expected a planar ruling");
            };
            near(a, wall.point(0., origin[2] / 6.));
            near(b, wall.point(1., origin[2] / 6.));
        }
    }
    assert!(
        loft.section([0., 0., 7.], [0., 0., 1.], TOL)
            .unwrap()
            .is_empty()
    );
    let straight = NonplanarLoft::new([&RECTANGLE; 2], 6., TOL).unwrap();
    let face = straight.section([0., 1., 0.], [0., 1., 0.], TOL).unwrap();
    assert_eq!(face.len(), 4);
    let tangent = [1. / 2_f64.sqrt(); 2];
    let tangent = straight
        .section([2., 1., 0.], [tangent[0], tangent[1], 0.], TOL)
        .unwrap();
    assert!(tangent.is_empty());
    let inside = straight.section([0., 0., 0.], [1., 0., 0.], TOL).unwrap();
    assert_eq!(inside.len(), 4);
    assert!(
        inside
            .iter()
            .all(|curve| matches!(curve, LoftSectionCurve::Line(_)))
    );
}

#[test]
fn cuts_with_cap_intersections_and_separate_rail_hits_have_closed_boundaries() {
    let loft = loft();
    for x in [-1.5, 0., 2., 2.5] {
        let curves = loft.section([x, 0., 0.], [1., 0., 0.], TOL).unwrap();
        assert!(curves.len() >= 3, "{x}: {curves:?}");
        let vertices: Vec<_> = curves
            .iter()
            .flat_map(|&c| [evaluate(c, 0.), evaluate(c, 1.)])
            .collect();
        for v in vertices.iter() {
            assert_eq!(
                vertices
                    .iter()
                    .filter(|&&p| norm(sub(*v, p)) < 1e-12)
                    .count(),
                2
            );
            assert!((v[0] - x).abs() < 1e-12);
        }
    }
    assert_eq!(loft.section([0.; 3], [0.; 3], TOL), Err(Error::Invalid));
    assert_eq!(
        loft.section([f64::NAN, 0., 0.], [0., 0., 1.], TOL),
        Err(Error::Invalid)
    );
    assert_eq!(loft.section([0.; 3], [0., 0., 1.], 0.), Err(Error::Invalid));
}

#[test]
fn saddle_cut_through_a_ruling_also_keeps_the_crossing_rail_branch() {
    let wall = LoftWall {
        start: [[0., 0., -1.], [2., 0., 1.]],
        end: [[0., 1., 1.], [2., 1., -1.]],
    };
    let mut curves = Vec::new();
    wall.section([[-1., 1.], [1., -1.]], TOL, &mut curves);
    assert_eq!(
        curves,
        [
            LoftSectionCurve::Line([[0., 0.5, 0.], [2., 0.5, 0.]]),
            LoftSectionCurve::Line([[1., 0., 0.], [1., 0.5, 0.]]),
            LoftSectionCurve::Line([[1., 0.5, 0.], [1., 1., 0.]]),
        ]
    );
}

#[test]
fn one_section_can_have_both_planar_and_twisted_wall_intersections() {
    let top = [[-1., -1.], [2., -1.], [3., 2.], [-1., 2.]];
    let loft = NonplanarLoft::new([&RECTANGLE, &top], 6., TOL).unwrap();
    let curves = loft.section([0., 0., 3.], [0., -0.6, 0.8], TOL).unwrap();
    assert!(matches!(
        curves.as_slice(),
        [
            LoftSectionCurve::Line(_),
            LoftSectionCurve::Quadratic { .. },
            LoftSectionCurve::Line(_),
            LoftSectionCurve::Line(_),
        ]
    ));
}

use super::*;

const X: Point = [1., 0., 0.];
const Y: Point = [0., 1., 0.];

fn sweep(direction: Point, laws: [Travel; 2]) -> TrimmedSweep {
    TrimmedSweep::new(
        vec![vec![[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]]],
        direction,
        laws,
        1e-6,
    )
    .unwrap()
}

fn evaluate(plane: [Rational; 4], point: Point) -> Rational {
    plane[..3]
        .iter()
        .zip(point)
        .fold(Rational::new(0, 1).unwrap(), |sum, (&n, x)| {
            sum.checked_add(n.checked_mul(binary_fraction(x).unwrap()).unwrap())
                .unwrap()
        })
        .checked_sub(plane[3])
        .unwrap()
}

#[test]
fn exact_binary_admission_preserves_values_and_refuses_precision_exhaustion() {
    for (value, expected) in [
        (0., (0, 1)),
        (-0., (0, 1)),
        (1., (1, 1)),
        (-5., (-5, 1)),
        (1.5, (3, 2)),
        (0.1, (3602879701896397, 36028797018963968)),
        (2_f64.powi(40), (1_i64 << 40, 1)),
        (2_f64.powi(-59), (1, 1_i64 << 59)),
        (1e18, (1_000_000_000_000_000_000, 1)),
    ] {
        assert_eq!(binary_fraction(value).unwrap().ratio(), expected);
    }
    for value in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        2_f64.powi(-60),
        2_f64.powi(62),
        2_f64.powi(63) + 2048.,
        1e18_f64.next_up(),
    ] {
        assert!(binary_fraction(value).is_none(), "{value:?}");
    }
}

#[test]
fn source_planes_match_cap_points_and_material_sides_in_every_cardinal_chart() {
    let source = sweep([0., 0., 1.], [[0.5, 0.25, -0.5], [5., 0.25, 0.5]]);
    let axes = [
        X,
        Y,
        [0., 0., 1.],
        [-1., 0., 0.],
        [0., -1., 0.],
        [0., 0., -1.],
    ];
    let origin = [8., -16., 0.5];
    let mut charts = 0;
    for u in axes {
        for v in axes {
            if dot(u, v) != 0. {
                continue;
            }
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let world = |p: Point| {
                std::array::from_fn(|i| origin[i] + u[i] * p[0] + v[i] * p[1] + n[i] * p[2])
            };
            let planes = source.exact_cap_halfspaces(origin, u, v).unwrap();
            for (end, plane) in planes.into_iter().enumerate() {
                for uv in [[-1., -1.], [1., 1.], [0., 0.]] {
                    assert_eq!(
                        evaluate(plane, world(source.point(end == 1, uv))),
                        Rational::new(0, 1).unwrap()
                    );
                }
                assert!(evaluate(plane, world([0., 0., 2.])) < Rational::new(0, 1).unwrap());
                let outside = if end == 0 { [0., 0., 0.] } else { [0., 0., 6.] };
                assert!(evaluate(plane, world(outside)) > Rational::new(0, 1).unwrap());
            }
            charts += 1;
        }
    }
    assert_eq!(charts, 24);
}

#[test]
fn oblique_coefficients_do_not_refit_the_rounded_endpoint() {
    let source = sweep([0.6, 0., 0.8], [[0.; 3], [5., 0., 0.]]);
    let planes = source.exact_cap_halfspaces([0.; 3], X, Y).unwrap();
    assert_eq!(source.point(true, [0.; 2])[2], 4.);
    assert_eq!(
        planes[1][3],
        Rational::new(18014398509481985, 4503599627370496).unwrap()
    );
    assert_ne!(planes[1][3], Rational::new(4, 1).unwrap());
    let other = sweep([0., 0., 1.], [[4., 0., 0.], [8., 0., 0.]])
        .exact_cap_halfspaces([0.; 3], X, Y)
        .unwrap();
    assert!(planes[1][3].checked_add(other[0][3]).unwrap() > Rational::new(0, 1).unwrap());
}

#[test]
fn optional_proof_refuses_noncardinal_charts_and_unrepresentable_source_values() {
    let source = sweep([0., 0., 1.], [[0.; 3], [5., 0., 0.]]);
    for (u, v) in [
        (X, X),
        ([0.8, 0.6, 0.], [-0.6, 0.8, 0.]),
        ([1., 1e-20, 0.], Y),
        ([f64::NAN, 0., 0.], Y),
    ] {
        assert!(source.exact_cap_halfspaces([0.; 3], u, v).is_none());
    }
    for value in [f64::NAN, f64::INFINITY, f64::MAX, f64::from_bits(1)] {
        assert!(source.exact_cap_halfspaces([value, 0., 0.], X, Y).is_none());
    }
    let direction = sweep([f64::from_bits(1), 0., 1.], [[0.; 3], [5., 0., 0.]]);
    assert!(direction.exact_cap_halfspaces([0.; 3], X, Y).is_none());
    let law = sweep([0., 0., 1.], [[0.; 3], [5., f64::from_bits(1), 0.]]);
    assert!(law.exact_cap_halfspaces([0.; 3], X, Y).is_none());
    let rounded = sweep([0.6, 0., 0.8], [[0.; 3], [5., -0.6, 0.]]);
    assert!(rounded.exact_cap_halfspaces([0.; 3], X, Y).is_none());
}

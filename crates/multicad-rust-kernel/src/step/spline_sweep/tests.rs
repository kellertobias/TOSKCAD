use super::*;
use crate::{
    normal_sweep::Edge as Analytic,
    spline_sweep::Cubic,
    step_read::{ImportProfile, read_analytic_compound, read_import_solid},
};
use std::f64::consts::FRAC_1_SQRT_2;

const X: Point = [1., 0., 0.];
const Y: Point = [0., 1., 0.];
const U: Point = [FRAC_1_SQRT_2, 0., FRAC_1_SQRT_2];
const ORIGIN: Point = [121., -22., 13.];
const TOL: f64 = 1e-7;

fn profile(bore: bool, gap: f64) -> Sweep {
    let x = 8.;
    let top = [7.5, 3. + 3_f64.sqrt() / 2.];
    let points = vec![
        [x + gap, 0.],
        [7., 0.],
        [6., -0.2],
        [3., -0.35],
        [1., 0.],
        [0., 0.],
    ];
    let cubic = Cubic::new(points, vec![-2., 0.5, 4.], true).unwrap();
    let line = |a, b| Edge::Analytic(Analytic::Line { a, b });
    Sweep::from_verified(
        vec![
            Edge::Cubic(cubic),
            line([x, 0.], [x, 3.]),
            Edge::Analytic(Analytic::Arc {
                center: [7., 3.],
                a: [x, 3.],
                b: top,
                clockwise: false,
            }),
            line(top, [0., 4.]),
            line([0., 4.], [0., 0.]),
        ],
        bore.then_some(Analytic::Circle {
            center: [4., 2.],
            radius_mm: 1.,
            clockwise: true,
        }),
        5.,
        TOL,
    )
    .unwrap()
}

fn imported(input: &str) -> (crate::step_read::Readback, Sweep) {
    let (shape, decoded) = read_import_solid(input, TOL).unwrap();
    let ImportProfile::SplineSweep { sweep } = decoded.profile else {
        panic!("the verified trim must decode to an exact spline sweep");
    };
    (shape, sweep)
}

fn cubic(sweep: &Sweep) -> &Cubic {
    sweep
        .outline()
        .iter()
        .find_map(|edge| {
            if let Edge::Cubic(cubic) = edge {
                Some(cubic)
            } else {
                None
            }
        })
        .unwrap()
}

fn near(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} != {b}");
}

#[test]
fn exact_step_keeps_pole_order_knots_and_approximation_free_endpoints() {
    let source = profile(true, 5e-11);
    let step = write_spline_sweep(&source, [0.; 3], X, Y, TOL).unwrap();
    let (shape, decoded) = imported(&step);
    let authored = cubic(&source);
    let imported = cubic(&decoded);
    assert!(imported.reversed());
    assert_eq!(imported.knots(), authored.knots());
    assert_eq!(imported.poles(), authored.poles());
    assert_eq!(shape.arcs.len(), 2);
    assert_eq!(shape.circles.len(), 2);
    assert_eq!(decoded.hole(), source.hole());
    near(shape.volume_mm3, source.volume_mm3(), 1e-8);
    // The curve endpoint stays distinct from the adjacent line vertex.
    assert_ne!(imported.endpoints()[1], decoded.outline()[1].point(0.));
}

#[test]
fn imported_curves_survive_transformed_reexport_and_compound_readback() {
    let normal = cross(U, Y);
    for bore in [false, true] {
        let source = profile(bore, 0.);
        let seed = write_spline_sweep(&source, [0.; 3], X, Y, TOL).unwrap();
        let (_, source) = imported(&seed);
        let step = write_spline_sweep(&source, ORIGIN, U, Y, TOL).unwrap();
        let (actual, decoded) = imported(&step);
        assert_eq!(
            actual.face_count,
            source.outline().len() + 2 + usize::from(bore)
        );
        assert_eq!(actual.circles.len(), 2 * usize::from(bore));
        assert_eq!(actual.arcs.len(), 2);
        assert_eq!(actual.splines.len(), 2);
        assert_eq!(actual.splines[0].knots(), cubic(&source).knots());
        assert_eq!(actual.splines[0].multiplicities(), [4, 2, 4]);
        assert!(cubic(&decoded).reversed());
        assert_eq!(cubic(&decoded).knots(), cubic(&source).knots());
        for (&[x, y], world) in cubic(&source)
            .poles()
            .iter()
            .zip(actual.splines[0].poles_mm())
        {
            for (i, value) in world.iter().enumerate() {
                near(*value, ORIGIN[i] + x * U[i] + y * Y[i], 1e-12);
            }
        }
        let expected: [Point; 2] = [false, true].map(|upper| {
            std::array::from_fn(|i| {
                let s = if upper { 1. } else { -1. };
                ORIGIN[i] + s * source.support([s * U[i], s * Y[i], s * normal[i]])
            })
        });
        for (actual, expected) in actual
            .bounds_mm
            .into_iter()
            .flatten()
            .zip(expected.into_iter().flatten())
        {
            near(actual, expected, 1e-10);
        }
        near(actual.volume_mm3, source.volume_mm3(), 1e-10);
    }
    let mut writer = Writer::new();
    writer
        .append_spline_sweep(&profile(false, 0.), ORIGIN, U, Y, TOL)
        .unwrap();
    writer
        .append_box([Rational::new(2, 1).unwrap(); 3], [0.; 3], X, Y)
        .unwrap();
    let compound = writer.finish().unwrap();
    let members = read_analytic_compound(&compound, TOL).unwrap();
    assert_eq!(members.len(), 2);
    assert_eq!(members[0].splines.len(), 2);
    assert!(members[1].splines.is_empty());
}

#[test]
fn refused_placement_does_not_change_accepted_members() {
    let sweep = profile(true, 5e-11);
    let mut writer = Writer::new();
    writer
        .append_spline_sweep(&sweep, ORIGIN, U, Y, TOL)
        .unwrap();
    for (origin, u, v, tolerance, error) in [
        (ORIGIN, U, U, TOL, StepError::InvalidPlacement),
        (ORIGIN, U, Y, f64::NAN, StepError::InvalidGeometry),
        ([1e10, 0., 0.], U, Y, TOL, StepError::Limit),
        (ORIGIN, [1. + 1e-11, 0., 0.], Y, 1e-10, StepError::Limit),
        (ORIGIN, U, Y, 1e-12, StepError::Limit),
    ] {
        assert_eq!(
            writer.append_spline_sweep(&sweep, origin, u, v, tolerance),
            Err(error)
        );
    }
    assert_eq!(
        writer.finish(),
        write_spline_sweep(&sweep, ORIGIN, U, Y, TOL)
    );
}

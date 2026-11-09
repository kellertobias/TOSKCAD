use multicad_rust_kernel::{
    Circle, Rational,
    polygon::PolygonPrism,
    step::{StepError, write_box, write_polygon_prism},
};
use std::collections::BTreeMap;

const ORIGIN: [f64; 3] = [0.; 3];
const X: [f64; 3] = [1., 0., 0.];
const Y: [f64; 3] = [0., 1., 0.];
const Z: [f64; 3] = [0., 0., 1.];

fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}

fn ring(points: &[[i64; 2]]) -> Vec<[Rational; 2]> {
    points.iter().map(|point| point.map(n)).collect()
}

fn entities(step: &str) -> impl Iterator<Item = (&str, &str)> {
    step.lines()
        .filter_map(|line| line.strip_prefix('#'))
        .map(|line| line.strip_suffix(';').unwrap().split_once('=').unwrap())
}

fn count(step: &str, kind: &str) -> usize {
    entities(step)
        .filter(|(_, body)| body.split_once('(').unwrap().0 == kind)
        .count()
}

fn closed_edges(step: &str, edge_count: usize) {
    let mut uses: BTreeMap<&str, [usize; 2]> = entities(step)
        .filter(|(_, body)| body.starts_with("EDGE_CURVE("))
        .map(|(id, _)| (id, [0, 0]))
        .collect();
    assert_eq!(uses.len(), edge_count);
    for (_, body) in entities(step) {
        if let Some(oriented) = body.strip_prefix("ORIENTED_EDGE('',*,*,#") {
            let (edge, sense) = oriented.split_once(',').unwrap();
            let orientation = match sense {
                ".F.)" => 0,
                ".T.)" => 1,
                _ => panic!("Invalid edge orientation {sense}"),
            };
            uses.get_mut(edge).unwrap()[orientation] += 1;
        }
    }
    assert!(uses.values().all(|&counts| counts == [1, 1]));
}

fn cartesian_points(step: &str) -> impl Iterator<Item = [f64; 3]> {
    entities(step).filter_map(|(_, body)| {
        let coordinates = body
            .strip_prefix("CARTESIAN_POINT('',(")?
            .strip_suffix("))")
            .unwrap();
        let coords: Vec<_> = coordinates
            .split(',')
            .map(|coordinate| coordinate.parse::<f64>().unwrap())
            .collect();
        Some(coords.try_into().unwrap())
    })
}

#[test]
fn box_step_is_deterministic_and_uses_a_shared_manifold_shell() {
    let size = [n(12), n(8), Rational::new(10, 3).unwrap()];
    let first = write_box(size, ORIGIN, X, Y).unwrap();
    let second = write_box(size, ORIGIN, X, Y).unwrap();
    assert_eq!(first, second);
    assert!(first.starts_with("ISO-10303-21;\nHEADER;\n"));
    assert!(first.ends_with("ENDSEC;\nEND-ISO-10303-21;\n"));
    assert!(first.contains("AUTOMOTIVE_DESIGN"));
    assert!(first.contains("SI_UNIT(.MILLI.,.METRE.)"));
    assert_eq!(count(&first, "MANIFOLD_SOLID_BREP"), 1);
    assert_eq!(count(&first, "ADVANCED_FACE"), 6);
    assert_eq!(count(&first, "FACE_BOUND"), 0);
    assert_eq!(count(&first, "PLANE"), 6);
    assert_eq!(count(&first, "LINE"), 12);
    assert!(cartesian_points(&first).any(|point| point == [12., 8., 10. / 3.]));
    closed_edges(&first, 12);
}

#[test]
fn placed_signed_concave_prism_retains_its_separate_cap_hole_loops() {
    let prism = PolygonPrism::new(
        vec![
            ring(&[[0, 0], [8, 0], [8, 3], [4, 3], [4, 7], [0, 7]]),
            ring(&[[1, 1], [1, 2], [2, 2], [2, 1]]),
        ],
        n(3),
    )
    .unwrap();
    // XZ profile normal is negative Y. A reverse sweep goes toward positive Y.
    let origin = [10., 20., 30.];
    let step = write_polygon_prism(&prism, origin, X, Z, true).unwrap();
    assert_eq!(count(&step, "ADVANCED_FACE"), 12);
    assert_eq!(count(&step, "FACE_OUTER_BOUND"), 12);
    assert_eq!(count(&step, "FACE_BOUND"), 2);
    assert_eq!(count(&step, "EDGE_LOOP"), 14);
    assert_eq!(count(&step, "VERTEX_POINT"), 20);
    assert_eq!(count(&step, "CLOSED_SHELL"), 1);
    closed_edges(&step, 30);
    assert!(cartesian_points(&step).any(|point| point == [14., 23., 37.]));
    assert!(cartesian_points(&step).any(|point| point == [12., 23., 32.]));
}

#[test]
fn step_rejects_round_trims_and_invalid_or_degenerate_placements() {
    let rectangular = ring(&[[0, 0], [8, 0], [8, 8], [0, 8]]);
    let prism = PolygonPrism::with_circular_holes(
        vec![rectangular],
        vec![Circle::new([n(3), n(3)], n(1)).unwrap()],
        n(2),
    )
    .unwrap();
    assert_eq!(
        write_polygon_prism(&prism, ORIGIN, X, Y, false),
        Err(StepError::Unsupported)
    );
    assert_eq!(
        write_box([n(4), n(4), n(4)], ORIGIN, X, X),
        Err(StepError::InvalidPlacement)
    );
    assert_eq!(
        write_box([n(4), n(4), n(4)], [f64::NAN, 0., 0.], X, Y),
        Err(StepError::InvalidPlacement)
    );
    assert_eq!(
        write_box([n(4), n(4), n(4)], [1e100; 3], X, Y),
        Err(StepError::InvalidGeometry)
    );
    assert_eq!(
        write_box([n(4), n(0), n(4)], ORIGIN, X, Y),
        Err(StepError::InvalidGeometry)
    );
}

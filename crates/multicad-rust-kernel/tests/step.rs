use multicad_rust_kernel::{
    Circle, Rational,
    cylinder::AxialCylinder,
    polygon::PolygonPrism,
    prism::Prism,
    step::{
        StepError, write_axial_cylinder, write_box, write_placed_cylinder, write_polygon_prism,
        write_rectangular_prism,
    },
    step_read::read_analytic_solid,
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
fn rectangular_prism_keeps_circular_cuts_and_rejects_a_rounded_exterior() {
    let stock = Prism::rectangular(1, [n(10), n(8), n(3)]).unwrap();
    let bored = stock
        .subtract_cylinder(2, Circle::new([n(4), n(5)], n(2)).unwrap(), n(-1), n(4))
        .unwrap();
    let encoded = write_rectangular_prism(&bored, ORIGIN, X, Y).unwrap();
    assert_eq!(count(&encoded, "CYLINDRICAL_SURFACE"), 1);
    assert_eq!(count(&encoded, "CIRCLE"), 2);
    assert_eq!(count(&encoded, "ADVANCED_FACE"), 7);
    closed_edges(&encoded, 15);
    let rounded = Prism::extrude(1, n(10), n(4), n(3)).unwrap();
    assert_eq!(
        write_rectangular_prism(&rounded, ORIGIN, X, Y),
        Err(StepError::Unsupported)
    );
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
fn mixed_polygon_prism_exports_shared_round_holes_in_both_sweep_directions() {
    let prism = PolygonPrism::with_circular_holes(
        vec![
            ring(&[[0, 0], [12, 0], [12, 8], [0, 8]]),
            ring(&[[1, 1], [1, 2], [2, 2], [2, 1]]),
        ],
        vec![
            Circle::new([n(4), n(3)], Rational::new(5, 4).unwrap()).unwrap(),
            Circle::new([n(8), n(3)], n(2)).unwrap(),
        ],
        Rational::new(10, 3).unwrap(),
    )
    .unwrap();
    let origin = [10., 20., 30.];
    for reverse in [false, true] {
        let step = write_polygon_prism(&prism, origin, X, Z, reverse).unwrap();
        assert_eq!(count(&step, "ADVANCED_FACE"), 12);
        assert_eq!(count(&step, "FACE_BOUND"), 6);
        assert_eq!(count(&step, "CIRCLE"), 4);
        assert_eq!(count(&step, "CYLINDRICAL_SURFACE"), 2);
        assert_eq!(
            entities(&step)
                .filter(|(_, body)| body.starts_with("ADVANCED_FACE(") && body.ends_with(".F.)"))
                .count(),
            2
        );
        let end = 20. + if reverse { 10. / 3. } else { -10. / 3. };
        assert!(cartesian_points(&step).any(|point| point == [15.25, end, 33.]));
        closed_edges(&step, 30);
    }
    assert_eq!(
        write_polygon_prism(&prism, [1e100, 0., 0.], X, Z, false),
        Err(StepError::InvalidGeometry)
    );
}

#[test]
fn axial_cylinder_exports_two_fractional_rims_and_one_periodic_wall() {
    let cylinder = AxialCylinder::new(
        1,
        Circle::new([n(4), n(6)], Rational::new(5, 4).unwrap()).unwrap(),
        n(-3),
        Rational::new(10, 3).unwrap(),
    )
    .unwrap();
    let step = write_axial_cylinder(&cylinder, Y).unwrap();
    assert_eq!(step, write_axial_cylinder(&cylinder, Y).unwrap());
    assert_eq!(count(&step, "ADVANCED_FACE"), 3);
    assert_eq!(count(&step, "PLANE"), 2);
    assert_eq!(count(&step, "FACE_BOUND"), 0);
    assert_eq!(count(&step, "CIRCLE"), 2);
    assert_eq!(count(&step, "CYLINDRICAL_SURFACE"), 1);
    assert_eq!(count(&step, "LINE"), 1);
    assert_eq!(
        entities(&step)
            .filter(|(_, body)| body.starts_with("CIRCLE("))
            .filter(|(_, body)| body.ends_with(",1.2500000000000000E0)"))
            .count(),
        2
    );
    assert!(cartesian_points(&step).any(|point| point == [4., 7.25, -3.]));
    assert!(cartesian_points(&step).any(|point| point == [4., 7.25, 1. / 3.]));
    closed_edges(&step, 3);
    assert_eq!(
        write_axial_cylinder(&cylinder, Z),
        Err(StepError::InvalidPlacement)
    );
    assert_eq!(
        write_axial_cylinder(&cylinder, [1., 0., 1e-8]),
        Err(StepError::InvalidPlacement)
    );
}

#[test]
fn placed_cylinder_step_retains_tilted_rims_and_exact_projected_bounds() {
    let cylinder =
        AxialCylinder::new(1, Circle::new([n(4), n(6)], n(2)).unwrap(), n(-3), n(5)).unwrap();
    let encoded = write_placed_cylinder(&cylinder, [10., 20., 30.], X, [0., 0.8, 0.6]).unwrap();
    let read = read_analytic_solid(&encoded, 1e-8).unwrap();
    assert_eq!(read.face_count, 3);
    assert_eq!(read.circles.len(), 2);
    assert!((read.volume_mm3 - 20. * std::f64::consts::PI).abs() < 1e-8);
    for (actual, expected) in read
        .bounds_mm
        .into_iter()
        .flatten()
        .zip([8., 15.4, 28.8, 12., 21.6, 35.2])
    {
        assert!((actual - expected).abs() < 1e-8);
    }
    for center in [[10., 20., 30.], [10., 17., 34.]] {
        assert!(read.circles.iter().any(|rim| {
            let alignment = rim.normal[1] * -0.6 + rim.normal[2] * 0.8;
            rim.center_mm == center && (alignment.abs() - 1.).abs() < 1e-8 && rim.radius_mm == 2.
        }));
    }
}

#[test]
fn step_rejects_invalid_or_degenerate_placements() {
    let center = Circle::new([n(100_000_000_000_000_000), n(0)], n(1)).unwrap();
    let collapsed_rim = AxialCylinder::new(1, center, n(0), n(2)).unwrap();
    assert_eq!(
        write_axial_cylinder(&collapsed_rim, X),
        Err(StepError::InvalidGeometry)
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

#[test]
fn invalid_placed_circular_sweeps_refuse_nonpositive_or_overlapping_dimensions() {
    use multicad_rust_kernel::step::{StepError, write_placed_circular_sweep};
    for (outer, inner, height) in [
        (4., Some(4.), 3.),
        (4., Some(5.), 3.),
        (4., Some(f64::NAN), 3.),
        (0., None, 3.),
        (4., None, -1.),
    ] {
        assert_eq!(
            write_placed_circular_sweep(outer, inner, height, [0.; 3], [1., 0., 0.], [0., 1., 0.]),
            Err(StepError::InvalidGeometry),
        );
    }
}

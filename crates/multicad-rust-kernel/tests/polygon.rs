use multicad_rust_kernel::{
    Circle, Error, Rational,
    polygon::{PolygonError, PolygonPrism, signed_area},
    prism::Measure,
};
use std::f64::consts::PI;

fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}

fn ring(points: &[[i64; 2]]) -> Vec<[Rational; 2]> {
    points.iter().map(|point| point.map(n)).collect()
}

fn concave_outer() -> Vec<[Rational; 2]> {
    ring(&[[0, 0], [10, 0], [10, 4], [4, 4], [4, 10], [0, 10]])
}

fn measure(constant: Rational, pi_factor: i64) -> Measure {
    Measure {
        constant,
        pi_factor: n(pi_factor),
    }
}

fn circle(x: i64, y: i64, radius: i64) -> Circle {
    Circle::new([n(x), n(y)], n(radius)).unwrap()
}

#[test]
fn concave_extrusion_keeps_ring_order_and_exact_mass_properties() {
    let outer = concave_outer();
    let small = ring(&[[1, 1], [1, 2], [2, 2], [2, 1]]);
    let long = ring(&[[1, 6], [1, 8], [2, 8], [2, 6]]);
    assert_eq!(signed_area(&outer).unwrap(), n(64));
    assert_eq!(signed_area(&small).unwrap(), n(-1));
    assert_eq!(signed_area(&long).unwrap(), n(-2));
    let rings = vec![outer, small, long];
    let height = Rational::new(3, 2).unwrap();
    let prism = PolygonPrism::new(rings.clone(), height).unwrap();
    assert_eq!(prism.rings(), rings);
    assert_eq!(prism.height(), height);
    assert_eq!(prism.area(), measure(n(61), 0));
    assert_eq!(prism.volume(), measure(Rational::new(183, 2).unwrap(), 0));
    assert_eq!(prism.centroid(), [487. / 122., 465. / 122.]);

    let shift = [n(1_000_000_000_000), n(-1_000_000_000_000)];
    let translated = rings
        .into_iter()
        .map(|ring| {
            ring.into_iter()
                .map(|point| {
                    std::array::from_fn(|axis| point[axis].checked_add(shift[axis]).unwrap())
                })
                .collect()
        })
        .collect();
    let moved = PolygonPrism::new(translated, height).unwrap();
    assert_eq!(moved.area(), prism.area());
    assert_eq!(moved.volume(), prism.volume());
    for (axis, shifted) in shift.into_iter().enumerate() {
        assert!(
            (moved.centroid()[axis] - shifted.as_f64() - prism.centroid()[axis]).abs() < 0.00013
        );
    }
}

#[test]
fn validates_budget_winding_and_degenerate_or_crossing_edges() {
    assert_eq!(
        PolygonPrism::new(vec![], n(1)),
        Err(PolygonError::TooFewVertices)
    );
    assert_eq!(
        PolygonPrism::new(vec![concave_outer()], n(0)),
        Err(PolygonError::NonPositiveHeight)
    );
    assert_eq!(
        PolygonPrism::new(vec![vec![[n(0); 2]; 1025]], n(1)),
        Err(PolygonError::VertexLimit)
    );
    assert_eq!(
        signed_area(&ring(&[[0, 0], [1, 1]])),
        Err(PolygonError::TooFewVertices)
    );
    let cases = [
        (
            ring(&[[0, 0], [4, 0], [4, 0], [4, 4], [0, 4]]),
            PolygonError::DegenerateEdge,
        ),
        (
            ring(&[[0, 0], [4, 0], [2, 0], [4, 4], [0, 4]]),
            PolygonError::SelfIntersection,
        ),
        (
            ring(&[[0, 0], [4, 4], [0, 4], [4, 0]]),
            PolygonError::SelfIntersection,
        ),
        (
            ring(&[[0, 0], [0, 4], [4, 4], [4, 0]]),
            PolygonError::InvalidWinding,
        ),
    ];
    for (boundary, error) in cases {
        assert_eq!(PolygonPrism::new(vec![boundary], n(1)), Err(error));
    }
    let ordinary_subdivision = ring(&[[0, 0], [2, 0], [4, 0], [4, 4], [0, 4]]);
    let prism = PolygonPrism::new(vec![ordinary_subdivision.clone()], n(1)).unwrap();
    assert_eq!(prism.rings(), &[ordinary_subdivision]);
    assert_eq!(prism.area(), measure(n(16), 0));

    let beyond_measure = ring(&[[0, 0], [10_000_000_000, 0], [0, 10_000_000_000]]);
    assert_eq!(
        PolygonPrism::new(vec![beyond_measure], n(1)),
        Err(PolygonError::Arithmetic(Error::OutOfRange))
    );
}

#[test]
fn polygon_with_round_and_straight_holes_has_exact_mass_coefficients() {
    let straight = ring(&[[1, 1], [1, 2], [2, 2], [2, 1]]);
    let circles = vec![circle(7, 2, 1), circle(2, 7, 1)];
    let prism =
        PolygonPrism::with_circular_holes(vec![concave_outer(), straight], circles.clone(), n(3))
            .unwrap();
    assert_eq!(prism.circular_holes(), circles);
    assert_eq!(prism.area(), measure(n(63), -2));
    assert_eq!(prism.volume(), measure(n(189), -6));
    let centroid = (246.5 - 9. * PI) / (63. - 2. * PI);
    for coordinate in prism.centroid() {
        assert!((coordinate - centroid).abs() < 1e-14);
    }
}

#[test]
fn round_holes_distinguish_segment_tangency_vertex_tangency_and_overlap() {
    let outer = concave_outer();
    let make = |hole| PolygonPrism::with_circular_holes(vec![outer.clone()], vec![hole], n(1));
    assert_eq!(make(circle(1, 7, 1)), Err(PolygonError::CircleContact));
    assert_eq!(make(circle(1, 7, 2)), Err(PolygonError::CircleOverlap));
    assert_eq!(make(circle(7, 7, 1)), Err(PolygonError::HoleOutside));
    let fraction = |a, b| Rational::new(a, b).unwrap();
    let vertex_tangent = Circle::new([fraction(17, 5), fraction(16, 5)], n(1)).unwrap();
    assert_eq!(make(vertex_tangent), Err(PolygonError::CircleContact));
    let center = [fraction(7, 2); 2];
    assert!(make(Circle::new(center, fraction(3, 5)).unwrap()).is_ok());
    assert_eq!(
        make(Circle::new(center, fraction(3, 4)).unwrap()),
        Err(PolygonError::CircleOverlap)
    );
    let tiny = Circle::new([n(2); 2], fraction(1, 10_000_000_000)).unwrap();
    assert_eq!(make(tiny), Err(PolygonError::Arithmetic(Error::OutOfRange)));
}

#[test]
fn mixed_and_round_holes_must_have_material_between_them() {
    let outer = concave_outer();
    let straight = ring(&[[1, 1], [1, 3], [3, 3], [3, 1]]);
    let fraction = |a, b| Rational::new(a, b).unwrap();
    let nested = Circle::new([n(2); 2], fraction(1, 4)).unwrap();
    let crossing = Circle::new([fraction(7, 2), n(2)], fraction(3, 4)).unwrap();
    for (hole, error) in [
        (nested, PolygonError::NestedHoles),
        (circle(4, 2, 1), PolygonError::CircleContact),
        (crossing, PolygonError::CircleOverlap),
    ] {
        assert_eq!(
            PolygonPrism::with_circular_holes(
                vec![outer.clone(), straight.clone()],
                vec![hole],
                n(1)
            ),
            Err(error)
        );
    }
    for (y, error) in [
        (8, PolygonError::CircleContact),
        (7, PolygonError::CircleOverlap),
    ] {
        assert_eq!(
            PolygonPrism::with_circular_holes(
                vec![outer.clone()],
                vec![circle(2, 6, 1), circle(2, y, 1)],
                n(1)
            ),
            Err(error)
        );
    }
}

#[test]
fn requires_strictly_contained_disjoint_holes() {
    let outer = concave_outer();
    for (hole, error) in [
        (
            ring(&[[5, 5], [5, 6], [6, 6], [6, 5]]),
            PolygonError::HoleOutside,
        ),
        (
            ring(&[[3, 3], [3, 5], [5, 5], [5, 3]]),
            PolygonError::RingIntersection,
        ),
        (
            ring(&[[0, 1], [0, 2], [1, 2], [1, 1]]),
            PolygonError::RingIntersection,
        ),
        (
            ring(&[[1, 1], [2, 1], [2, 2], [1, 2]]),
            PolygonError::InvalidWinding,
        ),
    ] {
        assert_eq!(
            PolygonPrism::new(vec![outer.clone(), hole], n(1)),
            Err(error)
        );
    }
    let enclosure = ring(&[[1, 1], [1, 3], [3, 3], [3, 1]]);
    let nested = ring(&[[2, 2], [2, 3], [3, 2]])
        .into_iter()
        .map(|point| point.map(|value| value.checked_sub(Rational::new(1, 2).unwrap()).unwrap()))
        .collect::<Vec<_>>();
    for (first, second) in [
        (enclosure.clone(), nested.clone()),
        (nested, enclosure.clone()),
    ] {
        assert_eq!(
            PolygonPrism::new(vec![outer.clone(), first, second], n(1)),
            Err(PolygonError::NestedHoles)
        );
    }
    let touching = ring(&[[3, 1], [3, 3], [4, 3], [4, 1]]);
    assert_eq!(
        PolygonPrism::new(vec![outer, enclosure, touching], n(1)),
        Err(PolygonError::RingIntersection)
    );
}

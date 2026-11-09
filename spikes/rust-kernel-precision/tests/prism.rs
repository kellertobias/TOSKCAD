use multicad_rust_kernel_precision::prism::{
    BoundaryCurve, Face, FaceKey, FaceRole, Location, Measure, PrismError, RoundedPrism,
};
use multicad_rust_kernel_precision::{Circle, Error, Rational, parse_length_mm};

fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}

fn q(value: &str) -> Rational {
    parse_length_mm(value).unwrap()
}

fn circle(x: i64, y: i64, radius: i64) -> Circle {
    Circle::new([n(x), n(y)], n(radius)).unwrap()
}

fn panel(height: Rational, cutter: Circle) -> RoundedPrism {
    RoundedPrism::extrude(10, n(40), n(10), height)
        .unwrap()
        .with_inner_loop(20, circle(12, 10, 3))
        .unwrap()
        .subtract_cylinder(30, cutter, n(-1), height.checked_add(n(1)).unwrap())
        .unwrap()
}

#[test]
fn baseline_panel_has_analytic_trims_and_exact_mass_properties() {
    let cutter = circle(28, 10, 4);
    let solid = panel(n(8), cutter);
    assert_eq!(
        solid.outer_loop(),
        [
            BoundaryCurve::Segment {
                from: [n(0), n(0)],
                to: [n(40), n(0)]
            },
            BoundaryCurve::RightSemicircle(circle(40, 10, 10)),
            BoundaryCurve::Segment {
                from: [n(40), n(20)],
                to: [n(0), n(20)]
            },
            BoundaryCurve::Segment {
                from: [n(0), n(20)],
                to: [n(0), n(0)]
            },
        ]
    );
    assert_eq!(
        solid.inner_loops().collect::<Vec<_>>(),
        [(20, circle(12, 10, 3)), (30, cutter)]
    );
    assert_eq!(cutter.center(), [n(28), n(10)]);
    assert_eq!(cutter.radius(), n(4));
    assert_eq!(
        solid.area(),
        Measure {
            constant: n(800),
            pi_factor: n(25)
        }
    );
    assert_eq!(
        solid.volume(),
        Measure {
            constant: n(6400),
            pi_factor: n(200)
        }
    );
}

#[test]
fn authored_face_keys_survive_fractional_height_and_diameter_edits() {
    use FaceRole::*;
    let original = panel(n(8), circle(28, 10, 4));
    let height = q("10 mm / 3");
    let adjusted_cutter = Circle::new([n(28), n(10)], q("4.25 mm")).unwrap();
    let edited = panel(height, adjusted_cutter);
    for (role, before, after) in [
        (Bottom, Face::Cap { z: n(0) }, Face::Cap { z: n(0) }),
        (Top, Face::Cap { z: n(8) }, Face::Cap { z: height }),
        (Left, Face::PlaneX { x: n(0) }, Face::PlaneX { x: n(0) }),
        (Front, Face::PlaneY { y: n(0) }, Face::PlaneY { y: n(0) }),
        (Back, Face::PlaneY { y: n(20) }, Face::PlaneY { y: n(20) }),
        (
            RoundedEnd,
            Face::Semicylinder {
                circle: circle(40, 10, 10),
            },
            Face::Semicylinder {
                circle: circle(40, 10, 10),
            },
        ),
    ] {
        let key = FaceKey { feature: 10, role };
        assert_eq!(original.face(key), Some(before));
        assert_eq!(edited.face(key), Some(after));
    }
    let bore = FaceKey {
        feature: 30,
        role: Bore,
    };
    assert_eq!(
        original.face(bore),
        Some(Face::Bore {
            circle: circle(28, 10, 4)
        })
    );
    assert_eq!(
        edited.face(bore),
        Some(Face::Bore {
            circle: adjusted_cutter
        })
    );
    assert_eq!(
        edited.face(FaceKey {
            feature: 20,
            role: Bore
        }),
        Some(Face::Bore {
            circle: circle(12, 10, 3)
        })
    );
    assert_eq!(
        edited.face(FaceKey {
            feature: 10,
            role: Bore
        }),
        None
    );
    assert_eq!(
        edited.face(FaceKey {
            feature: 30,
            role: Top
        }),
        None
    );
    assert_eq!(
        edited.volume(),
        Measure {
            constant: q("8000/3 mm"),
            pi_factor: q("1835/24 mm")
        }
    );
}

#[test]
fn material_probes_distinguish_holes_caps_seam_and_exterior() {
    use Location::*;
    let solid = panel(n(8), circle(28, 10, 4));
    for ([x, y, z], expected) in [
        ([40, 10, 3], Interior),
        ([49, 10, 3], Interior),
        ([50, 10, 3], Boundary),
        ([51, 10, 3], Exterior),
        ([49, 18, 3], Exterior),
        ([40, 20, 3], Boundary),
        ([10, 20, 3], Boundary),
        ([-1, 10, 3], Exterior),
        ([10, 21, 3], Exterior),
        ([10, 1, 0], Boundary),
        ([10, 1, 8], Boundary),
        ([10, 1, 9], Exterior),
        ([10, 1, -1], Exterior),
        ([28, 10, 3], Exterior),
        ([28, 10, 8], Exterior),
        ([24, 10, 8], Boundary),
        ([32, 10, 3], Boundary),
        ([12, 10, 3], Exterior),
        ([9, 10, 3], Boundary),
        ([8, 10, 3], Interior),
    ] {
        assert_eq!(
            solid.classify([n(x), n(y), n(z)]),
            Ok(expected),
            "{x},{y},{z}"
        );
    }
}

#[test]
fn rejects_tangent_and_overlapping_cuts_without_changing_the_source() {
    use PrismError::*;
    let solid = RoundedPrism::extrude(10, n(40), n(10), n(8))
        .unwrap()
        .with_inner_loop(20, circle(12, 10, 3))
        .unwrap();
    let before = solid.clone();
    for (feature, shape, expected) in [
        (10, circle(28, 10, 4), DuplicateFeature),
        (20, circle(28, 10, 4), DuplicateFeature),
        (30, circle(2, 10, 2), OuterContact),
        (30, circle(10, 19, 1), OuterContact),
        (30, circle(48, 10, 2), OuterContact),
        (30, circle(49, 10, 2), OutsideProfile),
        (30, circle(48, 18, 1), OutsideProfile),
        (30, circle(28, 2, 3), OutsideProfile),
        (30, circle(18, 10, 3), HoleContact),
        (30, circle(17, 10, 3), HoleOverlap),
        (30, circle(12, 10, 1), HoleOverlap),
    ] {
        assert_eq!(
            solid.subtract_cylinder(feature, shape, n(-1), n(9)),
            Err(expected)
        );
        assert_eq!(solid, before);
    }
    for (bottom, top) in [(0, 9), (-1, 8), (2, 6), (10, 12), (-3, -1)] {
        assert_eq!(
            solid.subtract_cylinder(30, circle(28, 10, 4), n(bottom), n(top)),
            Err(NotThrough)
        );
    }
    for cutter_x in ["17.999 mm", "18 mm", "18.001 mm"] {
        let c = Circle::new([q(cutter_x), n(10)], n(3)).unwrap();
        let result = solid.subtract_cylinder(30, c, n(-1), n(9));
        match cutter_x {
            "17.999 mm" => assert_eq!(result, Err(HoleOverlap)),
            "18 mm" => assert_eq!(result, Err(HoleContact)),
            _ => assert_eq!(result.unwrap().inner_loops().len(), 2),
        }
    }
}

#[test]
fn zero_dimensions_and_out_of_range_arithmetic_fail_closed() {
    use PrismError::*;
    for (length, radius, height) in [(0, 10, 8), (40, -1, 8), (40, 10, 0)] {
        assert_eq!(
            RoundedPrism::extrude(10, n(length), n(radius), n(height)),
            Err(NonPositiveExtent)
        );
    }
    assert_eq!(
        RoundedPrism::extrude(10, n(40), n(10), n(1_000_000_000_000_000_000)),
        Err(Arithmetic(Error::OutOfRange))
    );
    let solid = RoundedPrism::extrude(10, n(40), n(10), n(8)).unwrap();
    assert_eq!(
        solid.classify([n(1_000_000_000_000_000_000), n(10), n(1)]),
        Err(Arithmetic(Error::OutOfRange))
    );
    assert_eq!(solid.inner_loops().len(), 0);
}

#[test]
fn mass_properties_match_both_independently_measured_occt_panels() {
    // Measurements from the first release run in cad338-occt-baseline/raw.jsonl.
    // Both OCCT seam orientations must agree with one Rust analytic cylinder.
    fn evaluate(measure: Measure) -> f64 {
        let (a, b) = measure.constant.ratio();
        let (c, d) = measure.pi_factor.ratio();
        a as f64 / b as f64 + std::f64::consts::PI * c as f64 / d as f64
    }
    for (height, radius, baseline) in [
        (
            n(8),
            n(4),
            [
                (878.5398163397449, 7028.318530717948),
                (878.5398163397449, 7028.31853071795),
            ],
        ),
        (
            q("10/3 mm"),
            q("4.25 mm"),
            [
                (872.060281491716, 2906.8676049723827),
                (872.0602814917158, 2906.8676049723827),
            ],
        ),
    ] {
        let cutter = Circle::new([n(28), n(10)], radius).unwrap();
        let solid = panel(height, cutter);
        for (top_area, volume) in baseline {
            assert!((evaluate(solid.area()) - top_area).abs() < 1e-9);
            assert!((evaluate(solid.volume()) - volume).abs() < 1e-8);
        }
    }
}

#[test]
fn a_circle_can_cross_the_join_without_creating_an_artificial_wall() {
    let solid = RoundedPrism::extrude(10, n(40), n(10), n(8)).unwrap();
    for x in [39, 40, 41] {
        let edited = solid.with_inner_loop(20, circle(x, 10, 3)).unwrap();
        assert_eq!(
            edited.classify([n(40), n(10), n(4)]),
            Ok(Location::Exterior)
        );
        assert_eq!(edited.classify([n(40), n(6), n(4)]), Ok(Location::Interior));
    }
    for (x, expected) in [
        ("47.999 mm", Ok(1)),
        ("48 mm", Err(PrismError::OuterContact)),
        ("48.001 mm", Err(PrismError::OutsideProfile)),
    ] {
        let circle = Circle::new([q(x), n(10)], n(2)).unwrap();
        assert_eq!(
            solid
                .with_inner_loop(20, circle)
                .map(|s| s.inner_loops().len()),
            expected
        );
    }
}

#[test]
fn unresolved_near_boundary_predicates_do_not_publish_a_cut() {
    let solid = RoundedPrism::extrude(10, n(40), n(10), n(8))
        .unwrap()
        .with_inner_loop(20, circle(12, 10, 3))
        .unwrap();
    let before = solid.clone();
    // The coordinate and its difference from 12 mm fit. The denominator
    // of the squared difference exceeds the exact arithmetic budget.
    let x = q("12000000013/1000000001 mm");
    let expected = PrismError::Arithmetic(Error::OutOfRange);
    assert_eq!(solid.classify([x, n(10), n(1)]), Err(expected));
    let cutter = Circle::new([x, n(10)], n(1)).unwrap();
    assert_eq!(
        solid.subtract_cylinder(30, cutter, n(-1), n(9)),
        Err(expected)
    );
    assert_eq!(solid, before);
}

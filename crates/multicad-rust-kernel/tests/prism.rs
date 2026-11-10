use multicad_rust_kernel::prism::{
    BoundaryCurve, Face, FaceKey, FaceRole, Location, Measure, Prism, PrismError,
};
use multicad_rust_kernel::{Circle, Error, Rational, parse_length_mm};

fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}

fn q(value: &str) -> Rational {
    parse_length_mm(value).unwrap()
}

fn circle(x: i64, y: i64, radius: i64) -> Circle {
    Circle::new([n(x), n(y)], n(radius)).unwrap()
}

fn panel(height: Rational, cutter: Circle) -> Prism {
    Prism::extrude(10, n(40), n(10), height)
        .unwrap()
        .with_inner_loop(20, circle(12, 10, 3))
        .unwrap()
        .subtract_cylinder(30, cutter, n(-1), height.checked_add(n(1)).unwrap())
        .unwrap()
}

#[test]
fn rectangular_prism_has_an_exact_right_face_and_clears_a_through_bore() {
    let size = [n(12), n(5), n(8)];
    let box_solid = Prism::rectangular(10, size).unwrap();
    assert_eq!(box_solid.maximum(), size);
    assert_eq!(box_solid.rounded_end(), None);
    assert_eq!(box_solid.volume().constant, n(480));
    assert_eq!(
        box_solid.outer_loop()[1],
        BoundaryCurve::Segment {
            from: [n(12), n(0)],
            to: [n(12), n(5)],
        }
    );
    let face = |role| FaceKey { feature: 10, role };
    assert_eq!(
        box_solid.face(face(FaceRole::Right)),
        Some(Face::PlaneX { x: n(12) })
    );
    assert_eq!(box_solid.face(face(FaceRole::RoundedEnd)), None);
    let rounded = Prism::extrude(10, n(12), n(2), n(8)).unwrap();
    assert_eq!(rounded.face(face(FaceRole::Right)), None);
    assert_eq!(
        box_solid.classify([n(12), n(2), n(4)]).unwrap(),
        Location::Boundary
    );
    assert_eq!(
        box_solid.classify([n(13), n(2), n(4)]).unwrap(),
        Location::Exterior
    );
    let hole = |x| Circle::new([n(x), n(2)], n(1)).unwrap();
    let cut = box_solid
        .subtract_cylinder(11, hole(10), n(-1), n(9))
        .unwrap();
    assert_eq!(
        cut.volume(),
        Measure {
            constant: n(480),
            pi_factor: n(-8)
        }
    );
    assert_eq!(
        cut.classify([n(10), n(2), n(4)]).unwrap(),
        Location::Exterior
    );
    assert_eq!(
        box_solid.subtract_cylinder(11, hole(11), n(-1), n(9)),
        Err(PrismError::OuterContact)
    );
    assert_eq!(
        Prism::rectangular(10, [n(1), n(0), n(1)]),
        Err(PrismError::NonPositiveExtent)
    );
}

#[test]
fn baseline_panel_has_analytic_trims_and_exact_mass_properties() {
    let cutter = circle(28, 10, 4);
    let solid = panel(n(8), cutter);
    assert_eq!(solid.feature(), 10);
    assert_eq!(solid.dimensions(), [n(40), n(20), n(8)]);
    assert_eq!(solid.maximum(), [n(50), n(20), n(8)]);
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
    let solid = Prism::extrude(10, n(40), n(10), n(8))
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
    let cutter = circle(28, 10, 4);
    let through = solid.subtract_cylinder(30, cutter, n(-1), n(9)).unwrap();
    for (bottom, top) in [(0, 9), (-1, 8), (0, 8)] {
        assert_eq!(
            solid.subtract_cylinder(30, cutter, n(bottom), n(top)),
            Ok(through.clone())
        );
    }
    for (bottom, top) in [
        (n(2), n(6)),
        (n(10), n(12)),
        (n(-3), n(-1)),
        (q("0.000000001 mm"), n(8)),
        (n(0), q("7.999999999 mm")),
    ] {
        assert_eq!(
            solid.subtract_cylinder(30, cutter, bottom, top),
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
            Prism::extrude(10, n(length), n(radius), n(height)),
            Err(NonPositiveExtent)
        );
    }
    assert_eq!(
        Prism::extrude(10, n(40), n(10), n(1_000_000_000_000_000_000)),
        Err(Arithmetic(Error::OutOfRange))
    );
    let solid = Prism::extrude(10, n(40), n(10), n(8)).unwrap();
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
    let solid = Prism::extrude(10, n(40), n(10), n(8)).unwrap();
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
    let solid = Prism::extrude(10, n(40), n(10), n(8))
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

#[test]
fn a_valid_cut_with_unrepresentable_volume_does_not_publish() {
    let height = q("1/1000000001 mm");
    let solid = Prism::extrude(10, n(40), n(10), height).unwrap();
    let cutter = Circle::new([n(12), n(10)], q("1/100000000 mm")).unwrap();
    let ordinary_height = Prism::extrude(10, n(40), n(10), n(1)).unwrap();
    assert!(
        ordinary_height
            .subtract_cylinder(20, cutter, n(-1), n(2))
            .is_ok()
    );
    assert_eq!(
        solid.subtract_cylinder(20, cutter, n(-1), n(2)),
        Err(PrismError::Arithmetic(Error::OutOfRange))
    );
    assert_eq!(solid.inner_loops().len(), 0);
    assert_eq!(solid.classify([n(12), n(10), n(0)]), Ok(Location::Boundary));
}

#[test]
fn face_keys_use_the_full_feature_id_and_do_not_follow_loop_order() {
    let base = Prism::extrude(10, n(40), n(10), n(8)).unwrap();
    let bore_a = circle(12, 10, 3);
    let bore_b = circle(28, 10, 4);
    // IDs share their entire lower 64 bits, as can occur with document UUIDs.
    let id_a = (1_u128 << 64) + 20;
    let id_b = (2_u128 << 64) + 20;
    let a_then_b = base
        .with_inner_loop(id_a, bore_a)
        .unwrap()
        .with_inner_loop(id_b, bore_b)
        .unwrap();
    let b_then_a = base
        .with_inner_loop(id_b, bore_b)
        .unwrap()
        .with_inner_loop(id_a, bore_a)
        .unwrap();
    for (feature, circle) in [(id_a, bore_a), (id_b, bore_b)] {
        let key = FaceKey {
            feature,
            role: FaceRole::Bore,
        };
        assert_eq!(a_then_b.face(key), Some(Face::Bore { circle }));
        assert_eq!(b_then_a.face(key), Some(Face::Bore { circle }));
    }
    assert_eq!(a_then_b.volume(), b_then_a.volume());
}

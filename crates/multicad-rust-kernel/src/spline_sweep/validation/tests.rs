use super::*;

#[test]
fn authored_validation_refuses_nonfinite_oversized_degenerate_and_outside_material() {
    let line = |a, b| Edge::Analytic(normal_sweep::Edge::Line { a, b });
    for (a, b, expected) in [
        ([f64::NAN, 0.], [1., 0.], ConstructionError::Invalid),
        ([1e9, 0.], [1e9 + 1., 0.], ConstructionError::Limit),
        ([0., 0.], [1e-9, 0.], ConstructionError::Invalid),
        ([0., 0.], [0., 0.], ConstructionError::Invalid),
    ] {
        assert_eq!(authored(&[line(a, b)], None, 1e-7, None), Err(expected));
    }
    let circle = normal_sweep::Edge::Circle {
        center: [0., 0.],
        radius_mm: 1.,
        clockwise: false,
    };
    assert_eq!(
        authored(&[Edge::Analytic(circle)], None, 1e-7, None),
        Err(ConstructionError::Invalid)
    );
    let square = |s| {
        vec![
            line([s, s], [s + 4., s]),
            line([s + 4., s], [s + 4., s + 4.]),
            line([s + 4., s + 4.], [s, s + 4.]),
            line([s, s + 4.], [s, s]),
        ]
    };
    assert_eq!(
        authored(&square(1e7), None, 1e-7, None),
        Err(ConstructionError::Unresolved)
    );
    let outside = normal_sweep::Edge::Circle {
        center: [8., 8.],
        radius_mm: 0.1,
        clockwise: true,
    };
    assert_eq!(
        authored(&square(0.), Some(outside), 1e-7, None),
        Err(ConstructionError::Invalid)
    );
    let reverse = [line([0., 0.], [4., 0.]), line([4., 0.], [0., 0.])];
    assert_eq!(
        authored(&reverse, None, 1e-7, None),
        Err(ConstructionError::Unresolved)
    );
    let folded = Edge::Cubic(
        super::super::Cubic::new(
            vec![[0., 0.], [2., 1.], [-1., 1.], [1., 0.]],
            vec![0., 1.],
            false,
        )
        .unwrap(),
    );
    assert_eq!(
        authored(&[folded], None, 1e-7, None),
        Err(ConstructionError::Unresolved)
    );
}

#[test]
fn authored_arcs_prove_the_whole_tangent_sweep_and_refuse_radius_uncertainty() {
    let line = |a, b| Edge::Analytic(normal_sweep::Edge::Line { a, b });
    let q = 0.5_f64.sqrt();
    let a = [3. + q, 1. - q];
    let b = [3. + q, 1. + q];
    let curved_edge = normal_sweep::Edge::Arc {
        center: [3., 1.],
        a,
        b,
        clockwise: false,
    };
    let source = vec![
        line([0., -1.], [3., -1.]),
        line([3., -1.], a),
        Edge::Analytic(curved_edge),
        line(b, [3., 3.]),
        Edge::Cubic(
            super::super::Cubic::new(
                vec![[3., 3.], [2., 3.2], [1., 3.2], [0., 3.]],
                vec![0., 1.],
                false,
            )
            .unwrap(),
        ),
        line([0., 3.], [0., -1.]),
    ];
    assert_eq!(authored(&source, None, 1e-7, None), Ok(()));
    let solid = super::super::Sweep::new(source.clone(), None, 3., 1e-7).unwrap();
    assert!((solid.volume_mm3() - 3. * (12.3 + 2_f64.sqrt() + PI / 4.)).abs() < 1e-10);
    for (radius, delta) in [(1., 1e-4), (1e-9, 0.)] {
        let bad = Edge::Analytic(normal_sweep::Edge::Arc {
            center: [0., 0.],
            a: [radius, 0.],
            b: [0., radius + delta],
            clockwise: false,
        });
        assert_eq!(
            authored(&[bad], None, 1e-7, None),
            Err(ConstructionError::Unresolved)
        );
    }
    let semicircle = arc([0., 0.], 1., 0., PI);
    assert!(!semicircle.certified_forward([0., 1.]));
}

fn line(a: Point2, b: Point2) -> Piece {
    Piece::Line([a, b])
}
fn arc(center: Point2, radius: f64, start: f64, sweep: f64) -> Piece {
    Piece::Arc(Arc {
        center,
        radius,
        start,
        sweep,
    })
}
fn bezier() -> Piece {
    Piece::Bezier([[0., 0.], [1., 1.], [2., 1.], [3., 0.]])
}
fn shape(p: Piece) -> Boundary {
    Boundary {
        pieces: vec![p],
        bounds: p.bounds(),
    }
}

#[test]
fn subdivision_certifies_separation_and_refuses_crossings_or_exhausted_work() {
    let a = line([0., 0.], [4., 4.]);
    let b = line([0., 1.], [4., 5.]);
    let c = line([0., 4.], [4., 0.]);
    assert!(!separated(a.bounds(), b.bounds(), 1e-7));
    assert_eq!(
        clear_pair(
            a,
            b,
            1e-7,
            0,
            &mut Budget {
                remaining: 300,
                interrupt: None
            }
        ),
        Ok(())
    );
    assert_eq!(
        clear_pair(
            a,
            c,
            1e-7,
            0,
            &mut Budget {
                remaining: 300,
                interrupt: None
            }
        ),
        Err(ConstructionError::Unresolved)
    );
    assert_eq!(
        clear_pair(
            a,
            b,
            1e-7,
            0,
            &mut Budget {
                remaining: 0,
                interrupt: None
            }
        ),
        Err(ConstructionError::Limit)
    );
    let y = arc([0., 0.], 4., 0., FRAC_PI_2);
    assert_eq!(
        clear_pair(
            y,
            bezier(),
            1e-7,
            0,
            &mut Budget {
                remaining: 300,
                interrupt: None
            }
        ),
        Ok(())
    );
}

#[test]
fn circle_clearance_accounts_for_curves_inside_a_large_control_box() {
    let a = arc([0., 0.], 4., 0., FRAC_PI_2);
    assert!(
        clear_disk(
            a,
            [1., 1.],
            0.1,
            0,
            &mut Budget {
                remaining: 300,
                interrupt: None
            }
        )
        .is_ok()
    );
    assert!(
        clear_disk(
            bezier(),
            [1.5, 0.],
            0.1,
            0,
            &mut Budget {
                remaining: 300,
                interrupt: None
            }
        )
        .is_ok()
    );
    assert_eq!(
        clear_disk(
            line([0., 0.], [4., 4.]),
            [2., 2.],
            0.1,
            0,
            &mut Budget {
                remaining: 300,
                interrupt: None
            }
        ),
        Err(ConstructionError::Unresolved)
    );
}

#[test]
fn coincident_and_reversed_neighbors_cannot_certify_a_simple_outline() {
    let a = line([0., 0.], [1., 0.]);
    let b = line([1., 0.], [0., 0.]);
    assert_eq!(
        simple(
            &[shape(a), shape(b)],
            1e-7,
            &mut Budget {
                remaining: 300,
                interrupt: None
            }
        ),
        Err(ConstructionError::Unresolved)
    );
    let backtrack = arc([1., 0.], 1., PI, PI);
    assert!(!backtrack.forward_along([0., 1.]));
    assert_eq!(
        simple(
            &[shape(a), shape(backtrack)],
            1e-7,
            &mut Budget {
                remaining: 300,
                interrupt: None
            }
        ),
        Err(ConstructionError::Unresolved)
    );
}

#[test]
fn multiple_raw_rings_refuse_invalid_circle_members_before_certifying_occupancy() {
    let line = |a, b| Edge::Analytic(normal_sweep::Edge::Line { a, b });
    let exterior = vec![
        line([0., 0.], [10., 0.]),
        line([10., 0.], [10., 10.]),
        line([10., 10.], [0., 10.]),
        line([0., 10.], [0., 0.]),
    ];
    for radius in [0., f64::INFINITY] {
        let invalid = vec![Edge::Analytic(normal_sweep::Edge::Circle {
            center: [2., 2.],
            radius_mm: radius,
            clockwise: true,
        })];
        assert_eq!(
            authored_rings(
                &exterior,
                &[
                    invalid,
                    vec![
                        line([4., 4.], [4., 5.]),
                        line([4., 5.], [5., 5.]),
                        line([5., 5.], [5., 4.]),
                        line([5., 4.], [4., 4.])
                    ]
                ],
                1e-7,
                None
            ),
            Err(ConstructionError::Invalid)
        );
    }
    let mixed = vec![
        Edge::Analytic(normal_sweep::Edge::Circle {
            center: [2., 2.],
            radius_mm: 1.,
            clockwise: true,
        }),
        line([3., 2.], [3., 3.]),
        line([3., 3.], [3., 2.]),
    ];
    assert_eq!(
        authored_rings(&exterior, &[mixed], 1e-7, None),
        Err(ConstructionError::Invalid)
    );
}

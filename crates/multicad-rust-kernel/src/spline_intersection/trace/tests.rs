use super::*;

const FULL: [Uv; 2] = [[ZERO; 2], [ONE; 2]];

fn rational(n: i64, d: i64) -> Rational {
    Rational::new(n, d).unwrap()
}

fn circle(trim: [Uv; 2]) -> [Patch; 2] {
    let quarter = rational(1, 4);
    let controls = [quarter, rational(-1, 4), quarter];
    let bowl = Patch::new(
        controls.map(|u| controls.map(|v| u.checked_add(v).unwrap())),
        FULL,
    )
    .unwrap();
    let plane = Patch::new([[rational(1, 16); 3]; 3], trim).unwrap();
    [bowl, plane]
}

fn trace(trim: [Uv; 2]) -> Trace {
    let [bowl, plane] = circle(trim);
    bowl.trace(&plane).unwrap()
}

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-13, "{a} != {b}");
}

#[test]
fn split_circle_is_traced_from_exact_entry_to_exact_exit() {
    let mid = rational(1, 2);
    let Trace::Clipped(branches) = trace([[mid, ZERO], [ONE; 2]]) else {
        panic!("a right half-circle has one branch");
    };
    let [branch] = branches.branches() else {
        panic!("expected one branch");
    };
    let [start, end] = branch.endpoints();
    for (endpoint, sign, v) in [(start, false, 0.25), (end, true, 0.75)] {
        assert_eq!(endpoint.trim_coordinate(), (0, mid));
        assert_eq!(endpoint.free_coordinate(), (mid, rational(1, 16), sign));
        assert_eq!(endpoint.parameter_uv(), [0.5, v]);
    }
    assert_eq!(branches.point_at(0, 0.), Some([0.5, 0.25, 0.0625]));
    assert_eq!(branches.point_at(0, 1.), Some([0.5, 0.75, 0.0625]));
    let middle = branches.point_at(0, 0.5).unwrap();
    near(middle[0], 0.75);
    near(middle[1], 0.5);
    near(middle[2], 0.0625);
    assert_eq!(branches.point_at(1, 0.5), None);
    assert_eq!(branches.point_at(0, f64::NAN), None);
    assert_eq!(branches.point_at(0, 2.), None);
}

#[test]
fn finite_trim_boundaries_retain_two_or_four_distinct_branches() {
    let min = rational(3, 10);
    let max = rational(7, 10);
    for (trim, count) in [
        ([[min, ZERO], [max, ONE]], 2),
        ([[ZERO, min], [ONE, max]], 2),
        ([[min; 2], [max; 2]], 4),
    ] {
        let Trace::Clipped(result) = trace(trim) else {
            panic!("expected clipped circle")
        };
        assert_eq!(result.branches().len(), count);
        for (index, branch) in result.branches().iter().enumerate() {
            let [a, b] = branch.endpoints();
            assert_ne!(a, b);
            for i in 0..=8 {
                let fraction = f64::from(i) / 8.;
                let [u, v, z] = result.point_at(index, fraction).unwrap();
                for (axis, parameter) in [u, v].into_iter().enumerate() {
                    assert!(
                        (trim[0][axis].as_f64() - 1e-13..=trim[1][axis].as_f64() + 1e-13)
                            .contains(&parameter)
                    );
                }
                near((u - 0.5).hypot(v - 0.5), 0.25);
                near(z, 0.0625);
            }
        }
    }
}

#[test]
fn trace_keeps_unclipped_classification_and_refuses_degenerate_contacts() {
    assert!(matches!(
        trace(FULL),
        Trace::Unclipped(Intersection::Closed(_))
    ));
    assert_eq!(
        trace([[rational(7, 8), ZERO], [ONE; 2]]),
        Trace::Unclipped(Intersection::Disjoint)
    );
    for trim in [
        [[rational(1, 4), ZERO], [ONE; 2]],
        [[rational(13, 20), rational(7, 10)], [ONE; 2]],
    ] {
        let [a, b] = circle(trim);
        assert_eq!(a.trace(&b), Err(Error::TrimContact));
    }

    let mid = rational(1, 2);
    let [mut a, b] = circle([[mid, ZERO], [ONE; 2]]);
    a.trim[1][0] = mid;
    assert_eq!(a.trace(&b), Err(Error::TrimContact));
}

#[test]
fn branch_pairing_refuses_missing_inconsistent_or_numerically_collapsed_events() {
    let [bowl, plane] = circle([[rational(1, 2), ZERO], [ONE; 2]]);
    let Classified::Crossing(conic) = bowl.classify(&plane).unwrap() else {
        panic!()
    };
    let support = conic.lift(bowl.heights).unwrap();
    let events = conic.side(0, false, rational(1, 2), &support).unwrap();
    let [Some(start), Some(end)] = events else {
        panic!()
    };
    assert_eq!(pair_events(&[]), Err(Error::TrimContact));
    assert_eq!(pair_events(&[start, start]), Err(Error::TrimContact));
    let nearly_same_angle = Event {
        angle: start.angle.next_up(),
        ..end
    };
    assert_eq!(
        pair_events(&[start, nearly_same_angle]),
        Err(Error::EvaluationPrecision)
    );
}

#[test]
fn radical_order_is_exact_across_signs_cancellation_and_equal_forms() {
    let plus = |mean, radicand| Root {
        mean,
        radicand,
        sign: Ordering::Greater,
    };
    let minus = |mean, radicand| Root {
        mean,
        radicand,
        sign: Ordering::Less,
    };
    let near_zero = plus(
        rational(-1, 1),
        rational(999_999_999_999_999, 1_000_000_000_000_000),
    );
    assert_eq!(near_zero.value_sign(), Ok(Ordering::Less));
    assert!(near_zero.approximation().unwrap() < 0.);
    assert_eq!(
        plus(rational(-2, 1), rational(4, 1)).value_sign(),
        Ok(Ordering::Equal)
    );
    assert_eq!(
        minus(TWO, rational(4, 1)).compare(Root::rational(ZERO)),
        Ok(Ordering::Equal)
    );
    assert_eq!(
        plus(ONE, TWO).compare(plus(ZERO, rational(4, 1))),
        Ok(Ordering::Greater)
    );
    assert_eq!(
        minus(ZERO, TWO).compare(minus(ONE, rational(4, 1))),
        Ok(Ordering::Less)
    );
    assert_eq!(
        minus(ZERO, TWO).compare(plus(ZERO, ONE)),
        Ok(Ordering::Less)
    );
    assert_eq!(
        plus(ZERO, TWO).compare(minus(ZERO, ONE)),
        Ok(Ordering::Greater)
    );
    assert_eq!(
        plus(ONE, ONE).compare(plus(ZERO, rational(4, 1))),
        Ok(Ordering::Equal)
    );
    assert_eq!(
        plus(ONE, TWO).scaled(rational(-1, 1)).unwrap().value_sign(),
        Ok(Ordering::Less)
    );
    assert_eq!(
        plus(ONE, TWO).scaled(ZERO).unwrap().value_sign(),
        Ok(Ordering::Equal)
    );
}

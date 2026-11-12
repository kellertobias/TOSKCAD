use super::*;
use std::cell::Cell;
#[test]
fn cumulative_support_budget_and_cancellation_fail_then_valid_request_recovers() {
    let material = crate::radial_tensor::tests::material();
    for limit in [0, 1, 2, 8, 32] {
        assert_eq!(
            material.support_with_budget([1., 0., 0.], 1e-6, None, limit),
            Err(Error::Limit)
        );
    }
    let calls = Cell::new(0);
    let interrupt = || {
        let next = calls.get() + 1;
        calls.set(next);
        next == 9
    };
    assert_eq!(
        material.support_bounds([1., 0., 0.], 1e-6, Some(&interrupt)),
        Err(Error::Cancelled)
    );
    assert_eq!(calls.get(), 9);
    assert!(material.support_bounds([1., 0., 0.], 1e-6, None).is_ok());
}
#[test]
fn angular_enclosures_keep_zero_wraps_and_seam_uncertainty_explicit() {
    let p = |x, y| [I::point(x), I::point(y)];
    assert!(angle(p(1., 0.), p(1., 0.)).unwrap().is_none());
    let opposite = angle(p(1., 0.), p(-1., 0.)).unwrap().unwrap();
    assert!(opposite.lo < std::f64::consts::PI && opposite.hi > std::f64::consts::PI);
    let crossing = [I::new(-1., 1.); 2];
    assert!(angle(crossing, p(1., 0.)).unwrap().is_none());
    assert!(angle(p(1., 0.), crossing).unwrap().is_none());
    assert!(angle(p(1., 0.), p(0., -1.)).unwrap().unwrap().lo > 4.7);
}
#[test]
fn local_tensor_crossing_enclosures_refuse_empty_domains_and_unproved_precision() {
    use tensor::{Refinement, potential_radial_range as query};
    let body = crate::radial_tensor::tests::material();
    let chart = &body.ordered.caps.axial.chart;
    let patch = RetainedTensor::new(
        [1, 1],
        (0..=9)
            .flat_map(|i| (0..=9).map(move |j| [2., 2. * i as f64 / 9. - 1., 4. * j as f64 / 9.]))
            .collect(),
    )
    .unwrap();
    let work = || Work {
        remaining: 2_000_000,
        interrupt: None,
    };
    let domain = [I::new(0., 1.); 2];
    let margin = I::point(1e-6);
    let point = |x, y, z| [I::point(x), I::point(y), I::point(z)];
    assert!(
        query(
            &patch,
            chart,
            point(1., 0., 2.),
            domain,
            margin,
            Refinement::DEFAULT,
            &mut work()
        )
        .unwrap()
        .unwrap()
        .lo > 2.9
    );
    assert!(
        query(
            &patch,
            chart,
            point(3., 0., 2.),
            domain,
            margin,
            Refinement::DEFAULT,
            &mut work()
        )
        .unwrap()
        .unwrap()
        .hi < -3.9
    );
    for point in [point(0., 3., 2.), point(2., 0., 9.)] {
        assert!(
            query(
                &patch,
                chart,
                point,
                domain,
                margin,
                Refinement::DEFAULT,
                &mut work()
            )
            .unwrap()
            .is_none()
        );
    }
    let boundary = point(2., 0., 2.);
    let tight = query(
        &patch,
        chart,
        boundary,
        domain,
        margin,
        Refinement::DEFAULT,
        &mut work(),
    )
    .unwrap()
    .unwrap();
    assert!(tight.lo <= 0. && tight.hi >= 0.);
    for limits in [
        Refinement {
            depth: 0,
            cells: 32768,
        },
        Refinement {
            depth: 64,
            cells: 0,
        },
    ] {
        let unresolved = query(
            &patch,
            chart,
            boundary,
            domain,
            I::point(0.),
            limits,
            &mut work(),
        )
        .unwrap()
        .unwrap();
        assert!(unresolved.lo < 0. && unresolved.hi > 0.);
    }
    let narrow = [I::new(1_f64.next_down(), 1.), I::new(0., 1.)];
    let unresolved = query(
        &patch,
        chart,
        point(2., 1., 2.),
        narrow,
        I::point(0.),
        Refinement::DEFAULT,
        &mut work(),
    )
    .unwrap()
    .unwrap();
    assert!(unresolved.lo <= 0. && unresolved.hi >= 0.);
    assert_eq!(
        query(
            &patch,
            chart,
            boundary,
            domain,
            margin,
            Refinement::DEFAULT,
            &mut Work {
                remaining: 1,
                interrupt: None
            }
        )
        .err(),
        Some(Error::Limit)
    );
    assert_eq!(
        query(
            &patch,
            chart,
            boundary,
            domain,
            margin,
            Refinement::DEFAULT,
            &mut Work {
                remaining: 100_000,
                interrupt: Some(&|| true)
            }
        )
        .err(),
        Some(Error::Cancelled)
    );
    assert!(
        query(
            &patch,
            chart,
            boundary,
            domain,
            margin,
            Refinement::DEFAULT,
            &mut work()
        )
        .is_ok()
    );
}

#[test]
fn finite_inverse_domains_and_radial_signs_keep_uncertainty_and_sticky_controls() {
    assert_eq!(unit_window(I::new(-0.1, 1.1)).unwrap(), [0., 1.]);
    for v in [I::new(-2., -1.), I::new(2., 3.), I::point(0.)] {
        assert_eq!(unit_window(v), Err(Error::Unresolved));
    }
    let margin = I::point(1e-6);
    assert_eq!(radial_side(Some(I::new(1., 2.)), margin), Location::Inside);
    assert_eq!(
        radial_side(Some(I::new(-2., -1.)), margin),
        Location::Outside
    );
    for delta in [None, Some(I::new(-1e-7, 1e-7))] {
        assert_eq!(radial_side(delta, margin), Location::Unresolved);
    }
    let (used, limited, cancelled) = (Cell::new(0), Cell::new(false), Cell::new(false));
    let calls = Cell::new(0);
    let cancel = || {
        calls.set(calls.get() + 1);
        true
    };
    assert!(support_checkpoint(
        &used,
        &limited,
        &cancelled,
        10,
        Some(&cancel)
    ));
    assert!(support_checkpoint(
        &used,
        &limited,
        &cancelled,
        10,
        Some(&cancel)
    ));
    assert_eq!(calls.get(), 1);
    let (used, limited, cancelled) = (Cell::new(0), Cell::new(false), Cell::new(false));
    assert!(support_checkpoint(&used, &limited, &cancelled, 0, None));
    assert!(support_checkpoint(&used, &limited, &cancelled, 0, None));
    assert_eq!(used.get(), 1);
}
#[test]
fn actual_caps_seams_and_tight_tensor_queries_keep_uncertainty_explicit() {
    let body = crate::radial_tensor::tests::material();
    let chart = &body.ordered.caps.axial.chart;
    let center = chart.bottom_point;
    assert_eq!(body.classify(center, None).unwrap(), Location::Unresolved);
    let surface = &body.ordered.caps.axial.tensors[1].surface;
    let point = surface
        .point_bounds([0, 0], [[0.5, 0.5]; 2])
        .unwrap()
        .map(|r| (r[0] + r[1]) * 0.5);
    assert!(matches!(
        body.classify(point, None),
        Ok(Location::Unresolved) | Err(Error::Limit) | Err(Error::Unresolved)
    ));
    let rail = &body.ordered.rails[0];
    let q = ordering::bounds(
        rail,
        [0.5, 0.5],
        &mut Work {
            remaining: 100_000,
            interrupt: None,
        },
    )
    .unwrap();
    let world = |p: [f64; 3]| {
        std::array::from_fn(|i| {
            chart.bottom_point[i]
                + (0..3)
                    .map(|j| chart.axes[j][i].midpoint() * p[j])
                    .sum::<f64>()
        })
    };
    let point = world([
        q[0].midpoint() * 1.2,
        q[1].midpoint() * 1.2,
        chart.nominal_height * 0.5,
    ]);
    assert_eq!(body.classify(point, None).unwrap(), Location::Unresolved);
}

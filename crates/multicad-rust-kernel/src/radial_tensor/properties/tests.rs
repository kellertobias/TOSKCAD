use super::*;
use crate::tapered_thread::{
    calculation_cell, curve_first_bounds, curve_taylor9, normal_polynomial, tensor_first_bounds,
    tensor_taylor8,
};
use std::cell::Cell;
fn cam(grids: &[usize]) -> RadialBoundary {
    BoundaryOwnership::new(crate::radial_tensor::tests::cam::wedge(grids), 1e-7, None)
        .unwrap()
        .bind_surfaces(1e-7, None)
        .unwrap()
        .certify_axial(1e-7, 1e-6, None)
        .unwrap()
        .certify_caps(None)
        .unwrap()
        .certify_ordering(None)
        .unwrap()
        .certify_material(1e-7, None)
        .unwrap()
}
fn contains(range: I, value: f64) {
    assert!(
        range.lo <= value && range.hi >= value,
        "{range:?} misses {value}"
    );
}
fn patch() -> crate::tapered_thread::TensorCell {
    calculation_cell(std::array::from_fn(|i| {
        [
            I::point((i / 10) as f64),
            I::point((i % 10) as f64),
            I::point(((i / 10) * (i % 10)) as f64),
        ]
    }))
}
#[test]
fn numerical_parameter_cells_enclose_actual_first_and_eighth_order_source_derivatives() {
    let cell = patch();
    for along_v in [false, true] {
        let jets = tensor_taylor8(&cell, along_v);
        for coefficient in &jets[0][2..] {
            for &v in coefficient {
                contains(v, 0.);
            }
        }
    }
    let at = tensor_first_bounds(&cell, [I::point(0.25), I::point(0.75)]);
    for (p, expected) in
        at.into_iter()
            .zip([[2.25, 6.75, 15.1875], [9., 0., 60.75], [0., 9., 20.25]])
    {
        for (v, e) in p.into_iter().zip(expected) {
            contains(v, e);
        }
    }
    let line = (0..=9)
        .map(|i| [I::point(i as f64), I::point(0.), I::point(0.)])
        .collect::<Vec<_>>();
    let jet = curve_taylor9(&line);
    contains(jet[1][0], 9.);
    for coefficient in &jet[2..] {
        for &v in coefficient {
            contains(v, 0.);
        }
    }
    let at = curve_first_bounds(&line, I::point(0.25));
    contains(at[0][0], 2.25);
    contains(at[1][0], 9.);
}
#[test]
fn normal_polynomial_child_jets_preserve_actual_cross_product_and_jacobian_scale() {
    let geometry = patch();
    let normal = normal_polynomial(&geometry);
    let full = normal.taylor8([[0., 1.]; 2], false).unwrap();
    contains(full[0][2], 81.);
    contains(full[1][1], -729.);
    let [child, _] = geometry.subdivide(0).unwrap();
    let u = normal.taylor8(child.domain(), false).unwrap();
    contains(u[0][2], 40.5);
    contains(u[1][1], -182.25);
    let v = normal.taylor8(child.domain(), true).unwrap();
    contains(v[1][0], -364.5);
    let [quarter, _] = child.subdivide(1).unwrap();
    contains(
        normal.taylor8(quarter.domain(), false).unwrap()[0][2],
        20.25,
    );
    assert_eq!(
        normal.taylor8([[-1., 1.], [0., 1.]], false).err(),
        Some(Error::Invalid)
    );
    let original = normal.taylor8([[0., 1.]; 2], false).unwrap();
    contains(original[0][2], 81.);
}
#[test]
fn failed_property_requests_do_not_change_or_poison_the_immutable_source() {
    for grids in [&[1][..], &[1, 2][..]] {
        let body = cam(grids);
        let original = body.input().clone();
        for invalid in [0., -1., 1., f64::INFINITY, f64::NAN] {
            assert_eq!(
                body.properties_with_interrupt(invalid, None).err(),
                Some(Error::Invalid)
            );
        }
        assert_eq!(
            body.properties_with_interrupt(1e-16, None).err(),
            Some(Error::Unresolved)
        );
        for limit in [0, 1, 1000] {
            assert_eq!(
                body.properties_with_budget(1e-4, None, limit).err(),
                Some(Error::Limit)
            );
        }
        for stop in [1, 3, 10] {
            let calls = Cell::new(0);
            let cancel = || {
                calls.set(calls.get() + 1);
                calls.get() == stop
            };
            assert_eq!(
                body.properties_with_interrupt(1e-4, Some(&cancel)).err(),
                Some(Error::Cancelled)
            );
            assert_eq!(calls.get(), stop);
        }
        let calls = Cell::new(0);
        let count = || {
            calls.set(calls.get() + 1);
            false
        };
        let valid = body.properties_with_interrupt(1e-4, Some(&count)).unwrap();
        let stop = calls.get() / 2;
        assert!(stop > 10);
        calls.set(0);
        let cancel = || {
            calls.set(calls.get() + 1);
            calls.get() == stop
        };
        assert_eq!(
            body.properties_with_interrupt(1e-4, Some(&cancel)).err(),
            Some(Error::Cancelled)
        );
        assert_eq!(calls.get(), stop);
        assert_eq!(body.properties_with_interrupt(1e-4, None).unwrap(), valid);
        assert_eq!(body.input(), &original);
    }
}
fn work() -> Work<'static> {
    Work {
        remaining: 512_000_000,
        interrupt: None,
    }
}
#[test]
fn source_expansion_and_adaptive_cells_refuse_small_resource_budgets_atomically() {
    let body = cam(&[1]);
    let chart = &body.ordered.caps.axial.chart;
    for limit in [0, 3, 6] {
        assert_eq!(
            faces::with_limit(&body, &mut work(), limit).err(),
            Some(Error::Limit)
        );
    }
    let prepared = faces::build(&body, &mut work()).unwrap();
    let cap = &prepared[0];
    let cone = prepared.last().unwrap();
    let accuracy = || integrate::Accuracy {
        scales: [I::point(1.); measure::QUANTITIES],
        epsilon: 1e-20,
    };
    assert_eq!(
        integrate::with_limits(
            cap.sources.clone(),
            cap.density,
            chart,
            2.,
            accuracy(),
            integrate::Limits {
                cells: 1,
                depth: 48
            },
            &mut work()
        )
        .err(),
        Some(Error::Limit)
    );
    assert_eq!(
        integrate::with_limits(
            cap.sources.clone(),
            cap.density,
            chart,
            2.,
            accuracy(),
            integrate::Limits {
                cells: cap.sources.len(),
                depth: 48
            },
            &mut work()
        )
        .err(),
        Some(Error::Limit)
    );
    assert_eq!(
        integrate::with_limits(
            cap.sources.clone(),
            cap.density,
            chart,
            2.,
            accuracy(),
            integrate::Limits {
                cells: 32768,
                depth: 0
            },
            &mut work()
        )
        .err(),
        Some(Error::Unresolved)
    );
    assert_eq!(
        integrate::with_limits(
            cone.sources.clone(),
            cone.density,
            chart,
            2.,
            accuracy(),
            integrate::Limits {
                cells: 32768,
                depth: 0
            },
            &mut work()
        )
        .err(),
        Some(Error::Unresolved)
    );
    assert_eq!(
        integrate::with_limits(
            cone.sources.clone(),
            cone.density,
            chart,
            2.,
            accuracy(),
            integrate::Limits {
                cells: 1,
                depth: 48
            },
            &mut work()
        )
        .err(),
        Some(Error::Limit)
    );
    assert!(body.properties_with_interrupt(1e-4, None).is_ok());
}
#[test]
fn property_intervals_refuse_nonfinite_inverted_and_unresolved_bounds() {
    for v in [
        I::new(f64::NEG_INFINITY, f64::INFINITY),
        I::new(2., 1.),
        I::new(f64::NAN, 1.),
    ] {
        assert_eq!(require_width(v, 1.), Err(Error::Unresolved));
    }
    assert_eq!(require_width(I::new(0., 2.), 1.), Err(Error::Unresolved));
    assert!(require_width(I::new(0., 0.5), 1.).is_ok());
}

#[test]
fn circular_parameter_refinement_refuses_an_unrepresentable_midpoint() {
    let body = cam(&[1]);
    let faces = faces::build(&body, &mut work()).unwrap();
    let mut circle = faces
        .iter()
        .flat_map(|face| &face.sources)
        .find_map(|source| {
            if let source::Source::Circle(circle) = source {
                Some(circle.clone())
            } else {
                None
            }
        })
        .unwrap();
    circle.range = [0.5, 0.5_f64.next_up()];
    assert_eq!(
        source::Source::Circle(circle)
            .subdivide(false)
            .err()
            .unwrap(),
        Error::Unresolved
    );
    assert!(body.properties_with_interrupt(1e-4, None).is_ok());
}

#[test]
fn tied_adaptive_cells_preserve_both_integral_contributions() {
    let body = cam(&[1]);
    let faces = faces::build(&body, &mut work()).unwrap();
    let circle = faces[0]
        .sources
        .iter()
        .find(|source| matches!(source, source::Source::Circle(_)))
        .unwrap()
        .clone();
    let integrate = |sources| {
        integrate::face(
            sources,
            faces[0].density,
            &body.ordered.caps.axial.chart,
            2.,
            integrate::Accuracy {
                scales: [I::point(1.); measure::QUANTITIES],
                epsilon: 1e-7,
            },
            &mut work(),
        )
        .unwrap()
    };
    let single = integrate(vec![circle.clone()]);
    // Integrating two copies is a legitimate additive numerical problem:
    // their identical initial error scores must not discard either cell.
    let doubled = integrate(vec![circle.clone(), circle]);
    for (a, b) in single.into_iter().zip(doubled) {
        let expected = I::point(2.) * a;
        assert!(b.lo <= expected.hi && b.hi >= expected.lo);
        assert!(b.width() <= 1e-7);
    }
}

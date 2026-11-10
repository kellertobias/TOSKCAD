use super::*;
use crate::Circle;

fn integer(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}

fn rectangle() -> PolygonPrism {
    PolygonPrism::with_circular_holes(
        vec![
            vec![[0, 0], [20, 0], [20, 16], [0, 16]]
                .into_iter()
                .map(|p| p.map(integer))
                .collect(),
            vec![[9, 4], [9, 7], [11, 7], [11, 4]]
                .into_iter()
                .map(|p| p.map(integer))
                .collect(),
        ],
        vec![Circle::new([15, 8].map(integer), integer(2)).unwrap()],
        integer(10),
    )
    .unwrap()
}

fn pocket(center_mm: Point, radius_mm: f64, depth_mm: f64, end: bool) -> CircularPocket {
    CircularPocket {
        center_mm,
        radius_mm,
        depth_mm,
        end,
    }
}

fn close(value: f64, expected: f64) {
    assert!((value - expected).abs() < 1e-9, "{value} != {expected}");
}

#[test]
fn blind_recesses_preserve_exact_cap_centers_and_volume() {
    let stock = rectangle();
    let pockets = [
        pocket([5., 5.], 2., 3., false),
        pocket([5., 5.], 1., 4., true),
        pocket([15., 13.], 1., 2., true),
    ];
    validate(&stock, &pockets, 1e-7, 1e-5).unwrap();
    assert_eq!(
        [pockets[0].mouth_z_mm(10.), pockets[1].mouth_z_mm(10.)],
        [0., 10.]
    );
    assert_eq!(
        [pockets[0].floor_z_mm(10.), pockets[1].floor_z_mm(10.)],
        [3., 6.]
    );
    let base_area = 314. - 4. * PI;
    close(volume_mm3(&stock, &pockets), 10. * base_area - 18. * PI);
    let moment = [3140. - 60. * PI, 2527. - 32. * PI];
    for (end, radius_squared, weighted_center) in [(false, 4., [20., 20.]), (true, 2., [20., 18.])]
    {
        let (area, centroid) = cap_properties(&stock, &pockets, end);
        close(area, base_area - radius_squared * PI);
        for i in 0..2 {
            close(centroid[i], (moment[i] - PI * weighted_center[i]) / area);
        }
    }
}

#[test]
fn refuse_invalid_dimensions_contact_and_insufficient_axial_material() {
    let stock = rectangle();
    let source = pocket([5., 5.], 2., 3., false);
    for invalid in [
        pocket([f64::NAN, 5.], 2., 3., false),
        pocket([5., 5.], f64::INFINITY, 3., false),
        pocket([5., 5.], 0., 3., false),
        pocket([5., 5.], 0.00001, 3., false),
        pocket([5., 5.], 2., f64::NAN, false),
        pocket([5., 5.], 2., 0., false),
        pocket([5., 5.], 2., 0.00001, false),
        pocket([5., 5.], 2., 10., false),
    ] {
        assert_eq!(
            validate(&stock, &[invalid], 1e-7, 1e-4),
            Err(PocketError::InvalidInput)
        );
    }
    for (clearance, minimum) in [(f64::NAN, 1.), (-1., 1.), (0., f64::NAN), (0., 0.)] {
        assert_eq!(
            validate(&stock, &[source], clearance, minimum),
            Err(PocketError::InvalidInput)
        );
    }
    for rejected in [
        pocket([f64::MAX, 0.], 1., 2., false),
        pocket([5., 5.], f64::MAX, 2., false),
        pocket([5., 5.], 1e-14, 2., false),
        pocket([5., 5.], 1., 9.999_999_99, false),
        pocket([2., 5.], 2., 2., false),
        pocket([1., -1.], 0.2, 2., false),
        pocket([10., 5.], 0.2, 2., false),
        pocket([12., 8.], 1., 2., false),
    ] {
        assert_eq!(
            validate(&stock, &[rejected], 1e-7, 1e-15),
            Err(PocketError::UnresolvedBoundary)
        );
    }
    for conflict in [
        pocket([5., 5.], 1., 4., false),
        pocket([5., 5.], 1., 7., true),
        pocket([5., 5.], 1., 8., true),
        pocket([5., 8.], 1., 8., true),
    ] {
        assert_eq!(
            validate(&stock, &[source, conflict], 1e-7, 1e-5),
            Err(PocketError::UnresolvedBoundary)
        );
    }
    assert_eq!(
        validate(&stock, &[source; MAX_VERTICES], 1e-7, 1e-5),
        Err(PocketError::BudgetExceeded)
    );
}

#[test]
fn cap_center_uses_local_moments_and_degenerate_projection_is_bounded() {
    let origin = 1_000_000_i64;
    let stock = PolygonPrism::new(
        vec![
            vec![[0, 0], [20, 0], [20, 16], [0, 16]]
                .into_iter()
                .map(|point| point.map(|x| integer(origin + x)))
                .collect(),
        ],
        integer(10),
    )
    .unwrap();
    let origin = origin as f64;
    let pockets = [pocket([origin + 5., origin + 5.], 2., 3., true)];
    validate(&stock, &pockets, 1e-7, 1e-5).unwrap();
    let (_, centroid) = cap_properties(&stock, &pockets, true);
    close(centroid[0] - origin, (3200. - 20. * PI) / (320. - 4. * PI));
    close(centroid[1] - origin, (2560. - 20. * PI) / (320. - 4. * PI));
    close(segment_distance([3., 4.], [3., 4.]), 5.);
}

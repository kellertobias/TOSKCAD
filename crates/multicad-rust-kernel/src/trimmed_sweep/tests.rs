use super::*;

fn rectangle() -> Vec<Uv> {
    vec![[0., 0.], [10., 0.], [10., 8.], [0., 8.]]
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() <= 1e-10, "{actual} vs {expected}");
}

#[test]
fn sloped_caps_on_an_oblique_prism_preserve_the_hole_and_analytic_properties() {
    let hole = vec![[2., 2.], [2., 4.], [4., 4.], [4., 2.]];
    let rings = vec![rectangle(), hole];
    let direction = [0.6, 0., 0.8];
    let travels = [[-2., 0.2, 0.], [4., 0., 0.1]];
    let sweep = TrimmedSweep::new(rings.clone(), direction, travels, 1.).unwrap();
    assert_eq!(sweep.rings(), rings);
    assert_eq!(sweep.direction(), direction);
    assert_eq!(sweep.travel_coefficients(), travels);
    close(sweep.travel(false, [10., 0.]), 0.);
    close(sweep.travel(true, [0., 8.]), 4.8);
    close(sweep.volume_mm3(), 327.36);
    for (end, expected_cross) in [(false, [0.16, 0., -1.12]), (true, [0., -0.08, 1.])] {
        let cross_norm = norm(expected_cross);
        close(sweep.cap_area_mm2(end), 76. * cross_norm);
        for (actual, expected) in sweep.cap_normal(end).into_iter().zip(expected_cross) {
            close(actual, expected / cross_norm);
        }
        let [x, y] = [388. / 76., 308. / 76.];
        let t = if end { 4. + 0.1 * y } else { -2. + 0.2 * x };
        for (actual, expected) in sweep
            .cap_centroid(end)
            .into_iter()
            .zip([x + 0.6 * t, y, 0.8 * t])
        {
            close(actual, expected);
        }
    }
    close(sweep.support([1., 0., 0.]), 12.88);
    close(sweep.support([-1., 0., 0.]), 1.2);
    close(sweep.support([0., 0., -2.]), 3.2);
    assert_eq!(sweep.support([0.; 3]), 0.);
    assert!(sweep.support([f64::INFINITY, 0., 0.]).is_nan());
    let vertex = sweep.point(true, [10., 8.]);
    assert!(sweep.contains(vertex, 1e-12));
    assert!(sweep.contains([5., 5., 0.5], 0.));
    assert!(!sweep.contains([3., 3., 0.5], 0.));
    assert!(!sweep.contains([5., 5., 9.], 0.));
}

#[test]
fn containment_uses_cap_and_side_distances_in_world_units() {
    let direction = [0.6, 0., 0.8];
    let sweep =
        TrimmedSweep::new(vec![rectangle()], direction, [[0.; 3], [4., 0., 0.]], 4.).unwrap();
    // The point's UV is 0.1 outside the side. Perpendicular to the
    // generator it is 0.08 from the side, even though the same-Z gap is 0.1.
    let outside_side = [1.1, 4., 1.6];
    assert!(sweep.contains(outside_side, 0.081));
    assert!(!sweep.contains(outside_side, 0.079));
    assert!(sweep.contains([5., 4., 3.25], 0.051));
    assert!(!sweep.contains([5., 4., 3.25], 0.049));
    assert!(!sweep.contains([f64::NAN, 0., 0.], 0.));
    assert!(!sweep.contains([5., 4., 2.], f64::INFINITY));
    assert!(!sweep.contains([5., 4., 2.], -0.1));
    assert!(!sweep.contains([0., 0., f64::MAX], f64::MAX));
}

#[test]
fn concave_sections_can_use_a_plane_that_crosses_their_unused_bounding_box() {
    let ring = vec![[0., 0.], [6., 0.], [6., 2.], [2., 2.], [2., 6.], [0., 6.]];
    let sweep =
        TrimmedSweep::new(vec![ring], [0., 0., 1.], [[0.; 3], [10., -1., -1.]], 1.).unwrap();
    // Integrate 10 - x - y over 6x2 and 2x4 rectangles.
    close(sweep.volume_mm3(), 112.);
    assert!(sweep.contains([5., 1., 3.], 0.));
    assert!(!sweep.contains([3., 3., 1.], 0.));
}

#[test]
fn large_profile_translation_preserves_the_sloped_volume_and_support() {
    let [x, y] = [5e6, -5e6];
    let shifted = rectangle()
        .into_iter()
        .map(|[u, v]| [u + x, v + y])
        .collect();
    let direction = [0.6, 0., 0.8];
    let base = [2., 0.125, -0.125];
    let end = [8., -0.125, 0.25];
    let local = TrimmedSweep::new(vec![rectangle()], direction, [base, end], 1.).unwrap();
    let shifted = TrimmedSweep::new(
        vec![shifted],
        direction,
        [base, end].map(|[c, a, b]| [c - a * x - b * y, a, b]),
        1.,
    )
    .unwrap();
    assert!((shifted.volume_mm3() - local.volume_mm3()).abs() < 1e-8);
    assert!((shifted.support([1., 0., 0.]) - x - local.support([1., 0., 0.])).abs() < 1e-8);
    assert!((shifted.support([0., 1., 0.]) - y - local.support([0., 1., 0.])).abs() < 1e-8);
    close(shifted.support([0., 0., 1.]), local.support([0., 0., 1.]));
}

#[test]
fn rejects_crossing_pinched_and_unprovable_caps() {
    let dir = [0., 0., 1.];
    let clipped = |cap| TrimmedSweep::new(vec![rectangle()], dir, [[0.; 3], cap], 0.1);
    assert_eq!(clipped([5., -1., 0.]).unwrap_err(), Error::InvalidExtent);
    assert_eq!(clipped([10., -1., 0.]).unwrap_err(), Error::InvalidExtent);
    assert_eq!(clipped([0.05, 0., 0.]).unwrap_err(), Error::InvalidExtent);
    assert_eq!(
        clipped([f64::NAN, 0., 0.]).unwrap_err(),
        Error::InvalidExtent
    );
    assert_eq!(
        clipped([1., f64::MAX, 0.]).unwrap_err(),
        Error::InvalidExtent
    );
    let shifted = rectangle()
        .into_iter()
        .map(|[x, y]| [x + 1_000_000., y])
        .collect();
    assert_eq!(
        TrimmedSweep::new(
            vec![shifted],
            dir,
            [[1., -1., 0.], [1. + 1e-9, -1., 0.]],
            1e-12
        )
        .unwrap_err(),
        Error::InvalidExtent
    );
    assert_eq!(
        TrimmedSweep::new(vec![rectangle()], dir, [[0.; 3], [1., 0., 0.]], 0.).unwrap_err(),
        Error::InvalidExtent
    );
}

#[test]
fn rejects_invalid_profiles_directions_and_coordinate_budgets() {
    let travels = [[0.; 3], [1., 0., 0.]];
    let dir = [0., 0., 1.];
    let clipped = |rings| TrimmedSweep::new(rings, dir, travels, 0.1);
    for direction in [
        [f64::NAN, 0., 1.],
        [0., 0., 2.],
        [1., 0., 0.],
        [0., 0., -1.],
    ] {
        assert_eq!(
            TrimmedSweep::new(vec![rectangle()], direction, travels, 0.1).unwrap_err(),
            Error::InvalidDirection
        );
    }
    for rings in [
        vec![],
        vec![vec![[0.; 2]; 2]],
        vec![vec![[f64::NAN, 0.]; 3]],
        vec![rectangle().into_iter().rev().collect()],
    ] {
        assert_eq!(clipped(rings).unwrap_err(), Error::InvalidProfile);
    }
    for rings in [
        vec![vec![[0.; 2]; MAX_VERTICES + 1]],
        vec![vec![[2e7, 0.]; 3]],
    ] {
        assert_eq!(clipped(rings).unwrap_err(), Error::Limit);
    }
    assert_eq!(
        TrimmedSweep::new(vec![rectangle()], dir, [[0.; 3], [2e7, 0., 0.]], 0.1).unwrap_err(),
        Error::Limit
    );
    let near_limit = rectangle()
        .into_iter()
        .map(|[x, y]| [x + MAX_COORDINATE_MM - 10., y])
        .collect();
    assert_eq!(
        TrimmedSweep::new(vec![near_limit], [0.6, 0., 0.8], travels, 0.1).unwrap_err(),
        Error::Limit
    );
}

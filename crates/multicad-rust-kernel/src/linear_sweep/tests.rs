use super::*;

fn rectangle(lo: Point, hi: Point) -> Vec<Point> {
    vec![lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]]
}

fn hole(lo: Point, hi: Point) -> Vec<Point> {
    let mut ring = rectangle(lo, hi);
    ring.reverse();
    ring
}

fn translate(rings: &[Vec<Point>], [x, y]: Point) -> Vec<Vec<Point>> {
    rings
        .iter()
        .map(|ring| ring.iter().map(|&[u, v]| [u + x, v + y]).collect())
        .collect()
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-10, "{actual} != {expected}");
}

#[test]
fn directed_sweep_preserves_concavity_holes_order_and_cap_properties() {
    let base = vec![
        vec![
            [0., 0.],
            [6., 0.],
            [6., 5.],
            [4., 5.],
            [4., 3.],
            [2., 3.],
            [2., 5.],
            [0., 5.],
        ],
        hole([0.5, 1.], [1.5, 2.]),
    ];
    let top = translate(&base, [2.5, -4.]);
    let solid = Sweep::new(base.clone(), top.clone(), 7.).unwrap();
    assert_eq!(solid.base(), base);
    assert_eq!(solid.top(), top);
    assert_eq!(solid.height_mm(), 7.);
    assert_eq!(solid.cap_area_mm2(false), 25.);
    assert_eq!(solid.cap_area_mm2(true), 25.);
    near(solid.cap_centroid(false)[0], 77. / 25.);
    near(solid.cap_centroid(false)[1], 2.3);
    near(solid.cap_centroid(true)[0], 77. / 25. + 2.5);
    near(solid.cap_centroid(true)[1], -1.7);
    near(solid.volume_mm3(), 175.);
    near(solid.support([1., -1., 0.5]), 16.);
    assert_eq!(solid.support([0.; 3]), 0.);
    assert!(solid.support([f64::NAN, 0., 0.]).is_nan());
    assert!(solid.contains([5. + 1.25, 4. - 2., 3.5], 0.));
    assert!(!solid.contains([3. + 1.25, 4. - 2., 3.5], 0.));
    assert!(!solid.contains([1. + 1.25, 1.5 - 2., 3.5], 0.));
    assert!(solid.contains([0.5 + 1.25, 1.5 - 2., 3.5], 0.));
    assert!(solid.contains([0., 0., -0.01], 0.02));
    assert!(!solid.contains([0., 0., -0.01], 0.));
    assert!(!solid.contains([2.5, -4., 7.01], 0.));
    assert!(!solid.contains([0., 0., 0.], -1.));
    assert!(!solid.contains([f64::NAN, 0., 0.], 0.));
}

#[test]
fn draft_integrates_ruled_material_and_keeps_two_shrinking_holes() {
    let base = vec![
        rectangle([0., 0.], [8., 8.]),
        hole([1.5, 1.5], [2.5, 2.5]),
        hole([5.5, 5.5], [6.5, 6.5]),
    ];
    let top = translate(
        &[
            rectangle([-1., -1.], [9., 9.]),
            hole([1.75, 1.75], [2.25, 2.25]),
            hole([5.75, 5.75], [6.25, 6.25]),
        ],
        [1., -2.],
    );
    let solid = Sweep::new(base, top, 6.).unwrap();
    near(solid.cap_area_mm2(false), 62.);
    near(solid.cap_area_mm2(true), 99.5);
    assert_eq!(solid.cap_centroid(false), [4., 4.]);
    assert_eq!(solid.cap_centroid(true), [5., 2.]);
    near(solid.volume_mm3(), 481.);
    assert!(solid.contains([4.5, 3., 3.], 1e-8));
    assert!(!solid.contains([6.5, 5., 3.], 1e-8));
    assert!(solid.contains([0.1, -1.4, 3.], 1e-8));
    near(solid.support([2., 1., 1.]), 33.);
}

#[test]
fn draft_rejects_a_cap_pair_whose_holes_exchange_positions() {
    let outside = rectangle([0., 0.], [8., 8.]);
    let a = hole([1., 1.], [2., 2.]);
    let b = hole([6., 6.], [7., 7.]);
    assert_eq!(
        Sweep::new(
            vec![outside.clone(), a.clone(), b.clone()],
            vec![outside, b, a],
            3.
        )
        .unwrap_err(),
        Error::InvalidProfile
    );
}

#[test]
fn moving_edge_support_checks_the_interior_minimum() {
    let a = [[0., 0.]; 2];
    let b = [[1., 0.], [0., 1.]];
    let crossing = [[0., 1.], [-1., -4.]];
    assert!(!side_is_strict(a, b, crossing, 1., 0.));
    let stable = [[0., 1.], [-1., 1.]];
    assert!(side_is_strict(a, b, stable, 1., 0.));
    assert!(!side_is_strict(a, b, [[0., 0.], [-1., 1.]], 1., 0.));
    // Large changes in edge length do not shrink the accepted clearance
    // at the smaller end of a valid sweep.
    let shrinking = [[10., 0.], [0.1, 0.]];
    assert!(side_is_strict(a, shrinking, [[0., 0.5]; 2], 1., 0.1));
}

#[test]
fn draft_requires_strictly_convex_wires_and_contained_hole_trajectories() {
    let notch = vec![vec![
        [0., 0.],
        [6., 0.],
        [6., 6.],
        [3., 6.],
        [3., 3.],
        [0., 3.],
    ]];
    let larger = vec![notch[0].iter().map(|p| p.map(|v| 2. * v)).collect()];
    assert_eq!(
        Sweep::new(notch, larger, 3.).unwrap_err(),
        Error::InvalidProfile
    );

    let outside = rectangle([0., 0.], [8., 8.]);
    let inside = hole([2., 2.], [3., 3.]);
    let escaped = hole([9., 2.], [10., 3.]);
    assert!(!valid_draft(
        &[outside.clone(), inside],
        &[outside, escaped],
        0.
    ));
}

#[test]
fn invalid_geometry_and_numerical_limits_fail_explicitly() {
    let simple = vec![rectangle([0., 0.], [4., 5.])];
    let check = |base, top, height| Sweep::new(base, top, height).unwrap_err();
    for height in [f64::INFINITY, 0., -1.] {
        assert_eq!(
            check(simple.clone(), simple.clone(), height),
            Error::InvalidHeight
        );
    }
    assert_eq!(
        check(simple.clone(), simple.clone(), 1e7 + 1.),
        Error::Limit
    );
    for bad in [
        vec![],
        vec![vec![[0., 0.], [0., 1.]]],
        vec![vec![[0., 0.], [1., 0.], [0., 1.]]],
        vec![vec![[0., 0.], [f64::NAN, 0.], [4., 5.], [0., 5.]]],
        vec![vec![[0., 0.], [4., 5.], [4., 0.], [0., 5.]]],
        vec![hole([0., 0.], [4., 5.])],
        vec![vec![[0., 0.], [4., 0.], [3., 5.], [0., 5.]]],
    ] {
        assert_eq!(check(simple.clone(), bad, 1.), Error::InvalidProfile);
    }
    assert_eq!(
        check(simple.clone(), translate(&simple, [1e7, 0.]), 1.),
        Error::Limit
    );
    let cap = vec![rectangle([0., 0.], [0.1, 0.1])];
    assert_eq!(
        check(cap.clone(), cap, f64::from_bits(1)),
        Error::InvalidProfile
    );
    for moments in [[0.; 3], [1., f64::NAN, 0.]] {
        assert_eq!(
            Cap::from_moments(moments, [0.; 2]).unwrap_err(),
            Error::InvalidProfile
        );
    }
}

#[test]
fn translated_thin_profile_keeps_volume_after_large_offset() {
    let base = vec![rectangle([0., 0.], [8., 0.03125])];
    let top = translate(&base, [4e6, -2e6]);
    let solid = Sweep::new(base, top, 2.).unwrap();
    near(solid.volume_mm3(), 0.5);
    assert!(solid.contains([2e6 + 5., -1e6 + 0.015625, 1.], 0.));
    assert!(!solid.contains([2e6 + 5., -1e6 + 0.04, 1.], 0.));
}

#[test]
fn containment_uses_the_exact_coordinates_at_either_cap() {
    let base = vec![rectangle([8e6, 0.], [8e6 + 2., 1.])];
    let top = vec![rectangle([0.1, 0.], [2.1, 1.])];
    let solid = Sweep::new(base, top, 2.).unwrap();
    assert!(solid.contains([8e6, 0.5, 0.], 0.));
    assert!(solid.contains([0.1, 0.5, 2.], 0.));
    assert!(!solid.contains([0.1 - 1e-12, 0.5, 2.], 0.));
}

#[test]
fn explicit_profile_budget_preserves_the_complete_large_manufacturing_boundary() {
    let mut ring = vec![[0., 0.], [4093., 0.]];
    ring.extend((0..4094).rev().map(|i| [i as f64, 4. + (i % 2) as f64]));
    assert_eq!(ring.len(), 4096);
    let base = vec![ring];
    assert_eq!(
        Sweep::new(base.clone(), base.clone(), 2.).unwrap_err(),
        Error::Limit
    );
    let solid = Sweep::with_profile_budget(base.clone(), base.clone(), 2., 4096).unwrap();
    assert_eq!(solid.base(), base);
    assert_eq!(solid.top(), base);
    assert_eq!(solid.base()[0].len(), 4096);
    let twice_area = base[0]
        .iter()
        .zip(base[0].iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - a[1] * b[0])
        .sum::<f64>();
    near(solid.volume_mm3(), twice_area);
    let mesh = crate::linear_sweep_mesh::tessellate(
        &solid,
        crate::display::Tessellation {
            deflection: 0.1,
            angle: 0.3,
            triangle_limit: 200_000,
        },
    )
    .unwrap();
    assert_eq!(mesh.vertices.len(), 2 * 4096 * 3);
    assert_eq!(mesh.indices.len() / 3, 4 * 4096 - 4);
    let mut too_many = base;
    too_many[0].push([-1., 2.]);
    assert_eq!(
        Sweep::with_profile_budget(too_many.clone(), too_many, 2., 4096).unwrap_err(),
        Error::Limit
    );
    for budget in [0, 4097] {
        assert_eq!(
            Sweep::with_profile_budget(
                vec![rectangle([0., 0.], [2., 3.])],
                vec![rectangle([0., 0.], [2., 3.])],
                1.,
                budget
            )
            .unwrap_err(),
            Error::Limit
        );
    }
}

use super::*;

#[test]
fn rotated_stadium_has_exact_area_bounds_and_tangencies() {
    let profile = Stadium::new([[1., 2.], [4., 6.]], 2.).unwrap();
    assert_eq!(profile.centers(), [[1., 2.], [4., 6.]]);
    assert_eq!(profile.radius_mm(), 2.);
    assert_eq!(profile.spine_length_mm(), 5.);
    assert_eq!(profile.area_mm2(), 20. + 4. * PI);
    assert_eq!(profile.centroid(), [2.5, 4.]);
    for (actual, expected) in
        profile
            .corners()
            .into_iter()
            .zip([[2.6, 0.8], [5.6, 4.8], [2.4, 7.2], [-0.6, 3.2]])
    {
        assert!(length(subtract(actual, expected)) < 1e-14);
    }
    assert_eq!(profile.support([1., 0.]), 6.);
    assert_eq!(profile.support([0., 1.]), 8.);
    assert_eq!(profile.support([0., -2.]), 0.);
}

#[test]
fn rejects_degenerate_and_unrepresentable_profiles() {
    for (centers, radius) in [
        ([[0., 0.], [1., 0.]], 0.),
        ([[0., 0.], [1., 0.]], f64::NAN),
        ([[0., 0.], [0., 0.]], 1.),
        ([[0., 0.], [f64::INFINITY, 0.]], 1.),
        ([[0., 0.], [10., 0.]], 1e308),
        ([[0., 0.], [1e-200, 0.]], 1e-200),
        ([[1e16, 1e16], [1e16 + 4., 1e16]], 0.1),
        ([[0., 0.], [1e16, 1e16]], 0.1),
    ] {
        assert_eq!(
            Stadium::new(centers, radius),
            Err(PocketError::InvalidInput)
        );
    }
}

#[test]
fn segment_distance_handles_interior_crossings_contacts_and_points() {
    let segment = [[0., 0.], [6., 0.]];
    assert_eq!(segments_distance(segment, [[3., 1.], [3., -2.]]), 0.);
    assert_eq!(segments_distance(segment, [[3., 0.], [7., 0.]]), 0.);
    assert_eq!(segments_distance(segment, [[2., 4.], [2., 4.]]), 4.);
    assert_eq!(segments_distance(segment, [[2., 4.], [5., 4.]]), 4.);
    assert_eq!(segments_distance(segment, [[9., 4.], [9., 6.]]), 5.);
}

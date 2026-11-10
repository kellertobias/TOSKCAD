use super::*;

const TOLERANCE: f64 = 1e-7;

fn intervals(actual: Vec<Span>, expected: &[Span]) {
    assert_eq!(actual.len(), expected.len(), "{actual:?}");
    for (a, b) in actual.into_iter().flatten().zip(expected.iter().flatten()) {
        assert!((a - b).abs() < 1e-10, "{a} != {b}");
    }
}

fn view_cylinder(bore: Option<f64>) -> RoundOccluder {
    RoundOccluder::new([0.; 3], [0., 0., 1.], 3., 2., bore).unwrap()
}

#[test]
fn looking_through_a_bore_reveals_the_middle_of_a_rear_line() {
    let line = [[-5., 0., -1.], [5., 0., -1.]];
    let solid = view_cylinder(None);
    let annulus = view_cylinder(Some(1.));
    assert_eq!(annulus.bounds(), [[-3., -3., 0.], [3., 3., 2.]]);
    intervals(
        solid.hidden_intervals(line, TOLERANCE).unwrap(),
        &[[0.2, 0.8]],
    );
    intervals(
        annulus.hidden_intervals(line, TOLERANCE).unwrap(),
        &[[0.2, 0.4], [0.6, 0.8]],
    );
    intervals(
        annulus
            .hidden_intervals([line[1], line[0]], TOLERANCE)
            .unwrap(),
        &[[0.2, 0.4], [0.6, 0.8]],
    );
    let bore_tangent = [[-2., 1., -1.], [2., 1., -1.]];
    intervals(
        annulus.hidden_intervals(bore_tangent, TOLERANCE).unwrap(),
        &[[0., 1.]],
    );
    let crossing = [[-0.5, 0., -1.], [0.5, 0., 3.]];
    intervals(
        solid.hidden_intervals(crossing, TOLERANCE).unwrap(),
        &[[0., (3. - TOLERANCE) / 4.]],
    );
    for visible in [
        [[-2., 3., -1.], [2., 3., -1.]], // Exact outer silhouette.
        [[-1., 0., 2.], [1., 0., 2.]],   // Coincident with the near cap.
        [[0., 0., -1.], [0., 0., -2.]],  // Edge-on source.
        [[-5., 4., -1.], [5., 4., -1.]], // Outside the stock bounds.
    ] {
        assert!(
            solid
                .hidden_intervals(visible, TOLERANCE)
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn a_side_view_splits_at_the_cap_and_the_cylindrical_wall_depth() {
    let solid = RoundOccluder::new([0.; 3], [1., 0., 0.], 3., 10., None).unwrap();
    let depth = (8_f64.sqrt() + 2. - TOLERANCE) / 8.;
    intervals(
        solid
            .hidden_intervals([[-2., 1., -2.], [12., 1., 6.]], TOLERANCE)
            .unwrap(),
        &[[1. / 7., depth]],
    );
    intervals(
        solid
            .hidden_intervals([[-2., 1., -5.], [12., 1., -5.]], TOLERANCE)
            .unwrap(),
        &[[1. / 7., 6. / 7.]],
    );
    for tangent in [
        [[0., -2., -5.], [0., 2., -5.]], // Along the cap plane.
        [[2., 3., -5.], [8., 3., -5.]],  // Along the cylindrical wall silhouette.
    ] {
        assert!(
            solid
                .hidden_intervals(tangent, TOLERANCE)
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn a_rotated_line_coincident_with_the_outer_silhouette_stays_visible() {
    for degrees in [25_f64, 53.] {
        let (sin, cos) = degrees.to_radians().sin_cos();
        let axis = [cos, sin, 0.];
        let solid = RoundOccluder::new([0.; 3], axis, 3., 10., None).unwrap();
        let line = |offset: f64| {
            [2., 8.].map(|along| {
                [
                    axis[0] * along - axis[1] * offset,
                    axis[1] * along + axis[0] * offset,
                    -5.,
                ]
            })
        };
        for sign in [-1., 1.] {
            assert!(
                solid
                    .hidden_intervals(line(sign * 3.), TOLERANCE)
                    .unwrap()
                    .is_empty()
            );
            intervals(
                solid
                    .hidden_intervals(line(sign * (3. - 1e-6)), TOLERANCE)
                    .unwrap(),
                &[[0., 1.]],
            );
        }
    }
}

#[test]
fn oblique_round_stock_uses_the_cap_ellipse_and_bore_wall() {
    let solid = RoundOccluder::new([0.; 3], [0.8, 0., 0.6], 2., 6., None).unwrap();
    intervals(
        solid
            .hidden_intervals([[-3., 0., -10.], [9., 0., -10.]], TOLERANCE)
            .unwrap(),
        &[[1.8 / 12., 9. / 12.]],
    );

    let bore_edge = (1_f64 - (0.3_f64 / 0.8).powi(2)).sqrt();
    let line = [[0.3, -4., -5.], [0.3, 4., -5.]];
    for (base, axis) in [
        ([0.; 3], [0.6, 0., 0.8]),
        ([0.6, 0., 0.8], [-0.6, 0., -0.8]),
    ] {
        let annulus = RoundOccluder::new(base, axis, 3., 1., Some(1.)).unwrap();
        intervals(
            annulus.hidden_intervals(line, TOLERANCE).unwrap(),
            &[
                [0.125, (4. - bore_edge) / 8.],
                [(4. + bore_edge) / 8., 0.875],
            ],
        );
    }
}

#[test]
fn checks_inputs_before_classifying_visibility() {
    for (base, axis, radius, height, bore) in [
        ([f64::NAN, 0., 0.], [0., 0., 1.], 3., 2., None),
        ([0.; 3], [1.; 3], 3., 2., None),
        ([0.; 3], [0., 0., 1.], 0., 2., None),
        ([0.; 3], [0., 0., 1.], 3., 0., None),
        ([0.; 3], [0., 0., 1.], 3., 2., Some(3.)),
    ] {
        assert!(matches!(
            RoundOccluder::new(base, axis, radius, height, bore),
            Err(ProjectionError::Invalid)
        ));
    }
    let solid = view_cylinder(None);
    let line = [[-5., 0., -1.], [5., 0., -1.]];
    assert_eq!(
        solid.hidden_intervals(line, 0.),
        Err(ProjectionError::Invalid)
    );
    let invalid = [[1e9, 0., -1.], [5., 0., -1.]];
    assert_eq!(
        solid.hidden_intervals(invalid, TOLERANCE),
        Err(ProjectionError::Invalid)
    );
}

#[test]
fn separate_certified_side_on_occurrences_hide_coincident_closed_contours() {
    use crate::planar_projection::{Options, project_scene};

    let band = |depth| RoundOccluder::new([0., 0., depth], [1., 0., 0.], 3., 5., None).unwrap();
    let [near, far] = [
        band(10.).with_contour_owner(7),
        band(0.).with_contour_owner(8),
    ];
    assert!(near.closes_contour_of(Some(&far)));
    assert!(!near.closes_contour_of(Some(&band(0.).with_contour_owner(7))));
    assert!(!near.closes_contour_of(Some(&band(0.))));
    assert!(!near.closes_contour_of(None));
    assert!(
        !view_cylinder(None)
            .with_contour_owner(7)
            .closes_contour_of(Some(&far))
    );
    let annulus = RoundOccluder::new([0.; 3], [1., 0., 0.], 3., 5., Some(1.)).unwrap();
    assert!(!annulus.with_contour_owner(7).closes_contour_of(Some(&far)));

    let contour = [[-2., 3., 0.], [7., 3., 0.]];
    let mut hidden = Vec::new();
    near.hide(contour, TOLERANCE, true, |span| hidden.push(span));
    intervals(hidden, &[[2. / 9., 7. / 9.]]);
    let options = Options {
        hidden_lines: true,
        tolerance: TOLERANCE,
        tangent_edges: false,
        detail: None,
        segment_limit: 100,
    };
    let (_, edges) = project_scene(&[], &[near, far], options).unwrap();
    assert!(
        edges
            .iter()
            .all(|edge| edge.source == 0 || edge.edge.hidden)
    );
    assert_eq!(
        edges
            .iter()
            .filter(|edge| edge.source == 0 && !edge.edge.hidden)
            .count(),
        4
    );
    assert_eq!(
        edges
            .iter()
            .filter(|edge| edge.source == 1 && edge.edge.hidden)
            .count(),
        6
    );
}

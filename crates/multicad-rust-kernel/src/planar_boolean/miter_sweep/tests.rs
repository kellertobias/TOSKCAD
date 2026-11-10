use super::*;

const TOLERANCE: f64 = 1e-7;
const SQUARE: [[f64; 2]; 4] = [[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]];
const RIGHT_ANGLE: [Point; 3] = [[0.; 3], [0., 0., 12.], [8., 0., 12.]];

#[test]
fn miter_joins_an_asymmetric_section_with_outward_caps_and_authored_faces() {
    let section = [[-2., -1.], [1., -1.], [1., 3.], [-2., 3.]];
    for section in [section, section.map(|[x, y]| [x, 2. - y])] {
        let (geometry, pieces) = miter_sweep(&section, &RIGHT_ANGLE, TOLERANCE).unwrap();
        assert_eq!(pieces.len(), 2);
        assert_eq!(geometry.solid_count(), 1);
        assert_eq!(geometry.shell_count(), 1);
        // Both legs have half a millimeter extra length at the section centroid.
        assert!((geometry.volume_mm3() - 252.).abs() < TOLERANCE);
        assert_eq!(geometry.bounds(), [[-2., -1., 0.], [8., 3., 14.]]);
        for (i, [x, y]) in section.into_iter().enumerate() {
            assert_eq!(geometry.vertices()[i], [x, y, 0.]);
            assert_eq!(geometry.vertices()[i + 4], [x, y, 12. - x]);
            assert_eq!(geometry.vertices()[i + 8], [8., y, 12. - x]);
        }
        assert_eq!(geometry.faces().len(), 10);
        for (leg, cap, normal) in [(0, 0, [0., 0., -1.]), (1, 1, [1., 0., 0.])] {
            let face = geometry
                .faces()
                .iter()
                .find(|face| (face.source, face.source_face) == (leg, cap))
                .unwrap();
            assert_eq!(face.normal, normal);
        }
        assert!(geometry.faces().iter().all(|face| {
            face.source_face >= 2
                || (face.source, face.source_face) == (0, 0)
                || (face.source, face.source_face) == (1, 1)
        }));
    }
}

#[test]
fn skew_bend_bisects_and_transports_the_section_with_no_rotation_about_its_tangent() {
    let path = [[0.; 3], [0., 0., 12.], [3., 0., 16.]];
    let (geometry, _) = miter_sweep(&SQUARE, &path, TOLERANCE).unwrap();
    for (i, [x, y]) in SQUARE.into_iter().enumerate() {
        let joint = geometry.vertices()[i + 4];
        let end = geometry.vertices()[i + 8];
        assert!(norm(sub(joint, [x, y, 12. - x / 3.])) < TOLERANCE);
        assert!(norm(sub(end, [3. + 0.8 * x, y, 16. - 0.6 * x])) < TOLERANCE);
        let difference = sub(end, joint);
        assert!(norm(cross(difference, [0.6, 0., 0.8])) < TOLERANCE);
    }
    assert!((geometry.volume_mm3() - 68.).abs() < TOLERANCE);
}

#[test]
fn several_corners_and_collinear_segments_preserve_a_single_shell() {
    let path = [
        [0.; 3],
        [0., 0., 4.],
        [0., 0., 8.],
        [9., 0., 8.],
        [9., 10., 8.],
        [9., 10., 19.],
    ];
    let (geometry, pieces) = miter_sweep(&SQUARE, &path, TOLERANCE).unwrap();
    assert_eq!(geometry.solid_count(), 1);
    assert_eq!(geometry.shell_count(), 1);
    assert_eq!(geometry.faces().len(), 22);
    assert_eq!(pieces.len(), 5);
    assert!((geometry.volume_mm3() - 152.).abs() < TOLERANCE);
}

#[test]
fn limits_and_invalid_geometry_fail_closed() {
    let too_many_points = vec![[0.; 3]; MAX_PATH_POINTS + 1];
    let too_many_vertices = vec![[0.; 2]; MAX_SECTION_VERTICES + 1];
    assert_eq!(
        miter_sweep(&SQUARE, &too_many_points, TOLERANCE).unwrap_err(),
        Error::Limit
    );
    assert_eq!(
        miter_sweep(&too_many_vertices, &RIGHT_ANGLE, TOLERANCE).unwrap_err(),
        Error::Limit
    );
    assert!(miter_sweep(&SQUARE, &RIGHT_ANGLE, 0.).is_err());
    assert!(miter_sweep(&SQUARE[..2], &RIGHT_ANGLE, TOLERANCE).is_err());
    for path in [
        &RIGHT_ANGLE[..1],
        &[[0., 0., 1.], [0., 0., 2.]],
        &[[0.; 3], [1., 0., 2.]],
        &[[0.; 3], [0., 0., -2.]],
        &[[0.; 3], [0., 0., 12.], [0., f64::NAN, 2.]],
        &[[0.; 3], [0., 0., 12.], [0., 0., 12.]],
        &[[0.; 3], [0., 0., 12.], [0., 0., 3.]],
        &[[0.; 3], [0., 0., 0.5], [8., 0., 0.5]],
    ] {
        assert_eq!(
            miter_sweep(&SQUARE, path, TOLERANCE).unwrap_err(),
            Error::Invalid
        );
    }
    let concave = [[-2., -1.], [1., -1.], [0., 0.], [1., 3.], [-2., 3.]];
    assert_eq!(
        miter_sweep(&concave, &RIGHT_ANGLE, TOLERANCE).unwrap_err(),
        Error::Invalid
    );
    let crossing = [
        [0.; 3],
        [0., 0., 12.],
        [12., 0., 12.],
        [12., 0., 1.],
        [-4., 0., 1.],
    ];
    assert_eq!(
        miter_sweep(&SQUARE, &crossing, TOLERANCE).unwrap_err(),
        Error::Unresolved
    );
}

#[test]
fn overlapping_bounds_can_be_certified_by_a_supporting_plane_within_budget() {
    let angle = std::f64::consts::FRAC_1_SQRT_2;
    let first = ConvexPiece::from_box([-2., -0.5, 0.], [2., 0.5, 1.], TOLERANCE).unwrap();
    let a = first
        .transformed(|[x, y, z]| [angle * (x - y), angle * (x + y), z], TOLERANCE)
        .unwrap();
    let b = a
        .transformed(|[x, y, z]| [x - 1.5 * angle, y + 1.5 * angle, z], TOLERANCE)
        .unwrap();
    let a_points: Vec<_> = a.faces.iter().flatten().copied().collect();
    let b_points: Vec<_> = b.faces.iter().flatten().copied().collect();
    assert!(clipping::bounds_overlap(a.bounds, b.bounds, TOLERANCE));
    let mut budget = MAX_WORK;
    prove_separation(&a, &a_points, &b, &b_points, TOLERANCE, &mut budget).unwrap();
    assert_eq!(
        prove_separation(&a, &a_points, &b, &b_points, TOLERANCE, &mut 0),
        Err(Error::Limit)
    );
}

use super::*;

const TOL: f64 = 1e-7;

fn block(min: Point, max: Point) -> ConvexPiece {
    ConvexPiece::from_box(min, max, TOL).unwrap()
}

fn check_volume(actual: f64, expected: f64) {
    assert!((actual - expected).abs() <= 1e-8, "{actual} != {expected}");
}

fn options(triangle_limit: usize) -> Tessellation {
    Tessellation {
        deflection: 0.1,
        angle: 0.2,
        triangle_limit,
    }
}

#[test]
fn overlapping_touching_and_duplicate_boxes_share_one_certified_shell() {
    let pieces = [
        block([0., 0., 0.], [2., 2., 2.]),
        block([1., 0., 0.], [3., 2., 2.]),
        block([3., 0.5, 0.5], [4., 1.5, 1.5]),
        block([0., 0., 0.], [2., 2., 2.]),
    ];
    let union = Union::new(&pieces, TOL).unwrap();
    assert_eq!(union.solid_count(), 1);
    assert_eq!(union.bounds(), [[0., 0., 0.], [4., 2., 2.]]);
    check_volume(union.volume_mm3(), 13.);
    let mesh = union.mesh(options(1_000)).unwrap();
    check_volume(
        manufacturing::certify(&mesh).unwrap().signed_volume_mm3,
        13.,
    );
    assert!(
        union
            .faces()
            .iter()
            .all(|f| f.source != 3 && f.source_face < 6)
    );
    assert!(union.vertices().contains(&[3., 0.5, 0.5]));
    assert_eq!(union.mesh(options(1)), Err(MeshError::BudgetExceeded));
    assert_eq!(union.mesh(options(0)), Err(MeshError::InvalidOptions));
}

#[test]
fn sloped_prism_joins_box_without_erasing_exterior_boundary() {
    let wedge = ConvexPiece::new(
        vec![
            vec![[1., 0., 0.], [3., 0., 0.], [1., 0., 2.]],
            vec![[1., 2., 0.], [1., 2., 2.], [3., 2., 0.]],
            vec![[1., 0., 0.], [1., 2., 0.], [3., 2., 0.], [3., 0., 0.]],
            vec![[1., 0., 0.], [1., 0., 2.], [1., 2., 2.], [1., 2., 0.]],
            vec![[3., 0., 0.], [3., 2., 0.], [1., 2., 2.], [1., 0., 2.]],
        ],
        TOL,
    )
    .unwrap();
    let union = Union::new(&[block([0., 0., 0.], [2., 2., 2.]), wedge], TOL).unwrap();
    assert_eq!(union.solid_count(), 1);
    check_volume(union.volume_mm3(), 9.);
    assert!(
        union
            .faces()
            .iter()
            .any(|face| face.source == 1 && face.normal[0] > 0.7 && face.normal[2] > 0.7)
    );
}

#[test]
fn cavity_and_disconnected_shells_have_correct_oriented_volume() {
    let mut pieces = vec![
        block([0., 0., 0.], [3., 3., 1.]),
        block([0., 0., 2.], [3., 3., 3.]),
        block([0., 0., 1.], [1., 3., 2.]),
        block([2., 0., 1.], [3., 3., 2.]),
        block([1., 0., 1.], [2., 1., 2.]),
        block([1., 2., 1.], [2., 3., 2.]),
    ];
    let hollow = Union::new(&pieces, TOL).unwrap();
    assert_eq!(hollow.solid_count(), 1);
    assert_eq!(hollow.shell_count(), 2);
    check_volume(hollow.volume_mm3(), 26.);
    pieces.push(block([5., 0., 0.], [6., 1., 1.]));
    let joined = Union::new(&pieces, TOL).unwrap();
    assert_eq!(joined.solid_count(), 2);
    check_volume(joined.volume_mm3(), 27.);
}

#[test]
fn invalid_and_ambiguous_inputs_are_refused() {
    let cube = block([0.; 3], [2.; 3]);
    assert_eq!(Union::new(&[], TOL).unwrap_err(), Error::Invalid);
    assert_eq!(Union::new(&[], f64::NAN).unwrap_err(), Error::Invalid);
    assert_eq!(
        Union::new(&vec![cube.clone(); MAX_PIECES + 1], TOL).unwrap_err(),
        Error::Limit
    );
    assert_eq!(
        ConvexPiece::new(Vec::new(), TOL).unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        ConvexPiece::new(vec![Vec::new(); MAX_PIECE_FACES + 1], TOL).unwrap_err(),
        Error::Limit
    );
    assert_eq!(
        ConvexPiece::from_box([2.; 3], [0.; 3], TOL).unwrap_err(),
        Error::Invalid
    );

    let mut inward_face = cube.faces.clone();
    inward_face[0].reverse();
    assert_eq!(
        ConvexPiece::new(inward_face, TOL).unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        ConvexPiece::new(cube.faces[1..].to_vec(), TOL).unwrap_err(),
        Error::Unresolved
    );

    let mut boundary = shell::Boundary::new(TOL);
    let normal = [0., 0., 1.];
    let a = [0., 0., 0.];
    let b = [2., 0., 0.];
    assert_eq!(
        boundary.add(&[a, b, a], normal, 0, 0),
        Err(Error::Unresolved)
    );
    assert_eq!(
        boundary.add(&[[0., 0., TOL / 2.]], normal, 0, 0),
        Err(Error::Unresolved)
    );

    for touching in [block([2., 2., 0.], [3.; 3]), block([2.; 3], [3.; 3])] {
        assert_eq!(
            Union::new(&[cube.clone(), touching], TOL).unwrap_err(),
            Error::Unresolved
        );
    }
    assert_eq!(charge(&mut 1, 2), Err(Error::Limit));
}

#[test]
fn degenerate_warped_and_concave_loops_are_refused() {
    for face in [
        vec![[0., 0., 0.], [1., 0., 0.], [2., 0., 0.]],
        vec![[0., 0., 0.], [1., 0., 0.], [1., 1., 1.], [0., 1., 0.]],
        vec![[0., 0., 0.], [1., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
        vec![
            [0., 0., 0.],
            [2., 0., 0.],
            [1., 0.5, 0.],
            [2., 2., 0.],
            [0., 2., 0.],
        ],
    ] {
        assert_eq!(
            supporting_plane(&face, TOL).unwrap_err(),
            Error::Invalid,
            "{face:?}"
        );
    }
}

#[test]
fn clipped_faces_are_invariant_to_operand_order_and_containment() {
    let a = block([0., 0., 0.], [3., 3., 3.]);
    let b = block([2., 1., 1.], [4., 2., 4.]);
    let c = block([0.5, 0.5, 0.5], [1.5, 1.5, 1.5]);
    for pieces in [[a.clone(), b.clone(), c.clone()], [c, b, a]] {
        let union = Union::new(&pieces, TOL).unwrap();
        assert_eq!(union.solid_count(), 1);
        check_volume(union.volume_mm3(), 31.);
    }
}

#[test]
fn clipping_refuses_work_over_the_budget() {
    let pieces = [block([0.; 3], [2.; 3]), block([1.; 3], [3.; 3])];
    assert!(matches!(
        clipping::exterior(&pieces, TOL, 1),
        Err(Error::Limit)
    ));
}

#[test]
fn slab_cut_remap_and_translation_share_an_exact_seam() {
    let floor = 250.45;
    let radius = 1.75;
    let ceiling = floor + radius;
    let body = block([0., 0., 248.], [2., 3., 254.]);
    let slab = |limits| body.clip_z_band(limits, TOL).unwrap().unwrap();
    let [lower, center, upper] = [
        slab([248., floor]).translate_z(floor, 0., TOL).unwrap(),
        slab([floor, ceiling])
            .remap_quarter_circle(floor, radius, TOL)
            .unwrap(),
        slab([ceiling, 254.])
            .translate_z(ceiling, radius, TOL)
            .unwrap(),
    ];
    assert_eq!(lower.piece.bounds[1][2], 0.);
    assert_eq!(center.piece.bounds[0][2], 0.);
    assert_eq!(center.piece.bounds[1][2], radius);
    assert_eq!(upper.piece.bounds[0][2], radius);
    assert_eq!(
        center.source_faces.iter().filter(|i| i.is_none()).count(),
        2
    );
    assert_eq!(lower.source_faces.iter().filter(|i| i.is_none()).count(), 1);
    assert!(upper.source_faces.contains(&Some(1)));

    let union = Union::new(&[lower.piece, center.piece, upper.piece], TOL).unwrap();
    assert_eq!(union.solid_count(), 1);
    check_volume(union.volume_mm3(), 36.);
    assert!(
        union
            .faces()
            .iter()
            .all(|face| face.normal[2] == 0. || face.source != 1)
    );
}

#[test]
fn quarter_circle_maps_internal_horizontal_caps_nonlinearly() {
    let floor = 10.;
    let radius = 2.;
    let slab = block([0., 0., 10.5], [2., 2., 11.])
        .clip_z_band([floor, floor + radius], TOL)
        .unwrap()
        .unwrap();
    assert_eq!(slab.source_faces, (0..6).map(Some).collect::<Vec<_>>());
    let warped = slab.remap_quarter_circle(floor, radius, TOL).unwrap();
    assert_eq!(warped.piece.bounds[0][2], 1.75_f64.sqrt());
    assert_eq!(warped.piece.bounds[1][2], 3_f64.sqrt());
}

#[test]
fn slab_clipping_preserves_slopes_and_rejects_curving_a_sloped_face() {
    let wedge = ConvexPiece::new(
        vec![
            vec![[0., 0., 0.], [2., 0., 0.], [0., 0., 2.]],
            vec![[0., 1., 0.], [0., 1., 2.], [2., 1., 0.]],
            vec![[0., 0., 0.], [0., 1., 0.], [2., 1., 0.], [2., 0., 0.]],
            vec![[0., 0., 0.], [0., 0., 2.], [0., 1., 2.], [0., 1., 0.]],
            vec![[2., 0., 0.], [2., 1., 0.], [0., 1., 2.], [0., 0., 2.]],
        ],
        TOL,
    )
    .unwrap();
    let slab = wedge.clip_z_band([0.5, 1.5], TOL).unwrap().unwrap();
    assert!(slab.source_faces.contains(&Some(4)));
    let translated = slab.clone().translate_z(0.5, 0., TOL).unwrap();
    assert_eq!(translated.piece.bounds, [[0., 0., 0.], [1.5, 1., 1.]]);
    check_volume(
        Union::new(&[translated.piece], TOL).unwrap().volume_mm3(),
        1.,
    );
    assert_eq!(
        slab.remap_quarter_circle(0., 2., TOL).unwrap_err(),
        Error::Unresolved
    );
}

#[test]
fn slab_refuses_empty_or_ambiguous_material_and_bad_mappings() {
    let cube = block([0.; 3], [2.; 3]);
    assert!(cube.clip_z_band([2., 3.], TOL).unwrap().is_none());
    assert!(cube.clip_z_band([-1., 0.], TOL).unwrap().is_none());
    assert_eq!(cube.clip_z_band([1., 0.], TOL).unwrap_err(), Error::Invalid);
    assert_eq!(
        cube.clip_z_band([TOL / 2., 1.], TOL).unwrap_err(),
        Error::Unresolved
    );
    let full = cube.clip_z_band([0., 2.], TOL).unwrap().unwrap();
    assert_eq!(
        full.clone().translate_z(0., f64::NAN, TOL).unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        full.clone().remap_quarter_circle(0., 0.5, TOL).unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        full.remap_quarter_circle(0.5, 2., TOL).unwrap_err(),
        Error::Invalid
    );
}

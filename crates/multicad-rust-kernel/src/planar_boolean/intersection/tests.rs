use super::*;

const TOL: f64 = 1e-7;

#[test]
fn restored_cut_cells_do_not_hide_a_new_intersection_boundary() {
    let certificate = |pieces: &[ConvexPiece]| {
        let boundary = Union::new(pieces, TOL).unwrap();
        CutMaterial::certify_existing(&boundary, pieces, TOL).unwrap()
    };
    let [a, b, c, d] = [[0.; 3], [4., 0., 0.], [0., 4., 0.], [0., 0., 4.]];
    let tetra = certificate(&[ConvexPiece::new(
        vec![vec![a, c, b], vec![a, b, d], vec![a, d, c], vec![b, c, d]],
        TOL,
    )
    .unwrap()]);
    let first = certificate(&[block([1., 0., 0.], [5., 1., 1.])]);
    let second = certificate(&[block([1.5, 0., 0.], [4., 1., 1.])]);
    let common = CutMaterial::intersect_certified(&[&tetra, &first, &second], TOL)
        .unwrap()
        .unwrap();
    let slot = certificate(&[block([1.5, 0., 0.], [1.75, 1., 1.])]);
    let cut = CutMaterial::subtract_certified(&[&common, &slot], TOL)
        .unwrap()
        .unwrap();
    let restored = CutMaterial::union_certified(&[&cut, &slot], TOL).unwrap();
    let last = [block([1.75, 0., 0.], [4., 1., 1.])];
    let raw = Union::intersect(restored.pieces(), &last, TOL)
        .unwrap()
        .unwrap();
    volume(&raw, 1.25);
    for face in raw.faces() {
        let original = if face.source < restored.pieces().len() {
            &restored.pieces()[face.source]
        } else {
            &last[face.source - restored.pieces().len()]
        };
        let support = original.planes[face.source_face];
        assert!(dot(support.normal, face.normal) > 1. - 1e-12);
        assert!(
            face.vertices
                .iter()
                .all(|&i| support.distance(raw.vertices()[i]).abs() < TOL)
        );
    }
}

#[test]
fn raw_boundary_query_retains_its_larger_output_than_cell_recovery_budget() {
    let vertical: Vec<_> = (0..17)
        .map(|i| block([i as f64 * 2., 0., 0.], [i as f64 * 2. + 1., 34., 1.]))
        .collect();
    let horizontal: Vec<_> = (0..17)
        .map(|i| block([0., i as f64 * 2., 0.], [34., i as f64 * 2. + 1., 1.]))
        .collect();
    let common = Union::intersect(&vertical, &horizontal, TOL)
        .unwrap()
        .unwrap();
    assert_eq!(common.solid_count(), 289);
    volume(&common, 289.);
}

fn block(min: Point, max: Point) -> ConvexPiece {
    ConvexPiece::from_box(min, max, TOL).unwrap()
}

fn volume(common: &Union, expected: f64) {
    let mesh = common
        .mesh(Tessellation {
            deflection: 0.1,
            angle: 0.2,
            triangle_limit: 10_000,
        })
        .unwrap();
    for value in [
        common.volume_mm3(),
        manufacturing::certify(&mesh).unwrap().signed_volume_mm3,
    ] {
        assert!((value - expected).abs() <= 1e-8, "{value} != {expected}");
    }
}

#[test]
fn rotated_octagon_keeps_the_source_of_every_face() {
    let left = block([-1.; 3], [1.; 3]);
    let right_base = block([-1., -1., 0.], [1., 1., 2.]);
    let right = ConvexPiece::new(
        right_base
            .faces
            .into_iter()
            .map(|ring| {
                ring.into_iter()
                    .map(|[x, y, z]| {
                        let s = std::f64::consts::FRAC_1_SQRT_2;
                        [s * (x - y), s * (x + y), z]
                    })
                    .collect()
            })
            .collect(),
        TOL,
    )
    .unwrap();
    let common = Union::intersect(
        std::slice::from_ref(&left),
        std::slice::from_ref(&right),
        TOL,
    )
    .unwrap()
    .unwrap();
    assert_eq!(common.bounds(), [[-1., -1., 0.], [1.; 3]]);
    assert_eq!(common.solid_count(), 1);
    assert_eq!(common.shell_count(), 1);
    volume(&common, 8. * (2_f64.sqrt() - 1.));
    let mut seen = Vec::new();
    for face in common.faces() {
        let source = [&left, &right][face.source];
        let plane = source.planes[face.source_face];
        assert_eq!(face.normal, plane.normal);
        for &vertex in &face.vertices {
            assert!(plane.distance(common.vertices()[vertex]).abs() <= TOL);
        }
        seen.push([face.source, face.source_face]);
    }
    seen.sort_unstable();
    assert_eq!(
        seen,
        [
            [0, 1],
            [0, 2],
            [0, 3],
            [0, 4],
            [0, 5],
            [1, 0],
            [1, 2],
            [1, 3],
            [1, 4],
            [1, 5],
        ]
    );
}

#[test]
fn shared_support_keeps_left_ancestry_without_duplicate_faces() {
    let left = [block([0.; 3], [3.; 3]), block([2., 0., 0.], [5., 3., 3.])];
    let right = [block([1.; 3], [4., 3., 2.]), block([2.; 3], [4., 3., 3.])];
    for (a, b, contributes_right) in [(&left, &right, true), (&right, &left, false)] {
        let common = Union::intersect(a, b, TOL).unwrap().unwrap();
        assert_eq!(common.solid_count(), 1);
        volume(&common, 8.);
        assert_eq!(
            common.faces().iter().any(|f| f.source >= a.len()),
            contributes_right
        );
        let mut owners: Vec<_> = common
            .faces()
            .iter()
            .filter(|f| f.normal == [0., 1., 0.])
            .map(|f| f.source)
            .collect();
        owners.sort_unstable();
        owners.dedup();
        assert_eq!(owners, [0, 1]);
    }

    let common = Union::intersect(&left, &left, TOL).unwrap().unwrap();
    volume(&common, 45.);
    assert!(common.faces().iter().all(|f| f.source < left.len()));
}

#[test]
fn closed_cavity_and_separate_components_survive_restriction() {
    let hollow = [
        block([0.; 3], [3., 3., 1.]),
        block([0., 0., 2.], [3.; 3]),
        block([0., 0., 1.], [1., 3., 2.]),
        block([2., 0., 1.], [3., 3., 2.]),
        block([1., 0., 1.], [2., 1., 2.]),
        block([1., 2., 1.], [2., 3., 2.]),
        block([5., 0., 0.], [8., 3., 3.]),
    ];
    let windows = [
        block([0.5; 3], [2.5; 3]),
        block([6., 0.5, 0.5], [7., 1.5, 1.5]),
    ];
    let common = Union::intersect(&hollow, &windows, TOL).unwrap().unwrap();
    assert_eq!(common.solid_count(), 2);
    assert_eq!(common.shell_count(), 3);
    volume(&common, 8.);
}

fn triangular_prism(ring: [[f64; 2]; 3]) -> ConvexPiece {
    let base = ring.map(|[x, y]| [x, y, 0.]);
    let top = ring.map(|[x, y]| [x, y, 2.]);
    let mut faces = vec![base.into_iter().rev().collect(), top.to_vec()];
    for (i, j) in [(0, 1), (1, 2), (2, 0)] {
        faces.push(vec![base[i], base[j], top[j], top[i]]);
    }
    ConvexPiece::new(faces, TOL).unwrap()
}

#[test]
fn contact_is_empty_even_with_overlapping_aabbs() {
    let left = [block([0.; 3], [2.; 3])];
    for right in [
        block([3.; 3], [4.; 3]),
        block([2., 0., 0.], [3.; 3]),
        block([2., 2., 0.], [3.; 3]),
        block([2.; 3], [3.; 3]),
        block([2. + TOL / 2., 0., 0.], [3.; 3]),
    ] {
        assert_eq!(Union::intersect(&left, &[right], TOL), Ok(None));
    }
    let a = triangular_prism([[0., 0.], [2., 0.], [2., 2.]]);
    let b = triangular_prism([[0., 0.], [2., 2.], [0., 2.]]);
    assert_eq!(a.bounds, b.bounds);
    assert_eq!(Union::intersect(&[a], &[b], TOL), Ok(None));
}

#[test]
fn refuses_ambiguous_near_contact_and_caps_work() {
    let left = [block([0.; 3], [2.; 3])];
    let right = [block([2. - TOL / 2., 0., 0.], [3.; 3])];
    assert_eq!(Union::intersect(&left, &right, TOL), Err(Error::Unresolved));
    let narrow = [block([2. - TOL / 2., 0.5, 0.5], [3., 1.5, 1.5])];
    assert_eq!(
        Union::intersect(&left, &narrow, TOL),
        Err(Error::Unresolved)
    );
    assert_eq!(Union::intersect(&[], &right, TOL), Err(Error::Invalid));
    assert_eq!(Union::intersect(&left, &[], TOL), Err(Error::Invalid));
    assert_eq!(
        Union::intersect(&left, &right, f64::NAN),
        Err(Error::Invalid)
    );
    assert_eq!(
        Union::intersect(&vec![left[0].clone(); MAX_PIECES], &right, TOL),
        Err(Error::Limit)
    );
    assert_eq!(
        material::raw_intersection(&left, &left, TOL, 0),
        Err(Error::Limit)
    );
}

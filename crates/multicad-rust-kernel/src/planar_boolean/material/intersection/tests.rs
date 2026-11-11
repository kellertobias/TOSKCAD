use super::*;
const TOL: f64 = 1e-7;
fn block(min: Point, max: Point) -> ConvexPiece {
    ConvexPiece::from_box(min, max, TOL).unwrap()
}
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-8, "{a} != {b}");
}
fn certify(left: &[ConvexPiece], right: &[ConvexPiece], volume: f64) -> CutMaterial {
    let expected = Union::intersect(left, right, TOL).unwrap().unwrap();
    let actual = CutMaterial::intersection(left, right, TOL)
        .unwrap()
        .unwrap();
    assert_eq!(actual.boundary(), &expected);
    close(actual.boundary().volume_mm3(), volume);
    let rebuilt = Union::new(actual.pieces(), TOL).unwrap();
    close(rebuilt.volume_mm3(), volume);
    assert_eq!(rebuilt.bounds(), expected.bounds());
    let proof = manufacturing::certify(
        &actual
            .boundary()
            .mesh(crate::display::Tessellation {
                deflection: 0.1,
                angle: 0.2,
                triangle_limit: 65_536,
            })
            .unwrap(),
    )
    .unwrap();
    close(proof.signed_volume_mm3, volume);
    actual
}
#[test]
fn certified_intersection_recovers_concave_disconnected_and_cavity_material() {
    let u = [
        block([0.; 3], [1., 4., 2.]),
        block([1., 0., 0.], [3., 1., 2.]),
        block([3., 0., 0.], [4., 4., 2.]),
    ];
    let crop = [block([0.5, -1., 0.5], [3.5, 5., 1.5])];
    let common = certify(&u, &crop, 6.);
    assert_eq!(common.boundary().solid_count(), 1);
    let disconnected = certify(&[u[0].clone(), u[2].clone()], &crop, 4.);
    assert_eq!(disconnected.boundary().solid_count(), 2);
    let cavity = [
        block([0.; 3], [3., 3., 1.]),
        block([0., 0., 2.], [3., 3., 3.]),
        block([0., 0., 1.], [1., 3., 2.]),
        block([2., 0., 1.], [3., 3., 2.]),
        block([1., 0., 1.], [2., 1., 2.]),
        block([1., 2., 1.], [2., 3., 2.]),
    ];
    let common = certify(&cavity, &[block([-1.; 3], [4.; 3])], 26.);
    assert_eq!(common.boundary().shell_count(), 2);
    assert!(
        CutMaterial::intersection(&cavity, &[block([1.; 3], [2.; 3])], TOL)
            .unwrap()
            .is_none()
    );
}
#[test]
fn overlapping_cells_containment_source_order_and_retained_cut_remain_exact() {
    let left = [block([0.; 3], [2.; 3]), block([1., 0., 0.], [3., 2., 2.])];
    let right = [
        block([0.5, 0.5, 0.5], [2.5, 1.5, 1.5]),
        block([1., 0.5, 0.5], [2., 1.5, 1.5]),
    ];
    let material = certify(&left, &right, 2.);
    certify(&right, &left, 2.);
    let cutters = [block([1., 0., 0.], [2., 2., 2.])];
    let cut = material.subtract(&cutters, TOL).unwrap().unwrap();
    close(cut.volume_mm3(), 1.);
    assert_eq!(cut.solid_count(), 2);
    for face in cut.faces() {
        if face.source == 0 {
            assert!(face.source_face < material.boundary().faces().len());
        }
    }
    let duplicate = vec![block([0.; 3], [2.; 3]); 12];
    let material = certify(&duplicate, &duplicate, 8.);
    assert_eq!(material.pieces().len(), 1);
    assert!(material.boundary().faces().iter().all(|f| f.source == 0));
    let shifted = [block([1., 0., 0.], [3., 2., 2.])];
    let forward = certify(&[duplicate[0].clone()], &shifted, 4.);
    let reverse = certify(&shifted, &[duplicate[0].clone()], 4.);
    assert_ne!(forward.boundary().faces(), reverse.boundary().faces());
}
#[test]
fn empty_contacts_invalid_inputs_and_failed_recovery_do_not_mutate_operands() {
    let a = block([0.; 3], [2.; 3]);
    let original = a.faces.clone();
    for b in [
        block([2., 0., 0.], [3., 2., 2.]),
        block([2., 2., 0.], [3., 3., 2.]),
        block([3.; 3], [4.; 3]),
    ] {
        assert!(
            CutMaterial::intersection(std::slice::from_ref(&a), &[b], TOL)
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(
        CutMaterial::intersection(&[], std::slice::from_ref(&a), TOL).unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        CutMaterial::intersection(std::slice::from_ref(&a), std::slice::from_ref(&a), f64::NAN)
            .unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        CutMaterial::intersection(&vec![a.clone(); MAX_PIECES], std::slice::from_ref(&a), TOL)
            .unwrap_err(),
        Error::Limit
    );
    assert_eq!(
        common_cell(a.clone(), &a, TOL, &mut 0).unwrap_err(),
        Error::Limit
    );
    let mut budget = MAX_WORK;
    assert!(
        common_cell(a.clone(), &block([4.; 3], [5.; 3]), TOL, &mut budget)
            .unwrap()
            .is_none()
    );
    assert_eq!(a.faces, original);
    certify(std::slice::from_ref(&a), std::slice::from_ref(&a), 8.);
}

#[test]
fn recovery_budgets_accumulate_across_pairs_and_reject_inconsistent_certificates() {
    let a = block([0.; 3], [2.; 3]);
    let left = [a.clone(), a.clone()];
    let right = [a.clone()];
    let boundary = Union::intersect(&left, &right, TOL).unwrap().unwrap();
    for budget in [0, 1, 25, 900, 1000] {
        assert_eq!(
            recover_common(&boundary, &left, &right, TOL, &mut { budget }).unwrap_err(),
            Error::Limit
        );
    }
    let far = [block([3.; 3], [4.; 3])];
    assert_eq!(
        recover_common(&boundary, &right, &far, TOL, &mut { MAX_WORK }).unwrap_err(),
        Error::Unresolved
    );
    let wrong = Union::new(&[block([0.; 3], [3.; 3])], TOL).unwrap();
    assert_eq!(
        recover_common(&wrong, &right, &right, TOL, &mut { MAX_WORK }).unwrap_err(),
        Error::Unresolved
    );
    let material = certify(&left, &right, 8.);
    assert_eq!(material.pieces().len(), 1);
}
#[test]
fn more_than_the_positive_cell_budget_is_a_typed_failure_with_recovery() {
    let left: Vec<_> = (0..17)
        .map(|i| block([i as f64, 0., 0.], [(i + 1) as f64, 17., 1.]))
        .collect();
    let right: Vec<_> = (0..17)
        .map(|i| block([0., i as f64, 0.], [17., (i + 1) as f64, 1.]))
        .collect();
    assert_eq!(
        CutMaterial::intersection(&left, &right, TOL).unwrap_err(),
        Error::Limit
    );
    certify(&[block([0.; 3], [2.; 3])], &[block([1.; 3], [3.; 3])], 1.);
}

#[test]
fn overlapping_bounds_do_not_turn_a_separated_convex_pair_into_material() {
    let vertices = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    let faces = [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]]
        .map(|indices| indices.map(|i| vertices[i]).to_vec())
        .to_vec();
    let lower = ConvexPiece::new(faces, TOL).unwrap();
    let upper = ConvexPiece::new(
        lower
            .faces
            .iter()
            .map(|ring| ring.iter().rev().map(|p| p.map(|v| 1. - v)).collect())
            .collect(),
        TOL,
    )
    .unwrap();
    assert_eq!(lower.bounds, upper.bounds);
    assert!(
        CutMaterial::intersection(
            std::slice::from_ref(&lower),
            std::slice::from_ref(&upper),
            TOL
        )
        .unwrap()
        .is_none()
    );
    let material = certify(&[lower.clone(), upper], &[lower], 1. / 6.);
    assert_eq!(material.pieces().len(), 1);
}

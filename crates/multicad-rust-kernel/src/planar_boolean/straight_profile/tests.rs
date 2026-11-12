use super::*;
const TOL: f64 = 1e-7;

fn outer() -> Vec<[f64; 2]> {
    vec![[0., 0.], [12., 0.], [12., 10.], [0., 10.]]
}
fn hole(x: f64, y: f64) -> Vec<[f64; 2]> {
    vec![[x, y], [x, y + 2.], [x + 2., y + 2.], [x + 2., y]]
}
#[test]
fn authored_multiwire_material_retains_every_boundary_and_real_hole() {
    for holes in 0..=2 {
        let mut rings = vec![outer()];
        rings.extend([hole(2., 3.), hole(7., 3.)].into_iter().take(holes));
        let cells = straight_profile(&rings, 5., TOL, None).unwrap();
        let pieces: Vec<_> = cells.iter().map(|c| c.piece.clone()).collect();
        let union = Union::new(&pieces, TOL).unwrap();
        assert!((union.volume_mm3() - (120. - holes as f64 * 4.) * 5.).abs() < 1e-8);
        for ix in -1..26 {
            for iy in -1..22 {
                let [x, y] = [ix, iy].map(|n| n as f64 * 0.5 + 0.25);
                let expected = x > 0.
                    && x < 12.
                    && y > 0.
                    && y < 10.
                    && !(holes >= 1 && x > 2. && x < 4. && y > 3. && y < 5.)
                    && !(holes >= 2 && x > 7. && x < 9. && y > 3. && y < 5.);
                for z in [-0.5, 2.5, 5.5] {
                    let occupied = pieces.iter().any(|p| {
                        p.planes
                            .iter()
                            .all(|plane| plane.distance([x, y, z]) <= TOL)
                    });
                    assert_eq!(occupied, expected && z == 2.5, "{holes} {:?}", [x, y, z]);
                }
            }
        }
        let mut counts = vec![0; rings.iter().map(Vec::len).sum()];
        for c in &cells {
            assert_eq!(c.source_faces.len(), 5);
            assert_eq!(
                c.source_faces[..2],
                [ProfileFace::StartCap, ProfileFace::EndCap]
            );
            for f in &c.source_faces {
                if let ProfileFace::Edge(i) = f {
                    counts[*i] += 1;
                }
            }
        }
        assert!(counts.iter().all(|&n| n == 1));
    }
}

#[test]
fn concave_source_and_collinear_authored_vertices_remain_literal() {
    let rings = vec![vec![
        [0., 0.],
        [5., 0.],
        [10., 0.],
        [10., 8.],
        [7., 8.],
        [7., 3.],
        [3., 3.],
        [3., 8.],
        [0., 8.],
    ]];
    let cells = straight_profile(&rings, 2., TOL, None).unwrap();
    let union = Union::new(
        &cells.iter().map(|c| c.piece.clone()).collect::<Vec<_>>(),
        TOL,
    )
    .unwrap();
    assert!((union.volume_mm3() - 120.).abs() < 1e-8);
    assert!(
        cells
            .iter()
            .any(|c| c.source_faces.contains(&ProfileFace::Edge(0)))
    );
    assert!(
        cells
            .iter()
            .any(|c| c.source_faces.contains(&ProfileFace::Edge(1)))
    );
}

#[test]
fn invalid_unresolved_and_resource_cases_never_publish_material() {
    for rings in [
        vec![],
        vec![vec![]],
        vec![outer(), hole(11., 3.)],
        vec![outer(), outer()],
    ] {
        assert!(straight_profile(&rings, 2., TOL, None).is_err());
    }
    for height in [0., TOL, f64::NAN, f64::INFINITY, 1e9] {
        assert!(matches!(
            straight_profile(&[outer()], height, TOL, None),
            Err(Error::Invalid)
        ));
    }
    for coordinate in [f64::NAN, f64::INFINITY, 1e8] {
        let mut ring = outer();
        ring[0][0] = coordinate;
        assert!(straight_profile(&[ring], 2., TOL, None).is_err());
    }
    let mut reversed = outer();
    reversed.reverse();
    assert!(straight_profile(&[reversed], 2., TOL, None).is_err());
    assert!(matches!(
        straight_profile(&[vec![[0., 0.]; 257]], 2., TOL, None),
        Err(Error::Limit)
    ));
    assert!(matches!(
        straight_profile(&[outer()], 2., 0., None),
        Err(Error::Invalid)
    ));
}

#[test]
fn cancellation_at_each_observed_checkpoint_is_atomic_and_retryable() {
    use std::cell::Cell;
    let rings = vec![outer(), hole(3., 3.)];
    let calls = Cell::new(0);
    let observe = || {
        calls.set(calls.get() + 1);
        false
    };
    let expected = straight_profile(&rings, 4., TOL, Some(&observe)).unwrap();
    for at in 0..calls.get() {
        let count = Cell::new(0);
        let cancel = || {
            let n = count.get();
            count.set(n + 1);
            n == at
        };
        assert!(matches!(
            straight_profile(&rings, 4., TOL, Some(&cancel)),
            Err(Error::Cancelled)
        ));
    }
    assert_eq!(
        straight_profile(&rings, 4., TOL, None).unwrap().len(),
        expected.len()
    );
}

#[test]
fn proposed_partitions_cannot_move_vertices_reverse_or_degenerate_cells() {
    let points = vec![
        Point2::new(0., 0.),
        Point2::new(2., 0.),
        Point2::new(2., 2.),
        Point2::new(0., 2.),
    ];
    let edges = [[0, 1], [1, 2], [2, 3], [3, 0]];
    let section = Section {
        points: &points,
        edges: &edges,
        triangle_count: 2,
        height_mm: 3.,
        tolerance_mm: TOL,
    };
    let triangles = [[0, 1, 2], [0, 2, 3]];
    assert_eq!(
        certify_candidate(section, &points, &triangles, None)
            .unwrap()
            .len(),
        2
    );
    let mut moved = points.clone();
    moved[1].x += 0.25;
    assert!(certify_candidate(section, &moved, &triangles, None).is_err());
    for candidate in [
        vec![[0, 1, 2]],
        vec![[0, 1, 2], [0, 2, 99]],
        vec![[0, 2, 1], [0, 2, 3]],
        vec![[0, 0, 1], [0, 2, 3]],
        vec![[0, 1, 2], [0, 1, 2]],
    ] {
        assert!(certify_candidate(section, &points, &candidate, None).is_err());
    }
}

#[test]
fn incidence_certificate_requires_all_wires_and_opposed_internal_uses() {
    let edges = [[0, 1], [1, 2], [2, 0], [3, 4], [4, 5], [5, 3]];
    let labels = edges
        .iter()
        .copied()
        .enumerate()
        .map(|(i, e)| (e, i))
        .collect();
    // Outer triangle alone leaves the entire authored hole unaccounted for.
    let missing_hole = BTreeMap::from([([0, 1], 1), ([1, 2], 1), ([2, 0], 1)]);
    assert_eq!(
        certify_incidence(&missing_hole, &labels, &edges, None),
        Err(Error::Unresolved)
    );
    for uses in [
        BTreeMap::from([([0, 1], 2)]),
        BTreeMap::from([([0, 1], 1), ([1, 0], 1)]),
        BTreeMap::from([([0, 4], 1)]),
    ] {
        assert_eq!(
            certify_incidence(&uses, &labels, &edges, None),
            Err(Error::Unresolved)
        );
    }
}

#[test]
fn a_valid_many_hole_request_exceeds_the_material_cell_budget_before_allocation() {
    let mut rings = vec![vec![[0., 0.], [100., 0.], [100., 100.], [0., 100.]]];
    for i in 0..70 {
        let x = 4. + (i % 10) as f64 * 8.;
        let y = 4. + (i / 10) as f64 * 8.;
        rings.push(vec![[x, y], [x, y + 1.], [x + 1., y]]);
    }
    assert!(linear_sweep::validate_cap(&rings, TOL).is_ok());
    assert!(matches!(
        straight_profile(&rings, 2., TOL, None),
        Err(Error::Limit)
    ));
}

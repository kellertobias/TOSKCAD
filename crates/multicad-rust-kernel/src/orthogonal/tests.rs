use super::*;

fn cube(min: [f64; 3], max: [f64; 3]) -> Box3 {
    Box3 { min, max }
}

fn unit(at: [i32; 3]) -> Box3 {
    cube(at.map(f64::from), at.map(|x| f64::from(x + 1)))
}

fn apply(left: &[Box3], right: &[Box3], op: Operation) -> OrthogonalSolid {
    boolean(left, right, op, 1e-7).unwrap().unwrap()
}

fn rational(n: i64, d: i64) -> Rational {
    Rational::new(n, d).unwrap()
}

fn integer_ring(points: &[[i64; 2]]) -> Vec<[Rational; 2]> {
    points.iter().map(|p| p.map(|n| rational(n, 1))).collect()
}

#[test]
fn occupied_volume_counts_each_cell_once_and_keeps_positive_slivers() {
    let a = cube([0.; 3], [10.; 3]);
    let b = cube([5., 0., 0.], [15., 10., 10.]);
    let probe = cube([8., 0., 0.], [12., 10., 10.]);
    assert_eq!(intersection_volume(&[a, b], &[probe]).unwrap(), 400.);
    assert_eq!(intersection_volume(&[a], &[b]).unwrap(), 500.);
    assert_eq!(intersection_volume(&[a], &[]).unwrap(), 0.);
    let touching = cube([10., 0., 0.], [11., 1., 1.]);
    assert_eq!(intersection_volume(&[a], &[touching]).unwrap(), 0.);
    let small = 10. - 1e-8;
    let sliver = cube([small, 0., 0.], [11., 1., 1.]);
    assert_eq!(intersection_volume(&[a], &[sliver]).unwrap(), 10. - small);
}

#[test]
fn occupied_volume_rejects_limits_and_nonfinite_results() {
    let a = cube([0.; 3], [10.; 3]);
    let excessive = vec![a; MAX_CELLS + 1];
    assert_eq!(
        intersection_volume(&excessive, &[a]),
        Err(BooleanError::Limit)
    );
    let invalid = cube([0.; 3], [0.; 3]);
    assert_eq!(
        intersection_volume(&[a], &[invalid]),
        Err(BooleanError::Invalid)
    );
    let too_large = cube([0.; 3], [1e200; 3]);
    assert_eq!(
        intersection_volume(&[too_large], &[too_large]),
        Err(BooleanError::Invalid)
    );
}

#[test]
fn rectilinear_prism_decomposition_keeps_straight_holes_and_signed_height() {
    let outer = integer_ring(&[[0, 0], [6, 0], [6, 2], [3, 2], [3, 5], [0, 5]]);
    let hole = [[1, 1], [1, 3], [2, 3], [2, 1]]
        .map(|[x, y]| [rational(x, 1), rational(y, 2)])
        .to_vec();
    let prism = PolygonPrism::new(vec![outer, hole], rational(5, 2)).unwrap();
    for reverse in [false, true] {
        let [bottom, top] = if reverse { [-2.5, 0.] } else { [0., 2.5] };
        let expected = [
            cube([0., 0., bottom], [6., 0.5, top]),
            cube([0., 0.5, bottom], [1., 1.5, top]),
            cube([2., 0.5, bottom], [6., 1.5, top]),
            cube([0., 1.5, bottom], [6., 2., top]),
            cube([0., 2., bottom], [3., 5., top]),
        ];
        let cells = prism_cells(&prism, reverse).unwrap().unwrap();
        assert_eq!(cells, expected);
        let reconstructed = apply(&cells, &[], Operation::Union);
        assert_eq!(reconstructed.volume_mm3(), prism.volume().as_f64());
        assert_eq!(reconstructed.solid_count(), 1);
        assert!(
            boolean(
                &cells,
                &[cube([1., 0.5, bottom], [2., 1.5, top])],
                Operation::Intersect,
                0.
            )
            .unwrap()
            .is_none()
        );
    }
}

#[test]
fn unsupported_prism_profiles_and_collapsed_planes_are_explicit() {
    let triangle = PolygonPrism::new(
        vec![integer_ring(&[[0, 0], [6, 0], [0, 6]])],
        rational(1, 1),
    )
    .unwrap();
    assert!(prism_cells(&triangle, false).unwrap().is_none());
    let circular = PolygonPrism::with_circular_holes(
        vec![integer_ring(&[[0, 0], [6, 0], [6, 6], [0, 6]])],
        vec![crate::Circle::new([rational(3, 1); 2], rational(1, 1)).unwrap()],
        rational(1, 1),
    )
    .unwrap();
    assert!(prism_cells(&circular, false).unwrap().is_none());

    let far = 1i64 << 53;
    for [x, y] in [[far, 0], [0, far]] {
        let rectangle = PolygonPrism::new(
            vec![integer_ring(&[
                [x, y],
                [x + 1, y],
                [x + 1, y + 1],
                [x, y + 1],
            ])],
            rational(1, 1),
        )
        .unwrap();
        assert_eq!(
            prism_cells(&rectangle, false).unwrap_err(),
            BooleanError::Unresolved
        );
    }
}

#[test]
fn prism_sweep_refuses_a_profile_with_too_many_occupied_bands() {
    // Stagger all hole ends and add collinear outer vertices within the band
    // where every hole overlaps. The validated profile has 1,024 vertices,
    // but its exact sweep would create more than 65,536 material intervals.
    let outer = integer_ring(&[
        [0, 0],
        [510, 0],
        [510, 255],
        [510, 256],
        [510, 257],
        [510, 258],
        [510, 520],
        [0, 520],
    ]);
    let mut rings = vec![outer];
    for i in 0..254 {
        let (x, bottom, top) = (2 * i + 1, i + 1, i + 260);
        rings.push(integer_ring(&[
            [x, bottom],
            [x, top],
            [x + 1, top],
            [x + 1, bottom],
        ]));
    }
    let prism = PolygonPrism::new(rings, rational(1, 1)).unwrap();
    assert_eq!(prism_cells(&prism, false).unwrap_err(), BooleanError::Limit);
}

fn flux_volume(result: &OrthogonalSolid) -> f64 {
    result
        .quads()
        .iter()
        .map(|quad| {
            let [a, b, _, d] = quad.corners;
            let u: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
            let v: [f64; 3] = std::array::from_fn(|i| d[i] - a[i]);
            let cross = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            (0..3).map(|i| a[i] * cross[i]).sum::<f64>() / 3.
        })
        .sum()
}

#[test]
fn overlapping_boxes_support_all_three_operations_and_repeated_cuts() {
    let a = cube([-0., 0., 0.], [3., 2., 4.]);
    let b = cube([2., 1., 2.], [5., 3., 6.]);
    let intersection = apply(&[a], &[b], Operation::Intersect);
    assert_eq!(intersection.cells(), &[cube([2., 1., 2.], [3., 2., 4.])]);
    assert_eq!(intersection.volume_mm3(), 2.);
    assert_eq!(intersection.quads().len(), 6);
    assert_eq!(intersection.solid_count(), 1);

    let joined = apply(&[a, a], &[b], Operation::Union);
    assert_eq!(joined.volume_mm3(), 46.);
    assert_eq!(joined.bounds(), cube([0.; 3], [5., 3., 6.]));
    assert_eq!(joined.solid_count(), 1);
    assert!((flux_volume(&joined) - 46.).abs() < 1e-12);
    assert_eq!(joined.bounds().min[0].to_bits(), 0);

    let subtracted = apply(joined.cells(), &[b], Operation::Subtract);
    assert_eq!(subtracted.volume_mm3(), 22.);
    assert_eq!(subtracted.bounds(), a);
    assert!((flux_volume(&subtracted) - 22.).abs() < 1e-12);
    assert!(
        boolean(subtracted.cells(), &[a], Operation::Subtract, 1e-7)
            .unwrap()
            .is_none()
    );
}

#[test]
fn regularized_contact_does_not_create_a_sheet_or_merge_a_pinch() {
    let a = unit([0; 3]);
    for (neighbor, count) in [
        ([1, 0, 0], 1),
        ([1, 1, 0], 2),
        ([1, 1, 1], 2),
        ([2, 0, 0], 2),
    ] {
        let b = unit(neighbor);
        let joined = apply(&[a], &[b], Operation::Union);
        assert_eq!(joined.solid_count(), count);
        assert_eq!(joined.volume_mm3(), 2.);
        assert!(joined.quads().iter().all(|quad| quad.component < count));
        assert!(
            boolean(&[a], &[b], Operation::Intersect, 0.)
                .unwrap()
                .is_none()
        );
        assert_eq!(apply(&[a], &[b], Operation::Subtract).volume_mm3(), 1.);
    }
    assert!(boolean(&[], &[], Operation::Union, 0.).unwrap().is_none());
    assert_eq!(apply(&[], &[a], Operation::Union).cells(), &[a]);
}

#[test]
fn a_cavity_keeps_an_inward_shell_and_remains_a_single_material_component() {
    let exterior = cube([0.; 3], [3.; 3]);
    let cavity = unit([1; 3]);
    let result = apply(&[exterior], &[cavity], Operation::Subtract);
    assert_eq!(result.solid_count(), 1);
    assert_eq!(result.volume_mm3(), 26.);
    assert_eq!(result.cells().len(), 26);
    assert_eq!(result.quads().len(), 60);
    assert!((flux_volume(&result) - 26.).abs() < 1e-12);
    assert!(
        boolean(result.cells(), &[cavity], Operation::Intersect, 0.)
            .unwrap()
            .is_none()
    );
}

#[test]
fn translated_box_corpus_matches_an_independent_unit_cell_oracle() {
    use std::collections::BTreeSet;

    let a = cube([0.; 3], [3., 2., 4.]);
    for x in -2..=4 {
        for y in -2..=3 {
            for z in -2..=5 {
                let b = cube(
                    [x, y, z].map(f64::from),
                    [x + 2, y + 2, z + 2].map(f64::from),
                );
                for op in [Operation::Union, Operation::Subtract, Operation::Intersect] {
                    let occupied = |cell: Box3, point: [i32; 3]| {
                        (0..3).all(|i| {
                            let p = f64::from(point[i]) + 0.5;
                            cell.min[i] < p && p < cell.max[i]
                        })
                    };
                    let mut expected = BTreeSet::new();
                    for ix in -2..6 {
                        for iy in -2..5 {
                            for iz in -2..7 {
                                let p = [ix, iy, iz];
                                let (left, right) = (occupied(a, p), occupied(b, p));
                                if match op {
                                    Operation::Union => left || right,
                                    Operation::Subtract => left && !right,
                                    Operation::Intersect => left && right,
                                } {
                                    expected.insert(p);
                                }
                            }
                        }
                    }
                    let result = boolean(&[a], &[b], op, 0.).unwrap();
                    let Some(result) = result else {
                        assert!(expected.is_empty(), "{op:?} at {x},{y},{z}");
                        continue;
                    };
                    let mut actual = BTreeSet::new();
                    for cell in result.cells() {
                        for ix in cell.min[0] as i32..cell.max[0] as i32 {
                            for iy in cell.min[1] as i32..cell.max[1] as i32 {
                                for iz in cell.min[2] as i32..cell.max[2] as i32 {
                                    assert!(
                                        actual.insert([ix, iy, iz]),
                                        "overlapping result cells"
                                    );
                                }
                            }
                        }
                    }
                    assert_eq!(actual, expected, "{op:?} at {x},{y},{z}");
                    assert_eq!(result.volume_mm3(), expected.len() as f64);
                    assert!((flux_volume(&result) - expected.len() as f64).abs() < 1e-12);
                }
            }
        }
    }
}

#[test]
fn invalid_and_unresolved_geometry_fail_before_publication() {
    let a = unit([0; 3]);
    for bad in [
        cube([0.; 3], [0., 1., 1.]),
        cube([0.; 3], [1., f64::INFINITY, 1.]),
    ] {
        assert_eq!(
            boolean(&[bad], &[], Operation::Union, 1e-7).unwrap_err(),
            BooleanError::Invalid
        );
    }
    for tolerance in [f64::NAN, -1.] {
        assert_eq!(
            boolean(&[a], &[], Operation::Union, tolerance).unwrap_err(),
            BooleanError::Invalid
        );
    }
    let near = cube([1. + 1e-9, 0., 0.], [2., 1., 1.]);
    assert_eq!(
        boolean(&[a], &[near], Operation::Union, 1e-7).unwrap_err(),
        BooleanError::Unresolved
    );
    assert_eq!(
        apply(
            &[a],
            &[cube([1.125, 0., 0.], [2., 1., 1.])],
            Operation::Union
        )
        .solid_count(),
        2
    );
    let tiny = cube([0.; 3], [1e-200; 3]);
    assert_eq!(
        boolean(&[tiny], &[], Operation::Union, 0.).unwrap_err(),
        BooleanError::Unresolved
    );
}

#[test]
fn connected_nonmanifold_edge_and_vertex_pinches_are_refused() {
    let edge_pinch = [[0, 0, 0], [1, 1, 0], [0, 0, 1], [1, 0, 1], [1, 1, 1]].map(unit);
    let vertex_pinch = [
        [0, 0, 0],
        [0, -1, 0],
        [1, -1, 0],
        [2, -1, 0],
        [2, 0, 0],
        [2, 1, 0],
        [2, 1, 1],
        [1, 1, 1],
    ]
    .map(unit);
    for cells in [&edge_pinch[..], &vertex_pinch] {
        assert_eq!(
            boolean(cells, &[], Operation::Union, 0.).unwrap_err(),
            BooleanError::Unresolved
        );
    }
    let mut link = VertexLink::default();
    for (from, to) in [(0, 1), (1, 2), (2, 0), (3, 4), (4, 5), (5, 3)] {
        link.insert(from, to).unwrap();
    }
    assert!(!link.is_cycle());
}

#[test]
fn grid_cell_boundary_and_component_budgets_are_bounded() {
    let too_many_inputs = vec![unit([0; 3]); MAX_CELLS + 1];
    assert_eq!(
        boolean(&too_many_inputs, &[], Operation::Union, 0.).unwrap_err(),
        BooleanError::Limit
    );

    let too_large_grid: Vec<_> = (0..32).map(|i| unit([i * 2; 3])).collect();
    assert_eq!(
        boolean(&too_large_grid, &[], Operation::Union, 0.).unwrap_err(),
        BooleanError::Limit
    );

    let disconnected: Vec<_> = (0..=MAX_COMPONENTS as i32)
        .map(|i| unit([i * 2, 0, 0]))
        .collect();
    assert_eq!(
        boolean(&disconnected, &[], Operation::Union, 0.).unwrap_err(),
        BooleanError::Limit
    );

    let too_many_cells = cube([0.; 3], [49.; 3]);
    let subdivisions: Vec<_> = (0..49).map(|i| unit([i; 3])).collect();
    assert_eq!(
        boolean(&[too_many_cells], &subdivisions, Operation::Union, 0.).unwrap_err(),
        BooleanError::Limit
    );

    // Disjoint upright pillars attached to a continuous floor make the
    // exterior face budget bind before the filled-cell budget.
    let floor = cube([0.; 3], [47., 47., 1.]);
    let mut pillars = Vec::new();
    for x in (0..47).step_by(2) {
        for y in (0..47).step_by(2) {
            pillars.push(cube(
                [f64::from(x), f64::from(y), 1.],
                [f64::from(x + 1), f64::from(y + 1), 47.],
            ));
        }
    }
    pillars.extend((1..47).map(|z| unit([0, 0, z])));
    assert_eq!(
        boolean(&[floor], &pillars, Operation::Union, 0.).unwrap_err(),
        BooleanError::Limit
    );
}

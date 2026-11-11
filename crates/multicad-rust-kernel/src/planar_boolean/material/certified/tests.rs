use super::*;
use std::cell::Cell;
const TOL: f64 = 1e-7;
fn block(min: Point, max: Point) -> ConvexPiece {
    ConvexPiece::from_box(min, max, TOL).unwrap()
}
fn material(pieces: &[ConvexPiece]) -> CutMaterial {
    let boundary = Union::new(pieces, TOL).unwrap();
    CutMaterial::certify_existing(&boundary, pieces, TOL).unwrap()
}
fn cube(min: Point, max: Point) -> CutMaterial {
    material(&[block(min, max)])
}
fn volume(value: &CutMaterial, expected: f64) {
    assert!(
        (value.boundary.volume_mm3() - expected).abs() < 1e-7,
        "{} != {expected}",
        value.boundary.volume_mm3()
    );
    let rebuilt = Union::new(value.pieces(), TOL).unwrap();
    assert!((rebuilt.volume_mm3() - expected).abs() < 1e-7);
}
fn labels(result: &CutMaterial, inputs: &[&CutMaterial], subtract: bool) {
    for face in result.boundary.faces() {
        let original = &inputs[face.source].boundary.faces()[face.source_face];
        let sign = if subtract && face.source != 0 {
            -1.
        } else {
            1.
        };
        assert!(dot(face.normal, original.normal) * sign > 1. - 1e-12);
        let support = Plane {
            normal: original.normal,
            point: inputs[face.source].boundary.vertices[original.vertices[0]],
        };
        assert!(
            face.vertices
                .iter()
                .all(|&v| support.distance(result.boundary.vertices[v]).abs() < TOL)
        );
        let input = inputs[face.source].boundary();
        let triangles = shell::triangulate_face(input.vertices(), original).unwrap();
        for &vertex in &face.vertices {
            let point = result.boundary.vertices[vertex];
            assert!(
                triangles.iter().any(|triangle| {
                    triangle
                        .iter()
                        .zip(triangle.iter().cycle().skip(1))
                        .all(|(&a, &b)| {
                            dot(
                                cross(
                                    sub(input.vertices[b], input.vertices[a]),
                                    sub(point, input.vertices[a]),
                                ),
                                original.normal,
                            ) >= -TOL
                        })
                }),
                "output vertex outside its claimed ORIGINAL finite face"
            );
        }
    }
}

#[test]
fn original_single_concave_cap_ordinals_remain_authoritative_as_later_tools() {
    let points = [
        [0., 0.],
        [6., 0.],
        [6., 5.],
        [4., 5.],
        [4., 2.],
        [2., 2.],
        [2., 5.],
        [0., 5.],
    ];
    let lower: Vec<_> = points.map(|[x, y]| [x, y, 0.]).into();
    let upper: Vec<_> = points.map(|[x, y]| [x, y, 2.]).into();
    let mut rings = vec![lower.iter().rev().copied().collect(), upper.clone()];
    rings.extend((0..points.len()).map(|i| {
        let j = (i + 1) % points.len();
        vec![lower[i], lower[j], upper[j], upper[i]]
    }));
    let boundary = Union::from_planar_rings(&rings, TOL).unwrap();
    let cells = [
        block([0.; 3], [6., 2., 2.]),
        block([0., 2., 0.], [2., 5., 2.]),
        block([4., 2., 0.], [6., 5., 2.]),
    ];
    let u = CutMaterial::certify_existing(&boundary, &cells, TOL).unwrap();
    let crop = cube([-1.; 3], [7., 3., 3.]);
    let common = CutMaterial::intersect_certified(&[&u, &crop], TOL)
        .unwrap()
        .unwrap();
    volume(&common, 32.);
    labels(&common, &[&u, &crop], false);
    let whole = cube([0.; 3], [6., 5., 2.]);
    let rest = CutMaterial::subtract_certified(&[&whole, &u], TOL)
        .unwrap()
        .unwrap();
    volume(&rest, 12.);
    labels(&rest, &[&whole, &u], true);
    let joined = CutMaterial::union_certified(&[&u, &rest], TOL).unwrap();
    volume(&joined, 60.);
    labels(&joined, &[&u, &rest], false);
    assert_eq!(u.boundary(), &boundary);
}

#[test]
fn controlled_raw_gather_certification_and_cold_recovery_cancel_inside_their_work() {
    let stock = [block([0.; 3], [4.; 3])];
    let tools = [block([1.; 3], [3.; 3])];
    let whole = Union::new(&stock, TOL).unwrap();
    let cut = Union::subtract(&stock, &tools, TOL).unwrap().unwrap();
    let raw = [stock[0].clone(), block([2.; 3], [6.; 3])];
    for mode in 0..3 {
        let run = |interrupt: Option<&dyn Fn() -> bool>| -> Result<(), Error> {
            match mode {
                0 => Union::new_with_interrupt(&raw, TOL, interrupt).map(|_| ()),
                1 => CutMaterial::certify_existing_with_interrupt(&whole, &stock, TOL, interrupt)
                    .map(|_| ()),
                _ => CutMaterial::recover_with_interrupt(&cut, &stock, &tools, TOL, interrupt)
                    .map(|_| ()),
            }
        };
        let count = Cell::new(0usize);
        run(Some(&|| {
            count.set(count.get() + 1);
            false
        }))
        .unwrap();
        for cutoff in 0..count.get() {
            let calls = Cell::new(0);
            assert_eq!(
                run(Some(&|| {
                    let n = calls.get();
                    calls.set(n + 1);
                    n == cutoff
                })),
                Err(Error::Cancelled)
            );
        }
        run(None).unwrap();
    }
    assert_eq!(
        CutMaterial::recover_with_interrupt(&cut, &stock, &[], TOL, None).unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        CutMaterial::recover_with_interrupt(
            &cut,
            &vec![stock[0].clone(); MAX_PIECES],
            &tools,
            TOL,
            None
        )
        .unwrap_err(),
        Error::Limit
    );
    assert_eq!(
        CutMaterial::recover_with_interrupt(&cut, &vec![stock[0].clone(); 44], &tools, TOL, None)
            .unwrap_err(),
        Error::Limit
    );
}

#[test]
fn certified_operators_bound_inputs_recovery_growth_and_shared_work_with_retry() {
    let a = cube([0.; 3], [4.; 3]);
    let b = cube([1.; 3], [3.; 3]);
    assert_eq!(
        CutMaterial::intersect_certified(&[&a], TOL).unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        CutMaterial::union_certified(&[&a, &b], f64::NAN).unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        CutMaterial::subtract_certified(&vec![&a; MAX_PIECES + 1], TOL).unwrap_err(),
        Error::Limit
    );
    let many_cells = material(&vec![block([0.; 3], [4.; 3]); MAX_PIECES]);
    assert_eq!(
        CutMaterial::intersect_certified(&[&many_cells, &a], TOL).unwrap_err(),
        Error::Limit
    );
    for op in [Operation::Union, Operation::Intersect, Operation::Subtract] {
        let mut low = Work {
            remaining: 20,
            interrupt: None,
            uncertain: Cell::new(false),
        };
        assert_eq!(
            operation(&[&a, &b], TOL, op, &mut low).unwrap_err(),
            Error::Limit
        );
        execute(op, &[&a, &b], None).unwrap();
    }
    let copies = material(&vec![block([0.; 3], [4.; 3]); 44]);
    assert_eq!(
        CutMaterial::subtract_certified(&[&copies, &b], TOL).unwrap_err(),
        Error::Limit
    );
    let vertical: Vec<_> = (0..17)
        .map(|i| block([i as f64 * 2., 0., 0.], [i as f64 * 2. + 1., 34., 1.]))
        .collect();
    let horizontal: Vec<_> = (0..17)
        .map(|i| block([0., i as f64 * 2., 0.], [34., i as f64 * 2. + 1., 1.]))
        .collect();
    let v = material(&vertical);
    let h = material(&horizontal);
    assert_eq!(
        recover(&[&v, &h], Operation::Intersect, TOL, &mut Work::new(None)).unwrap_err(),
        Error::Limit
    );
    let far = cube([10.; 3], [12.; 3]);
    assert_eq!(
        recover(&[&a, &far], Operation::Intersect, TOL, &mut Work::new(None)).unwrap_err(),
        Error::Unresolved
    );
    assert!(
        CutMaterial::intersect_certified(&[&a, &b], TOL)
            .unwrap()
            .is_some()
    );
}

#[test]
fn unresolved_positive_slivers_are_not_reported_as_exact_empty_contact() {
    let a = cube([0.; 3], [1.; 3]);
    for overlap in [1e-8, 1e-11] {
        let b = cube([1. - overlap, 0., 0.], [2., 1., 1.]);
        assert!(
            matches!(
                CutMaterial::intersect_certified(&[&a, &b], TOL),
                Err(Error::Invalid | Error::Unresolved)
            ),
            "positive sliver {overlap} was swallowed as empty"
        );
    }
    let touching = cube([1., 0., 0.], [2., 1., 1.]);
    assert!(
        CutMaterial::intersect_certified(&[&a, &touching], TOL)
            .unwrap()
            .is_none()
    );
}
#[test]
fn nway_intersection_retains_original_operand_faces_and_reusable_common_cells() {
    let a = cube([0.; 3], [10.; 3]);
    let b = cube([2., -1., -1.], [8., 11., 11.]);
    let c = cube([-1., 3., -1.], [11., 7., 11.]);
    let d = cube([-1., -1., 4.], [11., 11., 6.]);
    let operands = [&a, &b, &c, &d];
    let result = CutMaterial::intersect_certified(&operands, TOL)
        .unwrap()
        .unwrap();
    volume(&result, 48.);
    labels(&result, &operands, false);
    let sources: BTreeSet<_> = result.boundary.faces().iter().map(|f| f.source).collect();
    assert_eq!(sources, BTreeSet::from([1, 2, 3]));
    let clip = cube([0.; 3], [5.; 3]);
    let repeated = CutMaterial::intersect_certified(&[&result, &clip], TOL)
        .unwrap()
        .unwrap();
    volume(&repeated, 6.);
    labels(&repeated, &[&result, &clip], false);
    let removed = CutMaterial::subtract_certified(&[&a, &result], TOL)
        .unwrap()
        .unwrap();
    volume(&removed, 952.);
    labels(&removed, &[&a, &result], true);
    let joined = CutMaterial::union_certified(&[&removed, &result], TOL).unwrap();
    volume(&joined, 1000.);
    labels(&joined, &[&removed, &result], false);
    assert_eq!(joined.boundary.shell_count(), 1);
}

#[test]
fn restored_tetra_cut_partition_is_not_an_exposed_intersection_tie() {
    let [a, b, c, d] = [[0.; 3], [4., 0., 0.], [0., 4., 0.], [0., 0., 4.]];
    let tetra = material(&[ConvexPiece::new(
        vec![vec![a, c, b], vec![a, b, d], vec![a, d, c], vec![b, c, d]],
        TOL,
    )
    .unwrap()]);
    let first = cube([1., 0., 0.], [5., 1., 1.]);
    let second = cube([1.5, 0., 0.], [4., 1., 1.]);
    let common = CutMaterial::intersect_certified(&[&tetra, &first, &second], TOL)
        .unwrap()
        .unwrap();
    volume(&common, 1.5);
    let slot = cube([1.5, 0., 0.], [1.75, 1., 1.]);
    let cut = CutMaterial::subtract_certified(&[&common, &slot], TOL)
        .unwrap()
        .unwrap();
    volume(&cut, 1.25);
    let restored = CutMaterial::union_certified(&[&cut, &slot], TOL).unwrap();
    volume(&restored, 1.5);
    let last = cube([1.75, 0., 0.], [4., 1., 1.]);
    let repeated = CutMaterial::intersect_certified(&[&restored, &last], TOL)
        .unwrap()
        .unwrap();
    volume(&repeated, 1.25);
    labels(&repeated, &[&restored, &last], false);
    assert!(
        repeated
            .boundary
            .faces()
            .iter()
            .any(|face| face.source == 1 && face.normal[0] < -0.99)
    );
}

#[test]
fn cavities_concave_disconnected_overlap_contacts_and_original_ties_are_exact() {
    let stock = [block([0.; 3], [8.; 3])];
    let holes = [block([2.; 3], [6.; 3])];
    let boundary = Union::subtract(&stock, &holes, TOL).unwrap().unwrap();
    let hollow = CutMaterial::recover(&boundary, &stock, &holes, TOL).unwrap();
    let same = cube([0.; 3], [8.; 3]);
    let result = CutMaterial::intersect_certified(&[&hollow, &same], TOL)
        .unwrap()
        .unwrap();
    volume(&result, 448.);
    assert_eq!(result.boundary.shell_count(), 2);
    labels(&result, &[&hollow, &same], false);
    assert!(result.boundary.faces().iter().all(|f| f.source == 0));
    let void = cube([3.; 3], [5.; 3]);
    assert!(
        CutMaterial::intersect_certified(&[&hollow, &void], TOL)
            .unwrap()
            .is_none()
    );
    let u = material(&[
        block([0.; 3], [6., 2., 2.]),
        block([0., 2., 0.], [2., 5., 2.]),
        block([4., 2., 0.], [6., 5., 2.]),
    ]);
    let disjoint = cube([10.; 3], [12.; 3]);
    let joined = CutMaterial::union_certified(&[&u, &disjoint], TOL).unwrap();
    volume(&joined, 56.);
    assert_eq!(joined.boundary.solid_count(), 2);
    let far = cube([20.; 3], [22.; 3]);
    assert!(
        CutMaterial::intersect_certified(&[&joined, &far], TOL)
            .unwrap()
            .is_none()
    );
    let touching = cube([6., 0., 0.], [8., 2., 2.]);
    assert!(
        CutMaterial::intersect_certified(&[&u, &touching], TOL)
            .unwrap()
            .is_none()
    );
    let uncut = CutMaterial::subtract_certified(&[&u, &touching], TOL)
        .unwrap()
        .unwrap();
    volume(&uncut, 48.);
    let duplicate = material(&[stock[0].clone(), stock[0].clone()]);
    let deduplicated = CutMaterial::intersect_certified(&[&duplicate, &same], TOL)
        .unwrap()
        .unwrap();
    volume(&deduplicated, 512.);
    assert_eq!(deduplicated.pieces().len(), 1);
    let equal = CutMaterial::union_certified(&[&same, &duplicate], TOL).unwrap();
    volume(&equal, 512.);
    assert!(equal.boundary.faces().iter().all(|f| f.source == 0));
    assert!(
        CutMaterial::subtract_certified(&[&same, &duplicate], TOL)
            .unwrap()
            .is_none()
    );
}

#[test]
fn subtraction_uses_all_tool_union_faces_and_not_recovered_partition_walls() {
    let stock = cube([0.; 3], [10.; 3]);
    let a = cube([2., 2., 2.], [6., 6., 6.]);
    let b = cube([4., 4., 4.], [8., 8., 8.]);
    let cut = CutMaterial::subtract_certified(&[&stock, &a, &b], TOL)
        .unwrap()
        .unwrap();
    volume(&cut, 880.);
    labels(&cut, &[&stock, &a, &b], true);
    let tools = CutMaterial::union_certified(&[&a, &b], TOL).unwrap();
    let same = CutMaterial::subtract_certified(&[&stock, &tools], TOL)
        .unwrap()
        .unwrap();
    volume(&same, 880.);
    labels(&same, &[&stock, &tools], true);
    let newtool = cube([0., 0., 0.], [1., 1., 1.]);
    let later = CutMaterial::subtract_certified(&[&cut, &newtool], TOL)
        .unwrap()
        .unwrap();
    volume(&later, 879.);
    labels(&later, &[&cut, &newtool], true);
}

fn execute(
    op: Operation,
    operands: &[&CutMaterial],
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<(), Error> {
    match op {
        Operation::Union => {
            CutMaterial::union_certified_with_interrupt(operands, TOL, interrupt).map(|_| ())
        }
        Operation::Intersect => {
            CutMaterial::intersect_certified_with_interrupt(operands, TOL, interrupt).map(|_| ())
        }
        Operation::Subtract => {
            CutMaterial::subtract_certified_with_interrupt(operands, TOL, interrupt).map(|_| ())
        }
    }
}
#[test]
fn every_cooperative_checkpoint_cancels_without_mutating_or_poisoning_retry() {
    let a = cube([0.; 3], [3.; 3]);
    let b = cube([1.; 3], [4.; 3]);
    let before = [a.boundary.clone(), b.boundary.clone()];
    for op in [Operation::Union, Operation::Intersect, Operation::Subtract] {
        let count = Cell::new(0usize);
        execute(
            op,
            &[&a, &b],
            Some(&|| {
                count.set(count.get() + 1);
                false
            }),
        )
        .unwrap();
        for cutoff in 0..count.get() {
            let calls = Cell::new(0);
            assert_eq!(
                execute(
                    op,
                    &[&a, &b],
                    Some(&|| {
                        let n = calls.get();
                        calls.set(n + 1);
                        n == cutoff
                    })
                ),
                Err(Error::Cancelled)
            );
        }
        execute(op, &[&a, &b], None).unwrap();
    }
    assert_eq!([a.boundary.clone(), b.boundary.clone()], before);
    let tool1 = cube([0.5; 3], [2.; 3]);
    let tool2 = cube([1.; 3], [2.5; 3]);
    let count = Cell::new(0usize);
    execute(
        Operation::Subtract,
        &[&a, &tool1, &tool2],
        Some(&|| {
            count.set(count.get() + 1);
            false
        }),
    )
    .unwrap();
    for cutoff in 0..count.get() {
        let calls = Cell::new(0usize);
        assert_eq!(
            execute(
                Operation::Subtract,
                &[&a, &tool1, &tool2],
                Some(&|| {
                    let n = calls.get();
                    calls.set(n + 1);
                    n == cutoff
                })
            ),
            Err(Error::Cancelled)
        );
    }
}

#[test]
fn partially_exposed_coplanar_patch_does_not_own_an_internal_section() {
    let stepped = material(&[block([0.; 3], [1.; 3]), block([1., 0., 0.], [2., 1., 2.])]);
    let lower = cube([0.; 3], [2., 1., 1.]);
    let result = CutMaterial::intersect_certified(&[&stepped, &lower], TOL)
        .unwrap()
        .unwrap();
    volume(&result, 2.);
    labels(&result, &[&stepped, &lower], false);
    let sources: BTreeSet<_> = result
        .boundary
        .faces()
        .iter()
        .filter(|f| f.normal[2] > 0.99)
        .map(|f| f.source)
        .collect();
    assert_eq!(sources, BTreeSet::from([0, 1]));
    // Visit cancellation in the matched finite-patch subtraction itself,
    // including the checkpoint immediately before its polygon partition.
    let count = Cell::new(0usize);
    execute(
        Operation::Intersect,
        &[&stepped, &lower],
        Some(&|| {
            count.set(count.get() + 1);
            false
        }),
    )
    .unwrap();
    for cutoff in 0..count.get() {
        let calls = Cell::new(0usize);
        assert_eq!(
            execute(
                Operation::Intersect,
                &[&stepped, &lower],
                Some(&|| {
                    let n = calls.get();
                    calls.set(n + 1);
                    n == cutoff
                })
            ),
            Err(Error::Cancelled)
        );
    }
    let charts = convex_charts(&lower.boundary, &mut { MAX_WORK }, None).unwrap();
    let chart = &charts[0];
    let mut work = Work::new(None);
    work.remaining = chart.ring.len() + 1;
    assert_eq!(
        remove_owned_patches(
            vec![chart.ring.clone()],
            chart,
            std::slice::from_ref(chart),
            TOL,
            &mut work
        ),
        Err(Error::Limit)
    );
    let piece = &lower.pieces[0];
    work.remaining = piece.planes.len() * chart.ring.len() + 1;
    assert_eq!(
        outside(
            vec![chart.ring.clone()],
            chart,
            std::slice::from_ref(piece),
            [false, false],
            TOL,
            &mut work
        ),
        Err(Error::Limit)
    );
    let disjoint = cube([10.; 3], [12.; 3]);
    assert!(
        CutMaterial::intersect_certified(&[&stepped, &disjoint], TOL)
            .unwrap()
            .is_none()
    );
}

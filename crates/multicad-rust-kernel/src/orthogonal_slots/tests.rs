use super::*;
use crate::{
    orthogonal::{Box3, Operation, boolean},
    orthogonal_boundary::build,
};

const CLEARANCE: f64 = 1e-7;
const MINIMUM: f64 = 1e-5;
const STOCK: Box3 = Box3 {
    min: [0., 0., 0.],
    max: [20., 18., 10.],
};

fn stock() -> (OrthogonalSolid, Boundary) {
    fixture(&[STOCK], &[])
}

fn fixture(positive: &[Box3], negative: &[Box3]) -> (OrthogonalSolid, Boundary) {
    let stock = boolean(positive, negative, Operation::Subtract, CLEARANCE)
        .unwrap()
        .unwrap();
    let boundary = build(stock.quads()).unwrap();
    (stock, boundary)
}

fn tool(
    stock: &OrthogonalSolid,
    boundary: &Boundary,
    start: [Point; 2],
    travel: Point,
    radius: f64,
) -> Result<OrthogonalSlot, PocketError> {
    identify_slot(stock, boundary, start, travel, radius, CLEARANCE, MINIMUM)
}

fn top(stock: &OrthogonalSolid, boundary: &Boundary) -> OrthogonalSlot {
    tool(
        stock,
        boundary,
        [[2., 5., 11.], [5., 5., 11.]],
        [0., 0., -6.],
        0.7,
    )
    .unwrap()
}

#[test]
fn certifies_a_rotated_slot_on_a_groove_floor_and_computes_removed_volume() {
    let (stock, boundary) = fixture(
        &[STOCK],
        &[Box3 {
            min: [3., -1., 8.],
            max: [10., 19., 11.],
        }],
    );
    let start = [[5., 3., 12.], [7., 8., 12.]];
    let slot = tool(&stock, &boundary, start, [0., 0., -6.], 0.6).unwrap();
    let face = &boundary.faces[slot.face];
    assert_eq!(slot.centers_mm, [[5., 3., 8.], [7., 8., 8.]]);
    assert_eq!(
        slot.floor_centers_mm(face.normal),
        [[5., 3., 6.], [7., 8., 6.]]
    );
    assert_eq!(slot.profile(face).unwrap().centroid(), [6., 5.5]);
    let area = (2. * 29_f64.sqrt() + 0.6 * PI) * 0.6;
    assert!((slot.area_mm2() - area).abs() < 1e-14);
    assert!((removed_volume_mm3(&[slot]) - 2. * area).abs() < 1e-14);
    validate(&stock, &boundary, &[slot], &[], CLEARANCE, MINIMUM).unwrap();
    assert_eq!(
        tool(&stock, &boundary, start, [0., 0., -13.], 0.6),
        Err(PocketError::UnresolvedBoundary)
    );
    assert_eq!(
        tool(
            &stock,
            &boundary,
            [[3.2, 3., 12.], [7., 8., 12.]],
            [0., 0., -6.],
            0.6
        ),
        Err(PocketError::UnresolvedBoundary)
    );
}

#[test]
fn rejects_a_slot_crossing_a_hole_or_earlier_stock() {
    let (stock, boundary) = fixture(
        &[STOCK],
        &[Box3 {
            min: [8., 7., -1.],
            max: [12., 11., 11.],
        }],
    );
    assert_eq!(
        tool(
            &stock,
            &boundary,
            [[5., 9., 11.], [15., 9., 11.]],
            [0., 0., -4.],
            0.7
        ),
        Err(PocketError::UnresolvedBoundary)
    );
    let (stock, boundary) = fixture(
        &[
            STOCK,
            Box3 {
                min: [4., 6., 13.],
                max: [6., 12., 15.],
            },
        ],
        &[],
    );
    assert_eq!(
        tool(
            &stock,
            &boundary,
            [[2., 9., 17.], [9., 9., 17.]],
            [0., 0., -10.],
            0.7
        ),
        Err(PocketError::UnresolvedBoundary)
    );
}

#[test]
fn certifies_parallel_opposite_and_perpendicular_pocket_separation() {
    let (stock, boundary) = stock();
    let slot = top(&stock, &boundary);
    let check = |other| validate(&stock, &boundary, &[slot, other], &[], CLEARANCE, MINIMUM);
    let side = |y| {
        tool(
            &stock,
            &boundary,
            [[-1., y, 6.], [-1., y + 2., 6.]],
            [4., 0., 0.],
            0.7,
        )
        .unwrap()
    };
    let other = |start, travel| tool(&stock, &boundary, start, travel, 0.7).unwrap();
    assert_eq!(check(slot), Err(PocketError::UnresolvedBoundary));
    assert_eq!(check(side(5.)), Err(PocketError::UnresolvedBoundary));
    assert_eq!(check(side(12.)), Ok(()));
    assert_eq!(
        check(other([[2., 5., -1.], [5., 5., -1.]], [0., 0., 3.])),
        Ok(())
    );
    assert_eq!(
        check(other([[2., 8., 11.], [5., 8., 11.]], [0., 0., -5.])),
        Ok(())
    );

    let circle = |x, y| {
        orthogonal_pockets::identify_pocket(
            &stock,
            &boundary,
            [x, y, 11.],
            [x, y, 7.],
            0.7,
            CLEARANCE,
            MINIMUM,
        )
        .unwrap()
    };
    assert_eq!(
        validate(
            &stock,
            &boundary,
            &[slot],
            &[circle(4., 5.)],
            CLEARANCE,
            MINIMUM
        ),
        Err(PocketError::UnresolvedBoundary)
    );
    assert_eq!(
        validate(
            &stock,
            &boundary,
            &[slot],
            &[circle(4., 10.)],
            CLEARANCE,
            MINIMUM
        ),
        Ok(())
    );
}

#[test]
fn invalid_tool_boundary_and_proof_budget_return_typed_errors() {
    let (stock, mut boundary) = stock();
    let slot = top(&stock, &boundary);
    let start = [[2., 5., 11.], [5., 5., 11.]];
    for (centers, travel, radius) in [
        (start, [1., 0., -6.], 0.7),
        (start, [0., 0., -6.], f64::NAN),
        (start, [0., 0., -6.], 0.),
        (start, [0., 0., 0.], 0.7),
        (start, [0., f64::NAN, -6.], 0.7),
        ([[2., 5., 11.], [5., 5., 11.1]], [0., 0., -6.], 0.7),
        ([start[0]; 2], [0., 0., -6.], 0.7),
    ] {
        assert_eq!(
            tool(&stock, &boundary, centers, travel, radius),
            Err(PocketError::InvalidInput)
        );
    }
    assert_eq!(
        identify_slot(&stock, &boundary, start, [0., 0., -6.], 0.7, -1., MINIMUM),
        Err(PocketError::InvalidInput)
    );
    let check =
        |slots: &[OrthogonalSlot]| validate(&stock, &boundary, slots, &[], CLEARANCE, MINIMUM);
    assert_eq!(
        check(&vec![slot; slot_capacity(&stock) + 1]),
        Err(PocketError::BudgetExceeded)
    );
    for bad in [
        OrthogonalSlot {
            face: boundary.faces.len(),
            ..slot
        },
        OrthogonalSlot {
            depth_mm: f64::NAN,
            ..slot
        },
        OrthogonalSlot {
            depth_mm: 0.,
            ..slot
        },
        OrthogonalSlot {
            radius_mm: MINIMUM / 2.,
            ..slot
        },
        OrthogonalSlot {
            centers_mm: [[2., 5., 10.], [2. + MINIMUM / 2., 5., 10.]],
            ..slot
        },
        OrthogonalSlot {
            centers_mm: start,
            ..slot
        },
    ] {
        assert_eq!(check(&[bad]), Err(PocketError::InvalidInput));
    }
    boundary.faces[slot.face]
        .quad_indices
        .push(stock.quads().len());
    assert_eq!(
        validate(&stock, &boundary, &[slot], &[], CLEARANCE, MINIMUM),
        Err(PocketError::UnresolvedBoundary)
    );
    boundary.faces[slot.face].normal = [0.; 3];
    assert_eq!(
        slot.profile(&boundary.faces[slot.face]),
        Err(PocketError::InvalidInput)
    );
    assert_eq!(
        validate(&stock, &boundary, &[slot], &[], CLEARANCE, MINIMUM),
        Err(PocketError::InvalidInput)
    );
}

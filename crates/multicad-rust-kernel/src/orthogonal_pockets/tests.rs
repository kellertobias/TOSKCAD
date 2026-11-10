use super::*;
use crate::{
    orthogonal::{Operation, boolean},
    orthogonal_boundary::build,
};

pub(crate) const CLEARANCE: f64 = 1e-7;
pub(crate) const MINIMUM: f64 = 1e-5;

pub(crate) fn grooved_stock() -> (OrthogonalSolid, Boundary) {
    let stock = boolean(
        &[Box3 {
            min: [0.; 3],
            max: [20., 16., 10.],
        }],
        &[
            Box3 {
                min: [-1., 5., 7.],
                max: [21., 8., 11.],
            },
            Box3 {
                min: [12., 10., -1.],
                max: [15., 13., 11.],
            },
        ],
        Operation::Subtract,
        CLEARANCE,
    )
    .unwrap()
    .unwrap();
    let boundary = build(stock.quads()).unwrap();
    (stock, boundary)
}

pub(crate) fn tool(
    stock: &OrthogonalSolid,
    boundary: &Boundary,
    start: Point,
    finish: Point,
    radius: f64,
) -> Result<OrthogonalPocket, PocketError> {
    identify_pocket(stock, boundary, start, finish, radius, CLEARANCE, MINIMUM)
}

#[test]
fn identifies_a_groove_floor_and_preserves_both_preexisting_cuts() {
    let (stock, boundary) = grooved_stock();
    let pocket = tool(&stock, &boundary, [10., 6.5, 11.], [10., 6.5, 5.], 1.).unwrap();
    assert_eq!(pocket.center_mm, [10., 6.5, 7.]);
    assert_eq!(
        pocket.floor_center_mm(boundary.faces[pocket.face].normal),
        [10., 6.5, 5.]
    );
    assert_eq!(pocket.area_mm2(), PI);
    validate(&stock, &boundary, &[pocket], CLEARANCE, MINIMUM).unwrap();
    assert!((volume_mm3(&stock, &[pocket]) - (2930. - 2. * PI)).abs() < 1e-10);

    for (start, finish) in [
        ([10., 6.5, 11.], [10., 6.5, -1.]),
        ([10., 4.2, 11.], [10., 4.2, 5.]),
        ([13., 11., 11.], [13., 11., 5.]),
        ([10., 6.5, 6.], [10., 6.5, 2.]),
        ([10., 6.5, 11.], [10., 6.5, 8.]),
    ] {
        assert_eq!(
            tool(&stock, &boundary, start, finish, 1.),
            Err(PocketError::UnresolvedBoundary)
        );
    }
}

#[test]
fn pairs_prove_depth_separation_and_perpendicular_clearance() {
    let (stock, boundary) = grooved_stock();
    let top = |depth| tool(&stock, &boundary, [5., 2., 11.], [5., 2., 10. - depth], 1.).unwrap();
    let bottom = |depth| tool(&stock, &boundary, [5., 2., -1.], [5., 2., depth], 1.).unwrap();
    let side = |y, z, depth| tool(&stock, &boundary, [-1., y, z], [depth, y, z], 1.).unwrap();
    let check =
        |pockets: &[OrthogonalPocket]| validate(&stock, &boundary, pockets, CLEARANCE, MINIMUM);
    assert_eq!(check(&[top(2.), bottom(3.)]), Ok(()));
    assert_eq!(
        check(&[top(6.), bottom(6.)]),
        Err(PocketError::UnresolvedBoundary)
    );
    assert_eq!(
        check(&[top(2.), top(2.)]),
        Err(PocketError::UnresolvedBoundary)
    );
    assert_eq!(
        check(&[top(4.), side(2., 6.8, 6.)]),
        Err(PocketError::UnresolvedBoundary)
    );
    assert_eq!(check(&[top(4.), side(3.5, 5.2, 4.2)]), Ok(()));
    assert_eq!(check(&[top(4.), side(3.5, 2., 4.2)]), Ok(()));
    let top_other = tool(&stock, &boundary, [8., 2., 11.], [8., 2., 7.], 1.).unwrap();
    assert_eq!(check(&[top(3.), top_other]), Ok(()));
}

#[test]
fn invalid_specs_and_budget_are_typed_errors() {
    let (stock, mut boundary) = grooved_stock();
    let start = [10., 6.5, 11.];
    let finish = [10., 6.5, 5.];
    let pocket = tool(&stock, &boundary, start, finish, 1.).unwrap();
    assert_eq!(
        identify_pocket(&stock, &boundary, start, finish, 1., -1., MINIMUM),
        Err(PocketError::InvalidInput)
    );
    assert_eq!(
        validate(&stock, &boundary, &[], CLEARANCE, f64::NAN),
        Err(PocketError::InvalidInput)
    );
    for (start, finish, radius) in [
        ([f64::INFINITY, 6.5, 11.], finish, 1.),
        (start, finish, f64::NAN),
        (start, finish, 0.),
        (start, [11., 6.5, 5.], 1.),
        (start, start, 1.),
    ] {
        assert_eq!(
            tool(&stock, &boundary, start, finish, radius),
            Err(PocketError::InvalidInput)
        );
    }
    let invalid_pocket = |pocket| validate(&stock, &boundary, &[pocket], CLEARANCE, MINIMUM);
    for invalid in [
        OrthogonalPocket {
            face: boundary.faces.len(),
            ..pocket
        },
        OrthogonalPocket {
            center_mm: [f64::NAN; 3],
            ..pocket
        },
        OrthogonalPocket {
            radius_mm: 0.,
            ..pocket
        },
        OrthogonalPocket {
            depth_mm: 0.,
            ..pocket
        },
    ] {
        assert_eq!(invalid_pocket(invalid), Err(PocketError::InvalidInput));
    }
    assert_eq!(
        invalid_pocket(OrthogonalPocket {
            center_mm: [10., 6.5, 8.],
            ..pocket
        }),
        Err(PocketError::UnresolvedBoundary)
    );
    assert_eq!(
        validate(
            &stock,
            &boundary,
            &[pocket; MAX_POCKETS + 1],
            CLEARANCE,
            MINIMUM
        ),
        Err(PocketError::BudgetExceeded)
    );
    boundary.faces[pocket.face]
        .quad_indices
        .push(stock.quads().len());
    assert_eq!(
        validate(&stock, &boundary, &[pocket], CLEARANCE, MINIMUM),
        Err(PocketError::UnresolvedBoundary)
    );
    boundary.faces[pocket.face].normal = [1., 0., 1.];
    assert_eq!(
        validate(&stock, &boundary, &[pocket], CLEARANCE, MINIMUM),
        Err(PocketError::InvalidInput)
    );
    assert_eq!(
        tool(&stock, &boundary, start, finish, 1.),
        Err(PocketError::InvalidInput)
    );
}

#[test]
fn a_tool_cannot_cross_one_component_to_drill_another() {
    let stock = boolean(
        &[
            Box3 {
                min: [0.; 3],
                max: [10., 10., 4.],
            },
            Box3 {
                min: [0., 0., 6.],
                max: [10.; 3],
            },
        ],
        &[],
        Operation::Union,
        CLEARANCE,
    )
    .unwrap()
    .unwrap();
    assert_eq!(stock.solid_count(), 2);
    let boundary = build(stock.quads()).unwrap();
    assert_eq!(
        tool(&stock, &boundary, [5., 5., 11.], [5., 5., 2.], 1.),
        Err(PocketError::UnresolvedBoundary),
    );
    let nearest = tool(&stock, &boundary, [5., 5., 11.], [5., 5., 8.], 1.).unwrap();
    assert_eq!(nearest.center_mm, [5., 5., 10.]);
    validate(&stock, &boundary, &[nearest], CLEARANCE, MINIMUM).unwrap();
}

#[test]
fn stepped_end_plate_retains_two_blind_insert_bores_and_one_through_bore() {
    let stock = boolean(
        &[
            Box3 {
                min: [0.; 3],
                max: [43., 86., 4.],
            },
            Box3 {
                min: [3.3, 1.8, 4.],
                max: [43., 84.2, 20.],
            },
        ],
        &[],
        Operation::Union,
        CLEARANCE,
    )
    .unwrap()
    .unwrap();
    let boundary = build(stock.quads()).unwrap();
    let bottom = tool(&stock, &boundary, [23., 0.8, 12.], [23., 9.8, 12.], 2.).unwrap();
    let top = tool(&stock, &boundary, [23., 85.2, 12.], [23., 76.2, 12.], 2.).unwrap();
    let start = [31., 43., -1.];
    let finish = [31., 43., 21.];
    let bore =
        identify_through_bore(&stock, &boundary, start, finish, 4., CLEARANCE, MINIMUM).unwrap();
    assert_eq!(bore.centers_mm, [[31., 43., 0.], [31., 43., 20.]]);
    assert_eq!(
        bore.faces.map(|i| boundary.faces[i].normal),
        [[0., 0., -1.], [0., 0., 1.]]
    );
    assert_eq!(bore.depth_mm(), 20.);
    assert_eq!(bore.area_mm2(), 16. * PI);
    let reverse =
        identify_through_bore(&stock, &boundary, finish, start, 4., CLEARANCE, MINIMUM).unwrap();
    assert_eq!(reverse.centers_mm, [bore.centers_mm[1], bore.centers_mm[0]]);
    assert_eq!(reverse.faces, [bore.faces[1], bore.faces[0]]);
    let pockets = [bottom, top];
    validate_with_through_bores(&stock, &boundary, &pockets, &[bore], CLEARANCE, MINIMUM).unwrap();
    let expected = stock.volume_mm3() - 384. * PI;
    assert!((volume_with_through_bores_mm3(&stock, &pockets, &[bore]) - expected).abs() < 1e-10);
    assert_eq!(
        validate_with_through_bores(&stock, &boundary, &[], &[bore, reverse], CLEARANCE, MINIMUM),
        Err(PocketError::UnresolvedBoundary)
    );
    let colliding = tool(&stock, &boundary, [31., 0.8, 12.], [31., 47., 12.], 2.).unwrap();
    assert_eq!(
        validate_with_through_bores(&stock, &boundary, &[colliding], &[bore], CLEARANCE, MINIMUM),
        Err(PocketError::UnresolvedBoundary)
    );
}

#[test]
fn a_through_tool_must_clear_both_mouths_and_never_cross_a_void() {
    let (stock, boundary) = grooved_stock();
    let bore = |start, finish, radius| {
        identify_through_bore(&stock, &boundary, start, finish, radius, CLEARANCE, MINIMUM)
    };
    let good = bore([10., 6.5, 11.], [10., 6.5, -1.], 1.).unwrap();
    assert_eq!(good.centers_mm, [[10., 6.5, 7.], [10., 6.5, 0.]]);
    for (start, finish, radius) in [
        ([10., 6.5, 11.], [10., 6.5, 0.], 1.),
        ([10., 6.5, 11.], [10., 6.5, 5.], 1.),
        ([10., 4.2, 11.], [10., 4.2, -1.], 1.),
        ([13., 11., 11.], [13., 11., -1.], 1.),
        ([-1., 11., 5.], [21., 11., 5.], 1.),
    ] {
        assert_eq!(
            bore(start, finish, radius),
            Err(PocketError::UnresolvedBoundary)
        );
    }
    let validate = |candidate| {
        validate_with_through_bores(&stock, &boundary, &[], &[candidate], CLEARANCE, MINIMUM)
    };
    assert_eq!(
        validate(OrthogonalThroughBore {
            radius_mm: 2.,
            ..good
        }),
        Err(PocketError::UnresolvedBoundary)
    );
    for invalid in [
        OrthogonalThroughBore {
            faces: [good.faces[0], good.faces[0]],
            ..good
        },
        OrthogonalThroughBore {
            faces: [good.faces[0], boundary.faces.len()],
            ..good
        },
        OrthogonalThroughBore {
            centers_mm: [[10., 6.5, 8.], good.centers_mm[1]],
            ..good
        },
        OrthogonalThroughBore {
            centers_mm: [good.centers_mm[0], [11., 6.5, 0.]],
            ..good
        },
        OrthogonalThroughBore {
            centers_mm: [good.centers_mm[0], [10., 6.5, f64::NAN]],
            ..good
        },
        OrthogonalThroughBore {
            radius_mm: 0.,
            ..good
        },
    ] {
        assert_eq!(validate(invalid), Err(PocketError::InvalidInput));
    }
}

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

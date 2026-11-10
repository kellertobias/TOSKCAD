use super::*;

const X: Point = [1., 0., 0.];
const Y: Point = [0., 1., 0.];
const ORIGIN: Point = [0.; 3];

fn size() -> [Rational; 3] {
    [5, 7, 9].map(|value| Rational::new(value, 1).unwrap())
}

#[test]
fn rejected_member_rolls_back_geometry_and_entity_ids() {
    let mut writer = Writer::default();
    assert_eq!(writer.finish(), Err(StepError::InvalidGeometry));

    writer = Writer::new();
    writer.append_box(size(), ORIGIN, X, Y).unwrap();
    assert_eq!(
        writer.append_box(size(), [1e100; 3], X, Y),
        Err(StepError::InvalidGeometry)
    );
    let single = write_box(size(), ORIGIN, X, Y).unwrap();
    assert_eq!(writer.finish().unwrap(), single);

    let mut writer = Writer::new();
    writer.append_box(size(), ORIGIN, X, Y).unwrap();
    assert!(writer.append_box(size(), [1e100; 3], X, Y).is_err());
    writer.append_box(size(), [14., 0., 0.], X, Y).unwrap();
    let actual = writer.finish().unwrap();
    let mut writer = Writer::new();
    writer.append_box(size(), ORIGIN, X, Y).unwrap();
    writer.append_box(size(), [14., 0., 0.], X, Y).unwrap();
    assert_eq!(actual, writer.finish().unwrap());
}

#[test]
fn compound_respects_member_and_part21_budgets() {
    let mut writer = Writer::new();
    for index in 0..MAX_COMPOUND_SOLIDS {
        writer
            .append_placed_circular_sweep(1., None, 2., [3. * index as f64, 0., 0.], X, Y)
            .unwrap();
    }
    assert_eq!(
        writer.append_box(size(), ORIGIN, X, Y),
        Err(StepError::Limit)
    );
    assert_eq!(
        writer
            .finish()
            .unwrap()
            .matches("MANIFOLD_SOLID_BREP(")
            .count(),
        MAX_COMPOUND_SOLIDS
    );

    let mut writer = Writer::new();
    writer.append_box(size(), ORIGIN, X, Y).unwrap();
    writer.file.next = MAX_STEP_ENTITIES;
    let before = writer.file.data.clone();
    assert_eq!(
        writer.append_box(size(), [10., 0., 0.], X, Y),
        Err(StepError::Limit)
    );
    assert_eq!(writer.file.data, before);
    assert_eq!(writer.file.next, MAX_STEP_ENTITIES);
    writer
        .file
        .data
        .push_str(&" ".repeat(MAX_PLANAR_STEP_BYTES));
    assert_eq!(
        writer.append_box(size(), [10., 0., 0.], X, Y),
        Err(StepError::Limit)
    );

    let mut writer = Writer::new();
    writer.append_box(size(), ORIGIN, X, Y).unwrap();
    writer.append_box(size(), [10., 0., 0.], X, Y).unwrap();
    writer.file.next = MAX_STEP_ENTITIES;
    assert_eq!(writer.finish(), Err(StepError::Limit));
}

#[test]
fn mixed_compound_keeps_placed_stadiums_and_machined_stock_as_separate_shells() {
    use crate::{
        orthogonal::{Box3, Operation, boolean},
        orthogonal_boundary::build,
        step_read::{ReadError, read_analytic_compound, read_analytic_solid},
    };
    let stock = boolean(
        &[Box3 {
            min: ORIGIN,
            max: [20., 16., 10.],
        }],
        &[],
        Operation::Union,
        1e-7,
    )
    .unwrap()
    .unwrap();
    let boundary = build(stock.quads()).unwrap();
    let face = boundary
        .faces
        .iter()
        .position(|face| face.normal == [0., 0., 1.])
        .unwrap();
    let pocket = OrthogonalPocket {
        face,
        center_mm: [16., 7., 10.],
        radius_mm: 1.,
        depth_mm: 2.,
    };
    let slot = OrthogonalSlot {
        face,
        centers_mm: [[4., 7., 10.], [9., 7., 10.]],
        radius_mm: 1.5,
        depth_mm: 3.,
    };
    let profile = Stadium::new([[2., 3.], [8., 3.]], 1.5).unwrap();
    let place = |[x, y, z]: Point| [y + 7., -x + 2., z + 9.];
    let round = |writer: &mut Writer| {
        writer.append_placed_circular_sweep(4., Some(1.), 3., [0., 0., 30.], X, Y)
    };
    let mut writer = Writer::new();
    writer
        .append_stadium_prism(&profile, 4., ORIGIN, X, Y, false)
        .unwrap();
    writer
        .append_slotted_orthogonal(&stock, &boundary, &[pocket], &[slot], &place)
        .unwrap();
    writer
        .append_stadium_prism(&profile, 4., [35., 0., 0.], Y, [0., 0., 1.], true)
        .unwrap();
    round(&mut writer).unwrap();
    let encoded = writer.finish().unwrap();
    assert_eq!(
        encoded
            .matches("ADVANCED_BREP_SHAPE_REPRESENTATION(")
            .count(),
        1
    );
    assert_eq!(encoded.matches("CLOSED_SHELL(").count(), 4);
    assert_eq!(encoded.matches("MANIFOLD_SOLID_BREP(").count(), 4);
    assert_eq!(
        read_analytic_solid(&encoded, 1e-7),
        Err(ReadError::Unsupported)
    );

    let readback = read_analytic_compound(&encoded, 1e-7).unwrap();
    let expected = [
        write_stadium_prism(&profile, 4., ORIGIN, X, Y, false).unwrap(),
        write_slotted_orthogonal(&stock, &boundary, &[pocket], &[slot], &place).unwrap(),
        write_stadium_prism(&profile, 4., [35., 0., 0.], Y, [0., 0., 1.], true).unwrap(),
        write_placed_circular_sweep(4., Some(1.), 3., [0., 0., 30.], X, Y).unwrap(),
    ];
    assert_eq!(
        readback,
        expected.map(|member| read_analytic_solid(&member, 1e-7).unwrap())
    );
}

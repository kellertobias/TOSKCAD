use super::*;
use crate::{
    orthogonal::{Box3, Operation, boolean},
    planar_boolean::{ConvexPiece, Union},
    step_read::{read_analytic_compound, read_import_compound},
};
fn fixture() -> (Boundary, Union) {
    let solid = boolean(
        &[Box3 {
            min: [0.; 3],
            max: [15., 10., 8.],
        }],
        &[Box3 {
            min: [2., 3., 1.],
            max: [7., 7., 4.],
        }],
        Operation::Subtract,
        1e-7,
    )
    .unwrap()
    .unwrap();
    let boundary = crate::orthogonal_boundary::build(solid.quads()).unwrap();
    let pieces = solid
        .cells()
        .iter()
        .map(|cell| ConvexPiece::from_box(cell.min, cell.max, 1e-7).unwrap())
        .collect::<Vec<_>>();
    (boundary, Union::new(&pieces, 1e-7).unwrap())
}
#[test]
fn planar_and_orthogonal_cavities_share_exact_one_solid_exchange() {
    let (boundary, union) = fixture();
    assert_eq!(union.shell_count(), 2);
    assert_eq!(union.solid_count(), 1);
    let u = [0.8, 0.6, 0.];
    let v = [-0.36, 0.48, 0.8];
    let w = cross(u, v);
    let origin = [13., -5., 41.];
    let mut writer = Writer::new();
    writer
        .append_orthogonal(&boundary, |point| {
            std::array::from_fn(|i| origin[i] + u[i] * point[0] + v[i] * point[1] + w[i] * point[2])
        })
        .unwrap();
    writer
        .append_planar_union(&union, [-20., 0., 0.], [1., 0., 0.], [0., 1., 0.])
        .unwrap();
    writer
        .append_box(
            [Rational::new(2, 1).unwrap(); 3],
            [50., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
        )
        .unwrap();
    let text = writer.finish().unwrap();
    assert_eq!(text.matches("=BREP_WITH_VOIDS(").count(), 2);
    assert_eq!(text.matches("=MANIFOLD_SOLID_BREP(").count(), 1);
    let members = read_analytic_compound(&text, 1e-7).unwrap();
    assert_eq!(members.len(), 3);
    for (member, volume) in members.iter().zip([1140., 1140., 8.]) {
        assert!((member.volume_mm3 - volume).abs() < 1e-7);
        assert_eq!(member.solid_count, 1);
    }
    assert_eq!(read_import_compound(&text, 1e-7).unwrap().len(), 3);
}
#[test]
fn rejected_void_transforms_and_added_recesses_leave_writer_unchanged() {
    let (boundary, union) = fixture();
    let unit = Union::new(
        &[ConvexPiece::from_box([0.; 3], [1.; 3], 1e-7).unwrap()],
        1e-7,
    )
    .unwrap();
    let mut writer = Writer::new();
    writer
        .append_planar_union(&unit, [0.; 3], [1., 0., 0.], [0., 1., 0.])
        .unwrap();
    assert_eq!(
        writer.append_orthogonal(&boundary, |_| [0.; 3]),
        Err(StepError::InvalidGeometry)
    );
    assert_eq!(
        writer.append_orthogonal(&boundary, |_| [f64::NAN; 3]),
        Err(StepError::InvalidGeometry)
    );
    assert_eq!(
        writer.append_planar_union(&union, [1e20; 3], [1., 0., 0.], [0., 1., 0.]),
        Err(StepError::InvalidGeometry)
    );
    assert_eq!(
        writer.append_orthogonal(&boundary, |[x, y, z]| [x + y, y, z]),
        Err(StepError::InvalidGeometry)
    );
    assert_eq!(
        writer.finish().unwrap(),
        write_planar_union(&unit, [0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap()
    );
    let pocket = OrthogonalPocket {
        face: 0,
        center_mm: [0.; 3],
        radius_mm: 1.,
        depth_mm: 1.,
    };
    assert!(!supports_pocketed_orthogonal(&boundary, &[pocket]));
    assert_eq!(
        encode_orthogonal(&mut StepFile::new(), &boundary, &[pocket], &[], &[], &|p| p).err(),
        Some(StepError::Unsupported)
    );
}

#[test]
fn more_than_one_cavity_is_not_published_as_independent_material_solids() {
    let outer = Box3 {
        min: [0.; 3],
        max: [15., 10., 8.],
    };
    let holes = [
        Box3 {
            min: [2., 3., 1.],
            max: [5., 7., 4.],
        },
        Box3 {
            min: [8., 3., 1.],
            max: [11., 7., 4.],
        },
    ];
    let solid = boolean(&[outer], &holes, Operation::Subtract, 1e-7)
        .unwrap()
        .unwrap();
    let boundary = crate::orthogonal_boundary::build(solid.quads()).unwrap();
    assert_eq!(boundary.shell_count, 3);
    assert!(!boundary.supports_step());
    let pieces = solid
        .cells()
        .iter()
        .map(|cell| ConvexPiece::from_box(cell.min, cell.max, 1e-7).unwrap())
        .collect::<Vec<_>>();
    let union = Union::new(&pieces, 1e-7).unwrap();
    assert_eq!(union.solid_count(), 1);
    assert_eq!(union.shell_count(), 3);
    assert!(!supports_planar_union(&union));
    assert_eq!(
        write_planar_union(&union, [0.; 3], [1., 0., 0.], [0., 1., 0.]),
        Err(StepError::Unsupported)
    );
}

#[test]
fn only_conforming_collinear_subdivisions_can_be_written_as_closed_shells() {
    let (mut boundary, _) = fixture();
    let [a, b] = [boundary.faces[0].rings[0][0], boundary.faces[0].rings[0][1]];
    let midpoint = std::array::from_fn(|i| a[i].midpoint(b[i]));
    boundary.faces[0].rings[0].insert(1, midpoint);
    assert!(!boundary.supports_step());
    let mut writer = Writer::new();
    let unchanged = |point: Point| point;
    assert_eq!(
        writer.append_orthogonal(&boundary, unchanged),
        Err(StepError::Unsupported)
    );
    let adjacent = boundary.faces.iter_mut().skip(1).find_map(|face| {
        let ring = &mut face.rings[0];
        let index = (0..ring.len()).find(|&i| ring[i] == b && ring[(i + 1) % ring.len()] == a)?;
        ring.insert(index + 1, midpoint);
        Some(())
    });
    assert!(adjacent.is_some());
    assert!(boundary.supports_step());
    writer.append_orthogonal(&boundary, unchanged).unwrap();
    let text = writer.finish().unwrap();
    let read = read_analytic_compound(&text, 1e-7).unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].solid_count, 1);
    assert_eq!(read[0].edges_mm.len(), 25);
    assert_eq!(read[0].volume_mm3, 1140.);
}

#[test]
fn a_retraced_same_face_spike_cannot_be_written_as_a_closed_box_boundary() {
    let (mut boundary, _) = fixture();
    let a = boundary.faces[0].rings[0][0];
    let b = boundary.faces[0].rings[0][1];
    let midpoint = std::array::from_fn(|i| a[i].midpoint(b[i]));
    boundary.faces[0].rings[0].splice(1..1, [midpoint, a]);
    assert!(!boundary.supports_step());
    assert_eq!(
        write_orthogonal(&boundary, |p| p),
        Err(StepError::Unsupported)
    );
}

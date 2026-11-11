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
    let (boundary, _) = fixture();
    let stock = ConvexPiece::from_box([0.; 3], [15., 10., 8.], 1e-7).unwrap();
    let holes =
        [2., 6., 10.].map(|x| ConvexPiece::from_box([x, 3., 1.], [x + 1., 4., 2.], 1e-7).unwrap());
    let union = Union::subtract(&[stock], &holes, 1e-7).unwrap().unwrap();
    assert_eq!(union.shell_count(), 4);
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
    assert_eq!(text.matches("=ORIENTED_CLOSED_SHELL(").count(), 4);
    assert_eq!(text.matches("=MANIFOLD_SOLID_BREP(").count(), 1);
    let members = read_analytic_compound(&text, 1e-7).unwrap();
    assert_eq!(members.len(), 3);
    for (member, volume) in members.iter().zip([1140., 1197., 8.]) {
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
fn several_cavities_are_published_as_one_material_solid() {
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
    assert!(boundary.supports_step());
    let pieces = solid
        .cells()
        .iter()
        .map(|cell| ConvexPiece::from_box(cell.min, cell.max, 1e-7).unwrap())
        .collect::<Vec<_>>();
    let union = Union::new(&pieces, 1e-7).unwrap();
    assert_eq!(union.solid_count(), 1);
    assert_eq!(union.shell_count(), 3);
    assert!(supports_planar_union(&union));
    let text = write_planar_union(&union, [0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
    assert_eq!(text.matches("=BREP_WITH_VOIDS(").count(), 1);
    assert_eq!(text.matches("=ORIENTED_CLOSED_SHELL(").count(), 2);
    let read = read_analytic_compound(&text, 1e-7).unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].solid_count, 1);
    assert_eq!(read[0].volume_mm3, 1128.);
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
        Err(StepError::InvalidGeometry)
    );
    let mut oversized = fixture().0;
    oversized.faces[0].rings[0] = vec![[0.; 3]; crate::orthogonal_boundary::box_void::MAX_WORK + 1];
    assert_eq!(
        writer.append_orthogonal(&oversized, unchanged),
        Err(StepError::Limit)
    );
    assert!(matches!(
        crate::orthogonal_boundary::box_void::boundary_certificate(&oversized),
        Err(crate::orthogonal::BooleanError::Limit)
    ));
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
        Err(StepError::InvalidGeometry)
    );
}

#[test]
fn resource_and_nonrectangular_refusals_keep_prior_members_and_typed_errors() {
    let piece = |min, max| ConvexPiece::from_box(min, max, 1e-7).unwrap();
    let stock = piece([0.; 3], [52.; 3]);
    let holes = (0..25)
        .map(|i| piece([(2 * i + 1) as f64; 3], [(2 * i + 2) as f64; 3]))
        .collect::<Vec<_>>();
    let limited = Union::subtract(&[stock], &holes, 1e-7).unwrap().unwrap();
    let mut writer = Writer::new();
    let limited_boundary = Boundary {
        shell_count: limited.shell_count(),
        faces: limited
            .faces()
            .iter()
            .map(|face| crate::orthogonal_boundary::BoundaryFace {
                component: 0,
                rings: vec![
                    face.vertices
                        .iter()
                        .map(|&i| limited.vertices()[i])
                        .collect(),
                ],
                quad_indices: Vec::new(),
                normal: face.normal,
            })
            .collect(),
    };
    assert_eq!(
        writer.append_orthogonal(&limited_boundary, |p| p),
        Err(StepError::Limit)
    );
    assert_eq!(
        writer.append_planar_union(&limited, [0.; 3], [1., 0., 0.], [0., 1., 0.]),
        Err(StepError::Limit)
    );
    let faces = union_faces(&limited);
    let mut file = StepFile::new();
    let before = file.data.clone();
    assert_eq!(
        encode(&mut file, &faces, &|p| p).err(),
        Some(StepError::Limit)
    );
    assert_eq!(file.data, before);
    let l_shaped = Union::subtract(
        &[piece([0.; 3], [15.; 3])],
        &[piece([2.; 3], [6.; 3]), piece([5., 2., 2.], [8., 4., 6.])],
        1e-7,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        writer.append_planar_union(&l_shaped, [0.; 3], [1., 0., 0.], [0., 1., 0.]),
        Err(StepError::Unsupported)
    );
    let valid = Union::subtract(
        &[piece([0.; 3], [15.; 3])],
        &[
            piece([2.; 3], [4.; 3]),
            piece([6.; 3], [8.; 3]),
            piece([10.; 3], [12.; 3]),
        ],
        1e-7,
    )
    .unwrap()
    .unwrap();
    writer
        .append_planar_union(&valid, [0.; 3], [1., 0., 0.], [0., 1., 0.])
        .unwrap();
    let text = writer.finish().unwrap();
    let read = read_analytic_compound(&text, 1e-7).unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].volume_mm3, 3351.);
}

#[test]
fn multiple_cavity_member_uses_one_slot_at_the_actual_compound_capacity() {
    let stock = ConvexPiece::from_box([0.; 3], [15.; 3], 1e-7).unwrap();
    let holes = [2., 6., 10.].map(|p| ConvexPiece::from_box([p; 3], [p + 2.; 3], 1e-7).unwrap());
    let union = Union::subtract(&[stock], &holes, 1e-7).unwrap().unwrap();
    let mut writer = Writer::new();
    writer
        .append_planar_union(&union, [0.; 3], [1., 0., 0.], [0., 1., 0.])
        .unwrap();
    for _ in 1..MAX_COMPOUND_SOLIDS {
        writer
            .append_box(
                [Rational::new(2, 1).unwrap(); 3],
                [50., 0., 0.],
                [1., 0., 0.],
                [0., 1., 0.],
            )
            .unwrap();
    }
    assert_eq!(
        writer.append_planar_union(&union, [0.; 3], [1., 0., 0.], [0., 1., 0.]),
        Err(StepError::Limit)
    );
    assert_eq!(
        writer.append_box(
            [Rational::new(2, 1).unwrap(); 3],
            [50., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.]
        ),
        Err(StepError::Limit)
    );
    let text = writer.finish().unwrap();
    assert_eq!(text.matches("=BREP_WITH_VOIDS(").count(), 1);
    assert_eq!(text.matches("=ORIENTED_CLOSED_SHELL(").count(), 3);
    let members = read_analytic_compound(&text, 1e-7).unwrap();
    assert_eq!(members.len(), MAX_COMPOUND_SOLIDS);
    assert_eq!(members[0].volume_mm3, 3351.);
    assert!(
        members[1..]
            .iter()
            .all(|member| member.solid_count == 1 && member.volume_mm3 == 8.)
    );
}

#[test]
fn common_count_and_graph_failures_leave_the_writer_and_encoder_atomic() {
    let (mut cavity, _) = fixture();
    let mut writer = Writer::new();
    let unchanged = |p: Point| p;
    cavity.shell_count = 1;
    assert_eq!(
        writer.append_orthogonal(&cavity, unchanged),
        Err(StepError::InvalidGeometry)
    );
    let mut file = StepFile::new();
    let before = file.data.clone();
    assert_eq!(
        encode_orthogonal(&mut file, &cavity, &[], &[], &[], &unchanged).err(),
        Some(StepError::InvalidGeometry)
    );
    assert_eq!(file.data, before);
    cavity.shell_count = 2;
    writer.append_orthogonal(&cavity, unchanged).unwrap();
    let mut over_budget = fixture().0;
    over_budget.shell_count = 1;
    over_budget.faces[0].rings[0] =
        vec![[0.; 3]; crate::orthogonal_boundary::box_void::MAX_WORK + 1];
    assert_eq!(
        writer.append_orthogonal(&over_budget, unchanged),
        Err(StepError::Limit)
    );
    assert_eq!(
        encode_orthogonal(&mut file, &over_budget, &[], &[], &[], &unchanged).err(),
        Some(StepError::Limit)
    );
    assert_eq!(file.data, before);
    let text = writer.finish().unwrap();
    let read = read_analytic_compound(&text, 1e-7).unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].volume_mm3, 1140.);
}

#[test]
fn valid_but_unsupported_multiple_outer_shells_never_reach_the_encoder() {
    let source = boolean(
        &[
            Box3 {
                min: [0.; 3],
                max: [2.; 3],
            },
            Box3 {
                min: [4.; 3],
                max: [6.; 3],
            },
        ],
        &[],
        Operation::Union,
        1e-7,
    )
    .unwrap()
    .unwrap();
    let boundary = crate::orthogonal_boundary::build(source.quads()).unwrap();
    let mut file = StepFile::new();
    let before = file.data.clone();
    assert_eq!(
        encode_orthogonal(&mut file, &boundary, &[], &[], &[], &|p| p).err(),
        Some(StepError::Unsupported)
    );
    assert_eq!(file.data, before);
}

#[test]
fn pocket_and_drill_entry_points_preserve_typed_common_admission_failures() {
    let stock = boolean(
        &[Box3 {
            min: [0.; 3],
            max: [15., 10., 8.],
        }],
        &[],
        Operation::Union,
        1e-7,
    )
    .unwrap()
    .unwrap();
    let (mut boundary, _) = fixture();
    boundary.shell_count = 1;
    let mut writer = Writer::new();
    let bore = OrthogonalThroughBore {
        faces: [0, 1],
        centers_mm: [[0.; 3]; 2],
        radius_mm: 1.,
    };
    assert_eq!(
        writer.append_pocketed_orthogonal(&stock, &boundary, &[], &|p| p),
        Err(StepError::InvalidGeometry)
    );
    assert_eq!(
        writer.append_drilled_orthogonal(&stock, &boundary, &[], &[bore], &|p| p),
        Err(StepError::InvalidGeometry)
    );
    boundary.faces[0].rings[0] = vec![[0.; 3]; crate::orthogonal_boundary::box_void::MAX_WORK + 1];
    assert_eq!(
        writer.append_pocketed_orthogonal(&stock, &boundary, &[], &|p| p),
        Err(StepError::Limit)
    );
    assert_eq!(
        writer.append_drilled_orthogonal(&stock, &boundary, &[], &[bore], &|p| p),
        Err(StepError::Limit)
    );
    let valid = crate::orthogonal_boundary::build(stock.quads()).unwrap();
    writer
        .append_pocketed_orthogonal(&stock, &valid, &[], &|p| p)
        .unwrap();
    assert_eq!(
        read_analytic_compound(&writer.finish().unwrap(), 1e-7)
            .unwrap()
            .len(),
        1
    );
}

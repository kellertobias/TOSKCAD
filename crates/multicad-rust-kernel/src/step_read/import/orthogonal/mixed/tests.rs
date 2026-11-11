use super::*;
use crate::{
    orthogonal::{Box3, Operation, boolean},
    polygon::MAX_VERTICES,
    step::{write_drilled_orthogonal, write_placed_circular_sweep, write_pocketed_orthogonal},
};

const TOLERANCE: f64 = 1e-7;
const XYZ: [Point; 3] = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];

fn fixture() -> (OrthogonalSolid, String) {
    let stock = boolean(
        &[Box3 {
            min: [0.; 3],
            max: [20., 14., 9.],
        }],
        &[Box3 {
            min: [8., 0., 6.],
            max: [12., 14., 10.],
        }],
        Operation::Subtract,
        0.,
    )
    .unwrap()
    .unwrap();
    let boundary = orthogonal_boundary::build(stock.quads()).unwrap();
    let face = boundary
        .faces
        .iter()
        .position(|face| face.normal == XYZ[0] && face.rings[0][0][0] == 20.)
        .unwrap();
    let pocket = OrthogonalPocket {
        face,
        center_mm: [20., 8., 4.],
        radius_mm: 1.5,
        depth_mm: 2.,
    };
    let step = write_pocketed_orthogonal(&stock, &boundary, &[pocket], &|p| p).unwrap();
    let oversized = OrthogonalPocket {
        radius_mm: 9.,
        ..pocket
    };
    assert_eq!(
        write_pocketed_orthogonal(&stock, &boundary, &[oversized], &|p| p),
        Err(crate::step::StepError::InvalidGeometry)
    );
    assert_eq!(
        write_pocketed_orthogonal(&stock, &boundary, &[pocket], &|p| {
            if p == pocket.center_mm {
                [f64::NAN; 3]
            } else {
                p
            }
        }),
        Err(crate::step::StepError::InvalidGeometry)
    );
    (stock, step)
}

fn drilled_stock() -> (
    OrthogonalSolid,
    Boundary,
    OrthogonalPocket,
    [OrthogonalThroughBore; 2],
) {
    let (stock, _) = fixture();
    let boundary = orthogonal_boundary::build(stock.quads()).unwrap();
    let pocket = orthogonal_pockets::identify_pocket(
        &stock,
        &boundary,
        [21., 8., 4.],
        [18., 8., 4.],
        1.5,
        TOLERANCE,
        TOLERANCE,
    )
    .unwrap();
    let cut = |start, end, radius| {
        orthogonal_pockets::identify_through_bore(
            &stock, &boundary, start, end, radius, TOLERANCE, TOLERANCE,
        )
        .unwrap()
    };
    let bores = [
        cut([5., 4., 10.], [5., 4., -1.], 1.),
        cut([15., -1., 4.], [15., 15., 4.], 1.25),
    ];
    (stock, boundary, pocket, bores)
}

#[test]
fn reconstructs_rotated_stock_and_both_orthogonal_through_axes() {
    let (stock, boundary, pocket, bores) = drilled_stock();
    let placement = [900., -200., 500.];
    let axes = [[0.8, 0.6, 0.], [-0.36, 0.48, 0.8], [0.48, -0.64, 0.6]];
    let place = |point: Point| {
        std::array::from_fn(|i| (0..3).fold(placement[i], |v, j| point[j].mul_add(axes[j][i], v)))
    };
    let text = write_drilled_orthogonal(&stock, &boundary, &[pocket], &bores, &place).unwrap();
    let (readback, rebuilt) = read_import_solid(&text, TOLERANCE).unwrap();
    let volume = orthogonal_pockets::volume_with_through_bores_mm3(&stock, &[pocket], &bores);
    assert!((readback.volume_mm3 - volume).abs() < TOLERANCE);
    assert_eq!(readback.circles.len(), 6);
    let ImportProfile::DrilledOrthogonal {
        solid,
        pockets,
        through,
    } = rebuilt.profile
    else {
        panic!("expected an exact drilled orthogonal body");
    };
    assert_eq!(pockets.len(), 1);
    assert_eq!(through.len(), 2);
    assert!((solid.volume_mm3() - stock.volume_mm3()).abs() < TOLERANCE);
    assert!(
        (orthogonal_pockets::volume_with_through_bores_mm3(&solid, &pockets, &through) - volume)
            .abs()
            < TOLERANCE
    );
    let placed = |point: Point| {
        let axes = [rebuilt.u, rebuilt.v, cross(rebuilt.u, rebuilt.v)];
        std::array::from_fn(|i| {
            (0..3).fold(rebuilt.origin_mm[i], |v, j| point[j].mul_add(axes[j][i], v))
        })
    };
    for expected in bores {
        let actual = through
            .iter()
            .find(|item| (item.radius_mm - expected.radius_mm).abs() < TOLERANCE)
            .unwrap();
        let actual = actual.centers_mm.map(placed);
        let expected = expected.centers_mm.map(place);
        for point in expected {
            assert!(
                actual
                    .iter()
                    .any(|&candidate| length(sub(point, candidate)) < TOLERANCE)
            );
        }
    }
}

#[test]
fn through_rims_require_opposed_supports_and_combined_cavity_clearance() {
    let (stock, boundary, pocket, bores) = drilled_stock();
    let text = write_drilled_orthogonal(&stock, &boundary, &[pocket], &bores, &|p| p).unwrap();
    let entities = parse_entities(&text, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let mut faces = shell.validated_faces().unwrap();
    let found = shell.orthogonal_recesses(&faces).unwrap();
    assert_eq!(found.through.len(), 2);
    assert_eq!(found.pockets.len(), 1);
    assert_eq!(found.omitted.len(), 4);

    let rim = found.through[0];
    let opposite = faces.iter().position(|face| matches!(face, Face::Planar { origin, normal, .. }
        if *normal == rim.outward.map(|n| -n) && dot(sub(*origin, rim.centers[1]), *normal).abs() < TOLERANCE
    )).unwrap();
    let Face::Planar { normal, .. } = &mut faces[opposite] else {
        unreachable!()
    };
    *normal = rim.outward;
    assert!(matches!(
        shell.orthogonal_recesses(&faces),
        Err(ReadError::Unsupported)
    ));
    let run = |rims: &[ThroughRims]| {
        shell.placed_through_bores(&stock, &boundary, rims, &[pocket], [0.; 3], XYZ)
    };
    assert_eq!(run(&found.through).unwrap().len(), 2);
    let axis = major(rim.outward);
    for side in 0..2 {
        let mut missing = rim;
        missing.centers[side][axis] += 1.;
        assert_eq!(run(&[missing]), Err(ReadError::Unsupported));
    }
    assert_eq!(run(&[rim; 2]), Err(ReadError::Unsupported));
    let limit = orthogonal_pockets::pocket_capacity(&stock);
    assert_eq!(run(&vec![rim; limit]), Err(ReadError::Limit));
}

#[test]
fn recess_recognition_checks_rim_and_support_identity_independently_of_order() {
    let (_, step) = fixture();
    let entities = parse_entities(&step, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let mut faces = shell.validated_faces().unwrap();
    assert!(matches!(
        shell.orthogonal(&faces).unwrap().profile,
        ImportProfile::PocketedOrthogonal { .. }
    ));
    let (wall, seam) = faces
        .iter()
        .enumerate()
        .find_map(|(i, face)| {
            if let Face::Cylindrical { seam, .. } = face {
                Some((i, *seam))
            } else {
                None
            }
        })
        .unwrap();
    let rims: Vec<_> = shell.indexed_edges(faces.len())[wall]
        .iter()
        .copied()
        .filter(|&edge| edge != seam)
        .collect();
    let [floor, mouth] = rims[..] else {
        panic!("expected two cylindrical rims")
    };
    let floor_face = shell.edges[&floor]
        .uses
        .iter()
        .find(|use_| use_.face != wall)
        .unwrap()
        .face;
    let original = shell.orthogonal_recesses(&faces).unwrap().pockets[0];
    let a = shell.edges.remove(&floor).unwrap();
    let b = shell.edges.remove(&mouth).unwrap();
    shell.edges.insert(floor, b);
    shell.edges.insert(mouth, a);
    let reordered = shell.orthogonal_recesses(&faces).unwrap().pockets[0];
    assert_eq!(
        (original.mouth, original.depth),
        (reordered.mouth, reordered.depth)
    );

    let floor_support = std::mem::replace(
        &mut faces[floor_face],
        Face::Cylindrical { seam, flux: -1. },
    );
    assert!(matches!(
        shell.orthogonal_recesses(&faces),
        Err(ReadError::Unsupported)
    ));
    faces[floor_face] = floor_support;

    let edge = shell.edges.get_mut(&mouth).unwrap();
    let circle = std::mem::replace(&mut edge.curve, Curve::Line);
    assert!(matches!(
        shell.orthogonal_recesses(&faces),
        Err(ReadError::Invalid)
    ));
    shell.edges.get_mut(&mouth).unwrap().curve = circle;

    let Face::Planar { normal, .. } = &mut faces[floor_face] else {
        panic!()
    };
    *normal = [-1., 0., 0.];
    assert!(matches!(
        shell.orthogonal_recesses(&faces),
        Err(ReadError::Unsupported)
    ));
}

#[test]
fn two_circular_end_caps_do_not_define_a_blind_recess() {
    let step = write_placed_circular_sweep(1., None, 2., [0.; 3], XYZ[0], XYZ[1]).unwrap();
    let entities = parse_entities(&step, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let mut faces = shell.validated_faces().unwrap();
    for face in &mut faces {
        if let Face::Cylindrical { flux, .. } = face {
            *flux = -*flux;
        }
    }
    assert!(matches!(
        shell.orthogonal_recesses(&faces),
        Err(ReadError::Unsupported)
    ));
}

#[test]
fn reconstructed_stock_caps_recess_work_before_searching_for_mouths() {
    let (stock, step) = fixture();
    let entities = parse_entities(&step, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let faces = shell.validated_faces().unwrap();
    let boundary = orthogonal_boundary::build(stock.quads()).unwrap();
    let recess = shell.orthogonal_recesses(&faces).unwrap().pockets[0];
    let other = Recess {
        mouth: [20., 12., 4.],
        ..recess
    };
    let ordered = shell
        .placed_recesses(&stock, &boundary, &[other, recess], [0.; 3], XYZ)
        .unwrap();
    assert_eq!(ordered.len(), 2);
    assert_eq!(ordered[0].center_mm, recess.mouth);
    assert_eq!(
        shell.placed_recesses(
            &stock,
            &boundary,
            &[Recess {
                depth: 25.,
                ..recess
            }],
            [0.; 3],
            XYZ
        ),
        Err(ReadError::Unsupported)
    );
    assert_eq!(
        shell.placed_recesses(&stock, &boundary, &[recess; 2], [0.; 3], XYZ),
        Err(ReadError::Unsupported)
    );
    let too_many = vec![recess; orthogonal_pockets::pocket_capacity(&stock) + 1];
    assert_eq!(
        shell.placed_recesses(&stock, &boundary, &too_many, [0.; 3], XYZ),
        Err(ReadError::Limit)
    );
}

#[test]
fn circular_opening_counts_against_the_step_face_edge_budget() {
    let (stock, _) = fixture();
    let mut boundary = orthogonal_boundary::build(stock.quads()).unwrap();
    let face = boundary
        .faces
        .iter()
        .position(|face| face.normal == XYZ[0])
        .unwrap();
    let ring = &mut boundary.faces[face].rings[0];
    let [start, end] = [ring[0], ring[1]];
    let subdivisions = MAX_VERTICES - ring.len() + 1;
    let points = (1..subdivisions)
        .map(|i| {
            let t = i as f64 / subdivisions as f64;
            std::array::from_fn(|axis| start[axis] + (end[axis] - start[axis]) * t)
        })
        .collect::<Vec<_>>();
    ring.splice(1..1, points.iter().copied());
    let paired = boundary
        .faces
        .iter_mut()
        .enumerate()
        .filter(|(index, _)| *index != face)
        .flat_map(|(_, face)| &mut face.rings)
        .find_map(|ring| {
            let index = (0..ring.len())
                .find(|&index| ring[index] == end && ring[(index + 1) % ring.len()] == start)?;
            ring.splice(index + 1..index + 1, points.iter().rev().copied());
            Some(())
        });
    assert!(paired.is_some());
    assert!(boundary.supports_step());
    let pocket = OrthogonalPocket {
        face,
        center_mm: [20., 8., 4.],
        radius_mm: 1.5,
        depth_mm: 2.,
    };
    assert_eq!(
        write_pocketed_orthogonal(&stock, &boundary, &[pocket], &|p| p),
        Err(crate::step::StepError::Unsupported)
    );
}

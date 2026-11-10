use super::*;
use crate::{
    orthogonal::{Box3, Operation, boolean},
    step::write_slotted_orthogonal,
};

const TOLERANCE: f64 = 1e-7;
const XYZ: [Point; 3] = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];

fn fixture(place: impl Fn(Point) -> Point) -> (OrthogonalSolid, [OrthogonalSlot; 2], String) {
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
    let face = |axis, support| {
        boundary
            .faces
            .iter()
            .position(|face| face.normal == XYZ[axis] && face.rings[0][0][axis] == support)
            .unwrap()
    };
    let right = face(0, 20.);
    let left_top = boundary
        .faces
        .iter()
        .position(|face| {
            face.normal == XYZ[2]
                && face.rings[0]
                    .iter()
                    .all(|point| point[2] == 9. && point[0] <= 8.)
        })
        .unwrap();
    let slots = [
        OrthogonalSlot {
            face: right,
            centers_mm: [[20., 3., 4.], [20., 7., 4.]],
            radius_mm: 1.,
            depth_mm: 2.,
        },
        OrthogonalSlot {
            face: left_top,
            centers_mm: [[2., 3., 9.], [4., 4., 9.]],
            radius_mm: 1.,
            depth_mm: 2.,
        },
    ];
    let pocket = OrthogonalPocket {
        face: right,
        center_mm: [20., 11., 4.],
        radius_mm: 0.8,
        depth_mm: 2.,
    };
    let step = write_slotted_orthogonal(&stock, &boundary, &[pocket], &slots, &place).unwrap();
    (stock, slots, step)
}

#[test]
fn recovers_mixed_blind_cavities_on_perpendicular_faces_after_rigid_placement() {
    let [sin, cos] = [0.5, 3_f64.sqrt() / 2.];
    let place = |[x, y, z]: Point| [cos * x - sin * y + 37., sin * x + cos * y - 11., z + 4.];
    let (stock, authored, text) = fixture(place);
    let (readback, imported) = read_import_solid(&text, TOLERANCE).unwrap();
    let ImportProfile::SlottedOrthogonal {
        solid,
        pockets,
        slots,
    } = imported.profile
    else {
        panic!("the blind cavities must remain exact");
    };
    assert_eq!((pockets.len(), slots.len()), (1, 2));
    assert_eq!(readback.semicircles.len(), 8);
    assert_eq!(readback.circles.len(), 2);
    let expected = stock.volume_mm3()
        - pockets
            .iter()
            .map(|pocket| pocket.area_mm2() * pocket.depth_mm)
            .sum::<f64>()
        - orthogonal_slots::removed_volume_mm3(&authored);
    assert!((readback.volume_mm3 - expected).abs() < TOLERANCE);
    assert!((solid.volume_mm3() - stock.volume_mm3()).abs() < TOLERANCE);
    let axes = [imported.u, imported.v, cross(imported.u, imported.v)];
    let world = |p: Point| {
        std::array::from_fn(|i| {
            (0..3).fold(imported.origin_mm[i], |sum, j| {
                p[j].mul_add(axes[j][i], sum)
            })
        })
    };
    for expected in authored {
        let centers = expected.centers_mm.map(place);
        assert!(slots.iter().any(|slot| {
            (slot.radius_mm - expected.radius_mm).abs() < TOLERANCE
                && (slot.depth_mm - expected.depth_mm).abs() < TOLERANCE
                && slot.centers_mm.map(world).iter().all(|p| {
                    centers
                        .iter()
                        .any(|center| length(sub(*p, *center)) < TOLERANCE)
                })
        }));
    }
}

#[test]
fn refuses_to_fill_unmatched_half_cylinders_or_remove_outward_walls() {
    let (_, _, text) = fixture(|p| p);
    let entities = parse_entities(&text, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let mut faces = shell.validated_faces().unwrap();
    let loops = std::mem::take(&mut shell.stadium_loops);
    assert_eq!(shell.orthogonal(&faces), Err(ReadError::Unsupported));
    shell.stadium_loops = loops;

    let (floor, _) = shell
        .stadium_loops
        .iter()
        .find(|(_, rims)| rims.iter().any(|rim| rim.ccw))
        .unwrap();
    assert!(matches!(
        shell.orthogonal_slots(&faces, &mut BTreeSet::from([*floor])),
        Err(ReadError::Unsupported)
    ));

    let face = faces
        .iter_mut()
        .find_map(|face| match face {
            Face::HalfCylinder(wall) => Some(wall),
            _ => None,
        })
        .unwrap();
    face.outward = true;
    assert_eq!(shell.orthogonal(&faces), Err(ReadError::Unsupported));
}

#[test]
fn rejects_inconsistent_floor_wall_and_mouth_incidence() {
    let (_, _, text) = fixture(|p| p);
    let entities = parse_entities(&text, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let mut faces = shell.validated_faces().unwrap();
    let face_edges = shell.indexed_edges(faces.len());
    let (&floor, rim) = shell
        .stadium_loops
        .iter()
        .find_map(|(face, rims)| rims.iter().find(|rim| rim.ccw).map(|rim| (face, rim.edges)))
        .unwrap();
    let walls = rim.map(|edge| shell.other_face(edge, floor).unwrap());
    let entry = |shell: &Shell<'_, '_>, i: usize| {
        let wall = walls[i];
        let opposite = face_edges[wall]
            .iter()
            .copied()
            .find(|id| !rim.contains(id) && matches!(shell.edges[id].curve, Curve::Semicircle(_)))
            .unwrap();
        (opposite, shell.other_face(opposite, wall).unwrap())
    };
    let [a, b] = [0, 1, 2, 3]
        .into_iter()
        .filter(|&i| matches!(faces[walls[i]], Face::HalfCylinder(_)))
        .collect::<Vec<_>>()[..]
    else {
        panic!("expected two semicylinders");
    };
    let (second_arc, mouth) = entry(&shell, b);
    let recognize =
        |shell: &Shell<'_, '_>, faces: &[Face]| shell.slot_recess(faces, &face_edges, floor, rim);

    let duplicate = shell
        .edges
        .get_mut(&rim[1])
        .unwrap()
        .uses
        .iter_mut()
        .find(|entry| entry.face != floor)
        .unwrap();
    duplicate.face = walls[0];
    assert!(matches!(
        recognize(&shell, &faces),
        Err(ReadError::Unsupported)
    ));
    shell
        .edges
        .get_mut(&rim[1])
        .unwrap()
        .uses
        .iter_mut()
        .find(|entry| entry.face != floor)
        .unwrap()
        .face = walls[1];

    let unsupported = || Face::Cylindrical { seam: 0, flux: -1. };
    let mouth_face = std::mem::replace(&mut faces[mouth], unsupported());
    assert!(matches!(
        recognize(&shell, &faces),
        Err(ReadError::Unsupported)
    ));
    faces[mouth] = mouth_face;

    let Face::Planar { normal, .. } = &mut faces[floor] else {
        unreachable!()
    };
    *normal = normal.map(|v| -v);
    assert!(matches!(
        recognize(&shell, &faces),
        Err(ReadError::Unsupported)
    ));
    let Face::Planar { normal, .. } = &mut faces[floor] else {
        unreachable!()
    };
    *normal = normal.map(|v| -v);

    let straight = walls[[0, 1, 2, 3]
        .into_iter()
        .find(|i| *i != a && *i != b)
        .unwrap()];
    let straight_face = std::mem::replace(&mut faces[straight], unsupported());
    assert!(matches!(
        recognize(&shell, &faces),
        Err(ReadError::Unsupported)
    ));
    faces[straight] = straight_face;

    let original = std::mem::replace(
        &mut shell.edges.get_mut(&second_arc).unwrap().curve,
        Curve::Line,
    );
    assert!(matches!(recognize(&shell, &faces), Err(ReadError::Invalid)));
    shell.edges.get_mut(&second_arc).unwrap().curve = original;
    assert!(recognize(&shell, &faces).is_ok());
}

#[test]
fn refuses_unrecognized_curves_on_stock_and_unplaceable_mouths() {
    let (_, _, text) = fixture(|p| p);
    let entities = parse_entities(&text, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let faces = shell.validated_faces().unwrap();
    let mut omitted = shell.orthogonal_recesses(&faces).unwrap().omitted;
    shell.orthogonal_slots(&faces, &mut omitted).unwrap();
    let stock = shell
        .edges
        .iter()
        .find(|(_, edge)| {
            matches!(edge.curve, Curve::Line)
                && edge.uses.iter().all(|entry| !omitted.contains(&entry.face))
        })
        .map(|(&id, _)| id)
        .unwrap();
    let mouth = shell
        .stadium_loops
        .values()
        .flatten()
        .filter(|rim| !rim.ccw)
        .flat_map(|rim| rim.edges)
        .find(|id| matches!(shell.edges[id].curve, Curve::Semicircle(_)))
        .unwrap();
    let arc = shell.edges[&mouth].curve;
    shell.edges.get_mut(&stock).unwrap().curve = arc;
    assert_eq!(shell.orthogonal(&faces), Err(ReadError::Unsupported));
    shell.edges.get_mut(&stock).unwrap().curve = Curve::Line;
    let Curve::Semicircle(arc) = &mut shell.edges.get_mut(&mouth).unwrap().curve else {
        unreachable!()
    };
    arc.radius_mm = 40.;
    assert_eq!(shell.orthogonal(&faces), Err(ReadError::Unsupported));
}

#[test]
fn rechecks_placement_and_combined_cavity_clearance_on_reconstructed_stock() {
    let (stock, _, text) = fixture(|p| p);
    let entities = parse_entities(&text, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let faces = shell.validated_faces().unwrap();
    let boundary = orthogonal_boundary::build(stock.quads()).unwrap();
    let mut drills = shell.orthogonal_recesses(&faces).unwrap();
    let recesses = drills.pockets;
    let omitted = &mut drills.omitted;
    let mut slots = shell.orthogonal_slots(&faces, omitted).unwrap();
    let mut pockets = shell
        .placed_recesses(&stock, &boundary, &recesses, [0.; 3], XYZ)
        .unwrap();
    slots.reverse();
    let accepted = shell
        .placed_slots(&stock, &boundary, &slots, &pockets, [0.; 3], XYZ)
        .unwrap();
    assert!(accepted[0].face < accepted[1].face);
    let right = accepted
        .iter()
        .find(|slot| slot.centers_mm[0][0] == 20.)
        .unwrap();
    pockets[0].center_mm = right.centers_mm[0];
    assert_eq!(
        shell.placed_slots(&stock, &boundary, &slots, &pockets, [0.; 3], XYZ),
        Err(ReadError::Unsupported)
    );
    assert_eq!(
        shell.placed_slots(&stock, &boundary, &[slots[0]; 2], &[], [0.; 3], XYZ),
        Err(ReadError::Unsupported)
    );
    assert_eq!(
        shell.placed_slots(
            &stock,
            &boundary,
            &[SlotRecess {
                depth: 30.,
                ..slots[0]
            }],
            &[],
            [0.; 3],
            XYZ
        ),
        Err(ReadError::Unsupported)
    );
    let over_budget = vec![slots[0]; orthogonal_slots::slot_capacity(&stock) + 1];
    assert_eq!(
        shell.placed_slots(&stock, &boundary, &over_budget, &[], [0.; 3], XYZ),
        Err(ReadError::Limit)
    );
}

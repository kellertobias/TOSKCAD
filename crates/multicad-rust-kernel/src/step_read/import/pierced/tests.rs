use super::*;
use crate::{normal_sweep::Sweep, step::write_pierced_normal_sweep};

const TOLERANCE: f64 = 1e-7;
const X: Point = [1., 0., 0.];
const Y: Point = [0., 1., 0.];
const Z: Point = [0., 0., 1.];

fn line(a: Point2, b: Point2) -> ProfileEdge {
    ProfileEdge::Line { a, b }
}

fn fixture() -> PiercedSweep {
    let sweep = Sweep::new(
        vec![vec![
            line([0., 0.], [24., 0.]),
            line([24., 0.], [24., 9.]),
            ProfileEdge::Arc {
                center: [21., 9.],
                a: [24., 9.],
                b: [21., 12.],
                clockwise: false,
            },
            line([21., 12.], [0., 12.]),
            line([0., 12.], [0., 0.]),
        ]],
        12.,
        TOLERANCE,
        1e-5,
    )
    .unwrap();
    let opening = |outline| Piercing {
        sides: [0, 3],
        origin: [0.; 3],
        u: X,
        v: Z,
        through: [0., 12., 0.],
        outline,
    };
    let rectangle = [[14., 5.], [18., 5.], [18., 8.], [14., 8.]];
    PiercedSweep::new(
        &sweep,
        vec![
            opening(vec![ProfileEdge::Circle {
                center: [7., 3.],
                radius_mm: 1.5,
                clockwise: false,
            }]),
            opening(
                rectangle
                    .into_iter()
                    .zip(rectangle.into_iter().cycle().skip(1))
                    .map(|(a, b)| line(a, b))
                    .collect(),
            ),
        ],
        TOLERANCE,
        1e-5,
    )
    .unwrap()
}

#[test]
fn certified_openings_round_trip_in_both_sweep_directions() {
    let pierced = fixture();
    for reverse in [false, true] {
        let text = write_pierced_normal_sweep(&pierced, [10., 20., 30.], X, Y, reverse).unwrap();
        let (readback, model) = read_import_solid(&text, TOLERANCE).unwrap();
        let ImportProfile::PiercedNormalSweep { sweep } = model.profile else {
            panic!("missing paired openings");
        };
        assert_eq!(readback.face_count, 12);
        assert_eq!(readback.arcs.len(), 2);
        assert_eq!(readback.circles.len(), 2);
        assert_eq!(sweep.openings().len(), 2);
        assert!((pierced.volume_mm3() - sweep.volume_mm3()).abs() < TOLERANCE);
        assert!((pierced.volume_mm3() - readback.volume_mm3).abs() < TOLERANCE);
    }
    assert_eq!(
        write_pierced_normal_sweep(&pierced, [1e20, 0., 0.], X, Y, false),
        Err(crate::step::StepError::InvalidGeometry)
    );
}

#[test]
fn malformed_stock_and_rim_graphs_cannot_publish_pierced_geometry() {
    let text = write_pierced_normal_sweep(&fixture(), [0.; 3], X, Y, false).unwrap();
    let entities = parse_entities(&text, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let mut faces = shell.validated_faces().unwrap();
    let mut edges = shell.indexed_edges(faces.len());
    let stock = shell
        .indexed_normal_sweep_profile(&edges[0], 0, Z, 12.)
        .unwrap();
    let first = stock.ordinals[0];
    let side = shell.adjacent_face(first, 0).unwrap();
    let test = |shell: &Shell<'_, '_>, faces: &[Face], edges: &[Vec<u32>]| {
        shell.match_pierced_normal_sweep(faces, edges, 0, 1)
    };
    assert!(test(&shell, &faces, &edges).is_some());
    let invalid_plane = Face::Planar {
        origin: [0., 0., 12.],
        normal: Y,
        area: 1.,
    };
    let saved = std::mem::replace(&mut faces[1], invalid_plane);
    assert!(test(&shell, &faces, &edges).is_none());
    faces[1] = saved;
    let other = edges[1][1];
    edges[1].push(other);
    assert!(test(&shell, &faces, &edges).is_none());
    edges[1].pop();

    let competing = stock.ordinals[1];
    let item = shell.edges.get_mut(&competing).unwrap();
    let original = item.uses.clone();
    item.uses
        .iter_mut()
        .find(|item| item.face != 0)
        .unwrap()
        .face = side;
    assert!(test(&shell, &faces, &edges).is_none());
    shell.edges.get_mut(&competing).unwrap().uses = original;

    let extra = edges[1]
        .iter()
        .copied()
        .find(|id| !edges[side].contains(id))
        .unwrap();
    edges[side].push(extra);
    assert!(test(&shell, &faces, &edges).is_none());
    edges[side].pop();
    let before = edges[side].clone();
    let end_edges = edges[1].clone();
    edges[side].retain(|id| end_edges.binary_search(id).is_err());
    assert!(test(&shell, &faces, &edges).is_none());
    edges[side] = before;

    let arc = edges[0]
        .iter()
        .copied()
        .find(|id| open_arc(shell.edges[id].curve))
        .unwrap();
    let arc_wall = shell.adjacent_face(arc, 0).unwrap();
    assert!(plane(&faces[arc_wall]).is_none());
    let original = std::mem::replace(
        &mut faces[arc_wall],
        Face::Planar {
            origin: [0.; 3],
            normal: X,
            area: 1.,
        },
    );
    assert!(shell.pierced_normal_sweep(&faces).is_none());
    faces[arc_wall] = original;
    let far_arc = edges[arc_wall]
        .iter()
        .copied()
        .find(|&id| id != arc && open_arc(shell.edges[&id].curve))
        .unwrap();
    let item = shell.edges.get_mut(&far_arc).unwrap();
    let curve = std::mem::replace(&mut item.curve, Curve::Line);
    assert!(shell.pierced_normal_sweep(&faces).is_none());
    shell.edges.get_mut(&far_arc).unwrap().curve = curve;
    assert!(shell.piercing_rims(0, vec![arc]).is_none());
    assert!(shell.piercing_rims(0, vec![first, first]).is_none());
    edges[side].push(arc);
    assert!(test(&shell, &faces, &edges).is_none());
    edges[side].pop();

    let rect_edges = |shell: &Shell<'_, '_>, side| {
        shell
            .edges
            .iter()
            .filter_map(|(&id, edge)| {
                let in_rect = |vertex| {
                    let [x, _, z] = shell.points[&vertex];
                    (14. ..=18.).contains(&x) && (5. ..=8.).contains(&z)
                };
                (edge.uses.iter().any(|item| item.face == side)
                    && edge.vertices.into_iter().all(in_rect))
                .then_some(id)
            })
            .collect::<Vec<_>>()
    };
    let rect_side = stock
        .ordinals
        .iter()
        .copied()
        .map(|id| shell.adjacent_face(id, 0).unwrap())
        .find(|&face| rect_edges(&shell, face).len() == 4)
        .unwrap();
    let mut rect = shell
        .piercing_rims(rect_side, rect_edges(&shell, rect_side))
        .unwrap()
        .pop()
        .unwrap();
    let end = *rect.edges.last().unwrap();
    let end_vertices = shell.rim_vertices(end, rect_side).unwrap();
    let edge = shell.edges.get_mut(&end).unwrap();
    let old_vertices = edge.vertices;
    *edge
        .vertices
        .iter_mut()
        .find(|vertex| **vertex == end_vertices[1])
        .unwrap() = u32::MAX;
    assert!(shell.piercing_rims(rect_side, rect.edges.clone()).is_none());
    shell.edges.get_mut(&end).unwrap().vertices = old_vertices;
    rect.edges.truncate(1);
    assert!(
        shell
            .piercing_profile(&stock, &rect, [0, 3], Y, [0., 12., 0.])
            .is_none()
    );
    rect.edges.clear();
    assert!(
        shell
            .piercing_profile(&stock, &rect, [0, 3], Y, [0., 12., 0.])
            .is_none()
    );
}

#[test]
fn through_pairing_requires_matching_holes_walls_and_stock_sides() {
    let text = write_pierced_normal_sweep(&fixture(), [0.; 3], X, Y, false).unwrap();
    let entities = parse_entities(&text, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let mut faces = shell.validated_faces().unwrap();
    let edges = shell.indexed_edges(faces.len());
    let stock = shell
        .indexed_normal_sweep_profile(&edges[0], 0, Z, 12.)
        .unwrap();
    let sides: BTreeMap<_, _> = stock
        .ordinals
        .iter()
        .enumerate()
        .map(|(i, &id)| (shell.adjacent_face(id, 0).unwrap(), i))
        .collect();
    let (wall, circle_edges) = faces
        .iter()
        .enumerate()
        .find_map(|(face, support)| {
            matches!(support, Face::Cylindrical { .. }).then(|| {
                (
                    face,
                    edges[face]
                        .iter()
                        .copied()
                        .filter(|id| matches!(shell.edges[id].curve, Curve::Circle(_)))
                        .collect::<Vec<_>>(),
                )
            })
        })
        .unwrap();
    let mut rims = circle_edges
        .into_iter()
        .map(|id| Rim {
            face: shell.adjacent_face(id, wall).unwrap(),
            edges: vec![id],
        })
        .collect::<Vec<_>>();
    let pair = |faces: &[Face], sides: BTreeMap<usize, usize>, rims: &[Rim]| {
        let context = PiercedShell {
            faces,
            face_edges: &edges,
            sides,
        };
        shell.pair_piercing_rims(&context, rims, &stock)
    };
    // The other rectangular tunnel cannot disappear when just the round
    // through hole is presented for reconstruction.
    assert!(pair(&faces, sides.clone(), &rims).is_none());
    assert!(pair(&faces, sides.clone(), &rims[..1]).is_none());
    let mut absent = edges.clone();
    absent[wall].retain(|id| *id != rims[1].edges[0]);
    let context = PiercedShell {
        faces: &faces,
        face_edges: &absent,
        sides: sides.clone(),
    };
    assert!(shell.pair_piercing_rims(&context, &rims, &stock).is_none());
    let mut fake = sides.clone();
    fake.insert(wall, 5);
    assert!(pair(&faces, fake, &rims).is_none());

    let bad_wall = Face::Planar {
        origin: [0.; 3],
        normal: Y,
        area: 1.,
    };
    let original = std::mem::replace(&mut faces[wall], bad_wall);
    assert!(pair(&faces, sides.clone(), &rims).is_none());
    assert!(
        shell
            .match_pierced_normal_sweep(&faces, &edges, 0, 1)
            .is_none()
    );
    faces[wall] = original;

    let first_edge = rims[0].edges[0];
    rims[1].edges.push(first_edge);
    assert!(pair(&faces, sides.clone(), &rims).is_none());
    rims[1].edges.pop();
    let second = rims[1].face;
    let (origin, normal) = plane(&faces[second]).unwrap();
    faces[second] = Face::Planar {
        origin,
        normal: normal.map(|n| -n),
        area: 1.,
    };
    assert!(pair(&faces, sides, &rims).is_none());
    faces[second] = Face::Planar {
        origin,
        normal,
        area: 1.,
    };

    // A consistent pair of rim translations does not by itself establish
    // clearance from a neighboring through-opening.
    for rim in &rims {
        let Curve::Circle(circle) = &mut shell.edges.get_mut(&rim.edges[0]).unwrap().curve else {
            unreachable!();
        };
        circle.center_mm[0] = 16.;
        circle.center_mm[2] = 6.;
    }
    assert!(
        shell
            .match_pierced_normal_sweep(&faces, &edges, 0, 1)
            .is_none()
    );
}

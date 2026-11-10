use super::*;
use crate::{Circle, Rational, polygon::PolygonPrism, step::write_pocketed_polygon_prism};

const TOLERANCE: f64 = 1e-7;
type Mutation = fn(&mut Shell<'_, '_>, &mut Vec<Face>, &mut [Vec<u32>]);

fn line_at(shell: &Shell<'_, '_>, edges: &[u32], position: usize) -> u32 {
    edges
        .iter()
        .copied()
        .filter(|id| matches!(shell.edges[id].curve, Curve::Line))
        .nth(position)
        .unwrap()
}

fn pocket_step() -> String {
    let n = |value| Rational::new(value, 1).unwrap();
    let ring = [[0, 0], [12, 0], [12, 10], [0, 10]]
        .map(|point| point.map(n))
        .to_vec();
    let through = Circle::new([n(9), n(5)], n(1)).unwrap();
    let prism = PolygonPrism::with_circular_holes(vec![ring], vec![through], n(6)).unwrap();
    write_pocketed_polygon_prism(
        &prism,
        &[
            CircularPocket {
                center_mm: [4., 5.],
                radius_mm: 2.,
                depth_mm: 1.,
                end: false,
            },
            CircularPocket {
                center_mm: [4., 5.],
                radius_mm: 1.5,
                depth_mm: 2.,
                end: true,
            },
        ],
        [0.; 3],
        [1., 0., 0.],
        [0., 1., 0.],
        false,
    )
    .unwrap()
}

#[test]
fn pocket_recognition_rejects_an_inconsistent_face_graph() {
    const BLIND_WALL: usize = 7;
    const FLOOR: usize = 8;
    let source = pocket_step();
    let entities = parse_entities(&source, TOLERANCE).unwrap();
    let test = |name: &str, alter: Mutation| {
        let mut shell = Shell::new(&entities, TOLERANCE, true);
        let mut faces = shell.validated_faces().unwrap();
        let mut face_edges = shell.indexed_edges(faces.len());
        let recognize = |shell: &Shell<'_, '_>, faces: &[Face], edges: &[Vec<u32>]| {
            shell.match_pocketed_polygon(faces, edges, 0, [0.; 3], [0., 0., 1.])
        };
        assert!(recognize(&shell, &faces, &face_edges).is_some());
        alter(&mut shell, &mut faces, &mut face_edges);
        assert!(recognize(&shell, &faces, &face_edges).is_none(), "{name}");
    };
    test("unpaired first wall", |shell, _, edges| {
        let first = line_at(shell, &edges[0], 0);
        let wall = shell.adjacent_face(first, 0).unwrap();
        let far = shell.opposite_line(&edges[wall], first).unwrap();
        shell
            .edges
            .get_mut(&far)
            .unwrap()
            .uses
            .retain(|use_| use_.face == wall);
    });
    test("nonplanar upper support", |_, faces, _| {
        faces.swap(1, 6);
    });
    test("incompatible upper support", |_, faces, _| {
        faces.swap(1, FLOOR);
    });
    test("missing upper rim", |_, _, edges| {
        edges[1].pop();
    });
    test("unpaired subsequent wall", |shell, _, edges| {
        let next = line_at(shell, &edges[0], 1);
        let wall = shell.adjacent_face(next, 0).unwrap();
        let far = shell.opposite_line(&edges[wall], next).unwrap();
        for use_ in &mut shell.edges.get_mut(&far).unwrap().uses {
            if use_.face == 1 {
                use_.face = FLOOR;
            }
        }
    });
    test("outward cylindrical wall", |_, faces, _| {
        let Face::Cylindrical { flux, .. } = &mut faces[BLIND_WALL] else {
            panic!()
        };
        *flux = flux.abs();
    });
    test("cylindrical wall has no floor rim", |_, _, edges| {
        let floor = edges[FLOOR][0];
        edges[BLIND_WALL].retain(|&edge| edge != floor);
    });
    test("floor faces out of the material", |_, faces, _| {
        let Face::Planar { normal, .. } = &mut faces[FLOOR] else {
            panic!()
        };
        *normal = [0., 0., 1.];
    });
    test("floor circle drifts laterally", |shell, _, edges| {
        let Curve::Circle(rim) = &mut shell.edges.get_mut(&edges[FLOOR][0]).unwrap().curve else {
            panic!()
        };
        rim.center_mm[0] += 0.01;
    });
    test(
        "opposite floors overlap inside the stock",
        |shell, _, edges| {
            let Curve::Circle(rim) = &mut shell.edges.get_mut(&edges[FLOOR][0]).unwrap().curve
            else {
                panic!()
            };
            rim.center_mm[2] = 5.;
        },
    );
    test("unexplained face", |_, faces, _| {
        faces.push(Face::Planar {
            origin: [0.; 3],
            normal: [0., 0., 1.],
            area: 1.,
        });
    });
}

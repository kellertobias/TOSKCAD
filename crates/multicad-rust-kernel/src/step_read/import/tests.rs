use super::*;
use crate::{Circle, Rational, cylinder::AxialCylinder, step};

const TOLERANCE: f64 = 1e-7;
const X: Point = [1., 0., 0.];
const Y: Point = [0., 1., 0.];
const Z: Point = [0., 0., 1.];

fn n(value: i64) -> Rational {
    Rational::new(value, 1).unwrap()
}

fn box_step() -> String {
    step::write_box([n(12), n(8), n(4)], [0.; 3], X, Y).unwrap()
}

#[test]
fn prism_candidate_rejects_incompatible_caps_and_shared_walls() {
    let source = box_step();
    let entities = parse_entities(&source, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let mut faces = shell.validated_faces().unwrap();
    let mut edges = shell.indexed_edges(faces.len());
    let prism = |shell: &Shell<'_, '_>, faces: &[Face], edges: &[Vec<u32>], bottom| {
        shell.prism(faces, edges, 0, Z, bottom)
    };
    assert!(prism(&shell, &faces, &edges, [0.; 3]).is_some());
    assert!(prism(&shell, &faces, &edges, [0., 0., 5.]).is_none());

    let incompatible = Face::Planar {
        origin: [0., 0., 4.],
        normal: Y,
        area: 96.,
    };
    let upper = std::mem::replace(&mut faces[1], incompatible);
    assert!(prism(&shell, &faces, &edges, [0.; 3]).is_none());
    faces[1] = Face::Cylindrical {
        seam: edges[1][0],
        flux: 1.,
    };
    assert!(prism(&shell, &faces, &edges, [0.; 3]).is_none());
    faces[1] = upper;

    let wall = shell.edges[&edges[0][0]]
        .uses
        .iter()
        .find(|use_| use_.face != 0)
        .unwrap()
        .face;
    let unrelated_rim = edges[1]
        .iter()
        .copied()
        .find(|id| edges[wall].binary_search(id).is_err())
        .unwrap();
    let index = edges[wall].partition_point(|&id| id < unrelated_rim);
    edges[wall].insert(index, unrelated_rim);
    assert!(prism(&shell, &faces, &edges, [0.; 3]).is_none());
    edges[wall].remove(index);

    let other = shell.edges.get_mut(&edges[0][1]).unwrap();
    other
        .uses
        .iter_mut()
        .find(|use_| use_.face != 0)
        .unwrap()
        .face = wall;
    assert!(prism(&shell, &faces, &edges, [0.; 3]).is_none());
}

#[test]
fn wall_match_requires_the_entire_analytic_side() {
    let stock = Circle::new([n(0), n(0)], n(2)).unwrap();
    let cylinder = AxialCylinder::new(1, stock, n(0), n(4)).unwrap();
    let source = step::write_axial_cylinder(&cylinder, X).unwrap();
    let entities = parse_entities(&source, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let faces = shell.validated_faces().unwrap();
    let edges = shell.indexed_edges(faces.len());
    let (bottom, top) = (edges[0][0], edges[1][0]);
    let verify = |shell: &Shell<'_, '_>, face| {
        shell.wall_matches(face, &edges[2], bottom, top, [0., 0., 4.])
    };
    assert!(verify(&shell, &faces[2]).is_some());
    assert!(verify(&shell, &faces[0]).is_none());
    let edge = shell.edges.get_mut(&top).unwrap();
    let Curve::Circle(ref mut circle) = edge.curve else {
        panic!("expected circle")
    };
    circle.radius_mm += TOLERANCE * 1.5;
    assert!(verify(&shell, &faces[2]).is_none());
}

#[test]
fn a_circular_rim_must_have_the_cap_winding() {
    let hole = Circle::new([n(4), n(4)], n(2)).unwrap();
    let stock = crate::polygon::PolygonPrism::with_circular_holes(
        vec![[[0, 0], [12, 0], [12, 8], [0, 8]].map(|p| p.map(n)).to_vec()],
        vec![hole],
        n(4),
    )
    .unwrap();
    let source = step::write_polygon_prism(&stock, [0.; 3], X, Y, false).unwrap();
    let entities = parse_entities(&source, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let faces = shell.validated_faces().unwrap();
    let edges = shell.indexed_edges(faces.len());
    let lower = faces
        .iter()
        .position(|face| matches!(face, Face::Planar { normal, .. } if normal[2] < 0.))
        .unwrap();
    assert!(shell.profile(&edges[lower], lower, Z, 4.).is_some());
    let circle = edges[lower]
        .iter()
        .find(|id| matches!(shell.edges[id].curve, Curve::Circle(_)))
        .unwrap();
    let rim = shell.edges.get_mut(circle).unwrap();
    let use_ = rim.uses.iter_mut().find(|use_| use_.face == lower).unwrap();
    use_.forward = !use_.forward;
    assert!(shell.profile(&edges[lower], lower, Z, 4.).is_none());
}

#[test]
fn profile_rejects_a_second_exterior_and_repeated_rim_start() {
    let source = box_step();
    let entities = parse_entities(&source, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let faces = shell.validated_faces().unwrap();
    let edges = shell.indexed_edges(faces.len());
    let lower = &edges[0];
    let upper = &edges[1];
    assert!(shell.profile(lower, 0, Z, 4.).is_some());
    let reversed_start = |edge: &Edge| {
        edge.vertices[usize::from(
            edge.uses
                .iter()
                .find(|use_| use_.face == 0)
                .unwrap()
                .forward,
        )]
    };
    let start = reversed_start(&shell.edges[&lower[0]]);
    for id in upper {
        let use_ = shell
            .edges
            .get_mut(id)
            .unwrap()
            .uses
            .iter_mut()
            .find(|use_| use_.face == 1)
            .unwrap();
        use_.face = 0;
        use_.forward = !use_.forward;
    }
    let two_outers: Vec<_> = lower.iter().chain(upper).copied().collect();
    assert!(shell.profile(&two_outers, 0, Z, 4.).is_none());
    let next = shell.edges.get_mut(&lower[1]).unwrap();
    let previous = reversed_start(next);
    next.vertices
        .iter_mut()
        .filter(|vertex| **vertex == previous)
        .for_each(|vertex| *vertex = start);
    assert!(shell.profile(lower, 0, Z, 4.).is_none());
}

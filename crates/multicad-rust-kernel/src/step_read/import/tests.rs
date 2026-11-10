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
fn stadium_recognition_ignores_wire_start_and_refuses_a_polygon_fallback() {
    let profile = Stadium::new([[0.; 2], [6., 0.]], 1.).unwrap();
    let text = step::write_stadium_prism(&profile, 4., [0.; 3], X, Y, false).unwrap();
    let entities = parse_entities(&text, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let faces = shell.validated_faces().unwrap();
    let face_edges = shell.indexed_edges(faces.len());
    let recognized = shell.recognize(&faces).unwrap();
    shell.stadium_loops.get_mut(&0).unwrap()[0]
        .edges
        .rotate_left(2);
    assert_eq!(shell.recognize(&faces), Some(recognized));
    assert!(
        shell
            .planar_profile(&face_edges[0], 0, Z, PlanarMatch::Straight(4., &[]))
            .is_none()
    );
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

    let missing_edge = edges[1].pop().unwrap();
    assert!(prism(&shell, &faces, &edges, [0.; 3]).is_none());
    edges[1].push(missing_edge);

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
fn sloped_trim_requires_reachable_planes_and_a_common_direction() {
    let sweep = TrimmedSweep::new(
        vec![vec![[0., 0.], [12., 0.], [12., 8.], [0., 8.]]],
        [0.6, 0., 0.8],
        [[0.; 3], [5., 0.1, -0.1]],
        TOLERANCE,
    )
    .unwrap();
    let source = step::write_trimmed_sweep(&sweep, [0.; 3], X, Y).unwrap();
    let entities = parse_entities(&source, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let mut faces = shell.validated_faces().unwrap();
    let edges = shell.indexed_edges(faces.len());
    assert!(shell.trimmed_sweep(&faces, &edges, 0, Z).is_some());
    assert!(
        shell
            .trimmed_sweep(&faces, &edges, 0, [0., 0., -1.])
            .is_none()
    );
    let top = std::mem::replace(
        &mut faces[1],
        Face::Cylindrical {
            seam: edges[1][0],
            flux: 1.,
        },
    );
    assert!(shell.trimmed_sweep(&faces, &edges, 0, Z).is_none());
    faces[1] = Face::Planar {
        origin: [0., 0., 4.],
        normal: Y,
        area: 96.,
    };
    assert!(shell.trimmed_sweep(&faces, &edges, 0, Z).is_none());
    faces[1] = top;

    let vertex = shell.edges[&edges[1][0]].vertices[0];
    shell.points.get_mut(&vertex).unwrap()[0] += 0.1;
    assert!(shell.trimmed_sweep(&faces, &edges, 0, Z).is_none());
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
fn planar_wall_generators_ignore_edge_direction_and_require_a_closed_quad() {
    let source = box_step();
    let entities = parse_entities(&source, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let faces = shell.validated_faces().unwrap();
    let edges = shell.indexed_edges(faces.len());
    let wall = &edges[2];
    let base = *wall.iter().find(|id| edges[0].contains(id)).unwrap();
    let top = *wall.iter().find(|id| edges[1].contains(id)).unwrap();
    let verify =
        |shell: &Shell<'_, '_>, rim_edges| shell.wall_generators(&faces[2], rim_edges, base, top);
    let expected = verify(&shell, wall).unwrap();
    assert!(verify(&shell, &wall[..3]).is_none());
    let curved = Face::Cylindrical {
        seam: base,
        flux: 1.,
    };
    assert!(shell.wall_generators(&curved, wall, base, top).is_none());
    let bridge = wall.iter().find(|&&id| id != base && id != top).unwrap();
    let vertices = &mut shell.edges.get_mut(bridge).unwrap().vertices;
    vertices.swap(0, 1);
    assert_eq!(verify(&shell, wall), Some(expected));
    shell.edges.get_mut(bridge).unwrap().vertices = [expected[0][0], expected[1][1]];
    assert!(verify(&shell, wall).is_none());
}

#[test]
fn a_circular_rim_must_have_the_cap_winding() {
    let hole = Circle::new([n(4), n(4)], n(2)).unwrap();
    let stock = crate::polygon::PolygonPrism::with_circular_holes(
        vec![
            [[0, 0], [12, 0], [12, 8], [0, 8]]
                .map(|p| p.map(n))
                .to_vec(),
        ],
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
    assert!(shell.profile(&edges[lower], lower, Z, 4., None).is_some());
    let circle = edges[lower]
        .iter()
        .find(|id| matches!(shell.edges[id].curve, Curve::Circle(_)))
        .unwrap();
    let rim = shell.edges.get_mut(circle).unwrap();
    let use_ = rim.uses.iter_mut().find(|use_| use_.face == lower).unwrap();
    use_.forward = !use_.forward;
    assert!(shell.profile(&edges[lower], lower, Z, 4., None).is_none());
}

#[test]
fn circular_profile_recognition_is_order_independent_and_refuses_eccentric_bores() {
    let annulus = crate::annulus::Annulus::new(1., 3., 4.).unwrap();
    let source = step::write_placed_annulus(&annulus, [0.; 3], X, Y).unwrap();
    let entities = parse_entities(&source, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let faces = shell.validated_faces().unwrap();
    let edges = shell.indexed_edges(faces.len());
    let [outer, inner] = edges[0][..] else {
        panic!("expected two full lower rims")
    };
    let expected = shell.profile(&[outer, inner], 0, Z, 4., None);
    assert!(expected.is_some());
    assert_eq!(shell.profile(&[inner, outer], 0, Z, 4., None), expected);
    let edge = shell.edges.get_mut(&inner).unwrap();
    let Curve::Circle(ref mut circle) = edge.curve else {
        panic!("expected circular bore")
    };
    circle.center_mm[0] += 0.1;
    assert!(shell.profile(&[outer, inner], 0, Z, 4., None).is_none());
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
    assert!(shell.profile(lower, 0, Z, 4., None).is_some());
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
    assert!(shell.profile(&two_outers, 0, Z, 4., None).is_none());
    let next = shell.edges.get_mut(&lower[1]).unwrap();
    let previous = reversed_start(next);
    next.vertices
        .iter_mut()
        .filter(|vertex| **vertex == previous)
        .for_each(|vertex| *vertex = start);
    assert!(shell.profile(lower, 0, Z, 4., None).is_none());
}

use super::*;
use crate::{Circle, Rational, polygon::PolygonPrism, step::write_chamfered_polygon_prism};

const Z: Point = [0., 0., 1.];
const ORIGIN: Point = [0.; 3];
const TOLERANCE: f64 = 1e-7;

fn fixture() -> String {
    let n = |value| Rational::new(value, 1).unwrap();
    let ring = [[0, 0], [18, 0], [18, 12], [0, 12]]
        .map(|point| point.map(n))
        .to_vec();
    let through = Circle::new([n(9), n(6)], n(2)).unwrap();
    let prism = PolygonPrism::with_circular_holes(vec![ring], vec![through], n(6)).unwrap();
    write_chamfered_polygon_prism(
        &prism,
        &[false, true].map(|end| BoreChamfer {
            hole: 0,
            end,
            size_mm: 1.,
        }),
        ORIGIN,
        [1., 0., 0.],
        [0., 1., 0.],
        false,
    )
    .unwrap()
}

#[test]
fn countersink_import_requires_all_walls_in_their_geometric_sequence() {
    let source = fixture();
    let entities = parse_entities(&source, TOLERANCE).unwrap();
    let mut shell = Shell::new(&entities, TOLERANCE, true);
    let mut faces = shell.validated_faces().unwrap();
    let mut edges = shell.indexed_edges(faces.len());
    let (rim, mouth) = edges[0]
        .iter()
        .find_map(|&edge| match shell.edges[&edge].curve {
            Curve::Circle(circle) => Some((edge, circle)),
            _ => None,
        })
        .unwrap();
    let cone = shell.adjacent_face(rim, 0).unwrap();
    let read_bore = |axis, caps, mut covered| {
        shell.chamfered_bore(&faces, &edges, caps, (rim, mouth), axis, &mut covered)
    };
    assert!(read_bore(Z, [0, 1], BTreeSet::new()).is_some());
    assert!(read_bore(Z, [0, 1], BTreeSet::from([cone])).is_none());
    assert!(read_bore([1., 0., 0.], [0, 1], BTreeSet::new()).is_none());
    // A cone after the cylinder must terminate at the upper stock cap.
    assert!(read_bore(Z, [0, usize::MAX], BTreeSet::new()).is_none());

    // Rims may appear later than the seam in another writer's edge order.
    edges[cone].sort_by_key(|edge| !matches!(shell.edges[edge].curve, Curve::Line));
    assert!(
        shell
            .match_chamfered_polygon(&faces, &edges, 0, ORIGIN, Z)
            .is_some()
    );
    let far = *edges[cone]
        .iter()
        .find(|&&edge| edge != rim && matches!(shell.edges[&edge].curve, Curve::Circle(_)))
        .unwrap();
    let wall_edges = edges[cone].clone();
    edges[cone].retain(|&edge| edge != far);
    assert!(
        shell
            .match_chamfered_polygon(&faces, &edges, 0, ORIGIN, Z)
            .is_none()
    );
    edges[cone] = wall_edges;
    for edge in [rim, far] {
        let uses = shell.edges.get_mut(&edge).unwrap();
        let saved = uses.uses.clone();
        uses.uses
            .retain(|use_| use_.face == if edge == rim { 0 } else { cone });
        assert!(
            shell
                .match_chamfered_polygon(&faces, &edges, 0, ORIGIN, Z)
                .is_none()
        );
        shell.edges.get_mut(&edge).unwrap().uses = saved;
    }

    // A disjoint face can never be silently ignored by recognition.
    faces.push(Face::Planar {
        origin: ORIGIN,
        normal: Z,
        area: 1.,
    });
    assert!(
        shell
            .match_chamfered_polygon(&faces, &edges, 0, ORIGIN, Z)
            .is_none()
    );
    faces.pop();

    // A conical wall with a non-45° slope has a valid analytic readback
    // but is outside this reconstruction contract.
    let wrong_mouth = CircleEdge {
        radius_mm: mouth.radius_mm + 0.25,
        ..mouth
    };
    shell.edges.get_mut(&rim).unwrap().curve = Curve::Circle(wrong_mouth);
    assert!(shell.chamfered_polygon(&faces).is_none());
    shell.edges.get_mut(&rim).unwrap().curve = Curve::Circle(mouth);

    // Reversing the surface turns the inward wall into a positive feature.
    let Face::Conical(wall) = &mut faces[cone] else {
        unreachable!()
    };
    wall.outward = true;
    assert!(shell.chamfered_polygon(&faces).is_none());
}

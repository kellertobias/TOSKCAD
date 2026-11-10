use super::*;
use crate::{Rational, planar_boolean::ConvexPiece, step};

const TOL: f64 = 1e-7;
const ORIGIN: Point = [13., -5., 41.];
const U: Point = [0.8, 0.6, 0.];
const V: Point = [-0.36, 0.48, 0.8];

fn sloped_union() -> Union {
    let box_ = ConvexPiece::from_box([0.; 3], [2.; 3], TOL).unwrap();
    let wedge = ConvexPiece::new(
        vec![
            vec![[1., 0., 0.], [3., 0., 0.], [1., 0., 2.]],
            vec![[1., 2., 0.], [1., 2., 2.], [3., 2., 0.]],
            vec![[1., 0., 0.], [1., 2., 0.], [3., 2., 0.], [3., 0., 0.]],
            vec![[1., 0., 0.], [1., 0., 2.], [1., 2., 2.], [1., 2., 0.]],
            vec![[3., 0., 0.], [3., 2., 0.], [1., 2., 2.], [1., 0., 2.]],
        ],
        TOL,
    )
    .unwrap();
    Union::new(&[box_, wedge], TOL).unwrap()
}

#[test]
fn imports_rotated_planar_fragments_with_shared_straight_edges() {
    let original = sloped_union();
    let text = step::write_planar_union(&original, ORIGIN, U, V).unwrap();
    let (readback, decoded) = read_import_solid(&text, TOL).unwrap();
    let ImportProfile::PlanarBoundary { shell } = decoded.profile else {
        panic!("expected general planar boundary");
    };
    assert_eq!(decoded.origin_mm, [0.; 3]);
    assert_eq!(decoded.u, [1., 0., 0.]);
    assert_eq!(decoded.v, [0., 1., 0.]);
    assert_eq!(
        decoded.height_mm,
        readback.bounds_mm[1][2] - readback.bounds_mm[0][2]
    );
    assert_eq!(shell.solid_count(), 1);
    assert_eq!(shell.shell_count(), 1);
    assert_eq!(shell.faces().len(), original.faces().len());
    assert_eq!(shell.vertices().len(), original.vertices().len());
    assert_eq!(shell.bounds(), readback.bounds_mm);
    assert!((shell.volume_mm3() - 9.).abs() < TOL);
    assert!((shell.volume_mm3() - readback.volume_mm3).abs() < TOL);
    let w = cross(U, V);
    for &p in original.vertices() {
        let placed: Point =
            std::array::from_fn(|i| ORIGIN[i] + p[0] * U[i] + p[1] * V[i] + p[2] * w[i]);
        assert!(
            shell
                .vertices()
                .iter()
                .any(|&q| length(sub(placed, q)) < TOL)
        );
    }
    for (a, b) in original.faces().iter().zip(shell.faces()) {
        let n = a.normal;
        let normal: Point = std::array::from_fn(|i| n[0] * U[i] + n[1] * V[i] + n[2] * w[i]);
        assert!(length(sub(normal, b.normal)) < TOL);
    }
    let mut edge_ids = BTreeSet::new();
    for face in shell.faces() {
        for (&a, &b) in face
            .vertices
            .iter()
            .zip(face.vertices.iter().cycle().skip(1))
        {
            edge_ids.insert([a.min(b), a.max(b)]);
        }
    }
    assert_eq!(edge_ids.len(), readback.edges_mm.len());
    for edge in edge_ids {
        let [a, b] = edge.map(|i| shell.vertices()[i]);
        assert!(
            readback
                .edges_mm
                .iter()
                .any(|&p| p == [a, b] || p == [b, a])
        );
    }
}

#[test]
fn imports_a_planar_member_alongside_a_preferred_sweep() {
    let mut writer = step::Writer::new();
    writer
        .append_planar_union(&sloped_union(), ORIGIN, U, V)
        .unwrap();
    writer
        .append_box(
            [Rational::new(2, 1).unwrap(); 3],
            [30., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
        )
        .unwrap();
    let members = read_import_compound(&writer.finish().unwrap(), TOL).unwrap();
    assert!(matches!(
        members[0].1.profile,
        ImportProfile::PlanarBoundary { .. }
    ));
    assert!(matches!(
        members[1].1.profile,
        ImportProfile::Polygon { .. }
    ));
}

#[test]
fn rejects_missing_and_multiple_directed_rings() {
    let points = BTreeMap::from([(0, [0., 0., 0.]), (1, [1., 0., 0.])]);
    assert_eq!(
        directed_ring(BTreeMap::new(), &points),
        Err(ReadError::Invalid)
    );
    assert_eq!(
        directed_ring(BTreeMap::from([(0, 1)]), &points),
        Err(ReadError::Invalid)
    );
    assert_eq!(
        directed_ring(BTreeMap::from([(1, 2), (2, 1)]), &points),
        Err(ReadError::Invalid)
    );
    assert_eq!(
        directed_ring(BTreeMap::from([(0, 0), (1, 1)]), &points),
        Err(ReadError::Unsupported)
    );
    assert_eq!(
        boundary_error(planar_boolean::Error::Limit),
        ReadError::Limit
    );
}

#[test]
fn keeps_structural_and_geometric_rejections_separate() {
    let text = step::write_planar_union(&sloped_union(), ORIGIN, U, V).unwrap();
    let entities = parse_entities(&text, TOL).unwrap();
    let mut parsed = Shell::new(&entities, TOL, false);
    let faces = parsed.validated_faces().unwrap();
    let first = parsed.edges.values_mut().next().unwrap();
    first.uses.push(first.uses[0]);
    assert_eq!(parsed.planar_boundary(&faces), Err(ReadError::Invalid));
    parsed.edges.values_mut().next().unwrap().uses.pop();
    parsed.points.insert(u32::MAX, ORIGIN);
    assert_eq!(parsed.planar_boundary(&faces), Err(ReadError::Invalid));
    parsed.points.remove(&u32::MAX);
    let point = parsed.points.values_mut().next().unwrap();
    point[0] += 0.1;
    assert_eq!(parsed.planar_boundary(&faces), Err(ReadError::Invalid));

    let mut curved_faces = faces;
    curved_faces[0] = Face::Cylindrical { seam: 0, flux: 0. };
    assert_eq!(
        parsed.planar_boundary(&curved_faces),
        Err(ReadError::Unsupported)
    );
}

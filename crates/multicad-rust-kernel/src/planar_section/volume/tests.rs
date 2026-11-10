use super::super::tests::{box_body, translate};
use super::*;

fn horizontal_face() -> Face {
    Face {
        point: [0.; 3],
        normal: [0., 0., 1.],
        edges: vec![],
    }
}

fn merge(mut first: Body, second: Body) -> Body {
    let offset = first.edges.len();
    first.edges.extend(second.edges);
    first.faces.extend(second.faces.into_iter().map(|mut face| {
        for id in &mut face.edges {
            *id += offset;
        }
        face
    }));
    first
}

#[test]
fn coincident_faces_keep_only_the_material_on_the_retained_side() {
    let below = box_body();
    let above = translate(box_body(), [12., 0., 6.]);
    let plane = Plane {
        axis: 2,
        offset: 6.,
        sign: -1.,
    };
    let (result, changed) = plane.clip(merge(below, above), 1e-7, &mut 1000).unwrap();
    assert!(changed);
    assert_eq!(result.faces.len(), 6);
    assert_eq!(result.edges.len(), 12);
    assert!(result.edges.iter().flatten().all(|p| p[0] <= 8.));
    assert_eq!(
        result
            .faces
            .iter()
            .filter(|f| f.point[2] == 6. && f.normal[2] == 1.)
            .count(),
        1
    );
}

#[test]
fn an_open_wall_away_from_the_cut_cannot_be_capped() {
    let mut body = box_body();
    body.faces.remove(0);
    let plane = Plane {
        axis: 2,
        offset: 3.,
        sign: -1.,
    };
    assert_eq!(
        plane.clip(body, 1e-7, &mut 1000).err(),
        Some(ProjectionError::Invalid)
    );
}

#[test]
fn assembled_faces_reject_odd_incidence_and_nonmanifold_sharing() {
    let a = [0.; 3];
    let b = [1., 0., 0.];
    let c = [0., 1., 0.];
    let d = [1., 1., 0.];
    let mut boundary = Boundary::default();
    boundary
        .push(horizontal_face(), vec![[a, b], [a, b], [a, a]])
        .unwrap();
    assert!(boundary.faces.is_empty());
    assert_eq!(
        boundary.push(horizontal_face(), vec![[a, b], [b, c], [c, d]]),
        Err(ProjectionError::Invalid)
    );
    let perimeter = vec![[a, b], [b, c], [c, a]];
    boundary.push(horizontal_face(), perimeter.clone()).unwrap();
    boundary.push(horizontal_face(), perimeter.clone()).unwrap();
    assert_eq!(
        boundary.push(horizontal_face(), perimeter),
        Err(ProjectionError::Invalid)
    );
}

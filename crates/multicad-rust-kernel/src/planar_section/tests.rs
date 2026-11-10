use super::*;
use crate::planar_projection::Face;

fn square([x0, y0, x1, y1]: [f64; 4]) -> Vec<Segment> {
    let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    (0..4).map(|i| [corners[i], corners[(i + 1) % 4]]).collect()
}

pub(super) fn box_body() -> Body {
    let corners = [
        [0., 0., 0.],
        [8., 0., 0.],
        [8., 4., 0.],
        [0., 4., 0.],
        [0., 0., 6.],
        [8., 0., 6.],
        [8., 4., 6.],
        [0., 4., 6.],
    ];
    let edges = [
        [0, 1],
        [1, 2],
        [2, 3],
        [3, 0],
        [4, 5],
        [5, 6],
        [6, 7],
        [7, 4],
        [0, 4],
        [1, 5],
        [2, 6],
        [3, 7],
    ]
    .map(|ends| ends.map(|i| corners[i]))
    .to_vec();
    let faces = [
        ([0., 0., -1.], [0, 1, 2, 3]),
        ([0., 0., 1.], [4, 5, 6, 7]),
        ([0., -1., 0.], [0, 4, 8, 9]),
        ([1., 0., 0.], [1, 5, 9, 10]),
        ([0., 1., 0.], [2, 6, 10, 11]),
        ([-1., 0., 0.], [3, 7, 11, 8]),
    ]
    .into_iter()
    .map(|(normal, edges_of_face)| Face {
        normal,
        point: edges[edges_of_face[0]][0],
        edges: edges_of_face.to_vec(),
    })
    .collect();
    Body { edges, faces }
}

fn options() -> Options {
    Options {
        depth: 2.,
        windows: [None; 2],
        tolerance: 1e-7,
        segment_limit: 100,
    }
}

fn segment_keys(segments: &[Segment]) -> BTreeSet<[Key; 2]> {
    segments
        .iter()
        .map(|segment| {
            let mut pair = segment.map(key);
            pair.sort_unstable();
            pair
        })
        .collect()
}

#[test]
fn normal_and_coincident_face_cuts_have_the_same_area_boundary() {
    let stock = box_body();
    let target = segment_keys(&square([0., 0., 8., 4.]));
    for depth in [0., 2., 6.] {
        let [actual] = section(std::slice::from_ref(&stock), Options { depth, ..options() })
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(segment_keys(&actual), target);
    }
    let empty = section(
        &[stock],
        Options {
            depth: 7.,
            ..options()
        },
    )
    .unwrap();
    assert!(empty[0].is_empty());
}

#[test]
fn section_windows_close_cut_boundaries_with_an_open_hole() {
    let mut edges = square([0., 0., 8., 4.]);
    edges.extend(square([2., 1., 4., 3.]));
    let actual = clip(&edges, 0, 3., 1., 1e-7, &mut 1000).unwrap();
    let outline = [
        [3., 0.],
        [8., 0.],
        [8., 4.],
        [3., 4.],
        [3., 3.],
        [4., 3.],
        [4., 1.],
        [3., 1.],
    ];
    let target: Vec<_> = (0..8).map(|i| [outline[i], outline[(i + 1) % 8]]).collect();
    assert_eq!(segment_keys(&actual), segment_keys(&target));

    let clipped = section(
        &[box_body()],
        Options {
            windows: [Some([0., 6.]), Some([1., 3.])],
            ..options()
        },
    )
    .unwrap();
    assert_eq!(
        segment_keys(&clipped[0]),
        segment_keys(&square([0., 1., 6., 3.]))
    );
}

#[test]
fn tangent_trees_are_not_material_and_odd_junctions_are_rejected() {
    let mut segments = square([0., 0., 8., 4.]);
    segments.push([[0., 0.], [-2., 0.]]);
    segments.push([[-3., 0.], [-2., 0.]]);
    let trimmed = closed_boundary(segments).unwrap();
    assert_eq!(trimmed.len(), 4);
    let mut odd = square([0., 0., 8., 4.]);
    odd.push([[0., 0.], [8., 4.]]);
    assert_eq!(closed_boundary(odd), Err(ProjectionError::Invalid));
}

#[test]
fn invalid_coordinates_references_and_budgets_fail_before_publication() {
    assert_eq!(
        section(
            &[box_body()],
            Options {
                depth: f64::NAN,
                ..options()
            }
        ),
        Err(ProjectionError::Invalid)
    );
    assert_eq!(
        section(
            &[box_body()],
            Options {
                segment_limit: 3,
                ..options()
            }
        ),
        Err(ProjectionError::Limit)
    );
    let mut nonfinite = box_body();
    nonfinite.edges[0][0][0] = f64::INFINITY;
    assert_eq!(
        section(&[nonfinite], options()),
        Err(ProjectionError::Invalid)
    );
    let mut bad_edge = box_body();
    bad_edge.faces[2].edges.push(13);
    assert_eq!(
        section(&[bad_edge], options()),
        Err(ProjectionError::Invalid)
    );
    assert_eq!(pairs(vec![[1., 2.]], 0), Err(ProjectionError::Invalid));
}

pub(super) fn translate(mut body: Body, shift: [f64; 3]) -> Body {
    let translate = |p: &mut [f64; 3]| p.iter_mut().zip(shift).for_each(|(v, d)| *v += d);
    for point in body.edges.iter_mut().flatten() {
        translate(point);
    }
    for face in &mut body.faces {
        translate(&mut face.point);
    }
    body
}

#[test]
fn depth_and_lateral_clips_preserve_a_closed_volume_and_source_slots() {
    let untouched = translate(box_body(), [1., 1., 1.]);
    let partly_visible = translate(box_body(), [0., 0., -4.]);
    let outside = translate(box_body(), [0., 0., 20.]);
    let retained = retain(
        vec![untouched, partly_visible, outside],
        Options {
            depth: 7.,
            windows: [Some([1., 9.]), Some([1., 5.])],
            ..options()
        },
        Some(0.),
    )
    .unwrap();
    assert_eq!(retained.trimmed, [false, true, true]);
    assert!(retained.bodies[2].edges.is_empty());
    let middle = &retained.bodies[1];
    assert_eq!(middle.faces.len(), 6);
    assert_eq!(middle.edges.len(), 12);
    let mut incidence = [0; 12];
    for &id in middle.faces.iter().flat_map(|face| &face.edges) {
        incidence[id] += 1;
    }
    assert_eq!(incidence, [2; 12]);
    for &[x, y, z] in middle.edges.iter().flatten() {
        assert!([1., 8.].contains(&x));
        assert!([1., 4.].contains(&y));
        assert!([0., 2.].contains(&z));
    }
    assert!(
        middle
            .faces
            .iter()
            .any(|face| face.normal == [0., 0., -1.] && face.point[2] == 0.)
    );
}

#[test]
fn volume_refuses_nonfinite_points_inverted_depth_and_missing_edge_indices() {
    assert_eq!(
        retain(vec![box_body()], options(), Some(3.)).err(),
        Some(ProjectionError::Invalid)
    );
    let mut invalid = box_body();
    invalid.edges[0][0][0] = f64::INFINITY;
    assert_eq!(
        retain(vec![invalid], options(), None).err(),
        Some(ProjectionError::Invalid)
    );
    let mut invalid = box_body();
    invalid.faces[2].edges.push(13);
    assert_eq!(
        retain(vec![invalid], options(), None).err(),
        Some(ProjectionError::Invalid)
    );
}

#[test]
fn window_inside_every_stock_face_retains_closed_box() {
    let section_options = Options {
        depth: 3.,
        windows: [Some([1., 7.]), Some([1., 3.])],
        ..options()
    };
    let bodies = vec![
        box_body(),
        translate(box_body(), [0., 0., 10.]),
        translate(box_body(), [0., 0., -10.]),
    ];
    let outlines = section(&bodies, section_options).unwrap();
    assert_eq!(outlines.iter().map(Vec::len).collect::<Vec<_>>(), [4, 0, 0]);
    let retained = retain(bodies, section_options, Some(1.)).unwrap();
    assert_eq!(retained.bodies[0].edges.len(), 12);
    let vectors = crate::planar_projection::project(
        &retained.bodies,
        crate::planar_projection::Options {
            tolerance: 1e-7,
            hidden_lines: false,
            tangent_edges: false,
            detail: None,
            segment_limit: 100,
        },
    )
    .unwrap();
    assert_eq!(vectors.len(), 4);
}

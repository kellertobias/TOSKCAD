use super::*;

pub(super) fn options() -> Options {
    Options {
        tolerance: 1e-7,
        hidden_lines: true,
        tangent_edges: false,
        detail: None,
        segment_limit: 1000,
    }
}

pub(crate) fn prism(rings: &[&[Point2]], low: f64, high: f64) -> Body {
    let mut edges = Vec::new();
    let mut bottom = Vec::new();
    let mut top = Vec::new();
    let mut faces = Vec::new();
    for ring in rings {
        let first = edges.len();
        let at = |point: Point2, z| [point[0], point[1], z];
        for (&p, &q) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            bottom.push(edges.len());
            top.push(edges.len() + 1);
            edges.extend([
                [at(p, low), at(q, low)],
                [at(p, high), at(q, high)],
                [at(p, low), at(p, high)],
            ]);
        }
        for (i, (&p, &q)) in ring.iter().zip(ring.iter().cycle().skip(1)).enumerate() {
            let a = first + i * 3;
            let b = first + (i + 1) % ring.len() * 3;
            let [dx, dy] = sub(q, p);
            let length = dx.hypot(dy);
            faces.push(Face {
                point: at(p, low),
                normal: [dy / length, -dx / length, 0.],
                edges: vec![a, a + 1, a + 2, b + 2],
            });
        }
    }
    faces.extend([
        Face {
            point: [0., 0., low],
            normal: [0., 0., -1.],
            edges: bottom,
        },
        Face {
            point: [0., 0., high],
            normal: [0., 0., 1.],
            edges: top,
        },
    ]);
    Body { edges, faces }
}

pub(crate) fn rect([x0, y0, x1, y1]: [f64; 4], low: f64, high: f64) -> Body {
    prism(&[&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]]], low, high)
}

fn line_ranges(segments: &[Segment], source: usize, y: f64, hidden: bool) -> Vec<Span> {
    let mut result: Vec<_> = segments
        .iter()
        .filter(|s| s.source == source && s.hidden == hidden && s.ends.iter().all(|p| p[1] == y))
        .map(|s| s.ends.map(|p| p[0]))
        .map(|[a, b]| [a.min(b), a.max(b)])
        .collect();
    result.sort_by(|a, b| a[0].total_cmp(&b[0]));
    result.dedup();
    result
}

#[test]
fn opening_in_foreground_plate_keeps_rear_line_visible_through_it() {
    let front = prism(
        &[
            &[[1., -2.], [9., -2.], [9., 2.], [1., 2.]],
            &[[4., -1.], [4., 1.], [6., 1.], [6., -1.]],
        ],
        2.,
        3.,
    );
    let bodies = [front, rect([0., 0., 10., 4.], -2., -1.)];
    let result = project(&bodies, options()).unwrap();
    assert_eq!(
        line_ranges(&result, 1, 0., false),
        [[0., 1.], [4., 6.], [9., 10.]]
    );
    assert_eq!(
        line_ranges(&result, 1, 0., true),
        [[0., 10.], [1., 4.], [6., 9.]]
    );
    let visible = project(
        &bodies,
        Options {
            hidden_lines: false,
            ..options()
        },
    )
    .unwrap();
    assert_eq!(
        visible,
        result.into_iter().filter(|s| !s.hidden).collect::<Vec<_>>()
    );
}

#[test]
fn round_occluders_merge_with_planar_faces_and_preserve_the_bore_opening() {
    let front = rect([4., -0.5, 5., 0.5], 4., 5.);
    let rear = rect([0., 0., 10., 4.], -2., -1.);
    let round = RoundOccluder::new([5., 0., 2.], [0., 0., 1.], 2., 1., Some(1.)).unwrap();
    let bodies = [front, rear];
    let projected = project_with_rounds(&bodies, &[round], options()).unwrap();
    let visible = line_ranges(&projected, 1, 0., false);
    assert_eq!(visible.len(), 3);
    for (actual, expected) in visible.into_iter().flatten().zip([0., 3., 5., 6., 7., 10.]) {
        assert!((actual - expected).abs() < 1e-7, "{actual} != {expected}");
    }
    let hidden = line_ranges(&projected, 1, 0., true);
    assert!(
        hidden
            .iter()
            .any(|[a, b]| (*a - 3.).abs() < 1e-7 && (*b - 5.).abs() < 1e-7)
    );
    assert!(
        hidden
            .iter()
            .any(|[a, b]| (*a - 6.).abs() < 1e-7 && (*b - 7.).abs() < 1e-7)
    );
    assert_eq!(
        project_bounded(&bodies, &[round; 100], options(), 200),
        Err(ProjectionError::Limit)
    );
}

#[test]
fn disk_clips_lines_and_compact_index_prunes_disjoint_bodies() {
    let bodies: Vec<_> = (0..16)
        .map(|i| rect([i as f64 * 12., 0., i as f64 * 12. + 10., 2.], 0., 1.))
        .collect();
    let result = project(
        &bodies,
        Options {
            detail: Some(([5., 0.], 2.)),
            ..options()
        },
    )
    .unwrap();
    assert_eq!(line_ranges(&result, 0, 0., false), [[3., 7.]]);
    assert_eq!(line_ranges(&result, 0, 0., true), [[3., 7.]]);
    assert_eq!(result.len(), 2);
}

#[test]
fn tangent_option_controls_a_coplanar_edge_but_keeps_the_silhouette() {
    let body = prism(
        &[&[[0., 0.], [5., 0.], [10., 0.], [10., 8.], [0., 8.]]],
        0.,
        3.,
    );
    let rotation = |[x, y, z]: Point| {
        let u = 2f64.sqrt().recip();
        [x * u - z * u, y, x * u + z * u]
    };
    let body = Body {
        edges: body.edges.iter().map(|edge| edge.map(rotation)).collect(),
        faces: body
            .faces
            .into_iter()
            .map(|f| Face {
                point: rotation(f.point),
                normal: rotation(f.normal),
                edges: f.edges,
            })
            .collect(),
    };
    let crisp = project(std::slice::from_ref(&body), options()).unwrap();
    let all = project(
        &[body],
        Options {
            tangent_edges: true,
            ..options()
        },
    )
    .unwrap();
    assert_eq!(all.len(), crisp.len() + 1);
    assert!(
        all.iter()
            .all(|line| line.ends.iter().flatten().all(|p| p.is_finite()))
    );
}

#[test]
fn crossing_a_tilted_face_changes_visibility_at_the_depth_intersection() {
    let mut front = rect([0., -2., 10., 2.], 0., 1.);
    let incline = |[x, y, z]: Point| [x, y, z + x * 0.5 - 3.5];
    for edge in &mut front.edges {
        *edge = edge.map(incline);
    }
    for face in &mut front.faces {
        face.point = incline(face.point);
        if face.normal[2] != 0. {
            let n = face.normal[2] / 1.25f64.sqrt();
            face.normal = [-0.5 * n, 0., n];
        }
    }
    let bodies = [front, rect([1., 0., 9., 4.], -2., 0.)];
    let result = project(&bodies, options()).unwrap();
    let visible = line_ranges(&result, 1, 0., false);
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0][0], 1.);
    assert!((visible[0][1] - 5.).abs() < 1e-6);
}

#[test]
fn invalid_sources_and_work_or_output_overflow_return_errors() {
    let bad_point = rect([0., 0., f64::INFINITY, 2.], 0., 1.);
    assert_eq!(
        project(&[bad_point], options()),
        Err(ProjectionError::Invalid)
    );
    let mut open = rect([0., 0., 2., 2.], 0., 1.);
    open.faces.pop();
    assert_eq!(project(&[open], options()), Err(ProjectionError::Invalid));
    let mut bad_plane = rect([0., 0., 2., 2.], 0., 1.);
    bad_plane.faces[0].normal = [0.; 3];
    assert_eq!(
        project(&[bad_plane], options()),
        Err(ProjectionError::Invalid)
    );
    let mut repeated_edge = rect([0., 0., 2., 2.], 0., 1.);
    repeated_edge.faces[3].edges.retain(|&edge| edge != 2);
    repeated_edge.faces[0].edges.push(2);
    assert_eq!(
        project(&[repeated_edge], options()),
        Err(ProjectionError::Invalid)
    );
    assert_eq!(
        project(
            &[Body {
                edges: vec![],
                faces: vec![]
            }],
            options()
        ),
        Err(ProjectionError::Invalid)
    );
    let body = rect([0., 0., 2., 2.], 0., 1.);
    assert_eq!(
        project_bounded(std::slice::from_ref(&body), &[], options(), 1),
        Err(ProjectionError::Limit)
    );
    assert_eq!(
        project(
            &[body],
            Options {
                segment_limit: 1,
                ..options()
            }
        ),
        Err(ProjectionError::Limit)
    );
    assert_eq!(project(&[], options()), Err(ProjectionError::Invalid));
}

#[test]
fn open_face_crossing_cannot_publish_an_unbounded_hidden_interval() {
    let face = TrimmedPlane {
        id: 1,
        point: [0., 0., 2.],
        normal: [0., 0., 1.],
        boundary: vec![[[1., -1.], [1., 1.]]],
        profiles: Vec::new(),
        bounds: Bounds::point([1., 1., 2.]).union(Bounds::point([1., -1., 2.])),
    };
    let line = Line::new([[0., 0., 0.], [2., 0., 0.]], 1e-7, [2, 3]).unwrap();
    let mut work = Work {
        remaining: 100,
        tolerance: 1e-7,
        crossings: Vec::new(),
        hidden: Vec::new(),
    };
    assert_eq!(
        face.hide(&line, [0., 2.], &mut work),
        Err(ProjectionError::Invalid)
    );
}

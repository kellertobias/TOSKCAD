use super::*;

#[test]
fn planar_trim_rejects_retraced_crossing_and_overlapping_segments() {
    let tolerance = 1e-7;
    let outer = Trim::Polygon(vec![[0., 0.], [12., 0.], [12., 12.], [0., 12.]]);
    let backwards = [[0., 0.], [3., 0.], [1., 0.], [0., 5.]];
    let crossing = [[0., 0.], [6., 4.], [0., 4.], [3., 0.]];
    assert!(!simple_ring(&backwards, tolerance));
    assert!(!simple_ring(&crossing, tolerance));

    let inside = vec![[2., 2.], [2., 8.], [8., 8.], [8., 2.]];
    let nested = vec![[3., 3.], [3., 4.], [4., 4.], [4., 3.]];
    let touches = vec![[9., 9.], [9., 12.], [11., 12.], [11., 9.]];
    let outside = vec![[13., 1.], [13., 2.], [14., 2.], [14., 1.]];
    assert_eq!(
        face_area(
            &outer,
            &[Trim::Polygon(inside.clone()), Trim::Polygon(nested)],
            tolerance
        ),
        Err(ReadError::Invalid)
    );
    assert_eq!(
        face_area(&outer, &[Trim::Polygon(touches)], tolerance),
        Err(ReadError::Invalid)
    );
    assert_eq!(
        face_area(&outer, &[Trim::Polygon(outside)], tolerance),
        Err(ReadError::Invalid)
    );
    let mut backwards_hole = inside;
    backwards_hole.reverse();
    assert_eq!(
        face_area(&outer, &[Trim::Polygon(backwards_hole)], tolerance),
        Err(ReadError::Invalid)
    );
}

fn incidence(
    edges: impl IntoIterator<Item = ([u32; 2], [usize; 2])>,
    face_count: usize,
) -> Result<()> {
    incidence_with_curves(
        edges
            .into_iter()
            .map(|(vertices, faces)| (vertices, faces, Curve::Line)),
        face_count,
    )
}

fn incidence_with_curves(
    edges: impl IntoIterator<Item = ([u32; 2], [usize; 2], Curve)>,
    face_count: usize,
) -> Result<()> {
    let shell = Shell {
        entities: &BTreeMap::new(),
        tolerance: 1e-7,
        allow_circles: false,
        points: BTreeMap::new(),
        edges: edges
            .into_iter()
            .enumerate()
            .map(|(id, (vertices, [a, b], curve))| {
                (
                    id as u32,
                    Edge {
                        vertices,
                        curve,
                        uses: vec![
                            EdgeUse {
                                face: a,
                                forward: true,
                            },
                            EdgeUse {
                                face: b,
                                forward: false,
                            },
                        ],
                    },
                )
            })
            .collect(),
    };
    let faces: Vec<_> = (0..face_count)
        .map(|_| Face::Planar {
            origin: [0.; 3],
            normal: [0., 0., 1.],
            area: 1.,
        })
        .collect();
    shell.check_incidence(&faces)
}

#[test]
fn edge_incidence_rejects_duplicate_edges_and_disconnected_shells_and_vertex_fans() {
    assert_eq!(
        incidence([([1, 2], [0, 1]), ([2, 1], [0, 1])], 2),
        Err(ReadError::Invalid)
    );
    assert_eq!(
        incidence([([1, 2], [0, 1]), ([3, 4], [2, 3])], 4),
        Err(ReadError::Invalid)
    );
    // Faces are globally connected, but the link of vertex 1 is two separate
    // closed fans. Every edge still has exactly two opposite uses.
    let vertex_fans = [
        ([1, 2], [0, 1]),
        ([1, 3], [1, 2]),
        ([1, 4], [2, 0]),
        ([1, 5], [3, 4]),
        ([1, 6], [4, 5]),
        ([1, 7], [5, 3]),
        ([8, 9], [2, 3]),
    ];
    assert_eq!(incidence(vertex_fans, 6), Err(ReadError::Invalid));
}

#[test]
fn only_periodic_seams_may_repeat_and_circular_edges_need_a_wall() {
    assert_eq!(incidence([([1, 2], [0, 0])], 1), Err(ReadError::Invalid));
    let circle = Curve::Circle(CircleEdge {
        center_mm: [0.; 3],
        normal: [0., 0., 1.],
        radius_mm: 1.,
    });
    assert_eq!(
        incidence_with_curves([([1, 1], [0, 1], circle)], 2),
        Err(ReadError::Invalid)
    );
}

#[test]
fn circular_trims_are_measured_and_require_clearance_in_either_order() {
    let circle = |center, radius, ccw| Trim::Circle {
        center,
        radius,
        ccw,
    };
    let square = |x| Trim::Polygon(vec![[x, -1.], [x, 1.], [x + 2., 1.], [x + 2., -1.]]);
    let outer = circle([0., 0.], 5., true);
    let inner = circle([0., 0.], 1., false);
    let tolerance = 1e-7;
    let area = face_area(&outer, &[inner, square(2.)], tolerance).unwrap();
    assert!((area - (24. * std::f64::consts::PI - 4.)).abs() < tolerance);
    let area = face_area(
        &outer,
        &[square(2.), circle([0., 0.], 1., false)],
        tolerance,
    )
    .unwrap();
    assert!((area - (24. * std::f64::consts::PI - 4.)).abs() < tolerance);
    for holes in [
        vec![circle([4., 0.], 1., false)],
        vec![square(3.)],
        vec![circle([0., 0.], 1., false), circle([2., 0.], 1., false)],
        vec![circle([0., 0.], 1., false), square(1.)],
    ] {
        assert_eq!(
            face_area(&outer, &holes, tolerance),
            Err(ReadError::Invalid)
        );
    }
}

mod exchange;

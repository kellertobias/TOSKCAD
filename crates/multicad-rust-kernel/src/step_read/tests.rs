use super::*;

#[test]
fn planar_trim_rejects_retraced_crossing_and_overlapping_segments() {
    let tolerance = 1e-7;
    let outer = [[0., 0.], [12., 0.], [12., 12.], [0., 12.]];
    let backwards = [[0., 0.], [3., 0.], [1., 0.], [0., 5.]];
    let crossing = [[0., 0.], [6., 4.], [0., 4.], [3., 0.]];
    assert!(!simple_ring(&backwards, tolerance));
    assert!(!simple_ring(&crossing, tolerance));

    let inside = vec![[2., 2.], [2., 8.], [8., 8.], [8., 2.]];
    let nested = vec![[3., 3.], [3., 4.], [4., 4.], [4., 3.]];
    let touches = vec![[9., 9.], [9., 12.], [11., 12.], [11., 9.]];
    let outside = vec![[13., 1.], [13., 2.], [14., 2.], [14., 1.]];
    assert_eq!(
        face_area(&outer, &[inside.clone(), nested], tolerance),
        Err(ReadError::Invalid)
    );
    assert_eq!(
        face_area(&outer, &[touches], tolerance),
        Err(ReadError::Invalid)
    );
    assert_eq!(
        face_area(&outer, &[outside], tolerance),
        Err(ReadError::Invalid)
    );
    let mut backwards_hole = inside;
    backwards_hole.reverse();
    assert_eq!(
        face_area(&outer, &[backwards_hole], tolerance),
        Err(ReadError::Invalid)
    );
}

fn incidence(
    edges: impl IntoIterator<Item = ([u32; 2], [usize; 2])>,
    face_count: usize,
) -> Result<()> {
    let shell = Shell {
        entities: &BTreeMap::new(),
        tolerance: 1e-7,
        points: BTreeMap::new(),
        edges: edges
            .into_iter()
            .enumerate()
            .map(|(id, (vertices, [a, b]))| {
                (
                    id as u32,
                    Edge {
                        vertices,
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
    shell.check_incidence(face_count)
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

mod exchange;

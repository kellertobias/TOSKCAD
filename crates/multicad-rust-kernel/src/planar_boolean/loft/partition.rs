use super::*;
use miter_sweep::{
    certify_section, triangle_rings, triangle_sources, triangulate_certified_section,
};

pub(super) fn loft(
    sections: &[Vec<[f64; 2]>],
    vertices: &[Point],
    clockwise: bool,
    tolerance: f64,
) -> Result<Vec<ProfileCell>, Error> {
    for section in sections {
        certify_section(section, clockwise, tolerance)?;
    }
    let count = sections[0].len();
    let rings = vertices.chunks_exact(count);
    for (bottom, top) in rings.clone().zip(rings.skip(1)) {
        prove_boundary(bottom, top, tolerance)?;
    }
    // One triangulation has to serve every cap of the material partition.
    // A diagonal valid at one station can run outside another section.
    let mut failure = Error::Unresolved;
    for section in sections {
        let material = triangulate_certified_section(section, clockwise).and_then(|triangles| {
            candidate(vertices, sections.len() - 1, count, &triangles, tolerance)
        });
        match material {
            Ok(cells) => return Ok(cells),
            Err(cause @ (Error::Limit | Error::Cancelled)) => failure = cause,
            Err(Error::Invalid | Error::Unresolved) => {}
        }
    }
    Err(failure)
}

fn candidate(
    vertices: &[Point],
    slabs: usize,
    count: usize,
    triangles: &[[usize; 3]],
    tolerance: f64,
) -> Result<Vec<ProfileCell>, Error> {
    let mut cells = Vec::with_capacity((slabs * triangles.len()).min(MAX_PIECES));
    for segment in 0..slabs {
        cells.extend(slab(
            vertices,
            segment,
            count,
            triangles,
            tolerance,
            MAX_PIECES - cells.len(),
        )?);
    }
    Ok(cells)
}

fn slab(
    vertices: &[Point],
    segment: usize,
    count: usize,
    triangles: &[[usize; 3]],
    tolerance: f64,
    remaining: usize,
) -> Result<Vec<ProfileCell>, Error> {
    let low = segment * count;
    let high = low + count;
    let bottom = &vertices[low..high];
    let top = &vertices[high..high + count];

    let parallel_partitions = triangles.iter().all(|&triangle| {
        let [a, b, c] = triangle;
        [[a, b], [b, c], [c, a]]
            .into_iter()
            .all(|[i, j]| prove_side(bottom[i], bottom[j], top[i], top[j]).is_ok())
    });
    if triangles.len() * if parallel_partitions { 1 } else { 3 } > remaining {
        return Err(Error::Limit);
    }
    if parallel_partitions {
        // Each checked prism keeps its cap triangle orientation. The shared
        // planar diagonals certify a conforming subdivision of the boundary.
        return triangles
            .iter()
            .map(|&triangle| {
                let piece = ConvexPiece::new(
                    triangle_rings(triangle, low, high)
                        .into_iter()
                        .map(|ring| ring.into_iter().map(|i| vertices[i]).collect())
                        .collect(),
                    tolerance,
                )?;
                Ok(ProfileCell {
                    piece,
                    segment,
                    source_faces: triangle_sources(triangle, count),
                })
            })
            .collect();
    }

    // Use one vertex ranking for the entire slab so neighbors choose the
    // same diagonal on shared quads. Positive tetrahedral orientations and
    // the embedded exterior certify degree one throughout the solid.
    for reverse in [false, true] {
        let cells: Vec<_> = triangles
            .iter()
            .flat_map(|&triangle| tetrahedra(triangle, low, high, reverse))
            .collect();
        if cells
            .iter()
            .all(|&tet| positive(tet.map(|i| vertices[i]), tolerance))
        {
            return cells
                .into_iter()
                .map(|tet| tetra_cell(tet, vertices, segment, count, tolerance))
                .collect();
        }
    }
    Err(Error::Unresolved)
}

fn tetrahedra(triangle: [usize; 3], low: usize, high: usize, reverse: bool) -> [[usize; 4]; 3] {
    let mut ordered = triangle;
    ordered.sort_unstable();
    if reverse {
        ordered.reverse();
    }
    let [a, b, c] = ordered;
    let mut cells = [
        [low + a, low + b, low + c, high + c],
        [low + a, low + b, high + c, high + b],
        [low + a, high + a, high + b, high + c],
    ];
    if ![
        triangle,
        [triangle[1], triangle[2], triangle[0]],
        [triangle[2], triangle[0], triangle[1]],
    ]
    .contains(&ordered)
    {
        for tet in &mut cells {
            tet.swap(1, 2);
        }
    }
    cells
}

fn positive([a, b, c, d]: [Point; 4], tolerance: f64) -> bool {
    let [ab, ac, ad] = [b, c, d].map(|p| sub(p, a));
    let base = cross(ab, ac);
    let largest = [
        base,
        cross(ab, ad),
        cross(ac, ad),
        cross(sub(c, b), sub(d, b)),
    ]
    .into_iter()
    .map(norm)
    .fold(0., f64::max);
    dot(base, ad) > tolerance * largest
}

fn tetra_cell(
    [a, b, c, d]: [usize; 4],
    vertices: &[Point],
    segment: usize,
    count: usize,
    tolerance: f64,
) -> Result<ProfileCell, Error> {
    let faces = [[a, c, b], [a, b, d], [b, c, d], [c, a, d]];
    let high = (segment + 1) * count;
    let source_faces = faces
        .map(|face| {
            if face.iter().all(|&v| v < high) {
                return ProfileFace::StartCap;
            }
            if face.iter().all(|&v| v >= high) {
                return ProfileFace::EndCap;
            }
            let mut ordinals = face.map(|v| v % count);
            ordinals.sort_unstable();
            let [a, b, c] = ordinals;
            if (a == b || b == c) && (a + 1) % count == c {
                ProfileFace::Edge(a)
            } else if (a == b || b == c) && (c + 1) % count == a {
                ProfileFace::Edge(c)
            } else {
                ProfileFace::Partition
            }
        })
        .to_vec();
    let piece = ConvexPiece::new(
        faces
            .map(|face| face.map(|i| vertices[i]).to_vec())
            .to_vec(),
        tolerance,
    )?;
    Ok(ProfileCell {
        piece,
        segment,
        source_faces,
    })
}

fn prove_boundary(bottom: &[Point], top: &[Point], tolerance: f64) -> Result<(), Error> {
    let count = bottom.len();
    for i in 0..count {
        let a = (i + 1) % count;
        for j in i + 2..count {
            let b = (j + 1) % count;
            if b == i {
                continue;
            }
            let lower = [i, a, j, b].map(|k| bottom[k]);
            let upper = [i, a, j, b].map(|k| top[k]);
            if can_meet(lower, upper, tolerance) {
                return Err(Error::Invalid);
            }
        }
    }
    Ok(())
}

fn can_meet(lower: [Point; 4], upper: [Point; 4], tolerance: f64) -> bool {
    let [a, b, c, d] = lower;
    let slack = tolerance + xy_roundoff(&lower).max(xy_roundoff(&upper));
    let mut interval = [0., 1.];
    for direction in [sub(b, a), sub(d, c)] {
        let [x, y, _] = scale(direction, 1. / norm(direction));
        for axis in [[x, y, 0.], [-y, x, 0.]] {
            let gaps = [lower, upper].map(|points| {
                let [a0, a1, b0, b1] = points.map(|p| dot(sub(p, a), axis));
                [
                    a0.max(a1) - b0.min(b1) + slack,
                    b0.max(b1) - a0.min(a1) + slack,
                ]
            });
            // Projection extrema are bounded by interpolating endpoint
            // ranges. Possible contact needs overlap on every axis at one
            // common height, including the parallel and collinear cases.
            for [start, end] in std::array::from_fn::<_, 2, _>(|i| [gaps[0][i], gaps[1][i]]) {
                if start < 0. && end < 0. {
                    return false;
                }
                if start < 0. {
                    interval[0] = f64::max(interval[0], start / (start - end));
                } else if end < 0. {
                    interval[1] = f64::min(interval[1], start / (start - end));
                }
                if interval[0] > interval[1] + 16. * f64::EPSILON {
                    return false;
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: f64 = 1e-7;

    #[test]
    fn a_shared_partition_can_need_the_opposite_diagonal() {
        let mut points = [
            [0., 0., 0.],
            [1., 0., 0.],
            [0.5, 1., 0.],
            [0., 0., 2.],
            [2., -3., 2.],
            [4., -4., 2.],
        ];
        let triangles = [[0, 1, 2]];
        let cells = slab(&points, 0, 3, &triangles, TOL, 3).unwrap();
        assert_eq!(cells.len(), 3);
        assert!(cells.iter().all(|cell| cell.piece.faces.len() == 4));
        points[2][0] = 0.2;
        assert_eq!(
            slab(&points, 0, 3, &triangles, TOL, 3).unwrap_err(),
            Error::Unresolved
        );
    }

    #[test]
    fn thin_cells_and_budget_exhaustion_stop_the_partition() {
        let ring = [
            [0., 0., 0.],
            [1., 1e-16, 0.],
            [2., 0., 0.],
            [2., 3., 0.],
            [0., 3., 0.],
        ];
        let points: Vec<_> = ring
            .into_iter()
            .chain(ring.map(|[x, y, _]| [x, y, 2.]))
            .collect();
        assert_eq!(
            slab(&points, 0, 5, &[[0, 2, 1]], TOL, 0).unwrap_err(),
            Error::Limit
        );
        assert_eq!(
            slab(&points, 0, 5, &[[0, 2, 1]], TOL, 1).unwrap_err(),
            Error::Invalid
        );
        assert_eq!(
            tetra_cell([0, 4, 3, 5], &points, 0, 5, TOL).unwrap_err(),
            Error::Invalid
        );
    }

    #[test]
    fn axis_overlaps_at_different_heights_do_not_imply_contact() {
        let start = [[0., 0., 0.], [3., 0., 0.], [-3., -1., 0.], [-3., 1., 0.]];
        let end = [[0., 0., 1.], [3., 0., 1.], [3., 4., 1.], [3., 6., 1.]];
        assert!(!can_meet(start, end, TOL));
    }
}

use super::*;
use crate::{
    display::triangulate_region,
    normal_sweep::{self, Edge, Sweep},
};
use spade::Point2;

const MAX_PATH_POINTS: usize = 65;
const MAX_SECTION_VERTICES: usize = MAX_PIECE_FACES - 2;
const Z: Point = [0., 0., 1.];

/// One convex material cell of a profile operation, with face labels in piece order.
/// Start and end caps describe this segment even when the adjacent segment
/// covers them. `Partition` walls only subdivide the interior of the segment.
#[derive(Clone, Debug)]
pub struct ProfileCell {
    pub piece: ConvexPiece,
    pub segment: usize,
    pub source_faces: Vec<ProfileFace>,
}

/// Sweep a simple polygon in XY along a polyline from the origin with an initial
/// +Z tangent. Sections transport by the minimum rotation at each corner.
/// Adjacent legs meet at an identical bisector ring. Nonadjacent cells require
/// a separating coordinate axis or support plane. Convex sections need only
/// one cell per leg; concave sections use a certified triangular partition.
///
/// Each path point owns one indexed ring in authored vertex order. Exterior
/// faces keep the leg index and an ordinal of 0 (start cap), 1 (end cap), or
/// 2 + the corresponding authored section edge index.
pub fn miter_sweep(
    section: &[[f64; 2]],
    path: &[[f64; 3]],
    tolerance_mm: f64,
) -> Result<(Union, Vec<ProfileCell>), Error> {
    validate_tolerance(tolerance_mm)?;
    if section.len() > MAX_SECTION_VERTICES || path.len() > MAX_PATH_POINTS {
        return Err(Error::Limit);
    }
    if section.len() < 3
        || path.len() < 2
        || path[0] != [0.; 3]
        || path[1][..2] != [0.; 2]
        || path[1][2] <= tolerance_mm
        || section
            .iter()
            .flatten()
            .chain(path.iter().flatten())
            .any(|&value| !value.is_finite() || value.abs() > MAX_COORDINATE)
    {
        return Err(Error::Invalid);
    }

    let mut tangents = vec![Z];
    for pair in path[1..].windows(2) {
        let vector = sub(pair[1], pair[0]);
        let length = norm(vector);
        if length <= tolerance_mm {
            return Err(Error::Invalid);
        }
        tangents.push(scale(vector, 1. / length));
    }

    let count = section.len();
    let mut offsets: Vec<_> = section.iter().map(|&[x, y]| [x, y, 0.]).collect();
    let origin = offsets[0];
    let clockwise = offsets
        .windows(2)
        .map(|edge| cross(sub(edge[0], origin), sub(edge[1], origin))[2])
        .sum::<f64>()
        < 0.;
    let partition = partition_section(section, clockwise, tolerance_mm)?;
    let legs = tangents.len();
    let cell_count = legs * partition.len().max(1);
    if cell_count > MAX_PIECES {
        return Err(Error::Limit);
    }
    let mut vertices = Vec::with_capacity(path.len() * count);
    vertices.extend_from_slice(&offsets);
    let mut normals = vec![Z];
    for (&corner, pair) in path[1..].iter().zip(tangents.windows(2)) {
        let (incoming, outgoing) = (pair[0], pair[1]);
        let cosine = dot(incoming, outgoing);
        if 1. + cosine <= 64. * f64::EPSILON {
            return Err(Error::Invalid);
        }
        let bisector = add(incoming, outgoing);
        let axis = cross(incoming, outgoing);
        for offset in &mut offsets {
            let distance = -dot(bisector, *offset) / dot(bisector, incoming);
            vertices.push(add(corner, add(*offset, scale(incoming, distance))));
            // Rodrigues rotation using the cross of the unit tangents, whose
            // length already supplies the sine. Parallel tangents give identity.
            let once = cross(axis, *offset);
            *offset = add(
                add(*offset, once),
                scale(cross(axis, once), 1. / (1. + cosine)),
            );
        }
        normals.push(scale(bisector, 1. / norm(bisector)));
    }
    let last = path[path.len() - 1];
    vertices.extend(offsets.into_iter().map(|point| add(last, point)));
    normals.push(tangents[tangents.len() - 1]);
    if !partition.is_empty() {
        for (ring, &normal) in vertices.chunks_exact(count).zip(&normals) {
            validate_section_plane(ring, normal, tolerance_mm)?;
        }
    }

    let mut material: Vec<ProfileCell> = Vec::with_capacity(cell_count);
    let mut faces = Vec::with_capacity(2 + legs * count);
    let mut budget = MAX_WORK;
    for (source, &tangent) in tangents.iter().enumerate() {
        let low = source * count;
        let high = low + count;
        let points = &vertices[low..high + count];
        if points[..count]
            .iter()
            .zip(&points[count..])
            .any(|(&a, &b)| {
                let offset = sub(b, a);
                (!partition.is_empty() && norm(cross(offset, tangent)) > tolerance_mm)
                    || [normals[source], normals[source + 1]]
                        .into_iter()
                        .any(|normal| dot(normal, offset) <= tolerance_mm)
            })
        {
            return Err(Error::Invalid);
        }
        let mut rings: Vec<Vec<usize>> =
            vec![(low..high).rev().collect(), (high..high + count).collect()];
        for i in 0..count {
            let j = (i + 1) % count;
            rings.push(vec![low + i, low + j, high + j, high + i]);
        }
        if clockwise {
            for ring in &mut rings {
                ring.reverse();
            }
        }
        let first_cell = material.len();
        let piece = |rings: &[Vec<usize>]| {
            ConvexPiece::new(
                rings
                    .iter()
                    .map(|ring| ring.iter().map(|&vertex| vertices[vertex]).collect())
                    .collect(),
                tolerance_mm,
            )
        };
        let wall_normals = if partition.is_empty() {
            let mut source_faces = vec![ProfileFace::StartCap, ProfileFace::EndCap];
            source_faces.extend((0..count).map(ProfileFace::Edge));
            material.push(ProfileCell {
                piece: piece(&rings)?,
                segment: source,
                source_faces,
            });
            Vec::new()
        } else {
            let mut wall_normals = vec![[0.; 3]; count];
            for &triangle in &partition {
                let cell_rings = triangle_rings(triangle, low, high);
                let piece = piece(&cell_rings)?;
                let source_faces = triangle_sources(triangle, count);
                // The CDT certifies that exactly one material triangle uses
                // each oriented exterior constraint. Its cell supplies the
                // corresponding authored wall's checked supporting normal.
                for (&source, plane) in source_faces.iter().zip(&piece.planes) {
                    if let ProfileFace::Edge(edge) = source {
                        wall_normals[edge] = plane.normal;
                    }
                }
                material.push(ProfileCell {
                    piece,
                    segment: source,
                    source_faces,
                });
            }
            wall_normals
        };

        // The cells within each leg partition the same section along parallel
        // generators. Neighboring legs occupy opposite sides of their shared
        // bisector. Only cells on nonadjacent legs need a separation test.
        for cell in &material[first_cell..] {
            for previous in material[..first_cell]
                .iter()
                .take_while(|previous| previous.segment + 1 < source)
            {
                prove_separation(&previous.piece, &cell.piece, tolerance_mm, &mut budget)?;
            }
        }
        for (source_face, ring) in rings.into_iter().enumerate() {
            if (source_face == 0 && source != 0) || (source_face == 1 && source + 1 != legs) {
                continue;
            }
            let normal = if partition.is_empty() {
                material[first_cell].piece.planes[source_face].normal
            } else if source_face == 0 {
                scale(normals[source], -1.)
            } else if source_face == 1 {
                normals[source + 1]
            } else {
                wall_normals[source_face - 2]
            };
            faces.push(Face {
                vertices: ring,
                normal,
                source,
                source_face,
            });
        }
    }
    Ok((Union::from_indexed_boundary(vertices, faces)?, material))
}

fn partition_section(
    section: &[[f64; 2]],
    clockwise: bool,
    tolerance_mm: f64,
) -> Result<Vec<[usize; 3]>, Error> {
    let count = section.len();
    let winding = if clockwise { -1. } else { 1. };
    let consecutive = (0..count).map(|i| [i, (i + 1) % count, (i + 2) % count]);
    let convex = consecutive.map(|indices| indices.map(|i| section[i])).all(
        |[[ax, ay], [bx, by], [cx, cy]]| {
            winding * ((bx - ax) * (cy - by) - (by - ay) * (cx - bx)) >= 0.
        },
    );
    if convex {
        // The single convex cell also proves its supporting halfspaces.
        return Ok(Vec::new());
    }

    triangulate_section(section, clockwise, tolerance_mm)
}

pub(super) fn triangulate_section(
    section: &[[f64; 2]],
    clockwise: bool,
    tolerance_mm: f64,
) -> Result<Vec<[usize; 3]>, Error> {
    certify_section(section, clockwise, tolerance_mm)?;
    let ring: Vec<_> = (0..section.len())
        .map(|i| {
            let edge = [i, (i + 1) % section.len()];
            if clockwise { [edge[1], edge[0]] } else { edge }
        })
        .collect();
    let points = section.iter().map(|&[x, y]| Point2::new(x, y)).collect();
    triangulate_region(points, &ring, 0)
        .map(|region| region.triangles)
        .map_err(|_| Error::Unresolved)
}

pub(super) fn certify_section(
    section: &[[f64; 2]],
    clockwise: bool,
    tolerance_mm: f64,
) -> Result<(), Error> {
    let edges = (0..section.len()).map(|i| {
        let [a, b] = [section[i], section[(i + 1) % section.len()]];
        let (a, b) = if clockwise { (b, a) } else { (a, b) };
        Edge::Line { a, b }
    });
    let certified = if clockwise {
        edges.rev().collect()
    } else {
        edges.collect()
    };
    Sweep::new(vec![certified], 1., tolerance_mm, tolerance_mm.next_up())
        .map(|_| ())
        .map_err(|error| match error {
            normal_sweep::Error::Limit => Error::Limit,
            _ => Error::Invalid,
        })
}

pub(super) fn triangle_rings(triangle: [usize; 3], low: usize, high: usize) -> Vec<Vec<usize>> {
    let [a, b, c] = triangle;
    vec![
        vec![low + c, low + b, low + a],
        vec![high + a, high + b, high + c],
        vec![low + a, low + b, high + b, high + a],
        vec![low + b, low + c, high + c, high + b],
        vec![low + c, low + a, high + a, high + c],
    ]
}

pub(super) fn triangle_sources([a, b, c]: [usize; 3], count: usize) -> Vec<ProfileFace> {
    let mut sources = vec![ProfileFace::StartCap, ProfileFace::EndCap];
    sources.extend([[a, b], [b, c], [c, a]].map(|[a, b]| {
        if (a + 1) % count == b {
            ProfileFace::Edge(a)
        } else if (b + 1) % count == a {
            ProfileFace::Edge(b)
        } else {
            ProfileFace::Partition
        }
    }));
    sources
}

fn validate_section_plane(ring: &[Point], normal: Point, tolerance: f64) -> Result<(), Error> {
    let plane = Plane {
        normal,
        point: ring[0],
    };
    if ring
        .iter()
        .any(|&point| plane.distance(point).abs() > tolerance)
    {
        return Err(Error::Unresolved);
    }
    Ok(())
}

fn prove_separation(
    a: &ConvexPiece,
    b: &ConvexPiece,
    tolerance_mm: f64,
    budget: &mut usize,
) -> Result<(), Error> {
    if !clipping::bounds_overlap(a.bounds, b.bounds, tolerance_mm) {
        return Ok(());
    }
    for (piece, other) in [(a, b), (b, a)] {
        // All miter cell vertices occur on its two cap faces.
        let caps = &other.faces[..2];
        let point_count = caps[0].len() + caps[1].len();
        charge(budget, piece.planes.len() * point_count)?;
        if piece.planes.iter().any(|plane| {
            caps.iter()
                .flatten()
                .all(|&point| plane.distance(point) > tolerance_mm)
        }) {
            return Ok(());
        }
    }
    Err(Error::Unresolved)
}

#[cfg(test)]
mod tests;

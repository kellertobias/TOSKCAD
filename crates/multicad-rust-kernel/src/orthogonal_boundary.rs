//! Consolidate conformal orthogonal surface tiles into analytic planar faces.

use crate::orthogonal::BoundaryQuad;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) mod box_void;

const MAX_QUADS: usize = 200_000;
type Point = [f64; 3];
type Edge = [usize; 2];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryError {
    Invalid,
    Unresolved,
    Limit,
}

#[derive(Debug)]
pub struct Boundary {
    pub faces: Vec<BoundaryFace>,
    /// Connected surface shells, including any internal cavity shells.
    pub shell_count: usize,
}

impl Boundary {
    /// The STEP profile supports one shell or proved separated rectangular cavities.
    pub fn supports_step(&self) -> bool {
        self.step_admission().unwrap_or(false)
    }

    pub(crate) fn step_admission(
        &self,
    ) -> std::result::Result<bool, crate::orthogonal::BooleanError> {
        use crate::orthogonal::BooleanError;
        box_void::validate_boundary_graph(self)?;
        if self.shell_count != 1 {
            match box_void::boundary_certificate(self) {
                Ok(_) => {}
                Err(BooleanError::Limit) => return Err(BooleanError::Limit),
                Err(_) => return Ok(false),
            }
        }
        Ok(self.faces.len() >= 4
            && self.faces.iter().all(|face| {
                face.rings.iter().map(Vec::len).sum::<usize>() <= crate::polygon::MAX_VERTICES
            }))
    }
}

#[derive(Debug)]
pub struct BoundaryFace {
    pub component: u32,
    /// The exterior wire followed by its holes. All wires follow the outward
    /// face orientation, so holes wind oppositely to the exterior.
    pub rings: Vec<Vec<Point>>,
    pub quad_indices: Vec<usize>,
    pub normal: Point,
}

type Result<T> = std::result::Result<T, BoundaryError>;

/// A coordinate identity with signed zeros normalized. Tolerances belong to
/// the solid Boolean classification, so this stage never welds nearby points.
pub(crate) fn point_key(point: Point) -> [u64; 3] {
    point.map(|x| if x == 0. { 0 } else { x.to_bits() })
}

#[derive(Clone, Copy)]
struct Tile {
    corners: [usize; 4],
    normal: Point,
    neighbors: [usize; 4],
}

#[derive(Clone, Copy)]
struct Use {
    quad: usize,
    side: usize,
    forward: bool,
}

fn ring_edges<T: Copy>(ring: &[T]) -> impl Iterator<Item = [T; 2]> + '_ {
    ring.iter()
        .copied()
        .zip(ring.iter().copied().cycle().skip(1))
        .map(|(a, b)| [a, b])
}

fn changed_axis(a: Point, b: Point) -> Result<usize> {
    let mut axes = (0..3).filter(|&i| a[i] != b[i]);
    let axis = axes.next().ok_or(BoundaryError::Invalid)?;
    if axes.next().is_some() {
        return Err(BoundaryError::Invalid);
    }
    Ok(axis)
}

fn tile_normal([a, b, c, d]: [Point; 4]) -> Result<Point> {
    let i = changed_axis(a, b)?;
    let j = changed_axis(a, d)?;
    if i == j {
        return Err(BoundaryError::Invalid);
    }
    let mut opposite = b;
    opposite[j] = d[j];
    if opposite != c {
        return Err(BoundaryError::Invalid);
    }
    let mut normal = [0.; 3];
    let forward = ((i + 1) % 3 == j) == ((b[i] > a[i]) == (d[j] > a[j]));
    normal[3 - i - j] = if forward { 1. } else { -1. };
    Ok(normal)
}

fn connect(tiles: &mut [Tile], first: Use, second: Use) -> Result<()> {
    if first.forward == second.forward || first.quad == second.quad {
        return Err(BoundaryError::Invalid);
    }
    tiles[first.quad].neighbors[first.side] = second.quad;
    tiles[second.quad].neighbors[second.side] = first.quad;
    Ok(())
}

fn index_tiles(quads: &[BoundaryQuad]) -> Result<(Vec<Point>, Vec<Tile>)> {
    let mut points = Vec::new();
    let mut vertices = BTreeMap::new();
    let mut uses = BTreeMap::<Edge, Option<Use>>::new();
    let mut tiles = Vec::with_capacity(quads.len());
    for (quad, source) in quads.iter().enumerate() {
        if source.corners.iter().flatten().any(|x| !x.is_finite()) {
            return Err(BoundaryError::Invalid);
        }
        let normal = tile_normal(source.corners)?;
        let corners = source.corners.map(|point| {
            *vertices
                .entry((source.component, point_key(point)))
                .or_insert_with(|| {
                    let index = points.len();
                    points.push(point.map(|x| if x == 0. { 0. } else { x }));
                    index
                })
        });
        tiles.push(Tile {
            corners,
            normal,
            neighbors: [usize::MAX; 4],
        });
        for (side, [a, b]) in ring_edges(&corners).enumerate() {
            let key = [a.min(b), a.max(b)];
            let next = Use {
                quad,
                side,
                forward: a < b,
            };
            match uses.entry(key) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(Some(next));
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let first = entry.get_mut().take().ok_or(BoundaryError::Invalid)?;
                    connect(&mut tiles, first, next)?;
                }
            }
        }
    }
    if uses.values().any(Option::is_some) {
        return Err(BoundaryError::Invalid);
    }
    Ok((points, tiles))
}

fn check_vertex_links(tiles: &[Tile], point_count: usize) -> Result<()> {
    // Each tile incident at a vertex has two neighbors around that vertex.
    // One cycle must visit all incident tiles, rather than several fans
    // meeting at a single nonmanifold vertex.
    let mut fans = vec![(usize::MAX, 0); point_count];
    for (index, tile) in tiles.iter().enumerate() {
        for vertex in tile.corners {
            fans[vertex] = (index, fans[vertex].1 + 1);
        }
    }
    for (vertex, (first, count)) in fans.into_iter().enumerate() {
        let mut current = first;
        let mut previous = usize::MAX;
        for visited in 1..=count {
            let tile = &tiles[current];
            let corner = tile
                .corners
                .iter()
                .position(|&x| x == vertex)
                .expect("connected edge shares vertex");
            let a = tile.neighbors[corner];
            let b = tile.neighbors[(corner + 3) % 4];
            if a == b {
                return Err(BoundaryError::Invalid);
            }
            let next = if a == previous { b } else { a };
            if (next == first) != (visited == count) {
                return Err(BoundaryError::Invalid);
            }
            previous = current;
            current = next;
        }
    }
    Ok(())
}

fn connected(seed: usize, tiles: &[Tile], seen: &mut [bool], coplanar: bool) -> Vec<usize> {
    let mut group = vec![seed];
    seen[seed] = true;
    let mut current = 0;
    while current < group.len() {
        for index in tiles[group[current]].neighbors {
            if !seen[index] && (!coplanar || tiles[index].normal == tiles[seed].normal) {
                seen[index] = true;
                group.push(index);
            }
        }
        current += 1;
    }
    group
}

fn count_shells(tiles: &[Tile]) -> usize {
    let mut seen = vec![false; tiles.len()];
    let mut count = 0;
    for seed in 0..tiles.len() {
        if !seen[seed] {
            connected(seed, tiles, &mut seen, false);
            count += 1;
        }
    }
    count
}

struct Region {
    quads: Vec<usize>,
    rings: Vec<Vec<usize>>,
    normal: Point,
}

fn trace_region(tiles: &[Tile], group: &[usize]) -> Option<Vec<Vec<usize>>> {
    let normal = tiles[group[0]].normal;
    let mut outgoing = BTreeMap::new();
    for &index in group {
        let tile = &tiles[index];
        for (side, [start, end]) in ring_edges(&tile.corners).enumerate() {
            if tiles[tile.neighbors[side]].normal != normal && outgoing.insert(start, end).is_some()
            {
                // A single STEP wire may not touch itself at a corner.
                return None;
            }
        }
    }
    let mut rings = Vec::new();
    while let Some((first, mut next)) = outgoing.pop_first() {
        let mut ring = vec![first];
        while next != first {
            ring.push(next);
            next = outgoing.remove(&next)?;
        }
        rings.push(ring);
    }
    Some(rings)
}

fn single_tile(tiles: &[Tile], quad: usize) -> Region {
    Region {
        quads: vec![quad],
        rings: vec![tiles[quad].corners.to_vec()],
        normal: tiles[quad].normal,
    }
}

fn regions(tiles: &[Tile]) -> Vec<Region> {
    let mut seen = vec![false; tiles.len()];
    let mut regions = Vec::new();
    for seed in 0..tiles.len() {
        if seen[seed] {
            continue;
        }
        let mut group = connected(seed, tiles, &mut seen, true);
        group.sort_unstable();
        match trace_region(tiles, &group) {
            Some(rings) => regions.push(Region {
                quads: group,
                rings,
                normal: tiles[seed].normal,
            }),
            // Keep exact conformal faces when a maximal plane would require a
            // self-touching wire. The underlying closed solid remains valid.
            None => {
                regions.extend(group.into_iter().map(|quad| single_tile(tiles, quad)));
            }
        }
    }
    regions
}

fn collinear(points: &[Point], [a, b, c]: [usize; 3]) -> bool {
    changed_axis(points[a], points[b]) == changed_axis(points[b], points[c])
}

fn simplify(regions: &mut [Region], points: &[Point]) {
    // Drop a collinear point only when the entire boundary uses the same two
    // incident segments. Removing a point from just one wire creates a T-junction.
    let mut neighbors = vec![BTreeSet::new(); points.len()];
    for ring in regions.iter().flat_map(|face| &face.rings) {
        for [a, b] in ring_edges(ring) {
            neighbors[a].insert(b);
            neighbors[b].insert(a);
        }
    }
    let removable = neighbors
        .iter()
        .enumerate()
        .map(|(b, adjacent)| {
            adjacent.len() == 2
                && collinear(
                    points,
                    [*adjacent.first().unwrap(), b, *adjacent.last().unwrap()],
                )
        })
        .collect::<Vec<_>>();
    for ring in regions.iter_mut().flat_map(|face| &mut face.rings) {
        ring.retain(|&point| !removable[point]);
    }
}

fn outer_ring(ring: &mut [usize], points: &[Point], normal: Point) -> bool {
    let axis = normal.iter().position(|&x| x != 0.).expect("axis normal");
    let u = (axis + 1) % 3;
    let v = (axis + 2) % 3;
    let corner = ring
        .iter()
        .enumerate()
        .min_by(|a, b| {
            let (a, b) = (*a.1, *b.1);
            points[a][u]
                .total_cmp(&points[b][u])
                .then(points[a][v].total_cmp(&points[b][v]))
        })
        .expect("nonempty region ring")
        .0;
    ring.rotate_left(corner);
    // At the lexicographically first corner, both incident segments point
    // into positive coordinates. The outgoing segment picks the winding.
    (points[ring[1]][u] > points[ring[0]][u]) == (normal[axis] > 0.)
}

fn face(mut region: Region, quads: &[BoundaryQuad], points: &[Point]) -> Result<BoundaryFace> {
    let mut outer = None;
    for (i, ring) in region.rings.iter_mut().enumerate() {
        if ring.len() < 4 {
            return Err(BoundaryError::Invalid);
        }
        if outer_ring(ring, points, region.normal) && outer.replace(i).is_some() {
            return Err(BoundaryError::Unresolved);
        }
    }
    let outer = outer.ok_or(BoundaryError::Unresolved)?;
    region.rings.swap(0, outer);
    Ok(BoundaryFace {
        component: quads[region.quads[0]].component,
        rings: region
            .rings
            .into_iter()
            .map(|ring| ring.into_iter().map(|i| points[i]).collect())
            .collect(),
        quad_indices: region.quads,
        normal: region.normal,
    })
}

/// Merge coplanar quads while retaining a common edge decomposition between
/// neighboring faces. Quads must describe a conformal, oriented grid shell.
pub fn build(quads: &[BoundaryQuad]) -> Result<Boundary> {
    if quads.is_empty() {
        return Err(BoundaryError::Invalid);
    }
    if quads.len() > MAX_QUADS {
        return Err(BoundaryError::Limit);
    }
    let (points, tiles) = index_tiles(quads)?;
    check_vertex_links(&tiles, points.len())?;
    let shell_count = count_shells(&tiles);
    let mut regions = regions(&tiles);
    simplify(&mut regions, &points);
    let faces = regions
        .into_iter()
        .map(|region| face(region, quads, &points))
        .collect::<Result<_>>()?;
    Ok(Boundary { faces, shell_count })
}

#[cfg(test)]
mod tests;

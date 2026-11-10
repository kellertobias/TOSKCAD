//! Regularized solid Boolean operations on conformal orthogonal box cells.
//! Coordinate compression uses the exact input plane coordinates. Distinct
//! planes inside the tolerance are refused rather than welded.

use crate::{Rational, polygon::PolygonPrism};
use std::collections::{BTreeMap, BTreeSet};

const MAX_GRID_POINTS: usize = 131_072;
pub const MAX_CELLS: usize = 65_536;
const MAX_QUADS: usize = 65_536;
const MAX_COMPONENTS: u32 = 1_024;
const UNVISITED: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Box3 {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

/// Four conformal corners in outward winding, with a zero-based solid index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundaryQuad {
    pub corners: [[f64; 3]; 4],
    pub component: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Union,
    Subtract,
    Intersect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BooleanError {
    Invalid,
    Unresolved,
    Limit,
}

#[derive(Debug)]
pub struct OrthogonalSolid {
    cells: Vec<Box3>,
    quads: Vec<BoundaryQuad>,
    bounds: Box3,
    volume_mm3: f64,
    solid_count: u32,
}

impl OrthogonalSolid {
    /// Nonoverlapping positive-volume cells, reusable as a Boolean operand.
    pub fn cells(&self) -> &[Box3] {
        &self.cells
    }

    pub fn quads(&self) -> &[BoundaryQuad] {
        &self.quads
    }

    pub fn bounds(&self) -> Box3 {
        self.bounds
    }

    pub fn volume_mm3(&self) -> f64 {
        self.volume_mm3
    }

    pub fn solid_count(&self) -> u32 {
        self.solid_count
    }
}

/// Decompose a validated rectilinear prism into disjoint full-height boxes.
/// Work stays in rational coordinates until each open band has been classified.
/// Nonrectilinear profiles and circular holes are outside this Boolean family.
pub fn prism_cells(
    prism: &PolygonPrism,
    reverse_sweep: bool,
) -> Result<Option<Vec<Box3>>, BooleanError> {
    if !prism.circular_holes().is_empty() {
        return Ok(None);
    }
    #[derive(Default)]
    struct BandChanges {
        start: Vec<Rational>,
        end: Vec<Rational>,
    }
    let mut bands = BTreeMap::<Rational, BandChanges>::new();
    let mut x_planes = BTreeSet::new();
    for ring in prism.rings() {
        for (&[ax, ay], &[bx, by]) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            x_planes.insert(ax);
            if ax == bx {
                bands.entry(ay.min(by)).or_default().start.push(ax);
                bands.entry(ay.max(by)).or_default().end.push(ax);
            } else if ay != by {
                return Ok(None);
            }
        }
    }
    if !x_planes
        .iter()
        .map(|x| x.as_f64())
        .is_sorted_by(|a, b| a < b)
        || !bands.keys().map(|y| y.as_f64()).is_sorted_by(|a, b| a < b)
    {
        return Err(BooleanError::Unresolved);
    }

    let height = prism.height().as_f64();
    let [bottom, top] = if reverse_sweep {
        [-height, 0.]
    } else {
        [0., height]
    };
    let mut active = BTreeSet::new();
    let mut cells = Vec::new();
    for ((&low_y, changes), &high_y) in bands.iter().zip(bands.keys().skip(1)) {
        // Close edges before opening their collinear successors.
        for x in &changes.end {
            active.remove(x);
        }
        active.extend(changes.start.iter().copied());
        if cells.len() + active.len() / 2 > MAX_CELLS {
            return Err(BooleanError::Limit);
        }
        let (low_y, high_y) = (low_y.as_f64(), high_y.as_f64());
        let mut crossings = active.iter();
        while let Some(low_x) = crossings.next() {
            let high_x = crossings
                .next()
                .expect("validated rings have paired band crossings");
            cells.push(Box3 {
                min: [low_x.as_f64(), low_y, bottom],
                max: [high_x.as_f64(), high_y, top],
            });
        }
    }
    Ok(Some(cells))
}

/// Treat each operand's boxes as a material union and discard zero-volume
/// intersections. Face-connected material forms one solid. Edge- and
/// point-touching components keep separate topology identities.
pub fn boolean(
    left: &[Box3],
    right: &[Box3],
    op: Operation,
    tolerance: f64,
) -> Result<Option<OrthogonalSolid>, BooleanError> {
    if !tolerance.is_finite() || tolerance < 0. {
        return Err(BooleanError::Invalid);
    }
    if left.len() > MAX_CELLS || right.len() > MAX_CELLS {
        return Err(BooleanError::Limit);
    }
    if left.is_empty() && right.is_empty() {
        return Ok(None);
    }
    let grid = Grid::new(left, right, tolerance)?;
    let lhs = grid.occupancy(left);
    let rhs = grid.occupancy(right);
    let mut material: Vec<_> = lhs
        .into_iter()
        .zip(rhs)
        .map(|(a, b)| {
            let filled = match op {
                Operation::Union => a > 0 || b > 0,
                Operation::Subtract => a > 0 && b == 0,
                Operation::Intersect => a > 0 && b > 0,
            };
            if filled { UNVISITED } else { 0 }
        })
        .collect();
    let solid_count = grid.label_components(&mut material)?;
    if solid_count == 0 {
        return Ok(None);
    }
    grid.boundary(&material, solid_count).map(Some)
}

/// Measure the common material of two unions without constructing result
/// faces. Coincident planes and touching cells contribute no volume.
pub fn intersection_volume(left: &[Box3], right: &[Box3]) -> Result<f64, BooleanError> {
    if left.len() > MAX_CELLS || right.len() > MAX_CELLS {
        return Err(BooleanError::Limit);
    }
    if left.is_empty() || right.is_empty() {
        return Ok(0.);
    }
    let grid = Grid::new(left, right, 0.)?;
    let left = grid.occupancy(left);
    let right = grid.occupancy(right);
    let mut volume = 0.;
    for (index, (a, b)) in left.into_iter().zip(right).enumerate() {
        if a > 0 && b > 0 {
            let at = grid.coordinates(index);
            let lo = grid.point(at);
            let hi = grid.point(at.map(|i| i + 1));
            volume += (0..3).map(|i| hi[i] - lo[i]).product::<f64>();
        }
    }
    if !volume.is_finite() {
        return Err(BooleanError::Invalid);
    }
    Ok(volume)
}

struct Grid {
    axes: [Vec<f64>; 3],
    dims: [usize; 3],
    stride: [usize; 3],
    points: usize,
}

fn canonical(value: f64) -> f64 {
    if value == 0. { 0. } else { value }
}

impl Grid {
    fn new(left: &[Box3], right: &[Box3], tolerance: f64) -> Result<Self, BooleanError> {
        let mut axes = std::array::from_fn(|_| Vec::new());
        for cell in left.iter().chain(right) {
            for (i, axis) in axes.iter_mut().enumerate() {
                let (lo, hi) = (cell.min[i], cell.max[i]);
                if !lo.is_finite() || !hi.is_finite() || lo >= hi {
                    return Err(BooleanError::Invalid);
                }
                axis.extend([canonical(lo), canonical(hi)]);
            }
        }
        for axis in &mut axes {
            axis.sort_unstable_by(f64::total_cmp);
            axis.dedup();
            if axis.windows(2).any(|pair| pair[1] - pair[0] < tolerance) {
                return Err(BooleanError::Unresolved);
            }
        }
        let dims = axes.each_ref().map(Vec::len);
        let points = dims
            .into_iter()
            .try_fold(1usize, |n, width| n.checked_mul(width))
            .filter(|&n| n <= MAX_GRID_POINTS)
            .ok_or(BooleanError::Limit)?;
        Ok(Self {
            axes,
            dims,
            stride: [1, dims[0], dims[0] * dims[1]],
            points,
        })
    }

    fn index(&self, [x, y, z]: [usize; 3]) -> usize {
        x + y * self.stride[1] + z * self.stride[2]
    }

    fn coordinates(&self, index: usize) -> [usize; 3] {
        [
            index % self.dims[0],
            (index / self.stride[1]) % self.dims[1],
            index / self.stride[2],
        ]
    }

    fn point(&self, at: [usize; 3]) -> [f64; 3] {
        std::array::from_fn(|axis| self.axes[axis][at[axis]])
    }

    fn plane(&self, axis: usize, value: f64) -> usize {
        let value = canonical(value);
        self.axes[axis]
            .binary_search_by(|plane| plane.total_cmp(&value))
            .expect("each operand plane was inserted into the grid")
    }

    fn occupancy(&self, boxes: &[Box3]) -> Vec<i32> {
        // Difference updates at eight corners and three prefix scans mark
        // overlapping boxes in O(boxes + grid points).
        let mut counts = vec![0; self.points];
        for cell in boxes {
            let lo: [usize; 3] = std::array::from_fn(|i| self.plane(i, cell.min[i]));
            let hi: [usize; 3] = std::array::from_fn(|i| self.plane(i, cell.max[i]));
            for bits in 0u8..8 {
                let point =
                    std::array::from_fn(|i| if bits & (1 << i) == 0 { lo[i] } else { hi[i] });
                let change = if bits.count_ones().is_multiple_of(2) {
                    1
                } else {
                    -1
                };
                counts[self.index(point)] += change;
            }
        }
        for (axis, stride) in self.stride.into_iter().enumerate() {
            for index in stride..self.points {
                if (index / stride) % self.dims[axis] != 0 {
                    counts[index] += counts[index - stride];
                }
            }
        }
        counts
    }

    fn neighbor(&self, index: usize, at: [usize; 3], axis: usize, positive: bool) -> Option<usize> {
        if positive {
            (at[axis] + 1 < self.dims[axis]).then_some(index + self.stride[axis])
        } else {
            (at[axis] > 0).then(|| index - self.stride[axis])
        }
    }

    fn label_components(&self, material: &mut [u32]) -> Result<u32, BooleanError> {
        let mut count = 0;
        let mut pending = Vec::new();
        for index in 0..self.points {
            if material[index] != UNVISITED {
                continue;
            }
            count += 1;
            if count > MAX_COMPONENTS {
                return Err(BooleanError::Limit);
            }
            material[index] = count;
            pending.push(index);
            while let Some(index) = pending.pop() {
                let at = self.coordinates(index);
                for axis in 0..3 {
                    for positive in [false, true] {
                        if let Some(next) = self.neighbor(index, at, axis, positive)
                            && material[next] == UNVISITED
                        {
                            material[next] = count;
                            pending.push(next);
                        }
                    }
                }
            }
        }
        Ok(count)
    }

    fn boundary(
        &self,
        material: &[u32],
        solid_count: u32,
    ) -> Result<OrthogonalSolid, BooleanError> {
        let mut cells = Vec::new();
        let mut quads = Vec::new();
        let mut links = BTreeMap::<(u32, usize), VertexLink>::new();
        let mut bounds = Box3 {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        };
        let mut volume_mm3 = 0.;
        for (index, &label) in material.iter().enumerate() {
            if label == 0 {
                continue;
            }
            if cells.len() == MAX_CELLS {
                return Err(BooleanError::Limit);
            }
            let at = self.coordinates(index);
            let min = self.point(at);
            let max = self.point(at.map(|i| i + 1));
            for i in 0..3 {
                bounds.min[i] = bounds.min[i].min(min[i]);
                bounds.max[i] = bounds.max[i].max(max[i]);
            }
            volume_mm3 += (0..3).map(|i| max[i] - min[i]).product::<f64>();
            cells.push(Box3 { min, max });
            for axis in 0..3 {
                for positive in [false, true] {
                    if self
                        .neighbor(index, at, axis, positive)
                        .is_some_and(|i| material[i] != 0)
                    {
                        continue;
                    }
                    if quads.len() == MAX_QUADS {
                        return Err(BooleanError::Limit);
                    }
                    let corners = quad(at, axis, positive);
                    for (i, &corner) in corners.iter().enumerate() {
                        let from = edge_direction(corner, corners[(i + 3) % 4]);
                        let to = edge_direction(corner, corners[(i + 1) % 4]);
                        links
                            .entry((label, self.index(corner)))
                            .or_default()
                            .insert(from, to)?;
                    }
                    quads.push(BoundaryQuad {
                        corners: corners.map(|p| self.point(p)),
                        component: label - 1,
                    });
                }
            }
        }
        if !volume_mm3.is_finite()
            || volume_mm3 <= 0.
            || links.values().any(|link| !link.is_cycle())
        {
            return Err(BooleanError::Unresolved);
        }
        Ok(OrthogonalSolid {
            cells,
            quads,
            bounds,
            volume_mm3,
            solid_count,
        })
    }
}

fn quad(mut cell: [usize; 3], axis: usize, positive: bool) -> [[usize; 3]; 4] {
    cell[axis] += usize::from(positive);
    let u = (axis + 1) % 3;
    let v = (axis + 2) % 3;
    let mut along_u = cell;
    let mut along_v = cell;
    let mut opposite = cell;
    along_u[u] += 1;
    along_v[v] += 1;
    opposite[u] += 1;
    opposite[v] += 1;
    if positive {
        [cell, along_u, opposite, along_v]
    } else {
        [cell, along_v, opposite, along_u]
    }
}

fn edge_direction(from: [usize; 3], to: [usize; 3]) -> usize {
    let axis = (0..3)
        .find(|&i| from[i] != to[i])
        .expect("nonzero grid edge");
    2 * axis + usize::from(to[axis] > from[axis])
}

#[derive(Default)]
struct VertexLink {
    next: [u8; 6],
    incoming: u8,
}

impl VertexLink {
    fn insert(&mut self, from: usize, to: usize) -> Result<(), BooleanError> {
        let mask = 1 << to;
        if self.next[from] != 0 || self.incoming & mask != 0 {
            return Err(BooleanError::Unresolved);
        }
        self.next[from] = mask;
        self.incoming |= mask;
        Ok(())
    }

    fn is_cycle(&self) -> bool {
        // In a manifold, the face corners around a vertex form one directed
        // cycle. Separate cycles indicate a pinch even if every edge has
        // exactly two incident faces.
        let first = 1u8 << self.incoming.trailing_zeros();
        let mut next = first;
        let mut seen = 0;
        while next != 0 && seen & next == 0 {
            seen |= next;
            next = self.next[next.trailing_zeros() as usize];
        }
        next == first && seen == self.incoming
    }
}

#[cfg(test)]
mod tests;

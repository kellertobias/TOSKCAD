//! Recover a rectilinear solid's local grid from its oriented STEP shell.

use super::*;
use crate::orthogonal::{BooleanError, OrientedFace, solid_from_oriented_faces};

#[cfg(test)]
mod tests;

struct Groups {
    parents: Vec<usize>,
    sizes: Vec<usize>,
}

impl Groups {
    fn new(count: usize) -> Self {
        Self {
            parents: (0..count).collect(),
            sizes: vec![1; count],
        }
    }

    fn root(&mut self, mut vertex: usize) -> usize {
        while self.parents[vertex] != vertex {
            self.parents[vertex] = self.parents[self.parents[vertex]];
            vertex = self.parents[vertex];
        }
        vertex
    }

    fn join(&mut self, a: usize, b: usize) {
        let (mut a, mut b) = (self.root(a), self.root(b));
        if a == b {
            return;
        }
        if self.sizes[a] < self.sizes[b] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parents[b] = a;
        self.sizes[a] += self.sizes[b];
    }

    fn project(&mut self, points: &mut [Point], axis: usize, roundoff: f64) -> Result<()> {
        let mut extents = vec![[f64::INFINITY, f64::NEG_INFINITY]; points.len()];
        for (index, point) in points.iter().enumerate() {
            let range = &mut extents[self.root(index)];
            *range = [range[0].min(point[axis]), range[1].max(point[axis])];
        }
        let mut classes: Vec<_> = extents
            .iter()
            .enumerate()
            .filter(|(_, range)| range[0].is_finite())
            .collect();
        classes.sort_unstable_by(|(_, a), (_, b)| compare(**a, **b));
        let mut representatives = vec![0.; points.len()];
        let mut start = 0;
        while let Some(&(_, &[min, mut max])) = classes.get(start) {
            if max - min > roundoff {
                return Err(ReadError::Unsupported);
            }
            let mut end = start + 1;
            while let Some(&(_, &[_, next_max])) = classes.get(end) {
                let end_max = max.max(next_max);
                if end_max - min > roundoff {
                    break;
                }
                max = end_max;
                end += 1;
            }
            let coordinate = zero(min.midpoint(max));
            for &(id, _) in &classes[start..end] {
                representatives[id] = coordinate;
            }
            start = end;
        }
        for (index, point) in points.iter_mut().enumerate() {
            point[axis] = representatives[self.root(index)];
        }
        Ok(())
    }
}

fn direction_sign(direction: Point) -> Point {
    if preferred_axis(direction) {
        direction.map(zero)
    } else {
        direction.map(|x| zero(-x))
    }
}

fn basis(segments: &[[usize; 2]], points: &[Point], roundoff: f64) -> Result<[Point; 3]> {
    let directions = || {
        segments
            .iter()
            .map(|&[a, b]| direction_sign(sub(points[b], points[a])))
    };
    let longest = |a: &Point, b: &Point| length(*a).total_cmp(&length(*b)).then(compare(*a, *b));
    // Use long, geometrically orthogonal edges so short trims far from
    // the world origin do not amplify serialization roundoff in the frame.
    let a = unit(directions().max_by(longest).ok_or(ReadError::Invalid)?)?;
    let b = directions()
        .filter(|&delta| dot(a, delta).abs() <= roundoff)
        .max_by(longest)
        .ok_or(ReadError::Unsupported)?;
    let c = unit(cross(a, b))?;
    let axes = [a, cross(c, a), c];
    let closest = |world_axis, exclude| {
        (0..3)
            .filter(|index| Some(*index) != exclude)
            .max_by(|&a, &b| {
                f64::abs(axes[a][world_axis])
                    .total_cmp(&f64::abs(axes[b][world_axis]))
                    .then(compare(axes[a], axes[b]))
            })
            .expect("three basis directions")
    };
    let z = closest(2, None);
    let x = closest(0, Some(z));
    let normal = axes[z].map(|value| zero(value * axes[z][2].signum()));
    let u = axes[x].map(|value| zero(value * axes[x][0].signum()));
    let v = unit(cross(normal, u))?.map(zero);
    Ok([cross(v, normal).map(zero), v, normal])
}

fn major(point: Point) -> usize {
    (0..3)
        .max_by(|&a, &b| point[a].abs().total_cmp(&point[b].abs()))
        .expect("three coordinates")
}

fn cardinal_coordinate(axis: Point) -> Option<usize> {
    if axis.iter().all(|&value| matches!(value, -1. | 0. | 1.)) {
        axis.iter().position(|&value| value != 0.)
    } else {
        None
    }
}

fn local_points(
    points: &[Point],
    segments: &[[usize; 2]],
    normals: &[(usize, f64)],
    successors: &[BTreeMap<usize, usize>],
    origin: Point,
    axes: [Point; 3],
    roundoff: f64,
) -> Result<Vec<Point>> {
    let mut local: Vec<_> = points
        .iter()
        .map(|&point| axes.map(|axis| dot(axis, sub(point, origin))))
        .collect();
    let mut groups: [Groups; 3] = std::array::from_fn(|_| Groups::new(points.len()));
    for &[a, b] in segments {
        let delta = sub(local[b], local[a]);
        let direction = major(delta);
        for (axis, group) in groups.iter_mut().enumerate() {
            if axis != direction {
                if delta[axis].abs() > roundoff {
                    return Err(ReadError::Unsupported);
                }
                group.join(a, b);
            }
        }
    }
    for (&(axis, _), successors) in normals.iter().zip(successors) {
        let mut vertices = successors.keys().copied();
        let first = vertices.next().ok_or(ReadError::Invalid)?;
        for vertex in vertices {
            groups[axis].join(first, vertex);
        }
    }
    // Edge and face connectivity proves coordinate equality. Independent
    // planes only share a rounded coordinate when their complete combined
    // span fits the projection roundoff bound, without transitive chaining.
    for (axis, group) in groups.iter_mut().enumerate() {
        let budget = if cardinal_coordinate(axes[axis]).is_some() {
            0.
        } else {
            roundoff
        };
        group.project(&mut local, axis, budget)?;
    }
    for (point, world) in local.iter().zip(points) {
        let placed = std::array::from_fn(|axis| {
            (0..3).fold(origin[axis], |sum, i| point[i].mul_add(axes[i][axis], sum))
        });
        if length(sub(placed, *world)) > 2. * roundoff {
            return Err(ReadError::Unsupported);
        }
    }
    Ok(local)
}

fn oriented_face(
    mut successors: BTreeMap<usize, usize>,
    points: &[Point],
    normal: Point,
) -> Result<OrientedFace> {
    let mut rings = Vec::new();
    while let Some((&start, _)) = successors.first_key_value() {
        let mut ring = Vec::new();
        let mut current = start;
        loop {
            ring.push(points[current]);
            current = successors.remove(&current).ok_or(ReadError::Invalid)?;
            if current == start {
                break;
            }
        }
        rings.push(ring);
    }
    // Preserve the face orientation already checked by the STEP reader.
    // The outer loop has positive area along the outward normal.
    let area = |ring: &[Point]| {
        let origin = ring[0];
        ring.iter()
            .zip(ring.iter().cycle().skip(1))
            .map(|(&a, &b)| dot(normal, cross(sub(a, origin), sub(b, origin))))
            .sum::<f64>()
    };
    rings.sort_by(|a, b| area(b).total_cmp(&area(a)));
    Ok(OrientedFace { normal, rings })
}

fn import_error(error: BooleanError) -> ReadError {
    match error {
        BooleanError::Invalid => ReadError::Invalid,
        BooleanError::Limit => ReadError::Limit,
        BooleanError::Unresolved => ReadError::Unsupported,
    }
}

impl Shell<'_, '_> {
    pub(super) fn orthogonal(&self, faces: &[Face]) -> Result<ImportSolid> {
        let indices: BTreeMap<_, _> = self
            .points
            .keys()
            .copied()
            .enumerate()
            .map(|(index, id)| (id, index))
            .collect();
        let points: Vec<_> = self.points.values().copied().collect();
        let mut origin_mm = points
            .iter()
            .copied()
            .min_by(|&a, &b| compare(a, b))
            .ok_or(ReadError::Invalid)?;
        let mut successors = vec![BTreeMap::new(); faces.len()];
        let mut segments = Vec::with_capacity(self.edges.len());
        for edge in self.edges.values() {
            if !matches!(edge.curve, Curve::Line) {
                return Err(ReadError::Unsupported);
            }
            let [a, b] = edge.vertices.map(|id| indices[&id]);
            segments.push([a, b]);
            for use_ in &edge.uses {
                let [a, b] = if use_.forward { [a, b] } else { [b, a] };
                if successors[use_.face].insert(a, b).is_some() {
                    return Err(ReadError::Invalid);
                }
            }
        }
        let scale = points.iter().flatten().map(|x| x.abs()).fold(1., f64::max);
        let roundoff = (32. * f64::EPSILON * scale).min(self.tolerance / 16.);
        let axes = basis(&segments, &points, roundoff)?;
        // Keep exact cardinal coordinates in their original range. Subtracting
        // a far-away anchor could erase a real small gap on an otherwise exact
        // axis before the grid could reject it.
        for axis in axes.into_iter().filter_map(cardinal_coordinate) {
            origin_mm[axis] = 0.;
        }
        let normals = faces
            .iter()
            .map(|face| {
                let Face::Planar { normal, .. } = face else {
                    return Err(ReadError::Unsupported);
                };
                let normal = axes.map(|axis| dot(axis, *normal));
                let axis = major(normal);
                if normal[axis].abs() < 1. - 1e-10 {
                    return Err(ReadError::Unsupported);
                }
                Ok((axis, normal[axis].signum()))
            })
            .collect::<Result<Vec<_>>>()?;
        let local = local_points(
            &points,
            &segments,
            &normals,
            &successors,
            origin_mm,
            axes,
            roundoff,
        )?;
        let oriented = normals
            .into_iter()
            .zip(successors)
            .map(|((axis, sign), successors)| {
                let normal = std::array::from_fn(|i| if i == axis { sign } else { 0. });
                oriented_face(successors, &local, normal)
            })
            .collect::<Result<Vec<_>>>()?;
        let solid = solid_from_oriented_faces(&oriented, self.tolerance).map_err(import_error)?;
        let bounds = solid.bounds();
        Ok(ImportSolid {
            origin_mm,
            u: axes[0],
            v: axes[1],
            height_mm: bounds.max[2] - bounds.min[2],
            profile: ImportProfile::Orthogonal { solid },
        })
    }
}

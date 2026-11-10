//! Finite closest pairs on planar boundary representations and their material.
use std::collections::BTreeMap;

type Point = [f64; 3];
type Segment = [Point; 2];

pub struct Face {
    pub point: Point,
    pub normal: Point,
    /// Straight edges of the complete trimmed face, including hole loops.
    pub boundary: Vec<Segment>,
}

#[derive(Default)]
pub struct Shape {
    pub points: Vec<Point>,
    pub edges: Vec<Segment>,
    pub faces: Vec<Face>,
    /// The closed B-rep encloses material. Selected subshapes are boundary only.
    pub solid: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistanceError {
    Invalid,
    Limit,
    Unresolved,
}

#[derive(Debug, PartialEq)]
pub struct Measurement {
    pub distance: f64,
    pub witnesses: Vec<[Point; 2]>,
    pub total_solutions: u32,
    pub inner_solution: bool,
}

const WORK_LIMIT: usize = 10_000_000;
const MAX_SOLUTIONS: usize = 100_000;
const MAX_WITNESSES: usize = 64;

pub fn minimum(
    left: &[Shape],
    right: &[Shape],
    tolerance: f64,
) -> Result<Measurement, DistanceError> {
    measure(left, right, tolerance, WORK_LIMIT)
}

fn measure(
    left: &[Shape],
    right: &[Shape],
    tolerance: f64,
    limit: usize,
) -> Result<Measurement, DistanceError> {
    if left.is_empty() || right.is_empty() || !(1e-7..=1e-3).contains(&tolerance) {
        return Err(DistanceError::Invalid);
    }
    let mut work = Work(limit);
    let prepare = |shapes: &[Shape], work: &mut Work| {
        shapes
            .iter()
            .map(|shape| prepare_bounds(shape, work))
            .collect::<Result<Vec<_>, DistanceError>>()
    };
    let a_bounds = prepare(left, &mut work)?;
    let b_bounds = prepare(right, &mut work)?;
    let mut closest = Closest {
        tolerance,
        distance: f64::INFINITY,
        witnesses: BTreeMap::new(),
        inner: false,
    };
    for (a, a_bounds) in left.iter().zip(a_bounds) {
        for (b, &b_bounds) in right.iter().zip(&b_bounds) {
            work.charge(1)?;
            if a_bounds.gap(b_bounds) > closest.distance + tolerance {
                continue;
            }
            compare(a, b, a_bounds, b_bounds, &mut closest, &mut work)?;
        }
    }
    let count = closest.witnesses.len();
    let mut witnesses: Vec<_> = closest.witnesses.into_values().collect();
    // Retain the actual minimum even when more equivalent pairs exist.
    witnesses.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(Measurement {
        distance: closest.distance,
        witnesses: witnesses
            .into_iter()
            .take(MAX_WITNESSES)
            .map(|(_, pair)| pair)
            .collect(),
        total_solutions: count as u32,
        inner_solution: closest.inner,
    })
}

#[derive(Clone, Copy)]
struct Bounds {
    min: Point,
    max: Point,
}

impl Bounds {
    fn contains(self, p: Point) -> bool {
        (0..3).all(|i| (self.min[i]..=self.max[i]).contains(&p[i]))
    }

    fn gap(self, other: Self) -> f64 {
        length(std::array::from_fn(|i| {
            (other.min[i] - self.max[i])
                .max(self.min[i] - other.max[i])
                .max(0.)
        }))
    }
}

fn valid_point(point: Point) -> bool {
    point
        .into_iter()
        .all(|value| value.is_finite() && value.abs() <= 1e9)
}

fn valid_edge([a, b]: Segment) -> bool {
    valid_point(a) && valid_point(b) && norm_squared(sub(b, a)) > 1e-20
}

fn prepare_bounds(shape: &Shape, work: &mut Work) -> Result<Bounds, DistanceError> {
    work.charge(shape.points.len())?;
    work.charge(shape.edges.len())?;
    work.charge(shape.faces.len())?;
    if shape.points.is_empty()
        || shape.points.iter().any(|&p| !valid_point(p))
        || shape.edges.iter().any(|&edge| !valid_edge(edge))
        || (shape.solid && (shape.faces.len() < 4 || shape.edges.len() < 6))
    {
        return Err(DistanceError::Invalid);
    }
    for face in &shape.faces {
        work.charge(face.boundary.len())?;
        if !valid_point(face.point)
            || (norm_squared(face.normal) - 1.).abs() > 1e-8
            || face.normal.iter().any(|value| !value.is_finite())
            || face.boundary.len() < 3
            || face.boundary.iter().any(|&edge| !valid_edge(edge))
        {
            return Err(DistanceError::Invalid);
        }
    }
    Ok(shape.points.iter().copied().fold(
        Bounds {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        },
        |bounds, point| Bounds {
            min: std::array::from_fn(|i| bounds.min[i].min(point[i])),
            max: std::array::from_fn(|i| bounds.max[i].max(point[i])),
        },
    ))
}

struct Work(usize);

impl Work {
    fn charge(&mut self, count: usize) -> Result<(), DistanceError> {
        self.0 = self.0.checked_sub(count).ok_or(DistanceError::Limit)?;
        Ok(())
    }
}

struct Closest {
    tolerance: f64,
    distance: f64,
    witnesses: BTreeMap<[i64; 6], (f64, [Point; 2])>,
    inner: bool,
}

impl Closest {
    fn add(&mut self, a: Point, b: Point) -> Result<(), DistanceError> {
        let distance = length(sub(b, a));
        if distance > self.distance + self.tolerance {
            return Ok(());
        }
        if distance < self.distance {
            self.distance = distance;
            self.witnesses
                .retain(|_, (d, _)| *d <= distance + self.tolerance);
        }
        let key = std::array::from_fn(|i| {
            let point = if i < 3 { a } else { b };
            (point[i % 3] / self.tolerance * 4.).round() as i64
        });
        let entry = self.witnesses.entry(key).or_insert((distance, [a, b]));
        if distance < entry.0 {
            *entry = (distance, [a, b]);
        }
        if self.witnesses.len() > MAX_SOLUTIONS {
            return Err(DistanceError::Limit);
        }
        Ok(())
    }
}

fn dot(a: Point, b: Point) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn cross(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[(i + 1) % 3] * b[(i + 2) % 3] - a[(i + 2) % 3] * b[(i + 1) % 3])
}

fn norm_squared(point: Point) -> f64 {
    dot(point, point)
}

fn length(point: Point) -> f64 {
    norm_squared(point).sqrt()
}

fn at(origin: Point, direction: Point, parameter: f64) -> Point {
    std::array::from_fn(|i| direction[i].mul_add(parameter, origin[i]))
}

fn nearest_point(point: Point, [a, b]: Segment) -> Point {
    let v = sub(b, a);
    at(
        a,
        v,
        (dot(sub(point, a), v) / norm_squared(v)).clamp(0., 1.),
    )
}

fn interior_pair([a, b]: Segment, [c, d]: Segment) -> Option<[Point; 2]> {
    let u = sub(b, a);
    let v = sub(d, c);
    let n = cross(u, v);
    let denominator = norm_squared(n);
    if denominator <= f64::EPSILON.powi(2) * norm_squared(u) * norm_squared(v) {
        return None;
    }
    let w = sub(c, a);
    let s = dot(cross(w, v), n) / denominator;
    let t = dot(cross(w, u), n) / denominator;
    ((0. ..=1.).contains(&s) && (0. ..=1.).contains(&t)).then(|| [at(a, u, s), at(c, v, t)])
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Location {
    Inside,
    Boundary,
    Outside,
}

impl Face {
    fn signed_distance(&self, p: Point) -> f64 {
        dot(sub(p, self.point), self.normal)
    }

    fn location(
        &self,
        p: Point,
        tolerance: f64,
        work: &mut Work,
    ) -> Result<Location, DistanceError> {
        work.charge(self.boundary.len())?;
        let drop = (0..3)
            .max_by(|&i, &j| self.normal[i].abs().total_cmp(&self.normal[j].abs()))
            .expect("three axes");
        let x = (drop + 1) % 3;
        let y = (drop + 2) % 3;
        let mut inside = false;
        for &[a, b] in &self.boundary {
            let near = [x, y]
                .into_iter()
                .all(|i| (a[i].min(b[i]) - tolerance..=a[i].max(b[i]) + tolerance).contains(&p[i]));
            if near && norm_squared(sub(p, nearest_point(p, [a, b]))) <= tolerance * tolerance {
                return Ok(Location::Boundary);
            }
            let ay = a[y] - p[y];
            let by = b[y] - p[y];
            if (ay > 0.) != (by > 0.) {
                let u = -ay / (by - ay);
                inside ^= (b[x] - a[x]).mul_add(u, a[x] - p[x]) > 0.;
            }
        }
        Ok(if inside {
            Location::Inside
        } else {
            Location::Outside
        })
    }

    fn crossing(
        &self,
        [a, b]: Segment,
        tolerance: f64,
        work: &mut Work,
    ) -> Result<Option<Point>, DistanceError> {
        let da = self.signed_distance(a);
        let db = self.signed_distance(b);
        if (da > 0. && db > 0.) || (da < 0. && db < 0.) || da == db {
            return Ok(None);
        }
        let hit = at(a, sub(b, a), da / (da - db));
        Ok((self.location(hit, tolerance, work)? == Location::Inside).then_some(hit))
    }
}

fn classify(
    point: Point,
    shape: &Shape,
    tolerance: f64,
    work: &mut Work,
) -> Result<Location, DistanceError> {
    for face in &shape.faces {
        work.charge(1)?;
        let distance = face.signed_distance(point);
        if distance.abs() <= tolerance
            && face.location(at(point, face.normal, -distance), tolerance, work)?
                != Location::Outside
        {
            return Ok(Location::Boundary);
        }
    }
    for ray in [
        [1., 0.371, 0.127],
        [0.229, 1., -0.513],
        [-0.631, 0.217, 1.],
        [1., -0.719, 0.421],
        [-0.319, 1., 0.815],
        [0.867, 0.649, 1.],
    ] {
        let mut inside = false;
        let mut reliable = true;
        for face in &shape.faces {
            work.charge(1)?;
            let denominator = dot(face.normal, ray);
            if denominator.abs() <= 8. * f64::EPSILON {
                continue;
            }
            let parameter = -face.signed_distance(point) / denominator;
            if parameter <= 0. {
                continue;
            }
            match face.location(at(point, ray, parameter), tolerance, work)? {
                Location::Inside => inside = !inside,
                Location::Outside => {}
                Location::Boundary => {
                    reliable = false;
                    break;
                }
            }
        }
        if reliable {
            return Ok(if inside {
                Location::Inside
            } else {
                Location::Outside
            });
        }
    }
    Err(DistanceError::Unresolved)
}

fn point_features(
    a: &Shape,
    b: &Shape,
    bounds: Bounds,
    reverse: bool,
    closest: &mut Closest,
    work: &mut Work,
) -> Result<(), DistanceError> {
    let tolerance = closest.tolerance;
    for &p in &a.points {
        let mut add = |q| {
            if reverse {
                closest.add(q, p)
            } else {
                closest.add(p, q)
            }
        };
        if b.solid && bounds.contains(p) && classify(p, b, tolerance, work)? == Location::Inside {
            add(p)?;
            closest.inner = true;
            continue;
        }
        if b.edges.is_empty() {
            if !reverse || !a.edges.is_empty() {
                work.charge(b.points.len())?;
                for &q in &b.points {
                    add(q)?;
                }
            }
        } else {
            work.charge(b.edges.len())?;
            for &edge in &b.edges {
                add(nearest_point(p, edge))?;
            }
        }
        for face in &b.faces {
            work.charge(1)?;
            let projected = at(p, face.normal, -face.signed_distance(p));
            if face.location(projected, tolerance, work)? == Location::Inside {
                add(projected)?;
            }
        }
    }
    Ok(())
}

fn crossing_features(
    a: &Shape,
    b: &Shape,
    closest: &mut Closest,
    work: &mut Work,
) -> Result<(), DistanceError> {
    for &edge in &a.edges {
        work.charge(b.faces.len())?;
        for face in &b.faces {
            if let Some(hit) = face.crossing(edge, closest.tolerance, work)? {
                closest.add(hit, hit)?;
            }
        }
    }
    Ok(())
}

fn compare(
    a: &Shape,
    b: &Shape,
    bounds_a: Bounds,
    bounds_b: Bounds,
    closest: &mut Closest,
    work: &mut Work,
) -> Result<(), DistanceError> {
    point_features(a, b, bounds_b, false, closest, work)?;
    point_features(b, a, bounds_a, true, closest, work)?;
    for &a in &a.edges {
        work.charge(b.edges.len())?;
        for &b in &b.edges {
            if let Some([p, q]) = interior_pair(a, b) {
                closest.add(p, q)?;
            }
        }
    }
    crossing_features(a, b, closest, work)?;
    crossing_features(b, a, closest, work)
}

#[cfg(test)]
mod tests;

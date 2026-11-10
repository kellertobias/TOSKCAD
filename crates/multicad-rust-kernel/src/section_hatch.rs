//! Material hatch strokes clipped to straight section boundaries.

use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::RangeInclusive,
};

type Point = [f64; 2];
type Segment = [Point; 2];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pattern {
    None,
    Lines,
    Cross,
    Alternating,
    Solid,
    Dots,
    SmallCrosses,
    Brick,
    Concrete,
    Grass,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub angle_rad: f64,
    pub spacing_mm: f64,
    pub line_width_mm: f64,
    pub tolerance_mm: f64,
    pub stroke_limit: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HatchError {
    Invalid,
    Limit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stroke {
    Line(Segment),
    Triangle([Point; 3]),
}

const MAX_BOUNDARY: usize = 200_000;
const MAX_CANDIDATES: usize = 60_000;
const MAX_WORK: usize = 20_000_000;
const MAX_LATTICE_INDEX: f64 = 20_000.;

/// Hatch a closed section with even-odd material, including disjoint islands,
/// holes, and islands within holes. Boundary endpoints must already coincide.
pub fn hatch(
    boundary: &[Segment],
    pattern: Pattern,
    options: Options,
) -> Result<Vec<Stroke>, HatchError> {
    hatch_with_budget(boundary, pattern, options, &mut Budget::default())
}

/// Consume a shared budget when publishing several section sources.
pub fn hatch_with_budget(
    boundary: &[Segment],
    pattern: Pattern,
    options: Options,
    budget: &mut Budget,
) -> Result<Vec<Stroke>, HatchError> {
    if !options.angle_rad.is_finite()
        || !(1e-5..=1e7).contains(&options.spacing_mm)
        || (matches!(pattern, Pattern::Alternating | Pattern::Dots)
            && (!options.line_width_mm.is_finite() || options.line_width_mm <= 0.))
        || !(1e-7..=1e-3).contains(&options.tolerance_mm)
    {
        return Err(HatchError::Invalid);
    }
    if pattern == Pattern::None || boundary.is_empty() {
        return Ok(Vec::new());
    }
    if boundary.len() > MAX_BOUNDARY {
        return Err(HatchError::Limit);
    }
    budget.charge(boundary.len())?;
    let cdt = triangulate_boundary(boundary, options.tolerance_mm)?;
    let mut output = Vec::new();
    if pattern == Pattern::Solid {
        fill(&cdt, &mut output, options.stroke_limit)?;
    } else {
        let (min, max) = bounds(boundary.iter().flatten().copied());
        let center = std::array::from_fn(|i| min[i] + (max[i] - min[i]) * 0.5);
        let (sin, cos) = options.angle_rad.sin_cos();
        let passes = [[[cos, sin], [-sin, cos]], [[-sin, cos], [-cos, -sin]]];
        for axes in passes
            .into_iter()
            .take(1 + usize::from(pattern == Pattern::Cross))
        {
            let local = boundary
                .iter()
                .map(|edge| edge.map(|point| axes.map(|axis| dot(sub(point, center), axis))))
                .collect::<Vec<_>>();
            let mut clipper = Clipper {
                boundary: &local,
                center,
                axes,
                tolerance: options.tolerance_mm,
                stroke_limit: options.stroke_limit,
                crossings: Vec::new(),
                spans: Vec::new(),
                output: &mut output,
                budget,
            };
            pattern_lines(&mut clipper, pattern, options)?;
        }
    }
    Ok(output)
}

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}

fn dot(a: Point, b: Point) -> f64 {
    a[0].mul_add(b[0], a[1] * b[1])
}

fn bounds(points: impl Iterator<Item = Point>) -> (Point, Point) {
    points.fold(
        ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
        |(min, max), point| {
            (
                std::array::from_fn(|i| min[i].min(point[i])),
                std::array::from_fn(|i| max[i].max(point[i])),
            )
        },
    )
}

type Triangulation2d = ConstrainedDelaunayTriangulation<Point2<f64>>;

fn triangulate_boundary(
    boundary: &[Segment],
    tolerance: f64,
) -> Result<Triangulation2d, HatchError> {
    let mut ids = BTreeMap::new();
    let mut points = Vec::new();
    let mut degree = Vec::new();
    let mut edges = BTreeSet::new();
    for &[a, b] in boundary {
        let delta = sub(a, b);
        if a.into_iter()
            .chain(b)
            .any(|x| !x.is_finite() || x.abs() > 1e8)
            || delta[0].hypot(delta[1]) <= tolerance
        {
            return Err(HatchError::Invalid);
        }
        let mut edge = [a, b].map(|point| {
            let key = point.map(|x| if x == 0. { 0 } else { x.to_bits() });
            let id = *ids.entry(key).or_insert_with(|| {
                let index = points.len();
                points.push(Point2::new(point[0], point[1]));
                degree.push(0usize);
                index
            });
            degree[id] += 1;
            id
        });
        edge.sort_unstable();
        if !edges.insert(edge) {
            return Err(HatchError::Invalid);
        }
    }
    if degree.iter().any(|n| !n.is_multiple_of(2)) {
        return Err(HatchError::Invalid);
    }
    let count = points.len();
    let edge_count = edges.len();
    let mut conflict = false;
    let cdt = Triangulation2d::try_bulk_load_cdt(points, edges.into_iter().collect(), |_| {
        conflict = true;
    })
    .map_err(|_| HatchError::Invalid)?;
    if conflict
        || cdt.num_vertices() != count
        || cdt.num_constraints() != edge_count
        || cdt.num_inner_faces() == 0
    {
        return Err(HatchError::Invalid);
    }
    Ok(cdt)
}

fn emit(output: &mut Vec<Stroke>, stroke: Stroke, limit: usize) -> Result<(), HatchError> {
    if output.len() >= limit {
        return Err(HatchError::Limit);
    }
    output.push(stroke);
    Ok(())
}

fn fill(cdt: &Triangulation2d, output: &mut Vec<Stroke>, limit: usize) -> Result<(), HatchError> {
    let mut material = vec![None; cdt.num_all_faces()];
    material[0] = Some(false);
    let mut work = Vec::new();
    for outer in cdt.convex_hull() {
        let face = outer.rev().face().as_inner().ok_or(HatchError::Invalid)?;
        let inside = outer.is_constraint_edge();
        if assign(&mut material[face.index()], inside)? {
            work.push((face, inside));
        }
    }
    // Every constraint toggles parity when crossed. Walking from the
    // exterior classifies nested islands without a point-in-ring scan per face.
    while let Some((face, inside)) = work.pop() {
        if inside {
            let points = face.vertices().map(|v| {
                let p = v.position();
                [p.x, p.y]
            });
            emit(output, Stroke::Triangle(points), limit)?;
        }
        for edge in face.adjacent_edges() {
            let neighbor = edge.rev().face();
            let next = inside ^ edge.is_constraint_edge();
            if assign(&mut material[neighbor.index()], next)?
                && let Some(neighbor) = neighbor.as_inner()
            {
                work.push((neighbor, next));
            }
        }
    }
    Ok(())
}

fn assign(state: &mut Option<bool>, value: bool) -> Result<bool, HatchError> {
    match *state {
        Some(previous) if previous != value => Err(HatchError::Invalid),
        Some(_) => Ok(false),
        None => {
            *state = Some(value);
            Ok(true)
        }
    }
}

fn lattice_range(min: f64, max: f64, spacing: f64) -> Result<RangeInclusive<i32>, HatchError> {
    let first = (min / spacing).floor() - 1.;
    let last = (max / spacing).ceil() + 1.;
    if first.abs().max(last.abs()) > MAX_LATTICE_INDEX {
        return Err(HatchError::Limit);
    }
    Ok(first as i32..=last as i32)
}

fn pattern_lines(
    clipper: &mut Clipper<'_>,
    pattern: Pattern,
    options: Options,
) -> Result<(), HatchError> {
    let (min, max) = bounds(clipper.boundary.iter().flatten().copied());
    let spacing = options.spacing_mm;
    let rows = lattice_range(min[1], max[1], spacing)?;
    let columns = match pattern {
        Pattern::None | Pattern::Solid | Pattern::Lines | Pattern::Cross => 0..=0,
        Pattern::Brick => lattice_range(min[0], max[0], 2. * spacing)?,
        _ => lattice_range(min[0], max[0], spacing)?,
    };
    let dot_half = (spacing * 0.045)
        .min(options.line_width_mm * 0.25)
        .max(1e-6);
    for i in rows {
        let y = f64::from(i) * spacing;
        let odd = i.rem_euclid(2) == 1;
        if matches!(pattern, Pattern::Lines | Pattern::Cross | Pattern::Brick)
            || pattern == Pattern::Alternating && !odd
        {
            clipper.line([[min[0], y], [max[0], y]])?;
        }
        match pattern {
            Pattern::Brick => {
                for j in columns.clone() {
                    let x = f64::from(2 * j + i32::from(odd)) * spacing;
                    clipper.line([[x, y], [x, y + spacing]])?;
                }
            }
            Pattern::Concrete | Pattern::Grass => {
                for j in columns.clone() {
                    let x = (f64::from(j) + if odd { 0.5 } else { 0. }) * spacing;
                    let point = |u: f64, v: f64| [x + u * spacing, y + v * spacing];
                    if pattern == Pattern::Grass {
                        let root = point(0., 0.);
                        for tip in [point(-0.2, 0.23), point(0., 0.31), point(0.2, 0.23)] {
                            clipper.line([root, tip])?;
                        }
                    } else if (i + j).rem_euclid(3) == 0 {
                        let [a, b, c] = [point(-0.18, -0.1), point(0.14, -0.06), point(0.04, 0.17)];
                        for edge in [[a, b], [b, c], [c, a]] {
                            clipper.line(edge)?;
                        }
                    } else {
                        clipper.line([point(-0.035, -0.04), point(0.035, 0.04)])?;
                    }
                }
            }
            Pattern::Dots | Pattern::SmallCrosses | Pattern::Alternating
                if pattern != Pattern::Alternating || odd =>
            {
                let cross = pattern == Pattern::SmallCrosses;
                let half = if cross { spacing * 0.17 } else { dot_half };
                for j in columns.clone() {
                    let x = f64::from(j) * spacing;
                    clipper.line([[x - half, y], [x + half, y]])?;
                    if cross {
                        clipper.line([[x, y - half], [x, y + half]])?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub struct Budget {
    candidates: usize,
    work: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            candidates: MAX_CANDIDATES,
            work: MAX_WORK,
        }
    }
}

impl Budget {
    fn charge(&mut self, work: usize) -> Result<(), HatchError> {
        self.work = self.work.checked_sub(work).ok_or(HatchError::Limit)?;
        Ok(())
    }
}

struct Clipper<'a> {
    boundary: &'a [Segment],
    center: Point,
    axes: [Point; 2],
    tolerance: f64,
    stroke_limit: usize,
    crossings: Vec<f64>,
    spans: Vec<[f64; 2]>,
    output: &'a mut Vec<Stroke>,
    budget: &'a mut Budget,
}

impl Clipper<'_> {
    fn line(&mut self, [a, b]: Segment) -> Result<(), HatchError> {
        self.budget.candidates = self
            .budget
            .candidates
            .checked_sub(1)
            .ok_or(HatchError::Limit)?;
        let delta = sub(b, a);
        let length = delta[0].hypot(delta[1]);
        if length <= self.tolerance {
            return Ok(());
        }
        self.budget.charge(self.boundary.len())?;
        let along = delta.map(|x| x / length);
        let normal = [-along[1], along[0]];
        let coords = |point| {
            let relative = sub(point, a);
            let offset = dot(relative, normal);
            [
                dot(relative, along),
                if offset.abs() <= self.tolerance {
                    0.
                } else {
                    offset
                },
            ]
        };
        self.crossings.clear();
        self.spans.clear();
        for edge in self.boundary {
            let [[x0, y0], [x1, y1]] = edge.map(coords);
            if (y0 > 0.) != (y1 > 0.) {
                let t = -y0 / (y1 - y0);
                self.crossings.push((x1 - x0).mul_add(t, x0));
            }
            if y0 == 0. && y1 == 0. {
                self.spans.push([x0.min(x1), x0.max(x1)]);
            }
        }
        self.crossings.sort_unstable_by(f64::total_cmp);
        // Closed even-degree contours cross a half-open line an even
        // number of times, including paired hits at tangent vertices.
        let (pairs, _) = self.crossings.as_chunks::<2>();
        self.spans.extend_from_slice(pairs);
        self.spans.sort_unstable_by(|a, b| a[0].total_cmp(&b[0]));
        let mut pending = [0., 0.];
        for span in self.spans.iter().copied() {
            let [start, end] = [span[0].max(0.), span[1].min(length)];
            if end - start <= self.tolerance {
                continue;
            }
            if start > pending[1] {
                if let Some(stroke) = self.stroke(a, along, pending) {
                    emit(self.output, stroke, self.stroke_limit)?;
                }
                pending = [start, end];
            } else {
                pending[1] = pending[1].max(end);
            }
        }
        if let Some(stroke) = self.stroke(a, along, pending) {
            emit(self.output, stroke, self.stroke_limit)?;
        }
        Ok(())
    }

    fn stroke(&self, a: Point, along: Point, span: [f64; 2]) -> Option<Stroke> {
        if span[1] - span[0] <= self.tolerance {
            return None;
        }
        let endpoints = span.map(|t| {
            let [u, v] = std::array::from_fn(|i| along[i].mul_add(t, a[i]));
            std::array::from_fn(|i| {
                self.axes[1][i].mul_add(v, self.axes[0][i].mul_add(u, self.center[i]))
            })
        });
        Some(Stroke::Line(endpoints))
    }
}

#[cfg(test)]
mod tests;

//! Orthographic visibility from straight B-rep edges and trimmed planar faces.

use crate::round_occlusion::RoundOccluder;

mod loft_faces;
mod profile;
mod scene;
pub(crate) use loft_faces::prepare_loft_faces;
use profile::ProfileTrim;
pub use scene::{
    AnalyticFace, CircularOpeningEnd, ContourCandidate, DepthRange, FaceOpening, FaceTrim,
    HiddenCandidate, LineCandidate, RimCandidate, RoundSegment, SceneExtras, SceneProjection,
    SpatialEllipseCandidate, SpatialRimCandidate, StadiumOpening, project_scene,
    project_scene_with_analytic_faces, project_scene_with_candidates,
};

type Point = [f64; 3];
type Point2 = [f64; 2];
type Span = [f64; 2];

/// Closed, oriented polyhedral geometry in drawing (u, v, depth) coordinates.
/// The viewer looks along negative depth. No tessellation is involved.
pub struct Body {
    pub edges: Vec<[Point; 2]>,
    pub faces: Vec<Face>,
}

pub struct Face {
    pub point: Point,
    pub normal: Point,
    /// Edge indices in all closed boundary loops, including holes.
    pub edges: Vec<usize>,
}

pub struct Options {
    pub tolerance: f64,
    pub hidden_lines: bool,
    pub tangent_edges: bool,
    pub detail: Option<(Point2, f64)>,
    pub segment_limit: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionError {
    Invalid,
    Limit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub source: usize,
    pub hidden: bool,
    pub ends: [Point2; 2],
}

const WORK_LIMIT: usize = 20_000_000;

pub fn project(bodies: &[Body], options: Options) -> Result<Vec<Segment>, ProjectionError> {
    project_with_rounds(bodies, &[], options)
}

/// Project planar body edges against both planar faces and bounded round
/// occluders. Round occluders contribute no edges of their own.
pub fn project_with_rounds(
    bodies: &[Body],
    rounds: &[RoundOccluder],
    options: Options,
) -> Result<Vec<Segment>, ProjectionError> {
    project_bounded(bodies, rounds, options, WORK_LIMIT)
}

fn dot<const N: usize>(a: [f64; N], b: [f64; N]) -> f64 {
    a.into_iter().zip(b).map(|(x, y)| x * y).sum()
}

fn sub<const N: usize>(a: [f64; N], b: [f64; N]) -> [f64; N] {
    std::array::from_fn(|i| a[i] - b[i])
}

fn planar([u, v, _]: Point) -> Point2 {
    [u, v]
}

#[derive(Clone, Copy)]
struct Bounds {
    min: Point,
    max: Point,
}

impl Bounds {
    fn point(point: Point) -> Self {
        Self {
            min: point,
            max: point,
        }
    }

    fn union(self, other: Self) -> Self {
        Self {
            min: std::array::from_fn(|i| self.min[i].min(other.min[i])),
            max: std::array::from_fn(|i| self.max[i].max(other.max[i])),
        }
    }

    fn overlaps(self, line: Self, tolerance: f64) -> bool {
        self.max[2] > line.min[2] + tolerance
            && (0..2).all(|i| {
                self.max[i] + tolerance >= line.min[i] && self.min[i] - tolerance <= line.max[i]
            })
    }

    fn overlaps_paper(self, other: Self, tolerance: f64) -> bool {
        (0..2).all(|i| {
            self.max[i] + tolerance >= other.min[i] && self.min[i] <= other.max[i] + tolerance
        })
    }
}

pub(crate) struct TrimmedPlane {
    id: usize,
    pub(crate) point: Point,
    pub(crate) normal: Point,
    boundary: Vec<[Point2; 2]>,
    profiles: Vec<ProfileTrim>,
    bounds: Bounds,
}

impl TrimmedPlane {
    pub(crate) fn paper_bounds(&self) -> [Point2; 2] {
        [self.bounds.min, self.bounds.max].map(planar)
    }
}

mod paper_index;
pub(crate) use paper_index::PaperIndex;

struct Line {
    start: Point,
    delta: Point,
    direction: Point2,
    length: f64,
    adjacent: [usize; 2],
}

impl Line {
    fn new([a, b]: [Point; 2], tolerance: f64, adjacent: [usize; 2]) -> Option<Self> {
        let delta = sub(b, a);
        let length = delta[0].hypot(delta[1]);
        (length > tolerance).then(|| Self {
            start: a,
            delta,
            direction: [delta[0] / length, delta[1] / length],
            length,
            adjacent,
        })
    }

    fn point(&self, along: f64) -> Point {
        let t = along / self.length;
        std::array::from_fn(|i| self.delta[i].mul_add(t, self.start[i]))
    }

    fn coordinates(&self, point: Point2) -> Point2 {
        let offset = sub(point, planar(self.start));
        let [x, y] = self.direction;
        [
            dot(offset, self.direction),
            x.mul_add(offset[1], -y * offset[0]),
        ]
    }

    fn span(&self, detail: Option<(Point2, f64)>) -> Span {
        if let Some((center, radius)) = detail {
            let [along, offset] = self.coordinates(center);
            let offset = offset.abs();
            if offset >= radius {
                return [0.; 2];
            }
            let half_width = ((radius - offset) * (radius + offset)).sqrt();
            return [
                (along - half_width).max(0.),
                (along + half_width).min(self.length),
            ];
        }
        [0., self.length]
    }
}

impl TrimmedPlane {
    fn hide(&self, line: &Line, mut span: Span, work: &mut Work) -> Result<(), ProjectionError> {
        // Ray/plane depth difference is affine along the line. Multiplication
        // by normal.z keeps this test stable for grazing faces.
        let ends = span.map(|s| line.point(s));
        let depths = ends.map(|end| dot(self.normal, sub(self.point, end)));
        // Resolve coplanarity conservatively when the view is almost edge-on.
        let roundoff = (0..3)
            .map(|i| {
                self.normal[i].abs()
                    * (self.point[i].abs() + ends[0][i].abs().max(ends[1][i].abs()))
            })
            .sum::<f64>()
            * (16. * f64::EPSILON);
        let threshold = (self.normal[2] * work.tolerance).max(roundoff);
        if depths.into_iter().fold(f64::NEG_INFINITY, f64::max) <= threshold {
            return Ok(());
        }
        let [a, b] = depths;
        if a <= threshold || b <= threshold {
            let root = (span[1] - span[0]).mul_add((threshold - a) / (b - a), span[0]);
            span[usize::from(a > threshold)] = root;
        }
        work.charge(self.boundary.len())?;
        work.crossings.clear();
        for edge in &self.boundary {
            let [[u0, v0], [u1, v1]] = edge.map(|point| line.coordinates(point));
            let [v0, v1] = [v0, v1].map(|v| if v.abs() <= work.tolerance { 0. } else { v });
            if (v0 > 0.) != (v1 > 0.) {
                work.crossings.push((u1 - u0).mul_add(-v0 / (v1 - v0), u0));
            }
            // Projected boundaries also hide rear edges lying directly along
            // a silhouette. A single vertex contact does not hide any length.
            if v0 == 0. && v1 == 0. {
                work.add_hidden([u0.min(u1), u0.max(u1)], span);
            }
        }
        for &profile in &self.profiles {
            work.charge(profile.line_work())?;
            if let Some(interval) = profile.line_span(line, !self.boundary.is_empty()) {
                work.crossings.extend(interval);
            }
        }
        work.crossings.sort_unstable_by(f64::total_cmp);
        let (intervals, remainder) = work.crossings.as_chunks::<2>();
        if !remainder.is_empty() {
            return Err(ProjectionError::Invalid);
        }
        for &[a, b] in intervals {
            let inside = [a.max(span[0]), b.min(span[1])];
            if inside[1] > inside[0] {
                work.hidden.push(inside);
            }
        }
        Ok(())
    }
}

trait BoundedFace {
    fn bounds(&self) -> Bounds;
}

impl BoundedFace for TrimmedPlane {
    fn bounds(&self) -> Bounds {
        self.bounds
    }
}

enum Children<T> {
    Leaves(Vec<T>),
    Branches([Box<Node<T>>; 2]),
}

struct Node<T = TrimmedPlane> {
    bounds: Bounds,
    children: Children<T>,
}

impl<T: BoundedFace> Node<T> {
    fn build(mut faces: Vec<T>) -> Option<Self> {
        let bounds = faces
            .iter()
            .map(BoundedFace::bounds)
            .reduce(Bounds::union)?;
        if faces.len() <= 8 {
            return Some(Self {
                bounds,
                children: Children::Leaves(faces),
            });
        }
        let extent = sub(bounds.max, bounds.min);
        let axis = usize::from(extent[1] > extent[0]);
        let center = |face: &T| {
            let bounds = face.bounds();
            bounds.min[axis] + bounds.max[axis]
        };
        let mid = faces.len() / 2;
        faces.select_nth_unstable_by(mid, |a, b| center(a).total_cmp(&center(b)));
        let right = faces.split_off(mid);
        let children = Children::Branches([faces, right].map(|faces| {
            Box::new(Self::build(faces).expect("split produces two nonempty groups"))
        }));
        Some(Self { bounds, children })
    }

    fn visit(
        &self,
        bounds: Bounds,
        work: &mut Work,
        face_visitor: &mut impl FnMut(&T, &mut Work) -> Result<(), ProjectionError>,
    ) -> Result<(), ProjectionError> {
        work.charge(1)?;
        if !self.bounds.overlaps(bounds, work.tolerance) {
            return Ok(());
        }
        match &self.children {
            Children::Leaves(faces) => {
                work.charge(faces.len())?;
                for face in faces {
                    if face.bounds().overlaps(bounds, work.tolerance) {
                        face_visitor(face, work)?;
                    }
                }
            }
            Children::Branches(nodes) => {
                for node in nodes {
                    node.visit(bounds, work, face_visitor)?;
                }
            }
        }
        Ok(())
    }
}

impl Node {
    fn hide(
        &self,
        line: &Line,
        span: Span,
        bounds: Bounds,
        work: &mut Work,
    ) -> Result<(), ProjectionError> {
        self.visit(bounds, work, &mut |face, work| {
            if !line.adjacent.contains(&face.id) {
                face.hide(line, span, work)?;
            }
            Ok(())
        })
    }
}

struct Work {
    remaining: usize,
    tolerance: f64,
    crossings: Vec<f64>,
    hidden: Vec<Span>,
}

impl Work {
    fn charge(&mut self, cost: usize) -> Result<(), ProjectionError> {
        self.remaining = self
            .remaining
            .checked_sub(cost)
            .ok_or(ProjectionError::Limit)?;
        Ok(())
    }

    fn add_hidden(&mut self, [a, b]: Span, [start, end]: Span) {
        let inside = [a.max(start), b.min(end)];
        if inside[1] > inside[0] {
            self.hidden.push(inside);
        }
    }
}

struct EdgeUse {
    adjacent: [usize; 2],
    tangent: bool,
}

fn prepare<'a>(
    bodies: impl IntoIterator<Item = &'a Body>,
    work: &mut Work,
) -> Result<(Vec<TrimmedPlane>, Vec<Vec<EdgeUse>>), ProjectionError> {
    let bodies = bodies.into_iter();
    let mut planes = Vec::new();
    let mut edge_uses = Vec::with_capacity(bodies.size_hint().0);
    let mut face_id = 0;
    for body in bodies {
        work.charge(body.edges.len())?;
        work.charge(body.faces.len())?;
        // Section clipping retains an empty slot for a removed source so that
        // each surviving vector still carries the original source index.
        if body.edges.is_empty() && body.faces.is_empty() {
            edge_uses.push(Vec::new());
            continue;
        }
        if body.edges.is_empty()
            || body.faces.is_empty()
            || body
                .edges
                .iter()
                .flatten()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > 1e8)
        {
            return Err(ProjectionError::Invalid);
        }
        let mut incidence = vec![[None; 2]; body.edges.len()];
        for face in &body.faces {
            face_id += 1;
            work.charge(face.edges.len())?;
            if face.edges.len() < 3
                || face.point.iter().any(|v| !v.is_finite() || v.abs() > 1e8)
                || !(1. - 1e-9..=1. + 1e-9).contains(&dot(face.normal, face.normal))
            {
                return Err(ProjectionError::Invalid);
            }
            let mut bounds: Option<Bounds> = None;
            let mut boundary = Vec::with_capacity(face.edges.len());
            for &edge in &face.edges {
                let used = incidence.get_mut(edge).ok_or(ProjectionError::Invalid)?;
                let normal = used
                    .iter_mut()
                    .find(|slot| slot.is_none())
                    .ok_or(ProjectionError::Invalid)?;
                *normal = Some((face_id, face.normal));
                for point in body.edges[edge] {
                    let point_bounds = Bounds::point(point);
                    bounds =
                        Some(bounds.map_or(point_bounds, |previous| previous.union(point_bounds)));
                }
                boundary.push(body.edges[edge].map(planar));
            }
            // An edge-on plane has no screen area. Model coordinates are
            // bounded, so the angular roundoff here lies within the tolerance.
            if face.normal[2].abs() > 16. * f64::EPSILON {
                planes.push(TrimmedPlane {
                    id: face_id,
                    point: face.point,
                    normal: face.normal.map(|n| n * face.normal[2].signum()),
                    boundary,
                    profiles: Vec::new(),
                    bounds: bounds.expect("face has edges"),
                });
            }
        }
        edge_uses.push(
            incidence
                .into_iter()
                .map(|ends| {
                    let [Some((a, normal_a)), Some((b, normal_b))] = ends else {
                        return Err(ProjectionError::Invalid);
                    };
                    if a == b {
                        return Err(ProjectionError::Invalid);
                    }
                    Ok(EdgeUse {
                        adjacent: [a, b],
                        tangent: dot(normal_a, normal_b).abs() >= 1. - 16. * f64::EPSILON,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    Ok((planes, edge_uses))
}

fn project_bounded(
    bodies: &[Body],
    rounds: &[RoundOccluder],
    options: Options,
    limit: usize,
) -> Result<Vec<Segment>, ProjectionError> {
    if bodies.is_empty() {
        return Err(ProjectionError::Invalid);
    }
    let mut work = projection_work(&options, limit)?;
    let (planes, edge_uses) = prepare(bodies, &mut work)?;
    let index = Node::build(planes).ok_or(ProjectionError::Invalid)?;
    project_straight(
        bodies,
        &edge_uses,
        Blockers {
            planes: Some(&index),
            analytic: None,
            rounds,
        },
        &options,
        &mut work,
    )
}

fn projection_work(options: &Options, limit: usize) -> Result<Work, ProjectionError> {
    if !(1e-7..=1e-3).contains(&options.tolerance)
        || options.detail.is_some_and(|(center, radius)| {
            center.iter().any(|v| !v.is_finite() || v.abs() > 1e8)
                || !(1e-5..=1e8).contains(&radius)
        })
    {
        return Err(ProjectionError::Invalid);
    }
    Ok(Work {
        remaining: limit,
        tolerance: options.tolerance,
        crossings: Vec::new(),
        hidden: Vec::new(),
    })
}

fn project_straight(
    bodies: &[Body],
    edge_uses: &[Vec<EdgeUse>],
    blockers: Blockers<'_>,
    options: &Options,
    work: &mut Work,
) -> Result<Vec<Segment>, ProjectionError> {
    let mut output = Vec::new();
    for (source, (body, uses)) in bodies.iter().zip(edge_uses).enumerate() {
        for (&edge, edge_use) in body.edges.iter().zip(uses) {
            if edge_use.tangent && !options.tangent_edges {
                continue;
            }
            let Some(line) = Line::new(edge, options.tolerance, edge_use.adjacent) else {
                continue;
            };
            let span = line.span(options.detail);
            if span[1] - span[0] <= options.tolerance {
                continue;
            }
            hide_line(&line, span, blockers, None, None, work)?;
            publish_spans(
                &mut work.hidden,
                span,
                options.tolerance,
                |hidden, [a, b]| {
                    if !hidden || options.hidden_lines {
                        if output.len() >= options.segment_limit {
                            return Err(ProjectionError::Limit);
                        }
                        output.push(Segment {
                            source,
                            hidden,
                            ends: [a, b].map(|along| planar(line.point(along))),
                        });
                    }
                    Ok(())
                },
            )?;
        }
    }
    Ok(output)
}

#[derive(Clone, Copy)]
struct Blockers<'a> {
    planes: Option<&'a Node>,
    analytic: Option<&'a scene::AnalyticIndex<'a>>,
    rounds: &'a [RoundOccluder],
}

fn hide_line(
    line: &Line,
    span: Span,
    blockers: Blockers<'_>,
    source: Option<usize>,
    own_face: Option<&AnalyticFace>,
    work: &mut Work,
) -> Result<(), ProjectionError> {
    work.hidden.clear();
    let points = span.map(|s| line.point(s));
    let [a, b] = points.map(Bounds::point);
    let bounds = a.union(b);
    if let Some(index) = blockers.planes {
        index.hide(line, span, bounds, work)?;
    }
    if let Some(index) = blockers.analytic {
        index.visit(bounds, work, &mut |face, work| {
            if own_face.is_some_and(|own_face| std::ptr::eq(own_face, *face)) {
                return Ok(());
            }
            face.hide_line(line, span, work)
        })?;
    }
    for (round_id, round) in blockers.rounds.iter().enumerate() {
        work.charge(1)?;
        if Some(round_id) != source && round.may_hide(points, work.tolerance) {
            work.charge(20)?;
            let contour = round.closes_contour_of(source.map(|id| &blockers.rounds[id]));
            round.hide(points, work.tolerance, contour, |interval| {
                let along = interval.map(|t| (span[1] - span[0]).mul_add(t, span[0]));
                work.hidden.push(along);
            });
        }
    }
    Ok(())
}

fn publish_spans(
    hidden: &mut [Span],
    span: Span,
    tolerance: f64,
    mut emit: impl FnMut(bool, Span) -> Result<(), ProjectionError>,
) -> Result<(), ProjectionError> {
    hidden.sort_unstable_by(|a, b| a[0].total_cmp(&b[0]));
    let mut publish = |hidden, a, b| {
        if b - a > tolerance {
            emit(hidden, [a, b])?;
        }
        Ok(())
    };
    let mut start = span[0];
    let mut covered = span[0];
    for &[a, b] in hidden.iter() {
        if a > covered + tolerance {
            publish(true, start, covered)?;
            publish(false, covered, a)?;
            start = a;
        }
        covered = covered.max(b);
    }
    publish(true, start, covered)?;
    publish(false, covered, span[1])
}

#[cfg(test)]
pub(crate) mod tests;

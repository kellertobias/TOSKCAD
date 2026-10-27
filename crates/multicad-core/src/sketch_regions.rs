//! Source-neutral closed profiles from authored planar curves. No tessellation,
//! solver handles or automatically repaired geometry participate in extraction.
use crate::{
    constraint_sketch::{CurveKind, source::Source},
    document::EntityId,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, f64::consts::TAU};

pub const DEFAULT_TOLERANCE_MM: f64 = 1e-7;
type Point = [f64; 2];

/// Stable selection intent: authored curve identities, independent of loop
/// starting point, traversal direction, source order and current dimensions.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundaryIntent {
    pub curves: BTreeSet<EntityId>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExactCurve {
    Line {
        a: Point,
        b: Point,
    },
    Arc {
        center: Point,
        a: Point,
        b: Point,
        clockwise: bool,
    },
    Circle {
        center: Point,
        radius_mm: f64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub authored: EntityId,
    pub reversed: bool,
    pub curve: ExactCurve,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Boundary {
    pub intent: BoundaryIntent,
    /// Connected exact curves in traversal order; IDs remain authored IDs even
    /// when a curve has been reversed for this traversal.
    pub edges: Vec<Edge>,
    pub signed_area_mm2: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub outer: Boundary,
    pub holes: Vec<Boundary>,
    pub area_mm2: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointLocation {
    Outside,
    Boundary,
    Inside,
}
impl Region {
    pub fn classify(&self, point: Point) -> PointLocation {
        if point.iter().any(|v| !v.is_finite()) {
            return PointLocation::Outside;
        }
        if std::iter::once(&self.outer)
            .chain(&self.holes)
            .flat_map(|b| &b.edges)
            .any(|e| e.curve.contains(point, DEFAULT_TOLERANCE_MM))
        {
            return PointLocation::Boundary;
        }
        if self
            .outer
            .edges
            .iter()
            .map(|e| e.curve.winding(point))
            .sum::<i32>()
            != 0
            && self
                .holes
                .iter()
                .all(|h| h.edges.iter().map(|e| e.curve.winding(point)).sum::<i32>() == 0)
        {
            PointLocation::Inside
        } else {
            PointLocation::Outside
        }
    }
    pub fn contains(&self, point: Point) -> bool {
        self.classify(point) == PointLocation::Inside
    }
}

/// Pin the selected bounded area, including its holes. A new inner loop changes
/// this intent instead of silently changing a previously selected profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionIntent {
    pub outer: BoundaryIntent,
    pub holes: BTreeSet<BoundaryIntent>,
}
impl Region {
    pub fn intent(&self) -> RegionIntent {
        RegionIntent {
            outer: self.outer.intent.clone(),
            holes: self.holes.iter().map(|b| b.intent.clone()).collect(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extraction {
    pub regions: Vec<Region>,
    /// Problems affect only the identified connected/crossing curve groups.
    /// Unrelated closed regions remain available for explicit selection.
    pub diagnostics: Vec<Diagnostic>,
}
impl Extraction {
    /// Exact authored identity match; no nearest area, index or coordinate
    /// fallback when a boundary has been removed, split or acquired a new hole.
    pub fn resolve(&self, intent: &RegionIntent) -> Option<&Region> {
        self.regions.iter().find(|r| r.intent() == *intent)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    InvalidSource,
    DegenerateCurve,
    OpenBoundary,
    AmbiguousJunction,
    Intersection,
    Overlap,
    DegenerateBoundary,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub problem: Problem,
    pub curves: Vec<EntityId>,
    pub at_mm: Option<Point>,
    pub message: String,
}

fn diagnostic(
    problem: Problem,
    curves: Vec<EntityId>,
    at_mm: Option<Point>,
    message: impl Into<String>,
) -> Diagnostic {
    Diagnostic {
        problem,
        curves,
        at_mm,
        message: message.into(),
    }
}
fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn distance(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
fn along(a: Point, delta: Point, t: f64) -> Point {
    [a[0] + delta[0] * t, a[1] + delta[1] * t]
}

impl ExactCurve {
    fn ends(&self) -> (Point, Point) {
        match *self {
            Self::Line { a, b } | Self::Arc { a, b, .. } => (a, b),
            Self::Circle { center, radius_mm } => {
                let p = [center[0] + radius_mm, center[1]];
                (p, p)
            }
        }
    }
    fn circle(&self) -> Option<(Point, f64)> {
        match *self {
            Self::Line { .. } => None,
            Self::Arc { center, a, .. } => Some((center, distance(center, a))),
            Self::Circle { center, radius_mm } => Some((center, radius_mm)),
        }
    }
    fn arc(&self) -> Option<(f64, f64)> {
        if let Self::Arc {
            center,
            a,
            b,
            clockwise,
        } = *self
        {
            let start = (a[1] - center[1]).atan2(a[0] - center[0]);
            let end = (b[1] - center[1]).atan2(b[0] - center[0]);
            Some((
                start,
                if clockwise {
                    -(start - end).rem_euclid(TAU)
                } else {
                    (end - start).rem_euclid(TAU)
                },
            ))
        } else {
            None
        }
    }
    fn reversed(&self) -> Self {
        match *self {
            Self::Line { a, b } => Self::Line { a: b, b: a },
            Self::Arc {
                center,
                a,
                b,
                clockwise,
            } => Self::Arc {
                center,
                a: b,
                b: a,
                clockwise: !clockwise,
            },
            Self::Circle { .. } => self.clone(),
        }
    }
    fn area(&self, origin: Point) -> f64 {
        match *self {
            Self::Line { a, b } => cross(sub(a, origin), sub(b, origin)) * 0.5,
            Self::Circle { radius_mm, .. } => std::f64::consts::PI * radius_mm * radius_mm,
            Self::Arc { center, a, b, .. } => {
                let sweep = self.arc().expect("arc").1;
                let center_offset = sub(center, origin);
                (center_offset[0] * (b[1] - a[1]) - center_offset[1] * (b[0] - a[0])
                    + distance(center, a).powi(2) * sweep)
                    * 0.5
            }
        }
    }
    fn contains(&self, p: Point, tolerance: f64) -> bool {
        match *self {
            Self::Line { a, b } => {
                let d = sub(b, a);
                let length = distance(a, b);
                let t = dot(sub(p, a), d) / dot(d, d);
                cross(sub(p, a), d).abs() <= tolerance * length
                    && t >= -tolerance / length
                    && t <= 1. + tolerance / length
            }
            Self::Circle { center, radius_mm } => {
                (distance(center, p) - radius_mm).abs() <= tolerance
            }
            Self::Arc { center, a, b, .. } => {
                if distance(p, a) <= tolerance || distance(p, b) <= tolerance {
                    return true;
                }
                let radius = distance(center, a);
                if (distance(center, p) - radius).abs() > tolerance {
                    return false;
                }
                let (start, sweep) = self.arc().expect("arc");
                let angle = (p[1] - center[1]).atan2(p[0] - center[0]);
                let travel = if sweep > 0. {
                    (angle - start).rem_euclid(TAU)
                } else {
                    (start - angle).rem_euclid(TAU)
                };
                travel <= sweep.abs() + tolerance / radius
            }
        }
    }
    fn endpoint(&self, p: Point, tolerance: f64) -> bool {
        !matches!(self, Self::Circle { .. }) && {
            let (a, b) = self.ends();
            distance(a, p) <= tolerance || distance(b, p) <= tolerance
        }
    }
    fn winding(&self, p: Point) -> i32 {
        match *self {
            Self::Line { a, b } => {
                if (a[1] <= p[1] && p[1] < b[1]) || (b[1] <= p[1] && p[1] < a[1]) {
                    let x = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
                    if x > p[0] {
                        return if b[1] > a[1] { 1 } else { -1 };
                    }
                }
                0
            }
            Self::Circle { center, radius_mm } => i32::from(distance(center, p) < radius_mm),
            Self::Arc { center, a, .. } => {
                let radius = distance(center, a);
                let y = (p[1] - center[1]) / radius;
                if y.abs() >= 1. {
                    return 0;
                } // Tangency does not cross the ray.
                let (start, sweep) = self.arc().expect("arc");
                [y.asin(), std::f64::consts::PI - y.asin()]
                    .into_iter()
                    .map(|angle| {
                        let travel = if sweep > 0. {
                            (angle - start).rem_euclid(TAU)
                        } else {
                            (start - angle).rem_euclid(TAU)
                        };
                        // Floating atan2 roundoff at a shared endpoint must follow
                        // the same lower-inclusive/upper-exclusive rule as lines.
                        let t = if travel < 1e-12 || (TAU - travel).abs() < 1e-12 {
                            0.
                        } else {
                            travel / sweep.abs()
                        };
                        let t = if (t - 1.).abs() < 1e-12 { 1. } else { t };
                        let dy = angle.cos() * sweep;
                        let included = if dy > 0. {
                            (0. ..1.).contains(&t)
                        } else {
                            t > 0. && t <= 1.
                        };
                        if included && center[0] + radius * angle.cos() > p[0] {
                            if dy > 0. { 1 } else { -1 }
                        } else {
                            0
                        }
                    })
                    .sum()
            }
        }
    }
}

#[derive(Default)]
struct Intersections {
    points: Vec<Point>,
    overlap: bool,
}
fn intersections(a: &ExactCurve, b: &ExactCurve, tol: f64) -> Intersections {
    let mut result = Intersections::default();
    match (a, b) {
        (ExactCurve::Line { a: p, b: q }, ExactCurve::Line { a: r, b: s }) => {
            let u = sub(*q, *p);
            let v = sub(*s, *r);
            let det = cross(u, v);
            if det == 0. {
                if cross(sub(*r, *p), u).abs() <= tol * distance(*p, *q) {
                    let t0 = dot(sub(*r, *p), u) / dot(u, u);
                    let t1 = dot(sub(*s, *p), u) / dot(u, u);
                    let low = t0.min(t1).max(0.);
                    let high = t0.max(t1).min(1.);
                    if high >= low - tol / distance(*p, *q) {
                        result.overlap = (high - low) * distance(*p, *q) > tol;
                        result.points.push(along(*p, u, (low + high) * 0.5));
                    }
                }
            } else {
                let t = cross(sub(*r, *p), v) / det;
                let p = along(*p, u, t);
                if a.contains(p, tol) && b.contains(p, tol) {
                    result.points.push(p);
                }
            }
        }
        (ExactCurve::Line { a: p, b: q }, circle) | (circle, ExactCurve::Line { a: p, b: q }) => {
            let (center, radius) = circle.circle().expect("circular curve");
            let d = sub(*q, *p);
            let t = dot(sub(center, *p), d) / dot(d, d);
            let nearest = along(*p, d, t);
            let h2 = radius * radius - distance(center, nearest).powi(2);
            if h2 >= -tol * (2. * radius + tol) {
                let delta = (h2.max(0.) / dot(d, d)).sqrt();
                for t in [t - delta, t + delta] {
                    let p = along(*p, d, t);
                    if a.contains(p, tol) && b.contains(p, tol) {
                        result.points.push(p);
                    }
                }
            }
        }
        _ => {
            let (c, r) = a.circle().expect("circle");
            let (d, s) = b.circle().expect("circle");
            let length = distance(c, d);
            if length <= tol && (r - s).abs() <= tol {
                if matches!(a, ExactCurve::Circle { .. }) || matches!(b, ExactCurve::Circle { .. })
                {
                    result.overlap = true;
                } else {
                    for curve in [a, b] {
                        let (start, sweep) = curve.arc().expect("arc");
                        let p = [
                            c[0] + r * (start + sweep * 0.5).cos(),
                            c[1] + r * (start + sweep * 0.5).sin(),
                        ];
                        if a.contains(p, tol)
                            && b.contains(p, tol)
                            && !a.endpoint(p, tol)
                            && !b.endpoint(p, tol)
                        {
                            result.overlap = true;
                        }
                    }
                    for p in [a.ends().0, a.ends().1, b.ends().0, b.ends().1] {
                        if a.contains(p, tol) && b.contains(p, tol) {
                            result.points.push(p);
                        }
                    }
                }
            } else if length > tol && length <= r + s + tol && length >= (r - s).abs() - tol {
                let x = (r * r - s * s + length * length) / (2. * length);
                let h = (r * r - x * x).max(0.).sqrt();
                let unit = sub(d, c).map(|v| v / length);
                let middle = along(c, unit, x);
                for sign in [-1., 1.] {
                    let p = [
                        middle[0] - sign * unit[1] * h,
                        middle[1] + sign * unit[0] * h,
                    ];
                    if a.contains(p, tol) && b.contains(p, tol) {
                        result.points.push(p);
                    }
                }
            }
        }
    }
    result
}

/// All bounded selectable areas: every closed loop is an outer boundary, with
/// its immediate children as holes. A nested circle therefore provides both a
/// surrounding annulus and an independently selectable disk. No material is
/// assigned until the author chooses regions. Intersecting or touching loops
/// are diagnosed instead of guessing a Boolean interpretation or trimming them.
pub fn extract(source: &Source) -> Extraction {
    extract_with_tolerance(source, DEFAULT_TOLERANCE_MM)
}
/// Exact geometric input from a checked evaluation. Callers retain the source
/// revision stamp; this function does not resolve dimensional expressions.
pub fn extract_geometry(
    points: &[crate::constraint_sketch::Point],
    curves: &[crate::constraint_sketch::Curve],
) -> Extraction {
    extract(&Source {
        u: [1., 0., 0.],
        sketch: crate::constraint_sketch::Sketch {
            points: points.to_vec(),
            curves: curves.to_vec(),
            constraints: vec![],
        },
    })
}
pub fn extract_with_tolerance(source: &Source, tolerance_mm: f64) -> Extraction {
    if source.has_expressions() {
        return Extraction {
            regions: vec![],
            diagnostics: vec![diagnostic(
                Problem::InvalidSource,
                vec![],
                None,
                "Parameter-driven profiles require current evaluated geometry",
            )],
        };
    }
    if !tolerance_mm.is_finite() || !(1e-10..=1e-3).contains(&tolerance_mm) {
        return Extraction {
            regions: vec![],
            diagnostics: vec![diagnostic(
                Problem::InvalidSource,
                vec![],
                None,
                "Region tolerance must be between 1e-10 and 1e-3 mm",
            )],
        };
    }
    if let Err(e) = source.validate() {
        return Extraction {
            regions: vec![],
            diagnostics: vec![diagnostic(
                Problem::InvalidSource,
                vec![],
                None,
                e.to_string(),
            )],
        };
    }
    let point = |id| {
        source
            .sketch
            .points
            .iter()
            .find(|p| p.id == id)
            .expect("validated point")
            .position_mm
    };
    let mut edges: Vec<_> = source
        .sketch
        .curves
        .iter()
        .filter(|c| !c.construction)
        .map(|c| Edge {
            authored: c.id,
            reversed: false,
            curve: match c.kind {
                CurveKind::Line { start, end } => ExactCurve::Line {
                    a: point(start),
                    b: point(end),
                },
                CurveKind::Circle { center, radius_mm } => ExactCurve::Circle {
                    center: point(center),
                    radius_mm,
                },
                CurveKind::Arc {
                    center,
                    start,
                    end,
                    clockwise,
                } => ExactCurve::Arc {
                    center: point(center),
                    a: point(start),
                    b: point(end),
                    clockwise,
                },
            },
        })
        .collect();
    edges.sort_by_key(|e| e.authored);
    let mut diagnostics = vec![];
    let mut invalid_edges = BTreeSet::new();
    let mut degenerate = BTreeSet::new();
    for (i, edge) in edges.iter().enumerate() {
        let (a, b) = edge.curve.ends();
        let invalid = match edge.curve {
            ExactCurve::Line { .. } => distance(a, b) <= tolerance_mm,
            ExactCurve::Circle { radius_mm, .. } => radius_mm <= tolerance_mm,
            ExactCurve::Arc { center, .. } => {
                distance(a, b) <= tolerance_mm
                    || distance(center, a) <= tolerance_mm
                    || (distance(center, a) - distance(center, b)).abs() > tolerance_mm
            }
        };
        if invalid {
            invalid_edges.insert(i);
            degenerate.insert(i);
            diagnostics.push(diagnostic(
                Problem::DegenerateCurve,
                vec![edge.authored],
                Some(a),
                "Curve is degenerate or arc radii disagree within the extraction tolerance",
            ));
        }
    }
    let mut nodes: Vec<(Point, Vec<(usize, bool)>)> = vec![];
    let mut endpoints = vec![None; edges.len()];
    for (i, edge) in edges.iter().enumerate() {
        if matches!(edge.curve, ExactCurve::Circle { .. }) {
            continue;
        }
        let mut pair = [0; 2];
        for (end, p) in [edge.curve.ends().0, edge.curve.ends().1]
            .into_iter()
            .enumerate()
        {
            let matches: Vec<_> = nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| distance(n.0, p) <= tolerance_mm)
                .map(|(i, _)| i)
                .collect();
            if matches.len() > 1 {
                invalid_edges.insert(i);
                for node in &matches {
                    invalid_edges.extend(nodes[*node].1.iter().map(|(i, _)| *i));
                }
                diagnostics.push(diagnostic(
                    Problem::AmbiguousJunction,
                    vec![edge.authored],
                    Some(p),
                    "Endpoint is within tolerance of several distinct junctions",
                ));
            }
            let node = matches.first().copied().unwrap_or_else(|| {
                nodes.push((p, vec![]));
                nodes.len() - 1
            });
            nodes[node].1.push((i, end == 1));
            pair[end] = node;
        }
        endpoints[i] = Some(pair);
    }
    for (p, incident) in &nodes {
        if incident.len() != 2 {
            invalid_edges.extend(incident.iter().map(|(i, _)| *i));
            diagnostics.push(diagnostic(
                if incident.len() < 2 {
                    Problem::OpenBoundary
                } else {
                    Problem::AmbiguousJunction
                },
                incident.iter().map(|(i, _)| edges[*i].authored).collect(),
                Some(*p),
                if incident.len() < 2 {
                    "Open profile endpoint"
                } else {
                    "More than two curves meet; select or trim an unambiguous boundary"
                },
            ));
        }
    }
    for i in 0..edges.len() {
        for j in i + 1..edges.len() {
            if degenerate.contains(&i) || degenerate.contains(&j) {
                continue;
            }
            let hits = intersections(&edges[i].curve, &edges[j].curve, tolerance_mm);
            if hits.overlap {
                invalid_edges.extend([i, j]);
                diagnostics.push(diagnostic(
                    Problem::Overlap,
                    vec![edges[i].authored, edges[j].authored],
                    None,
                    "Overlapping curves cannot define an unambiguous region",
                ));
            } else if let Some(p) = hits.points.into_iter().find(|p| {
                !(edges[i].curve.endpoint(*p, tolerance_mm)
                    && edges[j].curve.endpoint(*p, tolerance_mm))
            }) {
                invalid_edges.extend([i, j]);
                diagnostics.push(diagnostic(
                    Problem::Intersection,
                    vec![edges[i].authored, edges[j].authored],
                    Some(p),
                    "Crossing or touching boundaries require explicit trimming",
                ));
            }
        }
    }
    // A local ambiguity invalidates the complete connected wire, including a
    // loop crossed by a stray line. Independent loops remain usable.
    loop {
        let previous = invalid_edges.len();
        for (_, incident) in &nodes {
            if incident.iter().any(|(i, _)| invalid_edges.contains(i)) {
                invalid_edges.extend(incident.iter().map(|(i, _)| *i));
            }
        }
        if invalid_edges.len() == previous {
            break;
        }
    }
    let mut visited = BTreeSet::new();
    let mut boundaries = vec![];
    for seed in 0..edges.len() {
        if visited.contains(&seed) || invalid_edges.contains(&seed) {
            continue;
        }
        let mut wire = vec![];
        let mut current = seed;
        let mut reversed = false;
        loop {
            if !visited.insert(current) {
                break;
            }
            let mut edge = edges[current].clone();
            edge.reversed = reversed;
            if reversed {
                edge.curve = edge.curve.reversed();
            }
            wire.push(edge);
            let Some(pair) = endpoints[current] else {
                break;
            };
            let next_node = pair[usize::from(!reversed)];
            let (next, at_end) = *nodes[next_node]
                .1
                .iter()
                .find(|(i, _)| *i != current)
                .expect("degree two");
            current = next;
            reversed = at_end;
        }
        let origin = wire[0].curve.ends().0;
        let area = wire.iter().map(|e| e.curve.area(origin)).sum::<f64>();
        if area.abs() <= tolerance_mm * tolerance_mm {
            diagnostics.push(diagnostic(
                Problem::DegenerateBoundary,
                wire.iter().map(|e| e.authored).collect(),
                None,
                "Closed boundary has zero area",
            ));
            continue;
        }
        boundaries.push(Boundary {
            intent: BoundaryIntent {
                curves: wire.iter().map(|e| e.authored).collect(),
            },
            edges: wire,
            signed_area_mm2: area,
        });
    }
    let parents: Vec<_> = boundaries
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let p = b.edges[0].curve.ends().0;
            boundaries
                .iter()
                .enumerate()
                .filter(|(j, c)| {
                    *j != i
                        && c.signed_area_mm2.abs() > b.signed_area_mm2.abs()
                        && c.edges.iter().map(|e| e.curve.winding(p)).sum::<i32>() != 0
                })
                .min_by(|(_, a), (_, b)| {
                    a.signed_area_mm2.abs().total_cmp(&b.signed_area_mm2.abs())
                })
                .map(|(j, _)| j)
        })
        .collect();
    let regions = boundaries
        .iter()
        .enumerate()
        .map(|(i, outer)| {
            let holes: Vec<_> = boundaries
                .iter()
                .enumerate()
                .filter(|(j, _)| parents[*j] == Some(i))
                .map(|(_, b)| b.clone())
                .collect();
            Region {
                area_mm2: outer.signed_area_mm2.abs()
                    - holes.iter().map(|b| b.signed_area_mm2.abs()).sum::<f64>(),
                outer: outer.clone(),
                holes,
            }
        })
        .collect();
    Extraction {
        regions,
        diagnostics,
    }
}

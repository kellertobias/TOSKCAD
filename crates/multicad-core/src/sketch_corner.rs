//! Sketch corner fillet and chamfer. A treatment replaces the sharp corner
//! shared by exactly two curves (lines or arcs, meeting at an endpoint of
//! each) with a tangent arc or a chamfer line. The original corner point
//! stays as a construction "virtual sharp": a construction line held parallel
//! to each trimmed line, or a construction arc on each trimmed arc's own
//! center, so existing references to it remain meaningful and the solver
//! keeps the treatment's shape. The draft owns its proposed source; dropping
//! it changes nothing.
//!
//! Fillet: the tangent arc of the given radius whose tangent points lie on
//! both curves closest to the corner (shortest combined trim). It carries a
//! Tangent constraint to each trimmed curve and a Radius dimension.
//!
//! Chamfer: a line between the points on each curve at the given straight
//! distance from the corner. On a line that is the setback along the line;
//! on an arc it is the chord length from the corner, dimensioned as a point
//! distance. Corners whose curves already meet tangentially (or turn back on
//! each other) have no corner to treat and are refused.
use crate::{
    constraint_sketch::{Axis, ConstraintKind, Curve, CurveKind, Dimension, source::Source},
    document::{DocumentError, EntityId},
};
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

const MIN_LENGTH_MM: f64 = 1e-7;
const MAX_SIZE_MM: f64 = 1e9;
const MIN_ANGLE: f64 = 1e-6;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CornerTreatment {
    /// Tangent arc of this radius.
    Fillet { radius_mm: f64 },
    /// Straight chamfer set back this distance along both edges.
    Chamfer { distance_mm: f64 },
}
#[derive(Clone, Debug, PartialEq)]
pub struct CornerDraft {
    pub source: Source,
    /// The new arc or chamfer line, followed by the two construction curves.
    pub created_curves: Vec<EntityId>,
}

const NOT_A_CORNER: &str =
    "Fillet and chamfer need a corner shared by exactly two lines or arcs at their endpoints";

fn invalid(message: impl Into<String>) -> DocumentError {
    DocumentError::Invalid(message.into())
}
fn literal(value: f64) -> Dimension {
    Dimension::Literal {
        value,
        unit: "mm".into(),
    }
}

type V = [f64; 2];
fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1]]
}
fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1]]
}
fn scale(a: V, s: f64) -> V {
    [a[0] * s, a[1] * s]
}
fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross(a: V, b: V) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn norm(a: V) -> f64 {
    a[0].hypot(a[1])
}
/// Counter-clockwise perpendicular.
fn perp(a: V) -> V {
    [-a[1], a[0]]
}
fn angle(a: V) -> f64 {
    a[1].atan2(a[0])
}
fn rotate(a: V, by: f64) -> V {
    let (s, c) = by.sin_cos();
    [a[0] * c - a[1] * s, a[0] * s + a[1] * c]
}

#[derive(Clone, Copy, Debug)]
enum Shape {
    /// Unit direction from the corner to the line's other end, and its length.
    Line { dir: V, length: f64 },
    /// `away` is +1 when moving away from the corner along the arc is
    /// counter-clockwise; `sweep` is the arc's total angle.
    Arc {
        center: V,
        center_id: EntityId,
        radius: f64,
        away: f64,
        sweep: f64,
    },
}

/// One of the two curves at the corner.
#[derive(Clone, Copy, Debug)]
struct Edge {
    index: usize,
    corner_is_start: bool,
    corner: V,
    shape: Shape,
}

#[derive(Clone, Copy, Debug)]
enum Locus {
    Line { point: V, dir: V },
    Circle { center: V, radius: f64 },
}

impl Edge {
    /// Unit tangent at `t` pointing away from the corner along the curve.
    fn away(&self, t: V) -> V {
        match self.shape {
            Shape::Line { dir, .. } => dir,
            Shape::Arc {
                center,
                radius,
                away,
                ..
            } => scale(perp(sub(t, center)), away / radius),
        }
    }
    /// Curve length from the corner to `t` (on the curve), when `t` lies
    /// strictly inside the curve.
    fn along(&self, t: V) -> Option<f64> {
        let (s, length) = match self.shape {
            Shape::Line { dir, length } => (dot(sub(t, self.corner), dir), length),
            Shape::Arc {
                center,
                radius,
                away,
                sweep,
                ..
            } => {
                let a = (away * (angle(sub(t, center)) - angle(sub(self.corner, center))))
                    .rem_euclid(TAU);
                (a * radius, sweep * radius)
            }
        };
        (s > MIN_LENGTH_MM && s < length - MIN_LENGTH_MM).then_some(s)
    }
    /// The curve offset by `r` to one side (`side` = ±1).
    fn offset(&self, side: f64, r: f64) -> Option<Locus> {
        match self.shape {
            Shape::Line { dir, .. } => Some(Locus::Line {
                point: add(self.corner, scale(perp(dir), side * r)),
                dir,
            }),
            Shape::Arc { center, radius, .. } => {
                let radius = (radius + side * r).abs();
                (radius > MIN_LENGTH_MM).then_some(Locus::Circle { center, radius })
            }
        }
    }
    /// The point where a circle of radius `r` centered at `q` touches this
    /// curve's line or circle.
    fn touch(&self, q: V, r: f64) -> Option<V> {
        match self.shape {
            Shape::Line { dir, .. } => {
                Some(add(self.corner, scale(dir, dot(sub(q, self.corner), dir))))
            }
            Shape::Arc { center, radius, .. } => {
                let d = sub(q, center);
                let l = norm(d);
                if l <= MIN_LENGTH_MM {
                    return None;
                }
                let u = scale(d, radius / l);
                [add(center, u), sub(center, u)].into_iter().min_by(|a, b| {
                    (norm(sub(*a, q)) - r)
                        .abs()
                        .total_cmp(&(norm(sub(*b, q)) - r).abs())
                })
            }
        }
    }
    /// The point on the curve at straight distance `d` from the corner.
    fn at_distance(&self, d: f64) -> Option<V> {
        let t = match self.shape {
            Shape::Line { dir, .. } => add(self.corner, scale(dir, d)),
            Shape::Arc {
                center,
                radius,
                away,
                ..
            } => {
                if d >= 2. * radius {
                    return None;
                }
                let phi = 2. * (d / (2. * radius)).asin();
                add(center, rotate(sub(self.corner, center), away * phi))
            }
        };
        self.along(t).map(|_| t)
    }
}

fn intersect(a: Locus, b: Locus) -> Vec<V> {
    match (a, b) {
        (Locus::Line { point: p, dir: d }, Locus::Line { point: q, dir: e }) => {
            let det = cross(d, e);
            if det.abs() <= 1e-12 {
                return vec![];
            }
            vec![add(p, scale(d, cross(sub(q, p), e) / det))]
        }
        (Locus::Line { point, dir }, Locus::Circle { center, radius })
        | (Locus::Circle { center, radius }, Locus::Line { point, dir }) => {
            let f = sub(point, center);
            let b = dot(f, dir);
            let disc = b * b - (dot(f, f) - radius * radius);
            if disc < 0. {
                return vec![];
            }
            let root = disc.sqrt();
            vec![
                add(point, scale(dir, -b - root)),
                add(point, scale(dir, -b + root)),
            ]
        }
        (
            Locus::Circle {
                center: c1,
                radius: r1,
            },
            Locus::Circle {
                center: c2,
                radius: r2,
            },
        ) => {
            let d = sub(c2, c1);
            let l = norm(d);
            if l <= 1e-12 || l > r1 + r2 || l < (r1 - r2).abs() {
                return vec![];
            }
            let a = (r1 * r1 - r2 * r2 + l * l) / (2. * l);
            let h = (r1 * r1 - a * a).max(0.).sqrt();
            let base = add(c1, scale(d, a / l));
            let off = scale(perp(d), h / l);
            vec![add(base, off), sub(base, off)]
        }
    }
}

/// Corner points at which a treatment can apply: points that are an endpoint
/// of exactly two lines or arcs and belong to no other curve.
pub fn treatable_corners(source: &Source) -> Vec<EntityId> {
    source
        .sketch
        .points
        .iter()
        .filter(|point| corner_curves(source, point.id).is_ok())
        .map(|point| point.id)
        .collect()
}

fn corner_curves(source: &Source, corner: EntityId) -> Result<[(usize, bool); 2], DocumentError> {
    let mut curves = vec![];
    for (index, curve) in source.sketch.curves.iter().enumerate() {
        match curve.kind {
            CurveKind::Line { start, end } if start == corner || end == corner => {
                if start == end {
                    return Err(invalid("Corner line is degenerate"));
                }
                curves.push((index, start == corner));
            }
            CurveKind::Arc {
                center, start, end, ..
            } if [center, start, end].contains(&corner) => {
                if center == corner || start == end {
                    return Err(invalid(NOT_A_CORNER));
                }
                curves.push((index, start == corner));
            }
            CurveKind::Circle { center, .. } if center == corner => {
                return Err(invalid(NOT_A_CORNER));
            }
            CurveKind::BSpline { ref poles, .. } if poles.contains(&corner) => {
                return Err(invalid(NOT_A_CORNER));
            }
            _ => {}
        }
    }
    match curves.as_slice() {
        [a, b] => Ok([*a, *b]),
        _ => Err(invalid(NOT_A_CORNER)),
    }
}

fn edge(
    source: &Source,
    position: &dyn Fn(EntityId) -> Result<V, DocumentError>,
    corner: V,
    (index, corner_is_start): (usize, bool),
) -> Result<Edge, DocumentError> {
    let degenerate = || invalid("Corner curve is degenerate");
    let shape = match source.sketch.curves[index].kind {
        CurveKind::Line { start, end } => {
            let delta = sub(position(if corner_is_start { end } else { start })?, corner);
            let length = norm(delta);
            if length <= MIN_LENGTH_MM {
                return Err(degenerate());
            }
            Shape::Line {
                dir: scale(delta, 1. / length),
                length,
            }
        }
        CurveKind::Arc {
            center: center_id,
            start,
            end,
            clockwise,
        } => {
            let center = position(center_id)?;
            let radius = norm(sub(corner, center));
            if radius <= MIN_LENGTH_MM {
                return Err(degenerate());
            }
            let direction = if clockwise { -1. } else { 1. };
            let from = angle(sub(position(start)?, center));
            let to = angle(sub(position(end)?, center));
            let sweep = (direction * (to - from)).rem_euclid(TAU);
            if sweep * radius <= MIN_LENGTH_MM {
                return Err(degenerate());
            }
            Shape::Arc {
                center,
                center_id,
                radius,
                away: if corner_is_start {
                    direction
                } else {
                    -direction
                },
                sweep,
            }
        }
        _ => unreachable!("corner_curves returns lines and arcs"),
    };
    Ok(Edge {
        index,
        corner_is_start,
        corner,
        shape,
    })
}

/// The tangent fillet circle center and its tangent points nearest the corner.
fn fillet_geometry(edges: &[Edge; 2], r: f64) -> Option<(V, [V; 2], bool)> {
    let tolerance = 1e-7 * (1. + r);
    let mut best: Option<(f64, V, [V; 2], bool)> = None;
    for s0 in [1., -1.] {
        for s1 in [1., -1.] {
            let (Some(a), Some(b)) = (edges[0].offset(s0, r), edges[1].offset(s1, r)) else {
                continue;
            };
            for q in intersect(a, b) {
                let (Some(t0), Some(t1)) = (edges[0].touch(q, r), edges[1].touch(q, r)) else {
                    continue;
                };
                if [t0, t1]
                    .iter()
                    .any(|t| (norm(sub(*t, q)) - r).abs() > tolerance)
                    || norm(sub(t0, t1)) <= MIN_LENGTH_MM
                {
                    continue;
                }
                let (Some(l0), Some(l1)) = (edges[0].along(t0), edges[1].along(t1)) else {
                    continue;
                };
                // Entering the fillet at t0 continues curve 0 toward the
                // corner; leaving it at t1 must continue along curve 1.
                let heading = scale(edges[0].away(t0), -1.);
                let ccw = cross(sub(t0, q), heading) > 0.;
                let turn = if ccw { 1. } else { -1. };
                let leaving = scale(perp(sub(t1, q)), turn / r);
                if dot(leaving, edges[1].away(t1)) < 0.5 {
                    continue;
                }
                if best.as_ref().is_none_or(|b| l0 + l1 < b.0) {
                    best = Some((l0 + l1, q, [t0, t1], !ccw));
                }
            }
        }
    }
    best.map(|(_, q, t, clockwise)| (q, t, clockwise))
}

pub fn corner_draft(
    source: &Source,
    corner: EntityId,
    treatment: CornerTreatment,
) -> Result<CornerDraft, DocumentError> {
    source.validate()?;
    let size = match treatment {
        CornerTreatment::Fillet { radius_mm } => radius_mm,
        CornerTreatment::Chamfer { distance_mm } => distance_mm,
    };
    if !size.is_finite() || size <= MIN_LENGTH_MM || size > MAX_SIZE_MM {
        return Err(invalid(
            "Fillet radius or chamfer distance must be finite and within (1e-7, 1e9] mm",
        ));
    }
    let position = |id: EntityId| {
        source
            .sketch
            .points
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.position_mm)
            .ok_or(DocumentError::Missing(id))
    };
    let at = position(corner)?;
    let found = corner_curves(source, corner)?;
    let edges = [
        edge(source, &position, at, found[0])?,
        edge(source, &position, at, found[1])?,
    ];
    let between = dot(edges[0].away(at), edges[1].away(at))
        .clamp(-1., 1.)
        .acos();
    if !(MIN_ANGLE..=PI - MIN_ANGLE).contains(&between) {
        return Err(invalid(
            "Fillet and chamfer need two curves meeting at an angle; these meet tangentially or are collinear",
        ));
    }
    let mut fillet = None;
    let points = match treatment {
        CornerTreatment::Fillet { radius_mm } => {
            let (center, points, clockwise) =
                fillet_geometry(&edges, radius_mm).ok_or_else(|| {
                    invalid(format!(
                        "No tangent fillet of {radius_mm:.3} mm fits this corner; a corner curve is too short or curves away"
                    ))
                })?;
            fillet = Some((center, clockwise));
            points
        }
        CornerTreatment::Chamfer { distance_mm } => {
            match (
                edges[0].at_distance(distance_mm),
                edges[1].at_distance(distance_mm),
            ) {
                (Some(a), Some(b)) if norm(sub(a, b)) > MIN_LENGTH_MM => [a, b],
                _ => {
                    return Err(invalid(format!(
                        "The chamfer needs {distance_mm:.3} mm along each curve; a corner curve is too short"
                    )));
                }
            }
        }
    };
    let mut proposed = source.clone();
    let ids = [proposed.add_point(points[0]), proposed.add_point(points[1])];
    let mut edge_ids = [EntityId::new(); 2];
    for (i, edge) in edges.iter().enumerate() {
        let curve = &mut proposed.sketch.curves[edge.index];
        match &mut curve.kind {
            CurveKind::Line { start, end } | CurveKind::Arc { start, end, .. } => {
                if edge.corner_is_start {
                    *start = ids[i];
                } else {
                    *end = ids[i];
                }
            }
            _ => unreachable!("corner curves are lines and arcs"),
        }
        edge_ids[i] = curve.id;
    }
    let both_construction = edges
        .iter()
        .all(|e| source.sketch.curves[e.index].construction);
    let mut created = vec![];
    let mut push = |source: &mut Source, kind, helper: bool| {
        let id = EntityId::new();
        source.sketch.curves.push(Curve {
            id,
            kind,
            construction: helper,
        });
        created.push(id);
        id
    };
    match (treatment, fillet) {
        (CornerTreatment::Fillet { radius_mm }, Some((center, clockwise))) => {
            let center = proposed.add_point(center);
            let arc = push(
                &mut proposed,
                CurveKind::Arc {
                    center,
                    start: ids[0],
                    end: ids[1],
                    clockwise,
                },
                both_construction,
            );
            for edge in edge_ids {
                proposed.add_constraint(ConstraintKind::Tangent { a: edge, b: arc });
            }
            proposed.add_constraint(ConstraintKind::Radius {
                curve: arc,
                dimension: literal(radius_mm),
            });
        }
        _ => {
            push(
                &mut proposed,
                CurveKind::Line {
                    start: ids[0],
                    end: ids[1],
                },
                both_construction,
            );
        }
    }
    // The virtual sharp: each trimmed curve continues to the original corner
    // as construction geometry.
    for (i, edge) in edges.iter().enumerate() {
        match (edge.shape, &source.sketch.curves[edge.index].kind) {
            (Shape::Line { .. }, _) => {
                let helper = push(
                    &mut proposed,
                    CurveKind::Line {
                        start: ids[i],
                        end: corner,
                    },
                    true,
                );
                proposed.add_constraint(ConstraintKind::Parallel {
                    a: helper,
                    b: edge_ids[i],
                });
                if let CornerTreatment::Chamfer { distance_mm } = treatment {
                    proposed.add_constraint(ConstraintKind::Length {
                        line: helper,
                        dimension: literal(distance_mm),
                    });
                }
            }
            (Shape::Arc { center_id, .. }, &CurveKind::Arc { clockwise, .. }) => {
                let (start, end) = if edge.corner_is_start {
                    (corner, ids[i])
                } else {
                    (ids[i], corner)
                };
                push(
                    &mut proposed,
                    CurveKind::Arc {
                        center: center_id,
                        start,
                        end,
                        clockwise,
                    },
                    true,
                );
                if let CornerTreatment::Chamfer { distance_mm } = treatment {
                    proposed.add_constraint(ConstraintKind::Distance {
                        a: ids[i],
                        b: corner,
                        axis: Axis::Direct,
                        dimension: literal(distance_mm),
                    });
                }
            }
            _ => unreachable!("edge shapes follow their curve kinds"),
        }
    }
    proposed.validate()?;
    Ok(CornerDraft {
        source: proposed,
        created_curves: created,
    })
}

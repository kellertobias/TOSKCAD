//! Project exact planar and round edges against the same scene.

use super::*;
use crate::{
    curved_projection::{Curve, Detail, Edge, Rim, SpatialCurve, SpatialEdge},
    drawing_events::{self, Harmonic},
};

mod candidates;
use candidates::apply_openings;
pub use candidates::{FaceOpening, LineCandidate, RimCandidate, SceneExtras, SpatialRimCandidate};
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundSegment {
    /// Index in the round input slice, or the supplied candidate source.
    pub source: usize,
    pub edge: Edge,
}

/// Planar, round-stock, and caller-supplied vectors in input order.
pub type SceneProjection = (Vec<Segment>, Vec<RoundSegment>, Vec<RoundSegment>);

/// Project planar and round bodies with exact hidden spans on straight
/// edges and circular rims. Both output lists share the segment budget.
pub fn project_scene(
    bodies: &[Body],
    rounds: &[RoundOccluder],
    options: Options,
) -> Result<(Vec<Segment>, Vec<RoundSegment>), ProjectionError> {
    project_scene_bounded(bodies, rounds, options, WORK_LIMIT)
}

/// Project the scene and any supplied drawing-space edges, using both
/// ordinary bodies and blocker-only bodies to resolve visibility.
pub fn project_scene_with_candidates(
    bodies: &[Body],
    rounds: &[RoundOccluder],
    extras: SceneExtras<'_>,
    options: Options,
) -> Result<SceneProjection, ProjectionError> {
    project_with_extras(bodies, rounds, extras, options, WORK_LIMIT)
}

fn project_scene_bounded(
    bodies: &[Body],
    rounds: &[RoundOccluder],
    options: Options,
    limit: usize,
) -> Result<(Vec<Segment>, Vec<RoundSegment>), ProjectionError> {
    let (straight, round, _) =
        project_with_extras(bodies, rounds, SceneExtras::default(), options, limit)?;
    Ok((straight, round))
}

fn project_with_extras(
    bodies: &[Body],
    rounds: &[RoundOccluder],
    extras: SceneExtras<'_>,
    options: Options,
    limit: usize,
) -> Result<SceneProjection, ProjectionError> {
    let mut work = projection_work(&options, limit)?;
    let (mut planes, uses) = prepare(bodies.iter().chain(extras.blocker_bodies), &mut work)?;
    apply_openings(bodies, &extras, &mut planes, &mut work)?;
    let index = Node::build(planes);
    let all_rounds;
    let blockers = if extras.blocker_rounds.is_empty() {
        rounds
    } else {
        work.charge(extras.blocker_rounds.len())?;
        all_rounds = rounds
            .iter()
            .chain(extras.blocker_rounds)
            .copied()
            .collect::<Vec<_>>();
        &all_rounds
    };
    let straight = match &index {
        Some(index) => project_straight(bodies, &uses, index, blockers, &options, &mut work)?,
        None if rounds.is_empty()
            && extras.lines.is_empty()
            && extras.rims.is_empty()
            && extras.spatial_rims.is_empty() =>
        {
            return Err(ProjectionError::Invalid);
        }
        None => Vec::new(),
    };
    let detail = options
        .detail
        .map(|(center, radius)| Detail { center, radius });
    let scene = Scene {
        index: index.as_ref(),
        rounds: blockers,
        options: &options,
    };
    let remaining = options.segment_limit - straight.len();
    let mut curved = Vec::new();
    for (source, round) in rounds.iter().enumerate() {
        work.charge(1)?;
        let mut output = Output {
            source,
            own_round: Some(source),
            limit: remaining,
            edges: &mut curved,
        };
        for edge in round.drawing_edges(options.hidden_lines, detail)? {
            scene.edge(edge, None, &mut output, &mut work)?;
        }
    }
    let mut supplied = Vec::new();
    let limit = remaining - curved.len();
    for &line in extras.lines {
        work.charge(1)?;
        let mut output = Output {
            source: line.source,
            own_round: None,
            limit,
            edges: &mut supplied,
        };
        scene.edge(
            SpatialEdge {
                hidden: false,
                curve: line.spatial()?,
            },
            options.detail,
            &mut output,
            &mut work,
        )?;
    }
    for &candidate in extras.rims {
        work.charge(1)?;
        let mut output = Output {
            source: candidate.source,
            own_round: None,
            limit,
            edges: &mut supplied,
        };
        scene.supplied_rim(candidate.spatial()?, detail, &mut output, &mut work)?;
    }
    for &candidate in extras.spatial_rims {
        work.charge(1)?;
        let mut output = Output {
            source: candidate.source,
            own_round: None,
            limit,
            edges: &mut supplied,
        };
        scene.supplied_rim(candidate.spatial()?, detail, &mut output, &mut work)?;
    }
    Ok((straight, curved, supplied))
}

struct Scene<'a> {
    index: Option<&'a Node>,
    rounds: &'a [RoundOccluder],
    options: &'a Options,
}

struct Output<'a> {
    source: usize,
    own_round: Option<usize>,
    limit: usize,
    edges: &'a mut Vec<RoundSegment>,
}

impl Scene<'_> {
    fn supplied_rim(
        &self,
        mut rim: Rim,
        detail: Option<Detail>,
        output: &mut Output<'_>,
        work: &mut Work,
    ) -> Result<(), ProjectionError> {
        let full = [rim.ellipse.trim];
        let clipped;
        let trims = if let Some(detail) = detail {
            clipped = rim.ellipse.in_detail(detail);
            clipped.as_slice()
        } else {
            &full
        };
        for &[lo, hi] in trims {
            let split = rim.ellipse.minor == 0. && lo < PI && hi > PI;
            let mut emit = |trim| {
                rim.ellipse.trim = trim;
                self.edge(
                    SpatialEdge {
                        hidden: false,
                        curve: SpatialCurve::Rim(rim),
                    },
                    None,
                    output,
                    work,
                )
            };
            if split {
                // Each half of a collapsed ellipse is monotone on screen.
                // Its partner still has a distinct depth and visibility.
                emit([lo, PI])?;
                emit([PI, hi])?;
            } else {
                emit([lo, hi])?;
            }
        }
        Ok(())
    }

    fn edge(
        &self,
        edge: SpatialEdge,
        detail: Option<(Point2, f64)>,
        output: &mut Output<'_>,
        work: &mut Work,
    ) -> Result<(), ProjectionError> {
        let source = output.own_round;
        let tolerance = self.options.tolerance;
        let mut emit = |hidden, curve| {
            if hidden && !self.options.hidden_lines {
                return Ok(());
            }
            if matches!(curve, Curve::Line([a, b]) if (a[0]-b[0]).hypot(a[1]-b[1]) <= tolerance) {
                return Ok(());
            }
            if output.edges.len() == output.limit {
                return Err(ProjectionError::Limit);
            }
            output.edges.push(RoundSegment {
                source: output.source,
                edge: Edge { hidden, curve },
            });
            Ok(())
        };
        match edge.curve {
            SpatialCurve::Line(points) => {
                let Some(line) = Line::new(points, tolerance, [usize::MAX; 2]) else {
                    return Ok(());
                };
                let span = line.span(detail);
                let curve = |trim: Span| Curve::Line(trim.map(|at| planar(line.point(at))));
                if span[1] - span[0] <= tolerance {
                    return Ok(());
                }
                if edge.hidden {
                    emit(true, curve(span))
                } else {
                    hide_line(&line, span, self.index, self.rounds, source, work)?;
                    publish_spans(&mut work.hidden, span, tolerance, |hidden, trim| {
                        emit(hidden, curve(trim))
                    })
                }
            }
            SpatialCurve::Rim(rim) => {
                let span = rim.ellipse.trim;
                if edge.hidden {
                    emit(true, rim.trimmed(span))
                } else {
                    hide_rim(rim, self.index, self.rounds, source, work)?;
                    publish_spans(
                        &mut work.hidden,
                        span,
                        tolerance / rim.ellipse.major,
                        |hidden, trim| emit(hidden, rim.trimmed(trim)),
                    )
                }
            }
        }
    }
}

fn hide_rim(
    rim: Rim,
    index: Option<&Node>,
    rounds: &[RoundOccluder],
    source: Option<usize>,
    work: &mut Work,
) -> Result<(), ProjectionError> {
    work.hidden.clear();
    let [origin, cos, sin] = rim.basis();
    let extent: Point = std::array::from_fn(|i| cos[i].hypot(sin[i]));
    let bounds = Bounds {
        min: std::array::from_fn(|i| origin[i] - extent[i]),
        max: std::array::from_fn(|i| origin[i] + extent[i]),
    };
    if let Some(index) = index {
        index.visit(bounds, work, &mut |face, work| {
            face.hide_rim(rim, bounds, work)
        })?;
    }
    for (id, round) in rounds.iter().enumerate() {
        work.charge(1)?;
        let [min, max] = round.bounds();
        if Some(id) != source && (Bounds { min, max }).overlaps(bounds, work.tolerance) {
            // Charge the bounded conic and cap root work even when no
            // resulting interval covers the candidate rim.
            work.charge(160)?;
            round.hide_rim(rim, work.tolerance, &mut work.crossings, |span| {
                work.hidden.push(span)
            });
        }
    }
    Ok(())
}

impl TrimmedPlane {
    fn hide_rim(&self, rim: Rim, bounds: Bounds, work: &mut Work) -> Result<(), ProjectionError> {
        let [origin, cos, sin] = rim.basis();
        let depths = Harmonic([
            dot(self.normal, sub(self.point, origin)),
            -dot(self.normal, cos),
            -dot(self.normal, sin),
        ]);
        let roundoff = (0..3)
            .map(|i| {
                self.normal[i].abs()
                    * (self.point[i].abs() + bounds.min[i].abs().max(bounds.max[i].abs()))
            })
            .sum::<f64>()
            * (16. * f64::EPSILON);
        let threshold = (self.normal[2] * work.tolerance).max(roundoff);
        let [depth, cosine, sine] = depths.0;
        if depth + cosine.hypot(sine) <= threshold {
            return Ok(());
        }
        let trim = rim.ellipse.trim;
        let boundaries = self.boundary.len() + self.circles.len();
        work.charge(boundaries + 79 * self.circles.len())?;
        work.crossings.clear();
        depths.level(threshold, trim, &mut work.crossings);
        for &[a, b] in &self.boundary {
            let [x, y] = sub(b, a);
            let normal = [-y, x];
            Harmonic([
                dot(normal, sub(planar(origin), a)),
                dot(normal, planar(cos)),
                dot(normal, planar(sin)),
            ])
            .level(0., trim, &mut work.crossings);
        }
        for circle in &self.circles {
            let [x, y] =
                std::array::from_fn(|i| Harmonic([origin[i] - circle.center[i], cos[i], sin[i]]));
            drawing_events::circle(x, y, circle.radius, trim, &mut work.crossings);
        }
        work.charge((work.crossings.len() + 1).saturating_mul(boundaries))?;
        drawing_events::spans(
            &mut work.crossings,
            trim,
            |angle| {
                depths.at(angle) > threshold
                    && self.contains(planar(rim.point(angle)), work.tolerance)
            },
            |span| work.hidden.push(span),
        );
        Ok(())
    }

    fn contains(&self, point: Point2, tolerance: f64) -> bool {
        let mut inside = false;
        for &[a, b] in &self.boundary {
            let [x, y] = sub(b, a);
            let [u, v] = sub(point, a);
            let length = x.hypot(y);
            let threshold = length * tolerance;
            let cross = x.mul_add(v, -y * u);
            let along = x.mul_add(u, y * v);
            if length > 0.
                && cross.abs() <= threshold
                && (-threshold..=length * length + threshold).contains(&along)
            {
                return true;
            }
            if (a[1] > point[1]) != (b[1] > point[1]) && (cross > 0.) == (y > 0.) {
                inside = !inside;
            }
        }
        for circle in &self.circles {
            let [x, y] = sub(point, circle.center);
            let distance = x.hypot(y);
            let roundoff = 16.
                * f64::EPSILON
                * circle.center[0]
                    .abs()
                    .max(circle.center[1].abs())
                    .max(circle.radius);
            if (distance - circle.radius).abs() <= roundoff {
                // A ray on the cylindrical wall cannot certify positive
                // material coverage of the corresponding circular rim.
                return false;
            }
            if distance < circle.radius {
                inside = !inside;
            }
        }
        inside
    }
}

#[cfg(test)]
mod tests;

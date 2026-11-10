//! Project exact planar and round edges against the same scene.

use super::*;
use crate::{
    curved_projection::{Curve, Detail, Edge, Rim, SpatialCurve},
    drawing_events::{self, Harmonic},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundSegment {
    /// Index of the round body in the input slice.
    pub source: usize,
    pub edge: Edge,
}

/// Project planar and round bodies with exact hidden spans on straight
/// edges and circular rims. Both output lists share the segment budget.
pub fn project_scene(
    bodies: &[Body],
    rounds: &[RoundOccluder],
    options: Options,
) -> Result<(Vec<Segment>, Vec<RoundSegment>), ProjectionError> {
    project_scene_bounded(bodies, rounds, options, WORK_LIMIT)
}

fn project_scene_bounded(
    bodies: &[Body],
    rounds: &[RoundOccluder],
    options: Options,
    limit: usize,
) -> Result<(Vec<Segment>, Vec<RoundSegment>), ProjectionError> {
    let mut work = projection_work(&options, limit)?;
    let (planes, uses) = prepare(bodies, &mut work)?;
    let index = Node::build(planes);
    let straight = match &index {
        Some(index) => project_straight(bodies, &uses, index, rounds, &options, &mut work)?,
        None if rounds.is_empty() => return Err(ProjectionError::Invalid),
        None => Vec::new(),
    };
    let detail = options
        .detail
        .map(|(center, radius)| Detail { center, radius });
    let round_limit = options.segment_limit - straight.len();
    let mut curved = Vec::new();
    for (source, round) in rounds.iter().enumerate() {
        work.charge(1)?;
        for edge in round.drawing_edges(options.hidden_lines, detail)? {
            let mut emit = |hidden, curve| {
                if hidden && !options.hidden_lines {
                    return Ok(());
                }
                if matches!(curve, Curve::Line([a, b]) if (a[0]-b[0]).hypot(a[1]-b[1]) <= options.tolerance)
                {
                    return Ok(());
                }
                if curved.len() == round_limit {
                    return Err(ProjectionError::Limit);
                }
                curved.push(RoundSegment {
                    source,
                    edge: Edge { hidden, curve },
                });
                Ok(())
            };
            match edge.curve {
                SpatialCurve::Line(points) => {
                    let Some(line) = Line::new(points, options.tolerance, [usize::MAX; 2]) else {
                        continue;
                    };
                    let span = [0., line.length];
                    let curve = |trim: Span| Curve::Line(trim.map(|at| planar(line.point(at))));
                    if edge.hidden {
                        emit(true, curve(span))?;
                    } else {
                        hide_line(&line, span, index.as_ref(), rounds, Some(source), &mut work)?;
                        publish_spans(
                            &mut work.hidden,
                            span,
                            options.tolerance,
                            |hidden, trim| emit(hidden, curve(trim)),
                        )?;
                    }
                }
                SpatialCurve::Rim(rim) => {
                    let span = rim.ellipse.trim;
                    if edge.hidden {
                        emit(true, rim.trimmed(span))?;
                    } else {
                        hide_rim(rim, index.as_ref(), rounds, source, &mut work)?;
                        publish_spans(
                            &mut work.hidden,
                            span,
                            options.tolerance / rim.ellipse.major,
                            |hidden, trim| emit(hidden, rim.trimmed(trim)),
                        )?;
                    }
                }
            }
        }
    }
    Ok((straight, curved))
}

fn hide_rim(
    rim: Rim,
    index: Option<&Node>,
    rounds: &[RoundOccluder],
    source: usize,
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
        if id != source && (Bounds { min, max }).overlaps(bounds, work.tolerance) {
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
        work.charge(self.boundary.len())?;
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
        work.charge((work.crossings.len() + 1).saturating_mul(self.boundary.len()))?;
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
        inside
    }
}

#[cfg(test)]
mod tests;

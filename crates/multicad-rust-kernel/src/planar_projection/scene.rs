//! Project exact planar and round edges against the same scene.

use super::*;
use crate::{
    curved_projection::{Curve, Detail, Edge, Rim, SpatialCurve, SpatialEdge},
    drawing_events::{self, Harmonic},
};

mod analytic;
mod candidates;
mod depth;
pub use analytic::{AnalyticFace, FaceTrim};
use candidates::apply_openings;
pub use candidates::{
    CircularOpeningEnd, FaceOpening, HiddenCandidate, LineCandidate, RimCandidate, SceneExtras,
    SpatialEllipseCandidate, SpatialRimCandidate, StadiumOpening,
};
pub use depth::DepthRange;
use std::f64::consts::{PI, TAU};

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
    project_scene_with_analytic_faces(bodies, rounds, extras, &[], options)
}

/// Project normal scene candidates against additional finite analytic faces.
/// The faces are blockers only; supply their physical edges and silhouettes
/// as scene candidates to publish their geometry.
pub fn project_scene_with_analytic_faces(
    bodies: &[Body],
    rounds: &[RoundOccluder],
    extras: SceneExtras<'_>,
    analytic_faces: &[AnalyticFace],
    options: Options,
) -> Result<SceneProjection, ProjectionError> {
    project_with_extras(bodies, rounds, extras, analytic_faces, options, WORK_LIMIT)
}

pub(super) type AnalyticIndex<'a> = Node<&'a AnalyticFace>;

impl BoundedFace for &AnalyticFace {
    fn bounds(&self) -> Bounds {
        AnalyticFace::bounds(self)
    }
}

fn project_scene_bounded(
    bodies: &[Body],
    rounds: &[RoundOccluder],
    options: Options,
    limit: usize,
) -> Result<(Vec<Segment>, Vec<RoundSegment>), ProjectionError> {
    let (straight, round, _) =
        project_with_extras(bodies, rounds, SceneExtras::default(), &[], options, limit)?;
    Ok((straight, round))
}

fn project_with_extras(
    bodies: &[Body],
    rounds: &[RoundOccluder],
    extras: SceneExtras<'_>,
    analytic_faces: &[AnalyticFace],
    options: Options,
    limit: usize,
) -> Result<SceneProjection, ProjectionError> {
    let mut work = projection_work(&options, limit)?;
    let (mut planes, uses) = prepare(bodies.iter().chain(extras.blocker_bodies), &mut work)?;
    apply_openings(bodies, &extras, &mut planes, &mut work)?;
    let index = Node::build(planes);
    for face in analytic_faces {
        work.charge(1 + face.build_work())?;
    }
    let analytic = AnalyticIndex::build(analytic_faces.iter().collect());
    let all_rounds;
    let all_scene_rounds = if extras.blocker_rounds.is_empty() {
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
    let blockers = Blockers {
        planes: index.as_ref(),
        analytic: analytic.as_ref(),
        rounds: all_scene_rounds,
    };
    let straight = match &index {
        Some(_) => project_straight(bodies, &uses, blockers, &options, &mut work)?,
        None if analytic.is_none()
            && rounds.is_empty()
            && extras.lines.is_empty()
            && extras.rims.is_empty()
            && extras.spatial_rims.is_empty()
            && extras.spatial_ellipses.is_empty()
            && extras.intrinsic_hidden.is_empty() =>
        {
            return Err(ProjectionError::Invalid);
        }
        None => Vec::new(),
    };
    let detail = options
        .detail
        .map(|(center, radius)| Detail { center, radius });
    let scene = Scene {
        blockers,
        published_rounds: rounds.len(),
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
        scene.supplied_rim(candidate.spatial()?, false, detail, &mut output, &mut work)?;
    }
    let spatial_rims = extras
        .spatial_rims
        .iter()
        .map(|&rim| (rim.source, rim.spatial()))
        .chain(
            extras
                .spatial_ellipses
                .iter()
                .map(|&ellipse| (ellipse.source, ellipse.spatial())),
        );
    for (source, rim) in spatial_rims {
        work.charge(1)?;
        let mut output = Output {
            source,
            own_round: None,
            limit,
            edges: &mut supplied,
        };
        scene.supplied_rim(rim?, false, detail, &mut output, &mut work)?;
    }
    if options.hidden_lines {
        for &candidate in extras.intrinsic_hidden {
            work.charge(1)?;
            let source = match candidate {
                HiddenCandidate::Line(line) => line.source,
                HiddenCandidate::Rim(rim) => rim.source,
                HiddenCandidate::SpatialRim(rim) => rim.source,
            };
            let mut output = Output {
                source,
                own_round: None,
                limit,
                edges: &mut supplied,
            };
            match candidate {
                HiddenCandidate::Line(line) => scene.edge(
                    SpatialEdge {
                        hidden: true,
                        curve: line.spatial()?,
                    },
                    options.detail,
                    &mut output,
                    &mut work,
                )?,
                HiddenCandidate::Rim(rim) => {
                    scene.supplied_rim(rim.spatial()?, true, detail, &mut output, &mut work)?;
                }
                HiddenCandidate::SpatialRim(rim) => {
                    scene.supplied_rim(rim.spatial()?, true, detail, &mut output, &mut work)?;
                }
            }
        }
    }
    Ok((straight, curved, supplied))
}

struct Scene<'a> {
    blockers: Blockers<'a>,
    published_rounds: usize,
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
        hidden: bool,
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
                        hidden,
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
                    hide_line(&line, span, self.blockers, source, work)?;
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
                    hide_rim(rim, self.blockers, self.published_rounds, source, work)?;
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

fn trimmed_rim_bounds([origin, cosine, sine]: [Point; 3], extent: Point, trim: Span) -> Bounds {
    let trigonometry = trim.map(f64::sin_cos);
    let ranges = std::array::from_fn::<_, 3, _>(|i| {
        let [a, b] =
            trigonometry.map(|(sin, cos)| cosine[i].mul_add(cos, sine[i].mul_add(sin, origin[i])));
        let phase = sine[i].atan2(cosine[i]).rem_euclid(TAU);
        let antipode = (phase + PI).rem_euclid(TAU);
        let range = trim[0]..=trim[1];
        let min = if range.contains(&antipode) {
            origin[i] - extent[i]
        } else {
            a.min(b)
        };
        let max = if range.contains(&phase) {
            origin[i] + extent[i]
        } else {
            a.max(b)
        };
        // Enclose numerical variation at wrapped extrema and endpoints.
        let pad = 16. * f64::EPSILON * (origin[i].abs() + extent[i]).max(1.);
        [min - pad, max + pad]
    });
    Bounds {
        min: ranges.map(|range| range[0]),
        max: ranges.map(|range| range[1]),
    }
}

fn hide_rim(
    rim: Rim,
    blockers: Blockers<'_>,
    published_rounds: usize,
    source: Option<usize>,
    work: &mut Work,
) -> Result<(), ProjectionError> {
    work.hidden.clear();
    let [origin, cos, sin] = rim.basis();
    let extent: Point = std::array::from_fn(|i| cos[i].hypot(sin[i]));
    let full_bounds = Bounds {
        min: std::array::from_fn(|i| origin[i] - extent[i]),
        max: std::array::from_fn(|i| origin[i] + extent[i]),
    };
    let trim = rim.ellipse.trim;
    let bounds = if trim == [0., TAU] {
        full_bounds
    } else {
        trimmed_rim_bounds([origin, cos, sin], extent, trim)
    };
    if let Some(index) = blockers.planes {
        index.visit(bounds, work, &mut |face, work| {
            face.hide_rim(rim, full_bounds, work)
        })?;
    }
    if let Some(index) = blockers.analytic {
        index.visit(bounds, work, &mut |face, work| face.hide_rim(rim, work))?;
    }
    for (id, round) in blockers.rounds.iter().enumerate() {
        work.charge(1)?;
        let [min, max] = round.bounds();
        if Some(id) != source && (Bounds { min, max }).overlaps(bounds, work.tolerance) {
            let candidate = source.map(|id| &blockers.rounds[id]);
            if id < published_rounds
                && (source.is_none() || round.distinct_contour_owner(candidate))
                && round.covers_coincident_rim(rim, work.tolerance)
            {
                // A published opaque cap owns its nearer coincident
                // silhouette. Bands of the same solid retain their roles.
                work.hidden.clear();
                work.hidden.push(rim.ellipse.trim);
                return Ok(());
            }
            // Charge the bounded conic and cap root work even when no
            // resulting interval covers the candidate rim.
            work.charge(160)?;
            let contour = round.closes_contour_of(candidate);
            round.hide_rim(rim, work.tolerance, &mut work.crossings, contour, |span| {
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
        let boundaries = self.boundary.len() + self.profiles.len();
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
        for &profile in &self.profiles {
            work.charge(profile.rim_work())?;
            profile.rim_cuts([origin, cos, sin], trim, &mut work.crossings);
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
        for &profile in &self.profiles {
            let distance = profile.distance(point);
            let radius = profile.radius();
            let roundoff = 16.
                * f64::EPSILON
                * profile
                    .centers()
                    .into_iter()
                    .flatten()
                    .map(f64::abs)
                    .fold(radius, f64::max);
            if (distance - radius).abs() <= roundoff {
                // A ray on the cylindrical wall cannot certify positive
                // material coverage of the corresponding circular rim.
                return false;
            }
            if distance < radius {
                inside = !inside;
            }
        }
        inside
    }
}

#[cfg(test)]
mod tests;

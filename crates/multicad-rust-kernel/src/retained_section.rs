//! Exact retained-depth projection of stock and curved profiles swept along
//! a common screen axis. Open bands share one analytic depth envelope.
use crate::{orthogonal::Box3, planar_projection::ProjectionError, stadium::Stadium};
use std::collections::BTreeMap;
mod heights;
mod strips;
use heights::{Height, dot, sort_unique};
use strips::{Collector, Strip};

type Point = [f64; 2];
type Span = [f64; 2];
type Result<T> = std::result::Result<T, ProjectionError>;

const COORDINATE_LIMIT: f64 = 1e8;
const WORK_LIMIT: usize = 4_000_000;

#[derive(Clone, Copy, Debug)]
pub enum Profile {
    Circle { center: Point, radius: f64 },
    Stadium(Stadium),
}

#[derive(Clone, Copy, Debug)]
pub struct AxialProfile {
    /// The two U coordinates spanning the sweep.
    pub span: Span,
    /// Its constant cross section in (V, depth) coordinates.
    pub profile: Profile,
}

/// One independent source solid. The material is the stock and positive
/// sweeps with the cut sweeps removed. Stock cells use (U, V, depth) axes.
#[derive(Debug)]
pub struct Body {
    pub source: usize,
    pub stock: Vec<Box3>,
    pub cuts: Vec<AxialProfile>,
    pub positive: Vec<AxialProfile>,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub depth_mm: f64,
    pub far_mm: Option<f64>,
    pub windows_mm: [Option<Span>; 2],
    pub tolerance_mm: f64,
    pub segment_limit: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub source: usize,
    pub ends: [Point; 2],
}

fn bounded(value: f64) -> bool {
    value.is_finite() && value.abs() <= COORDINATE_LIMIT
}

fn numeric(a: f64, b: f64) -> f64 {
    32. * f64::EPSILON * a.abs().max(b.abs()).max(1.)
}

fn equal(a: f64, b: f64) -> bool {
    a == b || (a.is_finite() && b.is_finite() && (a - b).abs() <= numeric(a, b))
}

fn intersect(a: Span, b: Span) -> Option<Span> {
    let span = [a[0].max(b[0]), a[1].min(b[1])];
    (span[0] < span[1]).then_some(span)
}

fn charge(work: &mut usize, cost: usize) -> Result<()> {
    *work = work.checked_sub(cost).ok_or(ProjectionError::Limit)?;
    Ok(())
}

/// Project visible straight edges behind the cut, looking along decreasing
/// depth. All curved sweeps are perpendicular to the view and project edge-on.
/// Tangent seams, hidden edges and the independent section/hatch roles are
/// outside this operation. Shared screen spans retain source attribution.
pub fn project(bodies: &[Body], options: Options) -> Result<Vec<Segment>> {
    project_bounded(bodies, options, WORK_LIMIT)
}

fn project_bounded(bodies: &[Body], options: Options, mut work: usize) -> Result<Vec<Segment>> {
    let tolerance = options.tolerance_mm;
    let finite = [-COORDINATE_LIMIT, COORDINATE_LIMIT];
    let depth = [options.far_mm.unwrap_or(finite[0]), options.depth_mm];
    if !(1e-7..=1e-3).contains(&tolerance)
        || !depth.into_iter().all(bounded)
        || depth[1] - depth[0] <= tolerance
        || options
            .windows_mm
            .into_iter()
            .flatten()
            .any(|[a, b]| !bounded(a) || !bounded(b) || b - a <= tolerance)
    {
        return Err(ProjectionError::Invalid);
    }
    let window = options.windows_mm.map(|span| span.unwrap_or(finite));
    let mut strips = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        charge(
            &mut work,
            body.stock
                .len()
                .saturating_add(body.cuts.len())
                .saturating_add(body.positive.len())
                .saturating_add(1),
        )?;
        Collector::new(&mut strips, i, window, depth).add(body)?;
    }
    if strips.iter().all(|strip| strip.removed) {
        return Ok(Vec::new());
    }
    let mut u_events: Vec<_> = strips.iter().flat_map(|strip| strip.u).collect();
    sort_unique(&mut u_events);
    // Every support intersection is a V event. Between events the depth
    // ordering is fixed, so a midpoint selects the exact front support.
    let v_events = v_events(&strips, depth, &mut work)?;
    let y_count = v_events.len().saturating_sub(1);
    charge(&mut work, y_count.saturating_mul(bodies.len()))?;
    let empty = vec![vec![None; bodies.len()]; y_count];
    let mut previous = empty.clone();
    let mut result = Output {
        segments: BTreeMap::new(),
        bodies,
        tolerance,
    };
    for interval in u_events.windows(2) {
        let u = [interval[0], interval[1]];
        let band = band(
            &strips,
            u[0].midpoint(u[1]),
            &v_events,
            bodies.len(),
            depth,
            &mut work,
        )?;
        result.vertical(u[0], &v_events, &previous, &band);
        result.horizontal(u, &v_events, &band);
        previous = band;
    }
    result.vertical(
        *u_events.last().expect("visible stock has an axial span"),
        &v_events,
        &previous,
        &empty,
    );
    result.finish(options.segment_limit)
}

fn v_events(strips: &[Strip], depth: Span, work: &mut usize) -> Result<Vec<f64>> {
    let mut values: Vec<_> = strips.iter().flat_map(|strip| strip.v).collect();
    let surfaces: Vec<_> = strips
        .iter()
        .flat_map(|strip| strip.depth.map(|height| (strip.u, strip.v, height)))
        .collect();
    for (i, &(u, v, a)) in surfaces.iter().enumerate() {
        charge(work, surfaces.len() - i + 1)?;
        for level in depth {
            a.intersections(Height::flat(level), v, &mut values);
        }
        for &(other_u, other_v, b) in &surfaces[i + 1..] {
            // Both sides of a shared U plane participate in its visibility.
            if u[0] <= other_u[1]
                && other_u[0] <= u[1]
                && let Some(common) = intersect(v, other_v)
            {
                a.intersections(b, common, &mut values);
            }
        }
    }
    sort_unique(&mut values);
    Ok(values)
}

type Band = Vec<Vec<Option<Height>>>;

fn band(
    strips: &[Strip],
    u: f64,
    v_events: &[f64],
    body_count: usize,
    depth: Span,
    work: &mut usize,
) -> Result<Band> {
    let mut by_body = vec![Vec::new(); body_count];
    charge(work, strips.len())?;
    for strip in strips
        .iter()
        .filter(|strip| strip.u[0] < u && u < strip.u[1])
    {
        by_body[strip.body].push(strip);
    }
    v_events
        .windows(2)
        .map(|v| {
            let v = v[0].midpoint(v[1]);
            by_body
                .iter()
                .map(|active| {
                    charge(work, active.len().saturating_mul(2).saturating_add(1))?;
                    Ok(top(active, v, depth))
                })
                .collect()
        })
        .collect()
}

#[derive(Clone, Copy)]
struct Endpoint {
    depth: f64,
    edge: Height,
    removed: bool,
    entering: bool,
}

fn top(active: &[&Strip], v: f64, depth: Span) -> Option<Height> {
    let [far, near] = depth;
    let mut ends = Vec::new();
    for strip in active
        .iter()
        .filter(|strip| strip.v[0] < v && v < strip.v[1])
    {
        let [lo, hi] = strip.depth.map(|edge| edge.at(v));
        let Some([lo, hi]) = intersect([lo, hi], depth) else {
            continue;
        };
        let edges = [
            if lo == far {
                Height::flat(far)
            } else {
                strip.depth[0]
            },
            if hi == near {
                Height::flat(near)
            } else {
                strip.depth[1]
            },
        ];
        for (i, depth) in [lo, hi].into_iter().enumerate() {
            ends.push(Endpoint {
                depth,
                edge: edges[i],
                removed: strip.removed,
                entering: i == 1,
            });
        }
    }
    ends.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    // Sweep away from the viewer. The first material interval outside all
    // cut intervals supplies the upper boundary for this source.
    let mut material = 0_i32;
    let mut voids = 0_i32;
    let mut remaining = ends.as_slice();
    while let [first, ..] = remaining {
        let equal_depth = remaining.partition_point(|endpoint| equal(endpoint.depth, first.depth));
        for event in &remaining[..equal_depth] {
            let count = if event.removed {
                &mut voids
            } else {
                &mut material
            };
            *count += if event.entering { 1 } else { -1 };
        }
        remaining = &remaining[equal_depth..];
        if material > 0 && voids == 0 && !remaining.is_empty() {
            return Some(first.edge);
        }
    }
    None
}

fn front(a: &[Option<Height>], b: &[Option<Height>], v: f64) -> f64 {
    a.iter()
        .chain(b)
        .flatten()
        .map(|h| h.at(v))
        .fold(f64::NEG_INFINITY, f64::max)
}

struct Output<'a> {
    segments: BTreeMap<(usize, usize, u64), Vec<Span>>,
    bodies: &'a [Body],
    tolerance: f64,
}

impl Output<'_> {
    fn add(&mut self, body: usize, axis: usize, coordinate: f64, span: Span) {
        let key = if coordinate == 0. {
            0
        } else {
            coordinate.to_bits()
        };
        self.segments
            .entry((self.bodies[body].source, axis, key))
            .or_default()
            .push(span);
    }

    fn vertical(&mut self, u: f64, v_events: &[f64], left: &Band, right: &Band) {
        for ((v, left), right) in v_events.windows(2).zip(left).zip(right) {
            let sample = v[0].midpoint(v[1]);
            // A neighboring source can cover the rear of an edge-on recess
            // even when its footprint meets this source only at the seam.
            let foreground = front(left, right, sample);
            for (i, (&a, &b)) in left.iter().zip(right).enumerate() {
                let [a, b] =
                    [a, b].map(|height| height.map_or(f64::NEG_INFINITY, |h| h.at(sample)));
                if !equal(a, b) && equal(a.max(b), foreground) {
                    self.add(i, 0, u, [v[0], v[1]]);
                }
            }
        }
    }

    fn horizontal(&mut self, u: Span, v_events: &[f64], band: &Band) {
        let empty = vec![None; self.bodies.len()];
        let left = std::iter::once(&empty).chain(band.iter());
        let right = band.iter().chain(std::iter::once(&empty));
        for ((&v, left), right) in v_events.iter().zip(left).zip(right) {
            let foreground = front(left, right, v);
            for (i, (&a, &b)) in left.iter().zip(right).enumerate() {
                if let Some(depth) = crease(a, b, v)
                    && equal(depth, foreground)
                {
                    self.add(i, 1, v, u);
                }
            }
        }
    }

    fn finish(self, budget: usize) -> Result<Vec<Segment>> {
        let mut output = Vec::new();
        for ((source, axis, key), mut spans) in self.segments {
            spans.sort_unstable_by(|a, b| a[0].total_cmp(&b[0]));
            let mut merged: Vec<Span> = Vec::new();
            for span in spans {
                if let Some(previous) = merged.last_mut()
                    && (span[0] <= previous[1] || equal(span[0], previous[1]))
                {
                    previous[1] = previous[1].max(span[1]);
                } else {
                    merged.push(span);
                }
            }
            merged.retain(|span| span[1] - span[0] > self.tolerance);
            if output.len() + merged.len() > budget {
                return Err(ProjectionError::Limit);
            }
            let fixed = f64::from_bits(key);
            output.extend(merged.into_iter().map(|span| Segment {
                source,
                ends: span.map(|moving| {
                    if axis == 0 {
                        [fixed, moving]
                    } else {
                        [moving, fixed]
                    }
                }),
            }));
        }
        Ok(output)
    }
}

fn crease(a: Option<Height>, b: Option<Height>, v: f64) -> Option<f64> {
    match (a, b) {
        (None, None) => None,
        (Some(h), None) | (None, Some(h)) => Some(h.at(v)),
        (Some(a), Some(b)) => {
            let [lo, hi] = [a.at(v), b.at(v)];
            if equal(lo, hi) && dot(a.tangent(v), b.tangent(v)) >= 1. - 64. * f64::EPSILON {
                None
            } else {
                Some(lo.max(hi))
            }
        }
    }
}

#[cfg(test)]
mod tests;

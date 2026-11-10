//! Exact retained-depth projection of stock and curved profiles swept along
//! a common screen axis. Open bands share one analytic depth envelope.
use crate::{orthogonal::Box3, planar_projection::ProjectionError, stadium::Stadium};
use std::collections::BTreeMap;
mod crossed;
mod heights;
mod strips;
pub use crossed::project_crossed;
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
    let scene = Scene::new(bodies, options, &mut work)?;
    let mut output = Output::new(options.tolerance_mm);
    scene.scan(&mut output, &mut work)?;
    output.finish(options.segment_limit)
}

struct Scene<'a> {
    bodies: &'a [Body],
    strips: Vec<Strip>,
    u: Vec<f64>,
    v: Vec<f64>,
    depth: Span,
}

impl<'a> Scene<'a> {
    fn new(bodies: &'a [Body], options: Options, work: &mut usize) -> Result<Self> {
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
                work,
                body.stock
                    .len()
                    .saturating_add(body.cuts.len())
                    .saturating_add(body.positive.len())
                    .saturating_add(1),
            )?;
            Collector::new(&mut strips, i, window, depth).add(body)?;
        }
        if strips.iter().all(|strip| strip.removed) {
            strips.clear();
        }
        let mut u: Vec<_> = strips.iter().flat_map(|strip| strip.u).collect();
        sort_unique(&mut u);
        // Between support intersections the depth ordering is fixed.
        let v = v_events(&strips, depth, work)?;
        Ok(Self {
            bodies,
            strips,
            u,
            v,
            depth,
        })
    }

    fn scan(&self, output: &mut impl ScanOutput, work: &mut usize) -> Result<()> {
        let Some(&last) = self.u.last() else {
            return Ok(());
        };
        let y_count = self.v.len().saturating_sub(1);
        charge(work, y_count.saturating_mul(self.bodies.len()))?;
        let empty = vec![vec![None; self.bodies.len()]; y_count];
        let mut previous = empty.clone();
        for interval in self.u.windows(2) {
            let u = [interval[0], interval[1]];
            let band = band(
                &self.strips,
                u[0].midpoint(u[1]),
                &self.v,
                self.bodies.len(),
                self.depth,
                work,
            )?;
            self.vertical(output, u[0], &previous, &band);
            self.horizontal(output, u, &band);
            output.band(&self.v, &band);
            previous = band;
        }
        self.vertical(output, last, &previous, &empty);
        Ok(())
    }

    fn vertical(&self, output: &mut impl ScanOutput, u: f64, left: &Band, right: &Band) {
        for ((v, left), right) in self.v.windows(2).zip(left).zip(right) {
            let sample = v[0].midpoint(v[1]);
            // A neighboring source can cover the rear of an edge-on recess
            // even when its footprint meets this source only at the seam.
            let foreground = front(left, right, sample);
            for (i, (&a, &b)) in left.iter().zip(right).enumerate() {
                let [za, zb] =
                    [a, b].map(|height| height.map_or(f64::NEG_INFINITY, |h| h.at(sample)));
                if !equal(za, zb)
                    && equal(za.max(zb), foreground)
                    && let Some(height) = if za > zb { a } else { b }
                {
                    output.edge(Candidate {
                        source: self.bodies[i].source,
                        axis: 0,
                        fixed: u,
                        span: [v[0], v[1]],
                        height,
                    });
                }
            }
        }
    }

    fn horizontal(&self, output: &mut impl ScanOutput, u: Span, band: &Band) {
        let empty = vec![None; self.bodies.len()];
        let left = std::iter::once(&empty).chain(band.iter());
        let right = band.iter().chain(std::iter::once(&empty));
        for ((&v, left), right) in self.v.iter().zip(left).zip(right) {
            let foreground = front(left, right, v);
            for (i, (&a, &b)) in left.iter().zip(right).enumerate() {
                if let Some(depth) = crease(a, b, v)
                    && equal(depth, foreground)
                {
                    output.edge(Candidate {
                        source: self.bodies[i].source,
                        axis: 1,
                        fixed: v,
                        span: u,
                        height: Height::flat(depth),
                    });
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Candidate {
    source: usize,
    /// Fixed screen axis. The height function uses the other coordinate.
    axis: usize,
    fixed: f64,
    span: Span,
    height: Height,
}

trait ScanOutput {
    fn edge(&mut self, edge: Candidate);
    fn band(&mut self, _v_events: &[f64], _band: &Band) {}
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

struct Output {
    segments: BTreeMap<(usize, usize, u64), Vec<Span>>,
    tolerance: f64,
}

impl ScanOutput for Output {
    fn edge(&mut self, edge: Candidate) {
        let key = if edge.fixed == 0. {
            0
        } else {
            edge.fixed.to_bits()
        };
        self.segments
            .entry((edge.source, edge.axis, key))
            .or_default()
            .push(edge.span);
    }
}

impl Output {
    fn new(tolerance: f64) -> Self {
        Self {
            segments: BTreeMap::new(),
            tolerance,
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

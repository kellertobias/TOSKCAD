//! Finite plane and circular cylinder faces in drawing coordinates.
use super::*;
use crate::{curved_projection::roots, trim_curve::EllipseArc};
use std::f64::consts::{FRAC_PI_2, TAU};

const ANGULAR: f64 = 64. * f64::EPSILON;
const MAX_TRIMS: usize = 8192;

/// One oriented trim in drawing coordinates (right, up, depth).
#[derive(Clone, Copy, Debug)]
pub enum FaceTrim {
    Line([Point; 2]),
    Arc(EllipseArc),
}

/// A bounded outward face. Circular walls of an aperture have inward
/// radial normals, so their opposite branch supplies the first material hit.
pub struct AnalyticFace {
    surface: Surface,
    bounds: Bounds,
    // Actual limits, excluding caps that cannot cut this face's depth bounds.
    depth: [Option<f64>; 2],
}

enum Surface {
    Plane(Plane),
    Cylinder(Cylinder),
}

struct Plane {
    origin: Point,
    normal: Point,
    // Project a ray hit onto the face without dividing by normal.z.
    chart: [Point; 2],
    boundary: Vec<Boundary>,
    roundoff: f64,
}

#[derive(Clone, Copy)]
enum Boundary {
    Line([Point2; 2]),
    Arc {
        ends: [Point2; 2],
        center: Point2,
        radius: f64,
        side: f64,
    },
    Ellipse {
        ends: [Point2; 2],
        trim: EllipseTrim,
    },
}

#[derive(Clone, Copy)]
struct EllipseTrim {
    center: Point2,
    inverse: [Point2; 2],
    height: f64,
    horizontal: Point2,
}

struct Cylinder {
    origin: Point,
    screen_axis: Point,
    transverse: Point,
    ray_radial: Point,
    projected: f64,
    axial_depth: f64,
    stations: [[f64; 3]; 2],
    radius: f64,
    radial_ends: [Point2; 2],
    facing: f64,
}

impl AnalyticFace {
    /// Prepare a plane with one outer loop and any number of inner loops.
    /// The input loops may have either winding; finite coverage uses parity.
    /// Back-facing and edge-on faces do not obscure material.
    pub fn plane(
        origin: Point,
        outward_normal: Point,
        rings: Vec<Vec<FaceTrim>>,
        tolerance: f64,
    ) -> Result<Option<Self>, ProjectionError> {
        validate(origin, outward_normal, tolerance)?;
        if rings.is_empty() || rings.iter().any(Vec::is_empty) {
            return Err(ProjectionError::Invalid);
        }
        if rings.iter().map(Vec::len).sum::<usize>() > MAX_TRIMS {
            return Err(ProjectionError::Limit);
        }
        let normal = unit(outward_normal);
        if normal[2] <= ANGULAR {
            return Ok(None);
        }
        let reference = if normal[0].abs() <= normal[1].abs() {
            [1., 0., 0.]
        } else {
            [0., 1., 0.]
        };
        let u = unit(cross(reference, normal));
        let v = cross(normal, u);
        let chart = [u, v].map(|basis| {
            std::array::from_fn(|i| {
                if i == 2 {
                    0.
                } else {
                    normal[2].mul_add(basis[i], -basis[2] * normal[i])
                }
            })
        });
        let mut bounds = None::<Bounds>;
        let mut boundary = Vec::new();
        let local = |point| {
            if !bounded(point) {
                return Err(ProjectionError::Invalid);
            }
            let delta = sub(point, origin);
            if dot(delta, normal).abs() > tolerance {
                return Err(ProjectionError::Invalid);
            }
            Ok([u, v].map(|axis| dot(axis, delta) * normal[2]))
        };
        let mut extend = |b: Bounds| bounds = Some(bounds.map_or(b, |previous| previous.union(b)));
        for ring in rings {
            let mut trims = Vec::new();
            for trim in ring {
                match trim {
                    FaceTrim::Line([a, b]) => {
                        let ends = [local(a)?, local(b)?];
                        if length(sub(b, a)) <= tolerance {
                            return Err(ProjectionError::Invalid);
                        }
                        extend(Bounds::point(a).union(Bounds::point(b)));
                        trims.push(Boundary::Line(ends));
                    }
                    FaceTrim::Arc(arc) => {
                        let ellipse = arc.ellipse();
                        let [major, minor] = ellipse.semiaxes_mm();
                        if minor <= tolerance || length(cross(ellipse.normal(), normal)) > ANGULAR {
                            return Err(ProjectionError::Invalid);
                        }
                        let center = local(ellipse.center_mm())?;
                        let [a, b] = arc.endpoints();
                        let ends = [local(a)?, local(b)?];
                        if major == minor {
                            let [x, y] = sub(ends[0], center);
                            let start = y.atan2(x);
                            let sweep = arc.sweep_rad() * dot(ellipse.normal(), normal).signum();
                            arc_pieces(center, major * normal[2], start, sweep, ends, &mut trims);
                        } else {
                            let basis = [ellipse.major_axis(), ellipse.minor_axis()];
                            let radii = [major, minor];
                            let axes = std::array::from_fn(|i| {
                                [u, v].map(|axis| dot(axis, basis[i]) * (radii[i] * normal[2]))
                            });
                            ellipse_pieces(center, axes, arc, ends, &mut trims);
                        }
                        extend(arc_bounds(arc));
                    }
                }
            }
            let count = trims.len();
            for i in 0..count {
                let next = trims[(i + 1) % count].ends()[0];
                let end = trims[i].ends()[1];
                let [x, y] = sub(next, end);
                if x.hypot(y) > tolerance * normal[2] {
                    return Err(ProjectionError::Invalid);
                }
                trims[i].ends_mut()[1] = next;
            }
            boundary.extend(trims);
        }
        let bounds = bounds.ok_or(ProjectionError::Invalid)?;
        let roundoff = coordinate_roundoff(bounds) * normal[2];
        Ok(Some(Self {
            surface: Surface::Plane(Plane {
                origin,
                normal,
                chart,
                boundary,
                roundoff,
            }),
            bounds,
            depth: [None; 2],
        }))
    }

    /// Prepare one circular patch with an angular span up to a quarter turn.
    /// The positive axis goes from the supplied cap to the opposite cap.
    /// A cylinder along the viewing axis has no projected wall area.
    pub fn cylinder(
        cap: EllipseArc,
        axis: Point,
        height: f64,
        outward: bool,
        tolerance: f64,
    ) -> Result<Option<Self>, ProjectionError> {
        Self::cylinder_between(cap, axis, height, [None; 2], outward, tolerance)
    }

    /// Prepare a cylindrical sector from a perpendicular circle trim to a
    /// tilted ellipse trim on the same cylinder.
    pub fn tilted_cylinder(
        cap: EllipseArc,
        end: EllipseArc,
        axis: Point,
        outward: bool,
        tolerance: f64,
    ) -> Result<Option<Self>, ProjectionError> {
        let height = dot(
            sub(end.ellipse().center_mm(), cap.ellipse().center_mm()),
            unit(axis),
        );
        Self::cylinder_between(cap, axis, height, [None, Some(end)], outward, tolerance)
    }

    /// Prepare a cylindrical sector with two oblique trims. The circular
    /// chart lies at the first rim center and gives the azimuth of both trims.
    pub fn cylinder_with_trims(
        chart: EllipseArc,
        trims: [EllipseArc; 2],
        axis: Point,
        outward: bool,
        tolerance: f64,
    ) -> Result<Option<Self>, ProjectionError> {
        let height = dot(
            sub(trims[1].ellipse().center_mm(), chart.ellipse().center_mm()),
            unit(axis),
        );
        Self::cylinder_between(chart, axis, height, trims.map(Some), outward, tolerance)
    }

    fn cylinder_between(
        cap: EllipseArc,
        axis: Point,
        height: f64,
        trims: [Option<EllipseArc>; 2],
        outward: bool,
        tolerance: f64,
    ) -> Result<Option<Self>, ProjectionError> {
        let ellipse = cap.ellipse();
        let origin = ellipse.center_mm();
        validate(origin, axis, tolerance)?;
        let axis = unit(axis);
        let [radius, minor] = ellipse.semiaxes_mm();
        if radius != minor
            || !(tolerance..=1e7).contains(&radius)
            || !(tolerance..=1e7).contains(&height)
            || cap.sweep_rad().abs() > FRAC_PI_2 + ANGULAR
            || length(cross(axis, ellipse.normal())) > ANGULAR
        {
            return Err(ProjectionError::Invalid);
        }
        let projected = axis[0].hypot(axis[1]);
        if projected <= ANGULAR {
            return Ok(None);
        }
        let screen_axis = [axis[0] / projected, axis[1] / projected, 0.];
        let (axis, projected, axial_depth) = if axis[2].abs() <= ANGULAR {
            (screen_axis, 1., 0.)
        } else {
            (axis, projected, axis[2])
        };
        let transverse = [-screen_axis[1], screen_axis[0], 0.];
        let ray_radial = [
            -axial_depth * screen_axis[0],
            -axial_depth * screen_axis[1],
            projected,
        ];
        let mut radial_ends = [cap.start_rad(), cap.start_rad() + cap.sweep_rad()].map(|angle| {
            let (sin, cos) = angle.sin_cos();
            let u = ellipse.major_axis();
            let v = ellipse.minor_axis();
            let radial = std::array::from_fn(|i| cos.mul_add(u[i], sin * v[i]));
            [dot(radial, transverse), dot(radial, ray_radial)]
        });
        let turn = cross2(radial_ends[0], radial_ends[1]);
        if turn.abs() <= ANGULAR {
            return Err(ProjectionError::Invalid);
        }
        if turn < 0. {
            radial_ends.reverse();
        }
        let a = arc_bounds(cap);
        let clip = |rim: Option<EllipseArc>, height: f64| {
            let Some(end) = rim else {
                return Ok((
                    Bounds {
                        min: std::array::from_fn(|i| axis[i].mul_add(height, a.min[i])),
                        max: std::array::from_fn(|i| axis[i].mul_add(height, a.max[i])),
                    },
                    [height, 0., 0.],
                ));
            };
            let ellipse = end.ellipse();
            let normal = ellipse.normal();
            let alignment = dot(normal, axis);
            let [major, minor] = ellipse.semiaxes_mm();
            let shift = sub(ellipse.center_mm(), origin);
            if alignment.abs() <= ANGULAR
                || length(sub(shift, axis.map(|value| value * height))) > tolerance
                || (minor - radius).abs() > tolerance
                || (major * alignment.abs() - radius).abs() > tolerance
                || dot(ellipse.minor_axis(), axis).abs() > ANGULAR
            {
                return Err(ProjectionError::Invalid);
            }
            let radial = |point| {
                let delta = sub(point, origin);
                [dot(delta, transverse), dot(delta, ray_radial)]
            };
            let start = cap.endpoints().map(radial);
            let finish = end.endpoints().map(radial);
            let matches = |points: [Point2; 2]| {
                start
                    .into_iter()
                    .zip(points)
                    .all(|([x, y], [u, v])| (x - u).hypot(y - v) <= tolerance)
            };
            let middle = radial(ellipse.point_at(end.start_rad() + end.sweep_rad() * 0.5));
            let middle = middle.map(|value| value / radius);
            if (!matches(finish) && !matches([finish[1], finish[0]]))
                || cross2(radial_ends[0], middle) < -ANGULAR
                || cross2(middle, radial_ends[1]) < -ANGULAR
            {
                return Err(ProjectionError::Invalid);
            }
            let y = -dot(normal, transverse) / alignment;
            let q = -dot(normal, ray_radial) / alignment;
            Ok((arc_bounds(end), [height, y, q]))
        };
        let (a, lower) = clip(trims[0], 0.)?;
        let (b, upper) = clip(trims[1], height)?;
        let swing = radius * (upper[1] - lower[1]).hypot(upper[2] - lower[2]);
        if trims[1].is_some() && height - swing <= tolerance {
            return Err(ProjectionError::Invalid);
        }
        Ok(Some(Self {
            surface: Surface::Cylinder(Cylinder {
                origin,
                screen_axis,
                transverse,
                ray_radial,
                projected,
                axial_depth,
                stations: [lower, upper],
                radius,
                radial_ends,
                facing: if outward { 1. } else { -1. },
            }),
            bounds: a.union(b),
            depth: [None; 2],
        }))
    }

    /// Keep only ray hits within a retained view-depth interval. This clips
    /// the existing face; callers supply any newly exposed section caps.
    pub fn retained(mut self, range: DepthRange) -> Option<Self> {
        let [lo, hi] = [self.bounds.min[2], self.bounds.max[2]];
        if range.near < lo || range.far.is_some_and(|far| far > hi) {
            return None;
        }
        if let Some(far) = range.far.filter(|&far| far > lo) {
            self.bounds.min[2] = far;
            self.depth[0] = Some(far);
        }
        if range.near < hi {
            self.bounds.max[2] = range.near;
            self.depth[1] = Some(range.near);
        }
        Some(self)
    }

    pub(super) fn bounds(&self) -> Bounds {
        self.bounds
    }

    pub(super) fn build_work(&self) -> usize {
        match &self.surface {
            Surface::Plane(plane) => plane.boundary.len(),
            Surface::Cylinder(_) => 1,
        }
    }

    pub(in crate::planar_projection) fn hide_line(
        &self,
        line: &Line,
        span: Span,
        work: &mut Work,
    ) -> Result<(), ProjectionError> {
        self.hide(
            Path {
                origin: line.start,
                b: line.delta.map(|value| value / line.length),
                c: [0.; 3],
                harmonic: false,
            },
            span,
            work,
        )
    }

    pub(super) fn hide_rim(&self, rim: Rim, work: &mut Work) -> Result<(), ProjectionError> {
        let [origin, b, c] = rim.basis();
        self.hide(
            Path {
                origin,
                b,
                c,
                harmonic: true,
            },
            rim.ellipse.trim,
            work,
        )
    }

    fn hide(&self, path: Path, span: Span, work: &mut Work) -> Result<(), ProjectionError> {
        let roundoff = coordinate_roundoff(self.bounds).max(path.roundoff());
        let depth_gap = work.tolerance.max(roundoff);
        work.crossings.clear();
        match &self.surface {
            Surface::Plane(plane) => plane.hide(path, span, depth_gap, self.depth, work),
            Surface::Cylinder(cylinder) => {
                cylinder.hide(path, span, depth_gap, roundoff, self.depth, work)
            }
        }
    }
}

fn validate(point: Point, direction: Point, tolerance: f64) -> Result<(), ProjectionError> {
    if !bounded(point)
        || !(1. - 1e-9..=1. + 1e-9).contains(&dot(direction, direction))
        || !(1e-7..=1e-3).contains(&tolerance)
    {
        return Err(ProjectionError::Invalid);
    }
    Ok(())
}

fn bounded(point: Point) -> bool {
    point.into_iter().all(|v| v.is_finite() && v.abs() <= 1e8)
}

fn length([x, y, z]: Point) -> f64 {
    x.hypot(y).hypot(z)
}

fn unit(vector: Point) -> Point {
    let length = length(vector);
    vector.map(|v| v / length)
}

fn cross(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[(i + 1) % 3] * b[(i + 2) % 3] - a[(i + 2) % 3] * b[(i + 1) % 3])
}

fn cross2(a: Point2, b: Point2) -> f64 {
    a[0].mul_add(b[1], -a[1] * b[0])
}

fn coordinate_roundoff(bounds: Bounds) -> f64 {
    16. * f64::EPSILON
        * bounds
            .min
            .into_iter()
            .chain(bounds.max)
            .map(f64::abs)
            .fold(1., f64::max)
}

fn arc_bounds(arc: EllipseArc) -> Bounds {
    let ranges = std::array::from_fn::<_, 3, _>(|i| {
        let axis = std::array::from_fn(|j| f64::from(i == j));
        arc.range_along(axis)
    });
    Bounds {
        min: ranges.map(|range| range[0]),
        max: ranges.map(|range| range[1]),
    }
}

fn arc_pieces(
    center: Point2,
    radius: f64,
    start: f64,
    sweep: f64,
    [mut a, b]: [Point2; 2],
    output: &mut Vec<Boundary>,
) {
    let mut previous: f64 = 0.;
    for travel in monotone_steps(start, sweep, [FRAC_PI_2, 3. * FRAC_PI_2]) {
        let angle = start + sweep.signum() * travel;
        let next = if travel == sweep.abs() {
            b
        } else {
            let (sin, cos) = angle.sin_cos();
            [
                radius.mul_add(cos, center[0]),
                radius.mul_add(sin, center[1]),
            ]
        };
        let middle = start + sweep.signum() * previous.midpoint(travel);
        output.push(Boundary::Arc {
            ends: [a, next],
            center,
            radius,
            side: middle.cos().signum(),
        });
        a = next;
        previous = travel;
    }
}

fn monotone_steps(start: f64, sweep: f64, extrema: [f64; 2]) -> impl Iterator<Item = f64> {
    let mut cuts = extrema.map(|angle| ((angle - start) * sweep.signum()).rem_euclid(TAU));
    cuts.sort_unstable_by(f64::total_cmp);
    cuts.into_iter()
        .filter(move |&t| t > ANGULAR && t < sweep.abs() - ANGULAR)
        .chain([sweep.abs()])
}

fn ellipse_pieces(
    center: Point2,
    [a, b]: [Point2; 2],
    arc: EllipseArc,
    [mut first, last]: [Point2; 2],
    output: &mut Vec<Boundary>,
) {
    // Invert the two scaled support vectors for quadratic or quartic
    // intersection events. In a y-monotone piece only one sign of the
    // ellipse's horizontal root is possible.
    let det = cross2(a, b);
    let inverse = [[b[1] / det, -b[0] / det], [-a[1] / det, a[0] / det]];
    let height = a[1].hypot(b[1]);
    let phase = b[1].atan2(a[1]);
    let (sin, cos) = phase.sin_cos();
    let horizontal = [
        a[0].mul_add(cos, b[0] * sin),
        (-a[0]).mul_add(sin, b[0] * cos),
    ];
    let start = arc.start_rad();
    let sweep = arc.sweep_rad();
    let mut previous: f64 = 0.;
    for travel in monotone_steps(start, sweep, [phase, phase + 2. * FRAC_PI_2]) {
        let next = if travel == sweep.abs() {
            last
        } else {
            let (sin, cos) = (start + sweep.signum() * travel).sin_cos();
            std::array::from_fn(|i| a[i].mul_add(cos, b[i].mul_add(sin, center[i])))
        };
        let midpoint = start + sweep.signum() * previous.midpoint(travel);
        let side = (midpoint - phase).sin().signum();
        output.push(Boundary::Ellipse {
            ends: [first, next],
            trim: EllipseTrim {
                center,
                inverse,
                height,
                horizontal: [horizontal[0], side * horizontal[1]],
            },
        });
        first = next;
        previous = travel;
    }
}

impl Boundary {
    fn ends(&self) -> [Point2; 2] {
        match *self {
            Self::Line(ends) | Self::Arc { ends, .. } | Self::Ellipse { ends, .. } => ends,
        }
    }

    fn ends_mut(&mut self) -> &mut [Point2; 2] {
        match self {
            Self::Line(ends) | Self::Arc { ends, .. } | Self::Ellipse { ends, .. } => ends,
        }
    }

    fn ray(self, point: Point2, roundoff: f64) -> Ray {
        let [a, b] = self.ends();
        let [x, y] = point;
        let crosses = (a[1] > y) != (b[1] > y);
        match self {
            Self::Line(_) => {
                let edge = sub(b, a);
                let delta = sub(point, a);
                let length = edge[0].hypot(edge[1]);
                let cross = cross2(edge, delta);
                let along = dot(edge, delta);
                let margin = roundoff * length;
                if cross.abs() <= margin && (-margin..=length * length + margin).contains(&along) {
                    Ray::Boundary
                } else if crosses && (cross > 0.) == (edge[1] > 0.) {
                    Ray::Crossing
                } else {
                    Ray::None
                }
            }
            Self::Arc {
                center,
                radius,
                side,
                ..
            } => {
                let offset = (y - center[1]).abs();
                if y < a[1].min(b[1]) - roundoff || y > a[1].max(b[1]) + roundoff {
                    return Ray::None;
                }
                let width = ((radius - offset).max(0.) * (radius + offset)).sqrt();
                let hit = side.mul_add(width, center[0]);
                if (hit - x).abs() <= roundoff {
                    Ray::Boundary
                } else if crosses && x < hit {
                    Ray::Crossing
                } else {
                    Ray::None
                }
            }
            Self::Ellipse { trim, .. } => {
                if y < a[1].min(b[1]) - roundoff || y > a[1].max(b[1]) + roundoff {
                    return Ray::None;
                }
                let cosine = ((y - trim.center[1]) / trim.height).clamp(-1., 1.);
                let abs = cosine.abs();
                let sine = ((1. - abs) * (1. + abs)).sqrt();
                let [p, q] = trim.horizontal;
                let hit = p.mul_add(cosine, q.mul_add(sine, trim.center[0]));
                if (hit - x).abs() <= roundoff {
                    Ray::Boundary
                } else if crosses && x < hit {
                    Ray::Crossing
                } else {
                    Ray::None
                }
            }
        }
    }
}

enum Ray {
    None,
    Crossing,
    Boundary,
}

#[derive(Clone, Copy)]
struct Path {
    origin: Point,
    b: Point,
    c: Point,
    harmonic: bool,
}

impl Path {
    fn coordinate(self, origin: Point, axis: Point) -> Trace {
        Trace {
            coefficients: [
                dot(sub(self.origin, origin), axis),
                dot(self.b, axis),
                dot(self.c, axis),
            ],
            harmonic: self.harmonic,
        }
    }

    fn roundoff(self) -> f64 {
        16. * f64::EPSILON
            * self
                .origin
                .into_iter()
                .chain(self.b)
                .chain(self.c)
                .map(f64::abs)
                .fold(1., f64::max)
    }
}

#[derive(Clone, Copy)]
struct Trace {
    coefficients: [f64; 3],
    harmonic: bool,
}

impl Trace {
    fn at(self, t: f64) -> f64 {
        if self.harmonic {
            Harmonic(self.coefficients).at(t)
        } else {
            self.coefficients[1].mul_add(t, self.coefficients[0])
        }
    }

    fn offset(mut self, offset: f64) -> Self {
        self.coefficients[0] += offset;
        self
    }

    fn combine(self, a: f64, other: Self, b: f64) -> Self {
        Self {
            coefficients: std::array::from_fn(|i| {
                a.mul_add(self.coefficients[i], b * other.coefficients[i])
            }),
            harmonic: self.harmonic,
        }
    }

    fn scale(mut self, factor: f64) -> Self {
        self.coefficients = self.coefficients.map(|value| value * factor);
        self
    }

    fn level(self, value: f64, span: Span, cuts: &mut Vec<f64>) {
        if self.harmonic {
            Harmonic(self.coefficients).level(value, span, cuts);
        } else {
            let [a, b, _] = self.coefficients;
            if b != 0. {
                let at = (value - a) / b;
                if at > span[0] && at < span[1] {
                    cuts.push(at);
                }
            }
        }
    }

    fn circle(self, other: Self, radius: f64, span: Span, cuts: &mut Vec<f64>) {
        if self.harmonic {
            drawing_events::circle(
                Harmonic(self.coefficients),
                Harmonic(other.coefficients),
                radius,
                span,
                cuts,
            );
        } else {
            // Normalize the parameter to the candidate span before solving,
            // so a long edge cannot lose its quadratic terms to root scaling.
            let scale = span[1] - span[0];
            let start = [self.at(span[0]), other.at(span[0])];
            let delta = [self.coefficients[1] * scale, other.coefficients[1] * scale];
            let polynomial = [
                dot(start, start) - radius * radius,
                2. * dot(start, delta),
                dot(delta, delta),
            ];
            let extremum = -polynomial[1] / (2. * polynomial[2]);
            cuts.extend(
                roots(&polynomial, [0., 1.])
                    .into_iter()
                    .chain([extremum])
                    .filter(|&t| t > 0. && t < 1.)
                    .map(|t| scale.mul_add(t, span[0])),
            );
        }
    }
}

impl Plane {
    fn hide(
        &self,
        path: Path,
        span: Span,
        gap: f64,
        limits: [Option<f64>; 2],
        work: &mut Work,
    ) -> Result<(), ProjectionError> {
        let depth = path.coordinate(self.origin, self.normal);
        let threshold = -gap * self.normal[2];
        let [u, v] = self.chart.map(|axis| path.coordinate(self.origin, axis));
        depth.level(threshold, span, &mut work.crossings);
        let clip = (limits != [None; 2]).then(|| {
            let [nx, ny, nz] = self.normal;
            // The ray hit's relative depth times nz. Keeping the expression
            // scaled avoids division for a nearly edge-on supporting plane.
            let hit = path.coordinate(self.origin, [-nx, -ny, 0.]);
            let limits = limits.map(|limit| limit.map(|z| (z - self.origin[2]) * nz));
            for level in limits.into_iter().flatten() {
                hit.level(level, span, &mut work.crossings);
            }
            (
                hit,
                limits[0].unwrap_or(f64::NEG_INFINITY)..=limits[1].unwrap_or(f64::INFINITY),
            )
        });
        for &boundary in &self.boundary {
            match boundary {
                Boundary::Line([a, b]) => {
                    work.charge(1)?;
                    let [x, y] = sub(b, a);
                    u.offset(-a[0]).combine(-y, v.offset(-a[1]), x).level(
                        0.,
                        span,
                        &mut work.crossings,
                    );
                    // A candidate may coincide with a straight boundary;
                    // the end events still limit its covered interval.
                    let axis = usize::from(y.abs() > x.abs());
                    let coordinate = [u, v][axis];
                    for end in [a, b] {
                        coordinate.level(end[axis], span, &mut work.crossings);
                    }
                }
                Boundary::Arc {
                    ends,
                    center,
                    radius,
                    ..
                } => {
                    work.charge(if path.harmonic { 80 } else { 8 })?;
                    u.offset(-center[0]).circle(
                        v.offset(-center[1]),
                        radius,
                        span,
                        &mut work.crossings,
                    );
                    for end in ends {
                        v.level(end[1], span, &mut work.crossings);
                    }
                }
                Boundary::Ellipse { ends, trim } => {
                    work.charge(if path.harmonic { 80 } else { 8 })?;
                    let [u, vertical] = [u.offset(-trim.center[0]), v.offset(-trim.center[1])];
                    let [x, y] = trim.inverse.map(|[a, b]| u.combine(a, vertical, b));
                    x.circle(y, 1., span, &mut work.crossings);
                    for end in ends {
                        v.level(end[1], span, &mut work.crossings);
                    }
                }
            }
        }
        work.charge((work.crossings.len() + 1).saturating_mul(self.boundary.len()))?;
        drawing_events::spans(
            &mut work.crossings,
            span,
            |t| {
                if depth.at(t) >= threshold
                    || clip
                        .as_ref()
                        .is_some_and(|(hit, range)| !range.contains(&hit.at(t)))
                {
                    return false;
                }
                let point = [u.at(t), v.at(t)];
                let mut inside = false;
                for &boundary in &self.boundary {
                    match boundary.ray(point, self.roundoff) {
                        Ray::Boundary => return true,
                        Ray::Crossing => inside = !inside,
                        Ray::None => {}
                    }
                }
                inside
            },
            |interval| work.hidden.push(interval),
        );
        Ok(())
    }
}

impl Cylinder {
    fn hide(
        &self,
        path: Path,
        span: Span,
        gap: f64,
        roundoff: f64,
        limits: [Option<f64>; 2],
        work: &mut Work,
    ) -> Result<(), ProjectionError> {
        work.charge(if path.harmonic { 90 } else { 12 })?;
        let x = path.coordinate(self.origin, self.screen_axis);
        let y = path.coordinate(self.origin, self.transverse);
        let p = self.projected;
        let c = self.axial_depth;
        let depth = path
            .coordinate(self.origin, self.ray_radial)
            .offset((p * gap).max(roundoff));
        for [height, trim_y, trim_q] in self.stations {
            // The cap station is h + a*y + b*q, where q is the signed
            // radial depth of the first cylinder hit. At a crossing,
            // X + C*q = 0. Eliminate q with q² + y² = r² for the events.
            let position = x.combine(1., y, -p * trim_y).offset(-p * height);
            let coefficient = c - p * trim_q;
            if coefficient.abs() <= ANGULAR {
                position.level(0., span, &mut work.crossings);
            } else {
                work.charge(if path.harmonic { 80 } else { 8 })?;
                position.circle(
                    y.scale(coefficient),
                    coefficient.abs() * self.radius,
                    span,
                    &mut work.crossings,
                );
            }
        }
        for coordinate in [-self.radius, self.radius]
            .into_iter()
            .chain(self.radial_ends.map(|radial| self.radius * radial[0]))
        {
            y.level(coordinate, span, &mut work.crossings);
        }
        y.circle(depth, self.radius, span, &mut work.crossings);
        // The ray's radial coordinate at a view-depth plane. At a clip
        // crossing it lies on the full support circle; the midpoint test
        // below still checks the correct root and the physical patch trims.
        let clipped_radial =
            limits.map(|limit| limit.map(|z| x.scale(-c).offset(p * (z - self.origin[2]))));
        for level in clipped_radial.into_iter().flatten() {
            work.charge(if path.harmonic { 80 } else { 8 })?;
            y.circle(level, self.radius, span, &mut work.crossings);
        }
        work.charge(work.crossings.len() + 1)?;
        let radial_roundoff = 16. * f64::EPSILON * self.radius;
        drawing_events::spans(
            &mut work.crossings,
            span,
            |t| {
                let y = y.at(t);
                let offset = y.abs();
                if offset >= self.radius - radial_roundoff {
                    return false;
                }
                // Along a positive depth ray the radial coordinate increases
                // by p times the travel. An inward wall uses the negative
                // root, which can still be ahead of a farther candidate.
                let hit = self.facing * ((self.radius - offset) * (self.radius + offset)).sqrt();
                // Multiplying the station by p gives this expression without
                // dividing by a potentially small projected axis length.
                let axial = c.mul_add(hit, x.at(t));
                let [lo, hi] = self.stations.map(|[height, trim_y, trim_q]| {
                    p * trim_q.mul_add(hit, trim_y.mul_add(y, height))
                });
                let radial = [y / self.radius, hit / self.radius];
                (lo..=hi).contains(&axial)
                    && hit > depth.at(t)
                    && clipped_radial[0].is_none_or(|far| hit >= far.at(t))
                    && clipped_radial[1].is_none_or(|near| hit <= near.at(t))
                    && cross2(self.radial_ends[0], radial) >= -ANGULAR
                    && cross2(radial, self.radial_ends[1]) >= -ANGULAR
            },
            |interval| work.hidden.push(interval),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;

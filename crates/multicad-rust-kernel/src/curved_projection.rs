//! Exact orthographic profiles and bounded previews for isolated round solids.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

type Point = [f64; 2];
type Point3 = [f64; 3];
type Interval = [f64; 2];

#[derive(Clone, Copy)]
pub struct Cylinder {
    pub base: Point,
    /// Unit axis in drawing coordinates, with depth positive toward the viewer.
    pub axis: [f64; 3],
    pub radius: f64,
    pub height: f64,
}

#[derive(Clone, Copy)]
pub struct Detail {
    pub center: Point,
    pub radius: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ellipse {
    pub center: Point,
    pub u: Point,
    pub v: Point,
    pub major: f64,
    pub minor: f64,
    pub trim: Interval,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Line([Point; 2]),
    Ellipse(Ellipse),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    pub hidden: bool,
    pub curve: Curve,
}

#[derive(Clone, Copy)]
pub(crate) struct Rim {
    pub ellipse: Ellipse,
    pub depth: f64,
    pub depth_sine: f64,
}

impl Rim {
    pub(crate) fn point(self, angle: f64) -> Point3 {
        let [u, v] = self.ellipse.point(angle);
        [u, v, self.depth_sine.mul_add(angle.sin(), self.depth)]
    }

    pub(crate) fn basis(self) -> [Point3; 3] {
        let Ellipse {
            center,
            u,
            v,
            major,
            minor,
            ..
        } = self.ellipse;
        [
            [center[0], center[1], self.depth],
            [u[0] * major, u[1] * major, 0.],
            [v[0] * minor, v[1] * minor, self.depth_sine],
        ]
    }

    pub(crate) fn trimmed(self, trim: Interval) -> Curve {
        if self.ellipse.minor == 0. {
            let mut ends = trim.map(|angle| self.ellipse.point(angle));
            if trim[1] <= PI {
                ends.reverse();
            }
            Curve::Line(ends)
        } else {
            Curve::Ellipse(Ellipse {
                trim,
                ..self.ellipse
            })
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum SpatialCurve {
    Line([Point3; 2]),
    Rim(Rim),
}

pub(crate) struct SpatialEdge {
    pub hidden: bool,
    pub curve: SpatialCurve,
}

impl SpatialEdge {
    fn screen(self) -> Edge {
        let curve = match self.curve {
            SpatialCurve::Line(line) => Curve::Line(line.map(|[u, v, _]| [u, v])),
            SpatialCurve::Rim(rim) => rim.trimmed(rim.ellipse.trim),
        };
        Edge {
            hidden: self.hidden,
            curve,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Limit,
}

pub(crate) const ANGULAR_ROUNDOFF: f64 = 32. * f64::EPSILON;

fn dot(a: Point, b: Point) -> f64 {
    a[0].mul_add(b[0], a[1] * b[1])
}

fn shift(point: Point, direction: Point, distance: f64) -> Point {
    std::array::from_fn(|i| direction[i].mul_add(distance, point[i]))
}

fn difference(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn bounded(point: Point) -> bool {
    point
        .iter()
        .all(|value| value.is_finite() && value.abs() <= 1e8)
}

pub fn project(
    cylinder: Cylinder,
    hidden_lines: bool,
    detail: Option<Detail>,
) -> Result<Vec<Edge>, Error> {
    Ok(project_in_depth(cylinder, 0., None, hidden_lines, detail)?
        .into_iter()
        .map(SpatialEdge::screen)
        .collect())
}

/// Orthographic edges for a cylindrical annulus with a coaxial through bore.
pub fn project_annulus(
    outer: Cylinder,
    inner_radius: f64,
    hidden_lines: bool,
    detail: Option<Detail>,
) -> Result<Vec<Edge>, Error> {
    Ok(
        project_in_depth(outer, 0., Some(inner_radius), hidden_lines, detail)?
            .into_iter()
            .map(SpatialEdge::screen)
            .collect(),
    )
}

pub(crate) fn project_in_depth(
    cylinder: Cylinder,
    base_depth: f64,
    bore: Option<f64>,
    hidden_lines: bool,
    detail: Option<Detail>,
) -> Result<Vec<SpatialEdge>, Error> {
    let Cylinder {
        base,
        axis: [x, y, depth],
        radius,
        height,
    } = cylinder;
    if !bounded(base)
        || !base_depth.is_finite()
        || bore.is_some_and(|inner| !(1e-7..radius).contains(&inner))
        || !(1e-7..=1e7).contains(&radius)
        || !(1e-7..=1e7).contains(&height)
        || (x.mul_add(x, y.mul_add(y, depth * depth)) - 1.).abs() > 1e-9
        || ![x, y, depth].iter().all(|v| v.is_finite())
        || detail
            .is_some_and(|detail| !bounded(detail.center) || !(1e-5..=1e7).contains(&detail.radius))
    {
        return Err(Error::Invalid);
    }
    let projected = x.hypot(y);
    let top = shift(base, [x, y], height);
    let top_depth = depth.mul_add(height, base_depth);
    let cap_depths = [base_depth, top_depth];
    let [(near, near_depth), (far, far_depth)] = if depth >= 0. {
        [(top, top_depth), (base, base_depth)]
    } else {
        [(base, base_depth), (top, top_depth)]
    };
    let mut edges = Vec::with_capacity(if bore.is_some() { 11 } else { 5 });
    let mut add = |curve: SpatialCurve, hidden: bool| {
        if !hidden || hidden_lines {
            match (curve, detail) {
                (SpatialCurve::Rim(rim), Some(detail)) => {
                    edges.extend(rim.ellipse.in_detail(detail).into_iter().map(|trim| {
                        SpatialEdge {
                            hidden,
                            curve: SpatialCurve::Rim(Rim {
                                ellipse: Ellipse {
                                    trim,
                                    ..rim.ellipse
                                },
                                ..rim
                            }),
                        }
                    }))
                }
                (SpatialCurve::Line(line), Some(detail)) => {
                    let projected = line.map(|[u, v, _]| [u, v]);
                    if let Some(clipped) = line_in_detail(projected, detail) {
                        let delta = difference(projected[1], projected[0]);
                        let length = delta[0].hypot(delta[1]);
                        let direction = delta.map(|v| v / length);
                        let clipped = clipped.map(|[u, v]| {
                            let t = dot(difference([u, v], projected[0]), direction) / length;
                            [u, v, (line[1][2] - line[0][2]).mul_add(t, line[0][2])]
                        });
                        edges.push(SpatialEdge {
                            hidden,
                            curve: SpatialCurve::Line(clipped),
                        });
                    }
                }
                (curve, None) => edges.push(SpatialEdge { hidden, curve }),
            }
        }
    };
    let arc = |ellipse: Ellipse, depth| {
        SpatialCurve::Rim(Rim {
            ellipse,
            depth,
            depth_sine: -ellipse.major * projected,
        })
    };
    if projected <= ANGULAR_ROUNDOFF {
        let rim = Ellipse {
            center: near,
            u: [1., 0.],
            v: [0., 1.],
            major: radius,
            minor: radius,
            trim: [0., TAU],
        };
        add(arc(rim, near_depth), false);
        if let Some(inner) = bore {
            // Both caps project to the same two circles along the axis.
            add(
                arc(
                    Ellipse {
                        major: inner,
                        minor: inner,
                        ..rim
                    },
                    near_depth,
                ),
                false,
            );
        } else {
            add(arc(Ellipse { center: far, ..rim }, far_depth), true);
        }
        return Ok(edges);
    }
    let sign = if depth >= 0. { 1. } else { -1. };
    let v = [x, y].map(|value| value / projected * sign);
    let u = [v[1], -v[0]];
    let generator = |[a, b]: [Point; 2]| {
        SpatialCurve::Line([[a[0], a[1], base_depth], [b[0], b[1], top_depth]])
    };
    for at in [-radius, radius] {
        let a = shift(base, u, at);
        let b = shift(top, u, at);
        if a != b {
            add(generator([a, b]), false);
        }
    }
    if depth.abs() <= ANGULAR_ROUNDOFF {
        // Each cap rim projects onto a diameter. The front half is visible and
        // the rear half maps onto the same segment in the hidden layer.
        for (center, depth) in [base, top].into_iter().zip(cap_depths) {
            let rim = Ellipse {
                center,
                u,
                v,
                major: radius,
                minor: 0.,
                trim: [PI, TAU],
            };
            add(arc(rim, depth), false);
            add(
                arc(
                    Ellipse {
                        trim: [0., PI],
                        ..rim
                    },
                    depth,
                ),
                true,
            );
        }
    } else {
        let rim = Ellipse {
            center: near,
            u,
            v,
            major: radius,
            minor: radius * depth.abs().min(1.),
            trim: [0., TAU],
        };
        add(arc(rim, near_depth), false);
        // The near cap is wholly visible. The eye-facing half of the far rim
        // projects away from the near cap, and the wall hides its other half.
        for (trim, hidden) in [([PI, TAU], false), ([0., PI], true)] {
            add(
                arc(
                    Ellipse {
                        center: far,
                        trim,
                        ..rim
                    },
                    far_depth,
                ),
                hidden,
            );
        }
    }
    if let Some(inner) = bore {
        // A ray toward the eye from either concave tangent generator enters
        // the material immediately. In a side view the bore cap rims also
        // collapse onto subsegments of the already visible outer diameters.
        for at in [-inner, inner] {
            let a = shift(base, u, at);
            let b = shift(top, u, at);
            if a != b {
                add(generator([a, b]), true);
            }
        }
        if depth.abs() > ANGULAR_ROUNDOFF {
            let rim = Ellipse {
                center: near,
                u,
                v,
                major: inner,
                minor: inner * depth.abs().min(1.),
                trim: [0., TAU],
            };
            add(arc(rim, near_depth), false);
            let rim = Ellipse { center: far, ..rim };
            // At the near cap a ray from far-rim angle t has its radial
            // v component shifted by -height * projected / |depth|.
            // The entire ray is in the bore exactly when that endpoint is
            // in the inner disk, by convexity. Solving the disk inequality
            // gives sin(t) >= the ratio below.
            let ratio = height * projected / (2. * inner * depth.abs());
            if ratio < 1. {
                let start = ratio.asin();
                let end = PI - start;
                for (trim, hidden) in [
                    ([0., start], true),
                    ([start, end], false),
                    ([end, TAU], true),
                ] {
                    add(arc(Ellipse { trim, ..rim }, far_depth), hidden);
                }
            } else {
                add(arc(rim, far_depth), true);
            }
        }
    }
    Ok(edges)
}

fn line_in_detail(line: [Point; 2], detail: Detail) -> Option<[Point; 2]> {
    let [a, b] = line;
    let delta = difference(b, a);
    let length = delta[0].hypot(delta[1]);
    if length == 0. {
        return None;
    }
    let direction = delta.map(|value| value / length);
    let offset = difference(detail.center, a);
    let perpendicular = dot(offset, [-direction[1], direction[0]]).abs();
    if perpendicular >= detail.radius {
        return None;
    }
    let along = dot(offset, direction);
    let width = ((detail.radius - perpendicular) * (detail.radius + perpendicular)).sqrt();
    let trim = [(along - width).max(0.), (along + width).min(length)];
    (trim[0] < trim[1]).then(|| trim.map(|at| shift(a, direction, at)))
}

impl Ellipse {
    pub fn point(self, angle: f64) -> Point {
        let (sin, cos) = angle.sin_cos();
        shift(
            shift(self.center, self.u, self.major * cos),
            self.v,
            self.minor * sin,
        )
    }

    /// Trim a validated projected ellipse to a circular drawing detail.
    pub fn in_detail(self, detail: Detail) -> Vec<Interval> {
        let offset = difference(self.center, detail.center);
        let [x, y] = [dot(offset, self.u), dot(offset, self.v)];
        let scale = x.abs().max(y.abs()).max(self.major).max(detail.radius);
        let [x, y, a, b, r] = [x, y, self.major, self.minor, detail.radius].map(|v| v / scale);
        let inside = |angle: f64| {
            let (sin, cos) = angle.sin_cos();
            a.mul_add(cos, x).hypot(b.mul_add(sin, y)) <= r
        };
        // Three bounded tan-half-angle charts cover a full turn without a
        // pole. The disk boundary equation is a quartic in each chart.
        let mut intervals: Vec<Interval> = Vec::new();
        for (phase, sign) in [(0., 1.), (PI, -1.), (TAU, 1.)] {
            let start = self.trim[0].max(phase - FRAC_PI_2);
            let end = self.trim[1].min(phase + FRAC_PI_2);
            if start >= end {
                continue;
            }
            let [lo, hi] = [start, end].map(|angle| ((angle - phase) / 2.).tan());
            let [c, d] = [x + a * sign, x - a * sign];
            let e = 2. * b * sign;
            let radial = |x: f64| {
                let h = x.hypot(y);
                (h - r) * (h + r)
            };
            let polynomial = [
                radial(c),
                2. * y * e,
                2. * c.mul_add(d, (y - r) * (y + r)) + e * e,
                2. * y * e,
                radial(d),
            ];
            let mut cuts = vec![start];
            cuts.extend(
                roots(&polynomial, [lo, hi])
                    .into_iter()
                    .chain(roots(&derivative(&polynomial), [lo, hi]))
                    .map(|t| phase + 2. * t.atan())
                    .filter(|angle| *angle > start && *angle < end),
            );
            cuts.sort_unstable_by(f64::total_cmp);
            cuts.dedup();
            cuts.push(end);
            for pair in cuts.windows(2) {
                let [a, b] = [pair[0], pair[1]];
                if inside(a.midpoint(b)) {
                    if let Some(previous) = intervals.last_mut().filter(|p| p[1] == a) {
                        previous[1] = b;
                    } else {
                        intervals.push([a, b]);
                    }
                }
            }
        }
        intervals
    }
}

pub(crate) fn derivative(coefficients: &[f64]) -> Vec<f64> {
    coefficients
        .iter()
        .enumerate()
        .skip(1)
        .map(|(i, value)| i as f64 * value)
        .collect()
}

pub(crate) fn roots(coefficients: &[f64], [lo, hi]: Interval) -> Vec<f64> {
    let Some(degree) = coefficients.iter().rposition(|v| *v != 0.) else {
        return Vec::new();
    };
    let coefficients = &coefficients[..=degree];
    match coefficients {
        [_] => Vec::new(),
        [constant, slope] => {
            let root = -constant / slope;
            (root > lo && root < hi)
                .then_some(root)
                .into_iter()
                .collect()
        }
        _ => {
            let evaluate = |t| {
                coefficients
                    .iter()
                    .rev()
                    .fold(0_f64, |acc, c| acc.mul_add(t, *c))
            };
            let mut bounds = vec![lo];
            bounds.extend(roots(&derivative(coefficients), [lo, hi]));
            bounds.push(hi);
            let mut hits = Vec::new();
            for bound in bounds.windows(2) {
                let [mut a, mut b] = [bound[0], bound[1]];
                let first = evaluate(a);
                let last = evaluate(b);
                if first == 0. && a > lo {
                    hits.push(a);
                }
                if first != 0. && last != 0. && first.is_sign_positive() != last.is_sign_positive()
                {
                    // Consecutive derivative roots bound a monotone interval.
                    for _ in 0..56 {
                        let mid = a.midpoint(b);
                        if evaluate(mid).is_sign_positive() == first.is_sign_positive() {
                            a = mid;
                        } else {
                            b = mid;
                        }
                    }
                    hits.push(a.midpoint(b));
                }
            }
            hits
        }
    }
}

impl Curve {
    /// Preview only. A chord of a circle of the major radius bounds the
    /// deviation on the ellipse, since the ellipse is its linear contraction.
    pub fn preview(self, tolerance: f64, maximum_points: usize) -> Result<Vec<Point>, Error> {
        let ellipse = match self {
            Self::Ellipse(ellipse) => ellipse,
            Self::Line(points) => {
                return if maximum_points >= 2 {
                    Ok(points.into())
                } else {
                    Err(Error::Limit)
                };
            }
        };
        let [start, end] = ellipse.trim;
        if !(1e-7..=1e4).contains(&tolerance)
            || !(1e-7..=1e7).contains(&ellipse.major)
            || !(0. ..end).contains(&start)
            || end > TAU
        {
            return Err(Error::Invalid);
        }
        // Stable even when 1 - tolerance/major would round to one.
        let step = 4. * (tolerance / (2. * ellipse.major)).sqrt().min(1.).asin();
        let segments = ((end - start) / step.min(FRAC_PI_2)).ceil() as usize;
        let Some(count) = segments
            .checked_add(1)
            .filter(|count| *count <= maximum_points)
        else {
            return Err(Error::Limit);
        };
        Ok((0..count)
            .map(|index| {
                ellipse.point((end - start).mul_add(index as f64 / segments as f64, start))
            })
            .collect())
    }
}

#[cfg(test)]
mod tests;

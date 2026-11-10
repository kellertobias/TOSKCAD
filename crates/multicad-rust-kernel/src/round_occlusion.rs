//! Exact visibility intervals for drawing edges behind round stock.

use crate::{
    curved_projection::{self, Cylinder, Detail, Rim, SpatialEdge},
    drawing_events::{self, Harmonic},
    planar_projection::ProjectionError,
};

type Point = [f64; 3];
type Span = [f64; 2];

/// A bounded axial cylinder in drawing (u, v, depth) coordinates.
/// An optional coaxial bore removes material all the way through both caps.
#[derive(Clone, Copy, Debug)]
pub struct RoundOccluder {
    base: Point,
    axis: Point,
    screen_axis: [f64; 2],
    projected: f64,
    radius: f64,
    height: f64,
    bore: Option<f64>,
    bounds: [Point; 2],
}

fn bounded(point: Point) -> bool {
    point.iter().all(|v| v.is_finite() && v.abs() <= 1e8)
}

impl RoundOccluder {
    pub fn new(
        base: Point,
        axis: Point,
        radius: f64,
        height: f64,
        bore: Option<f64>,
    ) -> Result<Self, ProjectionError> {
        let norm = axis[0].hypot(axis[1]).hypot(axis[2]);
        if !bounded(base)
            || !(1. - 1e-9..=1. + 1e-9).contains(&norm)
            || !(1e-7..=1e7).contains(&radius)
            || !(1e-7..=1e7).contains(&height)
            || bore.is_some_and(|inner| !(1e-7..radius).contains(&inner))
        {
            return Err(ProjectionError::Invalid);
        }
        let axis = axis.map(|v| v / norm);
        let projected = axis[0].hypot(axis[1]);
        let screen_axis = if projected == 0. {
            [1., 0.]
        } else {
            [axis[0] / projected, axis[1] / projected]
        };
        let extents: [Span; 3] = std::array::from_fn(|i| {
            let end = height * axis[i];
            let radial = radius * axis[(i + 1) % 3].hypot(axis[(i + 2) % 3]);
            [
                base[i] + end.min(0.) - radial,
                base[i] + end.max(0.) + radial,
            ]
        });
        let bounds = std::array::from_fn(|side| extents.map(|extent| extent[side]));
        Ok(Self {
            base,
            axis,
            screen_axis,
            projected,
            radius,
            height,
            bore,
            bounds,
        })
    }

    pub fn bounds(&self) -> [Point; 2] {
        self.bounds
    }

    pub(crate) fn covers_coincident_rim(&self, rim: Rim, tolerance: f64) -> bool {
        if self.bore.is_some()
            || self.projected > curved_projection::ANGULAR_ROUNDOFF
            || rim.depth_sine != 0.
        {
            return false;
        }
        let same_coordinate = |a: f64, b: f64, scale: f64| {
            (a - b).abs() <= 8. * f64::EPSILON * a.abs().max(b.abs()).max(scale)
        };
        if !same_coordinate(rim.ellipse.major, self.radius, 0.)
            || !same_coordinate(rim.ellipse.minor, self.radius, 0.)
        {
            return false;
        }
        let near = if self.axis[2] > 0. {
            std::array::from_fn(|i| self.axis[i].mul_add(self.height, self.base[i]))
        } else {
            self.base
        };
        let depth_guard = tolerance.max(16. * f64::EPSILON * near[2].abs().max(rim.depth.abs()));
        near[2] - rim.depth > depth_guard
            && (0..2).all(|i| same_coordinate(rim.ellipse.center[i], near[i], self.radius))
    }

    pub(crate) fn drawing_edges(
        &self,
        hidden_lines: bool,
        detail: Option<Detail>,
    ) -> Result<Vec<SpatialEdge>, ProjectionError> {
        curved_projection::project_in_depth(
            Cylinder {
                base: [self.base[0], self.base[1]],
                axis: self.axis,
                radius: self.radius,
                height: self.height,
            },
            self.base[2],
            self.bore,
            hidden_lines,
            detail,
        )
        .map_err(|_| ProjectionError::Invalid)
    }

    /// Find hidden intervals of a straight edge as normalized parameters in
    /// `[0, 1]`. Positive depth is toward the viewer. Grazing contacts and
    /// geometry within the given depth tolerance stay visible.
    pub fn hidden_intervals(
        &self,
        edge: [Point; 2],
        tolerance: f64,
    ) -> Result<Vec<Span>, ProjectionError> {
        if !(1e-7..=1e-3).contains(&tolerance) || !edge.into_iter().all(bounded) {
            return Err(ProjectionError::Invalid);
        }
        let mut hidden = Vec::new();
        if (edge[1][0] - edge[0][0]).hypot(edge[1][1] - edge[0][1]) > tolerance
            && self.may_hide(edge, tolerance)
        {
            self.hide(edge, tolerance, |span| hidden.push(span));
        }
        Ok(hidden)
    }

    pub(crate) fn may_hide(&self, [a, b]: [Point; 2], tolerance: f64) -> bool {
        let [min, max] = self.bounds;
        max[2] > a[2].min(b[2]) + tolerance
            && (0..2).all(|i| a[i].min(b[i]) <= max[i] && a[i].max(b[i]) >= min[i])
    }

    pub(crate) fn hide(&self, edge: [Point; 2], tolerance: f64, mut emit: impl FnMut(Span)) {
        let [u, v] = self.screen_axis;
        let scale = edge
            .into_iter()
            .chain(self.bounds)
            .flatten()
            .map(f64::abs)
            .fold(0., f64::max);
        let depth_gap = tolerance.max(16. * f64::EPSILON * scale);
        let [[ax, ay, az], [bx, by, bz]] = edge.map(|point| {
            let [x, y, z]: Point = std::array::from_fn(|i| point[i] - self.base[i]);
            [u.mul_add(x, v * y), u.mul_add(y, -v * x), z + depth_gap]
        });
        let x = Affine::new(ax, bx);
        let y = Affine::new(ay, by);
        let z = Affine::new(az, bz);
        let p = self.projected;
        let c = self.axis[2];
        // Axial coordinate and the in-plane radial coordinate at the start
        // of each depth ray. A positive ray step adds c and subtracts p.
        let axial = x.combine(p, z, c);
        let radial = x.combine(c, z, -p);
        let screen_roundoff = 16.
            * f64::EPSILON
            * [ax, ay, bx, by, self.radius, self.height]
                .into_iter()
                .map(f64::abs)
                .fold(0., f64::max);
        let follows = |coordinate: Affine, value: f64| {
            [coordinate.at(0.), coordinate.at(1.)]
                .into_iter()
                .all(|end| (end - value).abs() <= screen_roundoff)
        };
        // Rotating a silhouette line into the axial frame can put both
        // endpoints a few ulps inside a tangent plane. This is contact with
        // the outline, and cannot certify that the line is behind material.
        if [-self.radius, self.radius]
            .into_iter()
            .any(|side| follows(y, side))
            || (c == 0. && [0., self.height].into_iter().any(|cap| follows(axial, cap)))
        {
            return;
        }
        let mut events = Events::new();
        for cap in [0., self.height] {
            events.level(axial, cap);
        }
        for radius in [Some(self.radius), self.bore].into_iter().flatten() {
            for level in [-radius, radius] {
                events.level(y, level);
            }
            events.circle(y, radial, radius);
            if c != 0. {
                // Project each cap rim without dividing by the potentially
                // tiny depth component of the axis.
                for cap in [0., self.height] {
                    events.circle(y.scale(c), x.offset(-p * cap), c.abs() * radius);
                }
            }
        }
        let ray = |t| self.ray_hits(axial.at(t), radial.at(t), y.at(t), self.radius, self.bore);
        let mut hidden: Option<Span> = None;
        for interval in events.sorted().windows(2) {
            let [start, end] = [interval[0], interval[1]];
            if start == end {
                continue;
            }
            if ray(start.midpoint(end)) {
                hidden = Some([hidden.map_or(start, |span| span[0]), end]);
            } else if let Some(previous) = hidden.take() {
                emit(previous);
            }
        }
        if let Some(last) = hidden {
            emit(last);
        }
    }

    pub(crate) fn hide_rim(
        &self,
        rim: Rim,
        tolerance: f64,
        cuts: &mut Vec<f64>,
        emit: impl FnMut(Span),
    ) {
        let [origin, cosine, sine] = rim.basis();
        let [x, y, z] =
            std::array::from_fn(|i| Harmonic([origin[i] - self.base[i], cosine[i], sine[i]]));
        let scale = origin
            .into_iter()
            .chain(self.bounds.into_iter().flatten())
            .map(f64::abs)
            .fold(rim.ellipse.major, f64::max);
        let depth_gap = tolerance.max(16. * f64::EPSILON * scale);
        let z = z.offset(depth_gap);
        let [u, v] = self.screen_axis;
        let [x, y] = [x.combine(u, y, v), x.combine(-v, y, u)];
        let p = self.projected;
        let c = self.axis[2];
        let axial = x.combine(p, z, c);
        let radial = x.combine(c, z, -p);
        let trim = rim.ellipse.trim;
        // In a coaxial view two circumferences can coincide along their
        // whole trim. A few ulps of trig error must not hide the silhouette.
        let radial_guard = 16. * f64::EPSILON * self.radius.max(rim.ellipse.major);
        let outer = self.radius - radial_guard;
        let bore = self.bore.map(|radius| radius + radial_guard);
        if bore.is_some_and(|inner| inner >= outer) {
            return;
        }
        cuts.clear();
        for cap in [0., self.height] {
            axial.level(cap, trim, cuts);
        }
        for radius in [Some(outer), bore].into_iter().flatten() {
            for level in [-radius, radius] {
                y.level(level, trim, cuts);
            }
            drawing_events::circle(y, radial, radius, trim, cuts);
            if c != 0. {
                for cap in [0., self.height] {
                    drawing_events::circle(
                        y.scale(c),
                        x.offset(-p * cap),
                        c.abs() * radius,
                        trim,
                        cuts,
                    );
                }
            }
        }
        drawing_events::spans(
            cuts,
            trim,
            |t| self.ray_hits(axial.at(t), radial.at(t), y.at(t), outer, bore),
            emit,
        );
    }

    fn ray_hits(&self, axial: f64, radial: f64, y: f64, outer: f64, bore: Option<f64>) -> bool {
        let c = self.axis[2];
        let p = self.projected;
        let mut front = [0., f64::INFINITY];
        if c == 0. {
            if axial <= 0. || axial >= self.height {
                return false;
            }
        } else {
            let [a, b] = [-axial / c, (self.height - axial) / c];
            front = [a.min(b).max(0.), a.max(b)];
            if front[1] <= front[0] {
                return false;
            }
        }
        if p == 0. {
            let distance = radial.hypot(y);
            return distance < outer && bore.is_none_or(|inner| distance > inner);
        }
        let Some(outer) = chord_half(outer, y.abs()) else {
            return false;
        };
        let [low, high] = [(radial - outer) / p, (radial + outer) / p];
        front = [front[0].max(low), front[1].min(high)];
        if front[1] <= front[0] {
            return false;
        }
        let Some(inner) = bore.and_then(|radius| chord_half(radius, y.abs())) else {
            return true;
        };
        front[0] < (radial - inner) / p || front[1] > (radial + inner) / p
    }
}

fn chord_half(radius: f64, offset: f64) -> Option<f64> {
    (offset < radius).then(|| (radius - offset).sqrt() * (radius + offset).sqrt())
}

#[derive(Clone, Copy)]
struct Affine {
    start: f64,
    delta: f64,
}

impl Affine {
    fn new(start: f64, end: f64) -> Self {
        Self {
            start,
            delta: end - start,
        }
    }

    fn at(self, t: f64) -> f64 {
        self.delta.mul_add(t, self.start)
    }

    fn scale(self, by: f64) -> Self {
        Self {
            start: self.start * by,
            delta: self.delta * by,
        }
    }

    fn offset(self, by: f64) -> Self {
        Self {
            start: self.start + by,
            ..self
        }
    }

    fn combine(self, a: f64, other: Self, b: f64) -> Self {
        Self {
            start: self.start.mul_add(a, other.start * b),
            delta: self.delta.mul_add(a, other.delta * b),
        }
    }
}

struct Events {
    roots: [f64; 20],
    len: usize,
}

impl Events {
    fn new() -> Self {
        // Two endpoints, two cap-depth events, and up to eight events per
        // circular boundary (two tangent, two depth, four cap intersections).
        let mut roots = [0.; 20];
        roots[1] = 1.;
        Self { roots, len: 2 }
    }

    fn add(&mut self, root: f64) {
        if 0. < root && root < 1. {
            self.roots[self.len] = root;
            self.len += 1;
        }
    }

    fn level(&mut self, affine: Affine, value: f64) {
        self.add((value - affine.start) / affine.delta);
    }

    fn circle(&mut self, x: Affine, y: Affine, radius: f64) {
        let speed = x.delta.hypot(y.delta);
        if speed == 0. {
            return;
        }
        let [u, v] = [x.delta / speed, y.delta / speed];
        let offset = (u.mul_add(y.start, -v * x.start)).abs();
        if offset > radius {
            return;
        }
        // A tangent contact does not hide or reveal a segment by itself,
        // but it must not be used as the sample for its surrounding span.
        let width = (radius - offset).sqrt() * (radius + offset).sqrt();
        let center = -u.mul_add(x.start, v * y.start);
        self.add((center - width) / speed);
        self.add((center + width) / speed);
    }

    fn sorted(&mut self) -> &[f64] {
        let roots = &mut self.roots[..self.len];
        roots.sort_unstable_by(f64::total_cmp);
        roots
    }
}

#[cfg(test)]
mod tests;

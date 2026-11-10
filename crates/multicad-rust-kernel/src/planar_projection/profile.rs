//! Exact circular and stadium trims of a plane perpendicular to the view.

use super::*;
use crate::{
    drawing_events::{self, Harmonic},
    stadium::{Stadium, segments_distance},
};

#[derive(Clone, Copy)]
pub(super) enum ProfileTrim {
    Circle { center: Point2, radius: f64 },
    Stadium(Stadium),
}

impl ProfileTrim {
    pub(super) fn centers(self) -> [Point2; 2] {
        match self {
            Self::Circle { center, .. } => [center; 2],
            Self::Stadium(stadium) => stadium.centers(),
        }
    }

    pub(super) fn radius(self) -> f64 {
        match self {
            Self::Circle { radius, .. } => radius,
            Self::Stadium(stadium) => stadium.radius_mm(),
        }
    }

    pub(super) fn distance(self, point: Point2) -> f64 {
        match self {
            Self::Circle { center, .. } => {
                let [x, y] = sub(point, center);
                x.hypot(y)
            }
            Self::Stadium(stadium) => segments_distance(stadium.centers(), [point; 2]),
        }
    }

    pub(super) fn bounds(self, depth: f64) -> Bounds {
        let [a, b] = self.centers();
        let r = self.radius();
        Bounds {
            min: [a[0].min(b[0]) - r, a[1].min(b[1]) - r, depth],
            max: [a[0].max(b[0]) + r, a[1].max(b[1]) + r, depth],
        }
    }

    pub(super) fn line_work(self) -> usize {
        match self {
            Self::Circle { .. } => 1,
            Self::Stadium(_) => 8,
        }
    }

    pub(super) fn rim_work(self) -> usize {
        match self {
            Self::Circle { .. } => 80,
            Self::Stadium(_) => 164,
        }
    }

    pub(super) fn line_span(self, line: &Line, allow_boundary: bool) -> Option<Span> {
        match self {
            Self::Circle { center, radius } => chord(line.coordinates(center), radius, 0.),
            Self::Stadium(stadium) => {
                let radius = stadium.radius_mm();
                let mut corners = stadium.corners().map(|point| line.coordinates(point));
                let roundoff = 16.
                    * f64::EPSILON
                    * corners
                        .iter()
                        .flatten()
                        .map(|v| v.abs())
                        .fold(radius, f64::max);
                for point in &mut corners {
                    if point[1].abs() <= roundoff {
                        point[1] = 0.;
                    }
                }
                let sides = rectangle_span(corners, allow_boundary);
                let caps = stadium
                    .centers()
                    .map(|point| chord(line.coordinates(point), radius, roundoff));
                caps.into_iter()
                    .chain([sides])
                    .flatten()
                    .reduce(|a, b| [a[0].min(b[0]), a[1].max(b[1])])
            }
        }
    }

    pub(super) fn rim_cuts(self, [origin, cos, sin]: [Point; 3], trim: Span, cuts: &mut Vec<f64>) {
        let radius = self.radius();
        let [a, b] = self.centers();
        let mut cap_cuts = |center: Point2| {
            let [x, y] = std::array::from_fn(|i| Harmonic([origin[i] - center[i], cos[i], sin[i]]));
            drawing_events::circle(x, y, radius, trim, cuts);
        };
        cap_cuts(a);
        if let Self::Stadium(stadium) = self {
            cap_cuts(b);
            // The whole circles and parallel supporting lines deliberately
            // overpartition the rim. The true capsule classifies each span.
            let [u, v] = sub(b, a).map(|value| value / stadium.spine_length_mm());
            let normal = [-v, u];
            let sides = Harmonic([
                dot(normal, sub(planar(origin), a)),
                dot(normal, planar(cos)),
                dot(normal, planar(sin)),
            ]);
            sides.level(-radius, trim, cuts);
            sides.level(radius, trim, cuts);
        }
    }
}

fn chord([along, offset]: Point2, radius: f64, roundoff: f64) -> Option<Span> {
    let offset = offset.abs();
    (offset < radius - roundoff.min(radius * 0.5)).then(|| {
        let half = ((radius - offset) * (radius + offset)).sqrt();
        [along - half, along + half]
    })
}

fn rectangle_span(corners: [Point2; 4], allow_boundary: bool) -> Option<Span> {
    let min = corners.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
    let max = corners
        .iter()
        .map(|p| p[1])
        .fold(f64::NEG_INFINITY, f64::max);
    if min > 0. || max < 0. || (!allow_boundary && (min == 0. || max == 0.)) {
        return None;
    }
    let mut span = [f64::INFINITY, f64::NEG_INFINITY];
    let mut include = |along: f64| {
        span = [span[0].min(along), span[1].max(along)];
    };
    for (&[a, p], &[b, q]) in corners.iter().zip(corners.iter().cycle().skip(1)) {
        if p == 0. {
            include(a);
        }
        if (p < 0. && q > 0.) || (p > 0. && q < 0.) {
            include((b - a).mul_add(-p / (q - p), a));
        }
    }
    (span[1] > span[0]).then_some(span)
}

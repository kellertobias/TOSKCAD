use super::{Point, Span, numeric};

/// Depth of one profile edge as a function of the drawing V coordinate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Height {
    Line {
        origin: Point,
        tangent: Point,
    },
    Arc {
        center: Point,
        radius: f64,
        sign: f64,
    },
}

impl Height {
    pub(super) fn flat(depth: f64) -> Self {
        Self::Line {
            origin: [0., depth],
            tangent: [1., 0.],
        }
    }

    pub(super) fn at(self, v: f64) -> f64 {
        match self {
            Self::Line {
                origin: [v0, d0],
                tangent: [dv, dd],
            } => (v - v0).mul_add(dd / dv, d0),
            Self::Arc {
                center: [v0, d0],
                radius,
                sign,
            } => {
                let distance = (v - v0).abs().min(radius);
                sign.mul_add(((radius - distance) * (radius + distance)).sqrt(), d0)
            }
        }
    }

    pub(super) fn tangent(self, v: f64) -> Point {
        match self {
            Self::Line { tangent, .. } => tangent,
            Self::Arc {
                center: [v0, _],
                radius,
                sign,
            } => {
                let ratio = ((v - v0) / radius).clamp(-1., 1.);
                [((1. - ratio) * (1. + ratio)).sqrt(), -sign * ratio]
            }
        }
    }

    pub(super) fn intersections(self, other: Self, v: Span, events: &mut Vec<f64>) {
        let mut event = |hit| {
            if v[0] < hit && hit < v[1] {
                events.push(hit);
            }
        };
        match (self, other) {
            (Self::Line { .. }, Self::Line { .. }) => {
                let [a, b] = v.map(|at| self.at(at) - other.at(at));
                if (a < 0. && b > 0.) || (a > 0. && b < 0.) {
                    event((v[1] - v[0]).mul_add(a / (a - b), v[0]));
                }
            }
            (Self::Line { origin, tangent }, Self::Arc { center, radius, .. })
            | (Self::Arc { center, radius, .. }, Self::Line { origin, tangent }) => {
                let delta = difference(center, origin);
                let along = dot(delta, tangent);
                let away = cross(delta, tangent).abs();
                if away <= radius {
                    let half = ((radius - away) * (radius + away)).sqrt();
                    for position in [along - half, along + half] {
                        event(tangent[0].mul_add(position, origin[0]));
                    }
                }
            }
            (
                Self::Arc {
                    center: a,
                    radius: ar,
                    ..
                },
                Self::Arc {
                    center: b,
                    radius: br,
                    ..
                },
            ) => {
                let [dv, dd] = difference(b, a);
                let distance = dv.hypot(dd);
                if distance == 0. || distance > ar + br || distance < (ar - br).abs() {
                    return;
                }
                // All intersections of the supporting circles are safe
                // events. Their arc branches will be classified afterward.
                let u = dv / distance;
                let n = dd / distance;
                let along = 0.5 * (distance + (ar - br) * (ar + br) / distance);
                let away = ((ar - along.abs()).max(0.) * (ar + along.abs())).sqrt();
                let center = u.mul_add(along, a[0]);
                event(n.mul_add(-away, center));
                event(n.mul_add(away, center));
            }
        }
    }
}

pub(super) fn difference(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}

pub(super) fn dot(a: Point, b: Point) -> f64 {
    a[0].mul_add(b[0], a[1] * b[1])
}

fn cross(a: Point, b: Point) -> f64 {
    a[0].mul_add(b[1], -a[1] * b[0])
}

pub(super) fn sort_unique(values: &mut Vec<f64>) {
    values.sort_unstable_by(f64::total_cmp);
    values.dedup_by(|a, b| (*a - *b).abs() <= numeric(*a, *b));
}

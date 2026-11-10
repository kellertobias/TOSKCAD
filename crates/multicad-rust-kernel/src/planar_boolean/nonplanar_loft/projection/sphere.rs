//! Exact spherical boundaries in a retained assembly view.
use super::{visibility::Candidate, *};
use crate::curved_projection::{Ellipse, Rim};
use std::f64::consts::TAU;

pub(super) struct Sphere {
    pub source: usize,
    center: Point,
    radius: f64,
    pub bounds: [Point; 2],
    depth: [f64; 2],
    pub near_trimmed: bool,
}

impl Sphere {
    pub(super) fn prepare(
        source: usize,
        center: Point,
        radius: f64,
        planes: Option<clip::Limits>,
        tolerance: f64,
        output: &mut Vec<Candidate>,
    ) -> Result<Option<Self>, Error> {
        if !(1e-7..=1e7).contains(&radius)
            || center
                .into_iter()
                .any(|x| !x.is_finite() || x.abs() > MAX_COORDINATE)
        {
            return Err(Error::Invalid);
        }
        let bounds = [center.map(|x| x - radius), center.map(|x| x + radius)];
        let Some(planes) = planes else {
            round::add_ellipse_candidates(
                source,
                2,
                center[2],
                circle(paper(center), radius),
                output,
            );
            return Ok(Some(Self {
                source,
                center,
                radius,
                bounds,
                depth: [f64::NEG_INFINITY, f64::INFINITY],
                near_trimmed: false,
            }));
        };
        let [min, max] = bounds;
        if planes.into_iter().enumerate().any(|(axis, window)| {
            window
                .is_some_and(|[lo, hi]| max[axis] <= lo + tolerance || min[axis] >= hi - tolerance)
        }) {
            return Ok(None);
        }
        let depth = planes[2].expect("a retained scene has depth bounds");
        let [far, near] = depth;
        if far < center[2] - tolerance && near > center[2] + tolerance {
            round::add_ellipse_candidates(
                source,
                2,
                center[2],
                circle(paper(center), radius),
                output,
            );
        }
        let mut near_trimmed = false;
        for (axis, plane) in planes.into_iter().enumerate() {
            for level in plane.into_iter().flatten() {
                let Some(rim) = cross_section(radius, level - center[axis], tolerance) else {
                    continue;
                };
                let u = (axis + 1) % 3;
                let v = (axis + 2) % 3;
                let chart = [center[u], center[v]];
                round::add_ellipse_candidates(source, axis, level, circle(chart, rim), output);
                if axis != 2 {
                    continue;
                }
                for (side, window) in planes[..2].iter().enumerate() {
                    for coordinate in window.iter().flatten().copied() {
                        let Some(half) = cross_section(rim, coordinate - center[side], tolerance)
                        else {
                            continue;
                        };
                        let across = 1 - side;
                        let mut a = center;
                        a[2] = level;
                        a[side] = coordinate;
                        a[across] -= half;
                        let mut b = a;
                        b[across] = center[across] + half;
                        output.push(Candidate::new(source, Curve::line(a, b), true));
                    }
                }
                if level == near {
                    let [u, v] = std::array::from_fn(|i| {
                        planes[i].map_or(0., |[lo, hi]| center[i] - center[i].clamp(lo, hi))
                    });
                    near_trimmed = u.hypot(v) < rim - tolerance;
                }
            }
        }
        Ok(Some(Self {
            source,
            center,
            radius,
            bounds,
            depth,
            near_trimmed,
        }))
    }

    pub(super) fn blocks(
        &self,
        point: Point,
        rim: Option<Rim>,
        tolerance: f64,
        epsilon: f64,
    ) -> bool {
        let [u, v, _] = sub(point, self.center);
        let radial = u.hypot(v);
        let [far, near] = self.depth;
        if let Some(half) = cross_section(self.radius, radial, epsilon) {
            return (self.center[2] + half).min(near)
                > (self.center[2] - half).max(far).max(point[2] + tolerance);
        }
        // Coincident projected outlines have a definite depth order even
        // though a generic tangent ray has no positive material interval.
        rim.is_some_and(|rim| {
            self.center[2] > point[2] + tolerance
                && (far..=near).contains(&self.center[2])
                && (self.radius - rim.ellipse.major).abs() <= epsilon
                && (self.radius - rim.ellipse.minor).abs() <= epsilon
                && length(minus(rim.ellipse.center, paper(self.center))) <= epsilon
                && rim.depth_cosine.hypot(rim.depth_sine) <= epsilon
        })
    }

    pub(super) fn depth_events(&self, curve: Curve, cuts: &mut Vec<f64>) {
        curve.sphere_crossings(self.center, self.radius, cuts);
    }
}

fn cross_section(radius: f64, offset: f64, tolerance: f64) -> Option<f64> {
    let offset = offset.abs();
    (offset < radius - tolerance).then(|| ((radius - offset) * (radius + offset)).sqrt())
}

fn circle(center: Paper, radius: f64) -> Ellipse {
    Ellipse {
        center,
        u: [1., 0.],
        v: [0., 1.],
        major: radius,
        minor: radius,
        trim: [0., TAU],
    }
}

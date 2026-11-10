use super::*;
use crate::stadium::{Stadium, segments_distance};
use stadium::{distance_to_spine, polygon_clear_of_stadium};
use std::f64::consts::PI;

impl CircleEdge {
    pub(super) fn bounds(self) -> [Point; 2] {
        std::array::from_fn(|side| {
            std::array::from_fn(|i| {
                let extent =
                    self.radius_mm * self.normal[(i + 1) % 3].hypot(self.normal[(i + 2) % 3]);
                if side == 0 {
                    self.center_mm[i] - extent
                } else {
                    self.center_mm[i] + extent
                }
            })
        })
    }

    pub(super) fn within_plane(self, plane: &Plane, tolerance: f64) -> bool {
        let center_deviation = dot(sub(self.center_mm, plane.origin), plane.normal).abs();
        let radial_deviation = self.radius_mm * length(cross(self.normal, plane.normal));
        center_deviation + radial_deviation <= tolerance
    }
}

impl Shell<'_, '_> {
    pub(super) fn circle_curve(
        &mut self,
        id: u32,
        [start, end]: [u32; 2],
        forward: bool,
    ) -> Result<Curve> {
        let [Value::String(_), axis, Value::Number(radius)] = self.args(id, "CIRCLE")? else {
            return Err(ReadError::Invalid);
        };
        let radius = *radius;
        let plane = self.placement(reference(axis)?)?;
        if radius <= self.tolerance || radius > 1e12 {
            return Err(ReadError::Invalid);
        }
        let point = sub(self.vertex(start)?, plane.origin);
        if dot(point, plane.normal).abs() > self.tolerance
            || (length(cross(point, plane.normal)) - radius).abs() > self.tolerance
        {
            return Err(ReadError::Invalid);
        }
        let circle = CircleEdge {
            center_mm: plane.origin,
            normal: plane
                .normal
                .map(|coordinate| if forward { coordinate } else { -coordinate }),
            radius_mm: radius,
        };
        if start != end {
            let other = sub(self.vertex(end)?, plane.origin);
            return self.semicircle_curve(circle, point, other);
        }
        Ok(Curve::Circle(circle))
    }

    pub(super) fn planar_trim(&self, wire: &[Traversal], plane: &Plane) -> Result<Trim> {
        if let [edge] = wire
            && let Curve::Circle(circle) = self.edges[&edge.edge].curve
        {
            if !circle.within_plane(plane, self.tolerance) {
                return Err(ReadError::Invalid);
            }
            let delta = sub(circle.center_mm, plane.origin);
            let ccw = (dot(circle.normal, plane.normal) > 0.) == edge.forward;
            return Ok(Trim::Circle {
                center: [dot(delta, plane.u), dot(delta, plane.v)],
                radius: circle.radius_mm,
                ccw,
            });
        }
        if wire
            .iter()
            .any(|edge| matches!(self.edges[&edge.edge].curve, Curve::Semicircle(_)))
        {
            return self.stadium_trim(wire, plane);
        }
        if wire.len() < 3 {
            return Err(ReadError::Invalid);
        }
        let mut seen = BTreeSet::new();
        let ring = wire
            .iter()
            .map(|edge| {
                if !matches!(self.edges[&edge.edge].curve, Curve::Line) || !seen.insert(edge.start)
                {
                    return Err(ReadError::Invalid);
                }
                let delta = sub(self.points[&edge.start], plane.origin);
                if dot(delta, plane.normal).abs() > self.tolerance {
                    return Err(ReadError::Invalid);
                }
                Ok([dot(delta, plane.u), dot(delta, plane.v)])
            })
            .collect::<Result<_>>()?;
        Ok(Trim::Polygon(ring))
    }

    pub(super) fn cylindrical_face(
        &mut self,
        surface: u32,
        bounds: &[u32],
        outward: bool,
        face: usize,
    ) -> Result<Face> {
        let [Value::String(_), axis, Value::Number(radius)] =
            self.args(surface, "CYLINDRICAL_SURFACE")?
        else {
            return Err(ReadError::Invalid);
        };
        let radius = *radius;
        let surface = self.placement(reference(axis)?)?;
        let [bound] = bounds else {
            return Err(ReadError::Unsupported);
        };
        let (_, wire, forward) = self.bound(*bound, true)?;
        let mut wire = self.traversals(wire, forward, face, MAX_VERTICES)?;
        let first = wire
            .iter()
            .position(|edge| !matches!(self.edges[&edge.edge].curve, Curve::Line))
            .ok_or(ReadError::Invalid)?;
        wire.rotate_left(first);
        let [a, seam_out, b, seam_back] = wire.as_slice() else {
            return Err(ReadError::Unsupported);
        };
        if matches!(self.edges[&a.edge].curve, Curve::Semicircle(_)) {
            return self.half_cylindrical_face(
                &surface,
                radius,
                outward,
                [a, seam_out, b, seam_back],
            );
        }
        let [Curve::Circle(circle_a), Curve::Circle(circle_b)] =
            [a, b].map(|edge| self.edges[&edge.edge].curve)
        else {
            return Err(ReadError::Invalid);
        };
        if a.edge == b.edge
            || seam_out.edge != seam_back.edge
            || seam_out.forward == seam_back.forward
            || seam_out.start == seam_out.end
            || radius <= self.tolerance
        {
            return Err(ReadError::Invalid);
        }
        for circle in [circle_a, circle_b] {
            let deviation = length(cross(sub(circle.center_mm, surface.origin), surface.normal));
            let tilt = circle.radius_mm * length(cross(circle.normal, surface.normal));
            if deviation > self.tolerance
                || tilt > self.tolerance
                || (circle.radius_mm - radius).abs() > self.tolerance
            {
                return Err(ReadError::Invalid);
            }
        }
        let height = dot(sub(circle_b.center_mm, circle_a.center_mm), surface.normal);
        let seam = sub(self.points[&seam_out.end], self.points[&seam_out.start]);
        let a_follows_axis = (dot(circle_a.normal, surface.normal) > 0.) == a.forward;
        let b_follows_axis = (dot(circle_b.normal, surface.normal) > 0.) == b.forward;
        if height.abs() <= self.tolerance
            || length(cross(seam, surface.normal)) > self.tolerance
            || a_follows_axis == b_follows_axis
            || a_follows_axis != (outward == (height > 0.))
        {
            return Err(ReadError::Invalid);
        }

        // Integrating the wall's radial normal over a full revolution cancels
        // both the origin and axial components of the position vector.
        let sign = if outward { 1. } else { -1. };
        Ok(Face::Cylindrical {
            seam: seam_out.edge,
            flux: sign * 2. * PI * radius.powi(2) * height.abs(),
        })
    }
}

pub(super) enum Trim {
    Polygon(Vec<Point2>),
    Circle {
        center: Point2,
        radius: f64,
        ccw: bool,
    },
    Stadium {
        profile: Stadium,
        ccw: bool,
    },
}

impl Trim {
    pub(super) fn signed_area(&self) -> f64 {
        match self {
            Self::Polygon(ring) => signed_area(ring),
            Self::Circle { radius, ccw, .. } => PI * radius.powi(2) * if *ccw { 1. } else { -1. },
            Self::Stadium { profile, ccw } => profile.area_mm2() * if *ccw { 1. } else { -1. },
        }
    }

    pub(super) fn simple(&self, tolerance: f64) -> bool {
        match self {
            Self::Polygon(ring) => simple_ring(ring, tolerance),
            Self::Circle { .. } | Self::Stadium { .. } => true,
        }
    }

    pub(super) fn strictly_contains(&self, inside: &Self, tolerance: f64) -> bool {
        match (self, inside) {
            (Self::Polygon(outer), Self::Polygon(inner)) => {
                contains(outer, inner[0]) && !rings_meet(outer, inner, tolerance)
            }
            (Self::Polygon(ring), Self::Circle { center, radius, .. }) => {
                contains(ring, *center) && polygon_clear_of_disk(ring, *center, *radius + tolerance)
            }
            (Self::Circle { center, radius, .. }, Self::Polygon(ring)) => ring
                .iter()
                .all(|&point| distance(*center, point) + tolerance < *radius),
            (
                Self::Circle { center, radius, .. },
                Self::Circle {
                    center: inner,
                    radius: inner_radius,
                    ..
                },
            ) => distance(*center, *inner) + inner_radius + tolerance < *radius,
            (Self::Polygon(ring), Self::Stadium { profile, .. }) => {
                contains(ring, profile.centers()[0])
                    && polygon_clear_of_stadium(ring, *profile, tolerance)
            }
            (Self::Stadium { profile, .. }, Self::Polygon(ring)) => ring
                .iter()
                .all(|&point| distance_to_spine(*profile, point) + tolerance < profile.radius_mm()),
            (Self::Circle { center, radius, .. }, Self::Stadium { profile, .. }) => profile
                .centers()
                .into_iter()
                .all(|point| distance(*center, point) + profile.radius_mm() + tolerance < *radius),
            (Self::Stadium { profile, .. }, Self::Circle { center, radius, .. }) => {
                distance_to_spine(*profile, *center) + radius + tolerance < profile.radius_mm()
            }
            (Self::Stadium { profile, .. }, Self::Stadium { profile: inner, .. }) => {
                inner.centers().into_iter().all(|point| {
                    distance_to_spine(*profile, point) + inner.radius_mm() + tolerance
                        < profile.radius_mm()
                })
            }
        }
    }

    pub(super) fn disjoint(&self, other: &Self, tolerance: f64) -> bool {
        match (self, other) {
            (Self::Polygon(a), Self::Polygon(b)) => {
                !contains(a, b[0]) && !contains(b, a[0]) && !rings_meet(a, b, tolerance)
            }
            (Self::Polygon(ring), Self::Circle { center, radius, .. })
            | (Self::Circle { center, radius, .. }, Self::Polygon(ring)) => {
                !contains(ring, *center)
                    && polygon_clear_of_disk(ring, *center, *radius + tolerance)
            }
            (
                Self::Circle { center, radius, .. },
                Self::Circle {
                    center: other,
                    radius: other_radius,
                    ..
                },
            ) => distance(*center, *other) > radius + other_radius + tolerance,
            (Self::Polygon(ring), Self::Stadium { profile, .. })
            | (Self::Stadium { profile, .. }, Self::Polygon(ring)) => {
                !contains(ring, profile.centers()[0])
                    && polygon_clear_of_stadium(ring, *profile, tolerance)
            }
            (Self::Circle { center, radius, .. }, Self::Stadium { profile, .. })
            | (Self::Stadium { profile, .. }, Self::Circle { center, radius, .. }) => {
                distance_to_spine(*profile, *center) > profile.radius_mm() + radius + tolerance
            }
            (Self::Stadium { profile, .. }, Self::Stadium { profile: other, .. }) => {
                segments_distance(profile.centers(), other.centers())
                    > profile.radius_mm() + other.radius_mm() + tolerance
            }
        }
    }
}

fn distance(a: Point2, b: Point2) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn polygon_clear_of_disk(ring: &[Point2], center: Point2, radius: f64) -> bool {
    edges(ring).all(|[a, b]| {
        let delta = [b[0] - a[0], b[1] - a[1]];
        let length_squared = delta[0].powi(2) + delta[1].powi(2);
        let projection =
            ((center[0] - a[0]) * delta[0] + (center[1] - a[1]) * delta[1]) / length_squared;
        let closest = std::array::from_fn(|i| a[i] + projection.clamp(0., 1.) * delta[i]);
        distance(center, closest) > radius
    })
}

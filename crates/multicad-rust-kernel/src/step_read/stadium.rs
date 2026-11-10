//! Exact trim and support checks for rounded-slot STEP boundaries.

use super::*;
use crate::stadium::{Stadium, segments_distance};
use std::f64::consts::PI;

pub(super) struct StadiumLoop {
    pub(super) edges: [u32; 4],
    pub(super) ccw: bool,
}

pub(super) struct HalfCylinder {
    pub(super) outward: bool,
    center: Point,
    radial_mid: Point,
    radius: f64,
    height: f64,
}

impl HalfCylinder {
    pub(super) fn flux(&self, anchor: Point) -> f64 {
        let sign = if self.outward { 1. } else { -1. };
        sign * self.height
            * self.radius
            * (PI * self.radius + 2. * dot(sub(self.center, anchor), self.radial_mid))
    }
}

impl SemicircleEdge {
    pub(super) fn circle(self) -> CircleEdge {
        CircleEdge {
            center_mm: self.center_mm,
            normal: self.normal,
            radius_mm: self.radius_mm,
        }
    }

    pub(super) fn include_bounds(self, bounds: &mut [Point; 2]) {
        let endpoints = cross(self.radial_mid, self.normal);
        for (i, end) in endpoints.into_iter().enumerate() {
            let extent = self.normal[(i + 1) % 3].hypot(self.normal[(i + 2) % 3]);
            let radial = self.radial_mid[i];
            let offsets = [
                if radial <= 0. { -extent } else { -end.abs() },
                if radial >= 0. { extent } else { end.abs() },
            ];
            bounds[0][i] = bounds[0][i].min(self.center_mm[i] + self.radius_mm * offsets[0]);
            bounds[1][i] = bounds[1][i].max(self.center_mm[i] + self.radius_mm * offsets[1]);
        }
    }
}

impl Shell<'_, '_> {
    pub(super) fn semicircle_curve(&self, circle: CircleEdge, a: Point, b: Point) -> Result<Curve> {
        if dot(b, circle.normal).abs() > self.tolerance
            || (length(cross(b, circle.normal)) - circle.radius_mm).abs() > self.tolerance
        {
            return Err(ReadError::Invalid);
        }
        let opposite: Point = std::array::from_fn(|i| a[i] + b[i]);
        if length(opposite) > self.tolerance {
            return Err(ReadError::Unsupported);
        }
        let radial_mid = unit(cross(circle.normal, a))?;
        Ok(Curve::Semicircle(SemicircleEdge {
            center_mm: circle.center_mm,
            normal: circle.normal,
            radius_mm: circle.radius_mm,
            radial_mid,
        }))
    }

    pub(super) fn half_cylindrical_face(
        &self,
        surface: &Plane,
        radius: f64,
        outward: bool,
        [a, rail_out, b, rail_back]: [&Traversal; 4],
    ) -> Result<Face> {
        let [Curve::Semicircle(arc_a), Curve::Semicircle(arc_b)] =
            [a, b].map(|edge| self.edges[&edge.edge].curve)
        else {
            return Err(ReadError::Invalid);
        };
        if rail_out.edge == rail_back.edge
            || radius <= self.tolerance
            || !matches!(
                (
                    self.edges[&rail_out.edge].curve,
                    self.edges[&rail_back.edge].curve
                ),
                (Curve::Line, Curve::Line)
            )
        {
            return Err(ReadError::Invalid);
        }
        for arc in [arc_a, arc_b] {
            let deviation = length(cross(sub(arc.center_mm, surface.origin), surface.normal));
            let tilt = arc.radius_mm * length(cross(arc.normal, surface.normal));
            if deviation > self.tolerance
                || tilt > self.tolerance
                || (arc.radius_mm - radius).abs() > self.tolerance
            {
                return Err(ReadError::Invalid);
            }
        }
        let centers = sub(arc_b.center_mm, arc_a.center_mm);
        let height = dot(centers, surface.normal);
        let follows = |arc: SemicircleEdge, edge: &Traversal| {
            (dot(arc.normal, surface.normal) > 0.) == edge.forward
        };
        let rail = |edge: &Traversal| sub(self.points[&edge.end], self.points[&edge.start]);
        if height.abs() <= self.tolerance
            || length(cross(centers, surface.normal)) > self.tolerance
            || radius * length(sub(arc_a.radial_mid, arc_b.radial_mid)) > self.tolerance
            || length(sub(rail(rail_out), centers)) > self.tolerance
            || length(sub(rail(rail_back), centers.map(|x| -x))) > self.tolerance
            || follows(arc_a, a) != (outward == (height > 0.))
            || follows(arc_a, a) == follows(arc_b, b)
        {
            return Err(ReadError::Invalid);
        }
        Ok(Face::HalfCylinder(HalfCylinder {
            outward,
            center: arc_a.center_mm,
            radial_mid: arc_a.radial_mid,
            radius,
            height: height.abs(),
        }))
    }

    pub(super) fn stadium_trim(&self, wire: &[Traversal], plane: &Plane) -> Result<Trim> {
        if wire.len() != 4 {
            return Err(ReadError::Unsupported);
        }
        let project = |point| {
            let delta = sub(point, plane.origin);
            [dot(delta, plane.u), dot(delta, plane.v)]
        };
        let arc = |edge: &Traversal| {
            let Curve::Semicircle(arc) = self.edges[&edge.edge].curve else {
                return None;
            };
            Some(arc)
        };
        let first = usize::from(arc(&wire[0]).is_none());
        let Some(a) = arc(&wire[first]) else {
            return Err(ReadError::Unsupported);
        };
        let Some(b) = arc(&wire[first + 2]) else {
            return Err(ReadError::Unsupported);
        };
        if ![wire[1 - first].edge, wire[3 - first].edge]
            .iter()
            .all(|id| matches!(self.edges[id].curve, Curve::Line))
        {
            return Err(ReadError::Unsupported);
        }
        let [a_edge, b_edge] = [&wire[first], &wire[first + 2]];
        let [a_center, b_center] = [a.center_mm, b.center_mm].map(project);
        let ccw = (dot(a.normal, plane.normal) > 0.) == a_edge.forward;
        let radius = a.radius_mm.midpoint(b.radius_mm);
        let profile = Stadium::new([a_center, b_center], radius).map_err(|_| ReadError::Invalid)?;
        if (a.radius_mm - b.radius_mm).abs() > self.tolerance
            || (dot(b.normal, plane.normal) > 0.) != (ccw == b_edge.forward)
            || !a.circle().within_plane(plane, self.tolerance)
            || !b.circle().within_plane(plane, self.tolerance)
        {
            return Err(ReadError::Invalid);
        }
        let spine = unit(sub(b.center_mm, a.center_mm))?;
        if radius * length(sub(a.radial_mid, spine.map(|x| -x))) > self.tolerance
            || radius * length(sub(b.radial_mid, spine)) > self.tolerance
        {
            return Err(ReadError::Invalid);
        }
        let [p0, p1, p2, p3] = profile.corners();
        let targets = if ccw {
            [p3, p0, p1, p2]
        } else {
            [p0, p3, p2, p1]
        };
        let observed = [a_edge.start, a_edge.end, b_edge.start, b_edge.end];
        for (target, id) in targets.into_iter().zip(observed) {
            let point = self.points[&id];
            let uv = project(point);
            if dot(sub(point, plane.origin), plane.normal).abs() > self.tolerance
                || (uv[0] - target[0]).hypot(uv[1] - target[1]) > self.tolerance
            {
                return Err(ReadError::Invalid);
            }
        }
        Ok(Trim::Stadium { profile, ccw })
    }
}

pub(super) fn polygon_clear_of_stadium(ring: &[Point2], profile: Stadium, tolerance: f64) -> bool {
    edges(ring)
        .all(|edge| segments_distance(edge, profile.centers()) > profile.radius_mm() + tolerance)
}

pub(super) fn distance_to_spine(profile: Stadium, point: Point2) -> f64 {
    segments_distance([point; 2], profile.centers())
}

#[cfg(test)]
mod tests;

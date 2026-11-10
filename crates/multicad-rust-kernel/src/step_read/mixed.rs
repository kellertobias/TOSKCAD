//! Certify general circular trims and their partial cylindrical supports.
use super::*;
use crate::normal_sweep::{Edge as ProfileEdge, Sweep};
use import::{frame_axes, reference_axis};
use std::f64::consts::TAU;

#[cfg(test)]
mod tests;

pub(super) struct PartialCylinder {
    center: Point,
    radial_mid: Point,
    radius: f64,
    height: f64,
    sweep: f64,
    outward: bool,
}

pub(super) struct IndexedSweep {
    pub(super) origin_mm: Point,
    pub(super) u: Point,
    pub(super) v: Point,
    pub(super) sweep: Sweep,
    pub(super) ordinals: Vec<u32>,
}

impl PartialCylinder {
    pub(super) fn flux(&self, anchor: Point) -> f64 {
        let sign = if self.outward { 1. } else { -1. };
        sign * self.height
            * self.radius
            * (self.sweep * self.radius
                + 2. * (self.sweep * 0.5).sin() * dot(sub(self.center, anchor), self.radial_mid))
    }
}

fn arc_angle(normal: Point, a: Point, b: Point) -> f64 {
    dot(cross(a, b), normal).atan2(dot(a, b)).rem_euclid(TAU)
}

impl ArcEdge {
    fn radial_mid(self) -> Point {
        let (sin, cos) = (self.sweep_rad * 0.5).sin_cos();
        let side = cross(self.normal, self.radial_start);
        std::array::from_fn(|i| cos * self.radial_start[i] + sin * side[i])
    }

    pub(super) fn include_bounds(self, bounds: &mut [Point; 2]) {
        for (coordinate, axis) in [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]
            .into_iter()
            .enumerate()
        {
            let planar = cross(cross(self.normal, axis), self.normal);
            let span = length(planar);
            if span == 0. {
                continue;
            }
            for (index, sign) in [-1., 1.].into_iter().enumerate() {
                let radial = planar.map(|v| v * sign / span);
                if arc_angle(self.normal, self.radial_start, radial) <= self.sweep_rad {
                    let point = self.center_mm[coordinate] + sign * self.radius_mm * span;
                    let value = &mut bounds[index][coordinate];
                    *value = if index == 0 {
                        value.min(point)
                    } else {
                        value.max(point)
                    };
                }
            }
        }
    }

    fn circle(self) -> CircleEdge {
        CircleEdge {
            center_mm: self.center_mm,
            normal: self.normal,
            radius_mm: self.radius_mm,
        }
    }
}

impl Shell<'_, '_> {
    pub(super) fn normal_sweep_profile(
        &self,
        ids: &[u32],
        lower: usize,
        direction: Point,
        height_mm: f64,
    ) -> Option<ImportSolid> {
        let indexed = self.indexed_normal_sweep_profile(ids, lower, direction, height_mm)?;
        Some(ImportSolid {
            origin_mm: indexed.origin_mm,
            u: indexed.u,
            v: indexed.v,
            height_mm,
            profile: ImportProfile::NormalSweep {
                sweep: indexed.sweep,
            },
        })
    }

    pub(super) fn indexed_normal_sweep_profile(
        &self,
        ids: &[u32],
        lower: usize,
        direction: Point,
        height_mm: f64,
    ) -> Option<IndexedSweep> {
        let [u, v] = frame_axes(direction, reference_axis(direction));
        let origin_mm = ids
            .iter()
            .flat_map(|id| self.edges[id].vertices.map(|vertex| self.points[&vertex]))
            .min_by(|a, b| {
                a.iter()
                    .zip(b)
                    .map(|(a, b)| a.total_cmp(b))
                    .find(|order| !order.is_eq())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })?;
        let plane = Plane {
            origin: origin_mm,
            normal: direction,
            u,
            v,
        };
        let mut successors = BTreeMap::new();
        let mut rings = Vec::new();
        for &id in ids {
            let edge = &self.edges[&id];
            // Reverse the lower cap's outward traversal for positive sweep.
            let forward = !edge.uses.iter().find(|use_| use_.face == lower)?.forward;
            let [start, end] = if forward {
                edge.vertices
            } else {
                [edge.vertices[1], edge.vertices[0]]
            };
            let directed = Traversal {
                edge: id,
                start,
                end,
                forward,
            };
            let curve = self.profile_edge(&directed, &plane).ok()?;
            if matches!(curve, ProfileEdge::Circle { .. }) {
                rings.push(vec![(id, curve)]);
            } else if successors.insert(start, (end, id, curve)).is_some() {
                return None;
            }
        }
        while let Some((&start, _)) = successors.first_key_value() {
            let mut current = start;
            let mut ring = Vec::new();
            loop {
                let (next, id, edge) = successors.remove(&current)?;
                ring.push((id, edge));
                if next == start {
                    break;
                }
                current = next;
            }
            rings.push(ring);
        }
        let (mut ordinals, mut profile): (Vec<Vec<u32>>, Vec<Vec<ProfileEdge>>) = rings
            .into_iter()
            .map(|ring| ring.into_iter().unzip())
            .unzip();
        let exterior = profile
            .iter()
            .position(|ring| crate::normal_sweep::signed_area(ring) > 0.)?;
        ordinals.swap(0, exterior);
        profile.swap(0, exterior);
        let sweep =
            Sweep::new(profile, height_mm, self.tolerance, self.tolerance.next_up()).ok()?;
        Some(IndexedSweep {
            origin_mm,
            u,
            v,
            sweep,
            ordinals: ordinals.into_iter().flatten().collect(),
        })
    }

    pub(super) fn open_circle_curve(
        &self,
        circle: CircleEdge,
        a: Point,
        b: Point,
    ) -> Result<Curve> {
        let opposite = std::array::from_fn(|i| a[i] + b[i]);
        if length(opposite) <= self.tolerance {
            return self.semicircle_curve(circle, a, b);
        }
        if dot(b, circle.normal).abs() > self.tolerance
            || (length(cross(b, circle.normal)) - circle.radius_mm).abs() > self.tolerance
            || length(sub(a, b)) <= self.tolerance
        {
            return Err(ReadError::Invalid);
        }
        let radial_start = unit(a)?;
        let radial_end = unit(b)?;
        Ok(Curve::Arc(ArcEdge {
            center_mm: circle.center_mm,
            normal: circle.normal,
            radius_mm: circle.radius_mm,
            radial_start,
            radial_end,
            sweep_rad: arc_angle(circle.normal, radial_start, radial_end),
        }))
    }

    pub(super) fn partial_cylindrical_face(
        &self,
        surface: &Plane,
        radius: f64,
        outward: bool,
        [a, rail_out, b, rail_back]: [&Traversal; 4],
    ) -> Result<Face> {
        let [Curve::Arc(low), Curve::Arc(high)] = [a, b].map(|edge| self.edges[&edge.edge].curve)
        else {
            return Err(ReadError::Invalid);
        };
        if rail_out.edge == rail_back.edge
            || radius <= self.tolerance
            || ![rail_out, rail_back]
                .iter()
                .all(|rail| matches!(self.edges[&rail.edge].curve, Curve::Line))
        {
            return Err(ReadError::Invalid);
        }
        for arc in [low, high] {
            let deviation = length(cross(sub(arc.center_mm, surface.origin), surface.normal));
            let tilt = arc.radius_mm * length(cross(arc.normal, surface.normal));
            if deviation > self.tolerance
                || tilt > self.tolerance
                || (arc.radius_mm - radius).abs() > self.tolerance
            {
                return Err(ReadError::Invalid);
            }
        }
        let centers = sub(high.center_mm, low.center_mm);
        let height = dot(centers, surface.normal);
        let follows =
            |arc: ArcEdge, edge: &Traversal| (dot(arc.normal, surface.normal) > 0.) == edge.forward;
        let rail = |edge: &Traversal| sub(self.points[&edge.end], self.points[&edge.start]);
        let radial_mid = low.radial_mid();
        if height.abs() <= self.tolerance
            || radius * (low.sweep_rad - high.sweep_rad).abs() > self.tolerance
            || radius * length(sub(radial_mid, high.radial_mid())) > self.tolerance
            || length(sub(rail(rail_out), centers)) > self.tolerance
            || length(sub(rail(rail_back), centers.map(|v| -v))) > self.tolerance
            || follows(low, a) != (outward == (height > 0.))
            || follows(low, a) == follows(high, b)
        {
            return Err(ReadError::Invalid);
        }
        Ok(Face::PartialCylinder(PartialCylinder {
            center: low.center_mm,
            radial_mid,
            radius,
            height: height.abs(),
            sweep: low.sweep_rad,
            outward,
        }))
    }

    pub(super) fn mixed_trim(&self, wire: &[Traversal], plane: &Plane) -> Result<Trim> {
        let edges = wire
            .iter()
            .map(|item| self.profile_edge(item, plane))
            .collect::<Result<Vec<_>>>()?;
        Ok(Trim::Mixed(edges))
    }

    fn profile_edge(&self, item: &Traversal, plane: &Plane) -> Result<ProfileEdge> {
        let project = |point| {
            let delta = sub(point, plane.origin);
            (dot(delta, plane.normal).abs() <= self.tolerance)
                .then(|| [dot(delta, plane.u), dot(delta, plane.v)])
                .ok_or(ReadError::Invalid)
        };
        let a = project(self.points[&item.start])?;
        let b = project(self.points[&item.end])?;
        let curve = self.edges[&item.edge].curve;
        let circle = match curve {
            Curve::Line => return Ok(ProfileEdge::Line { a, b }),
            Curve::Arc(arc) => arc.circle(),
            Curve::Semicircle(arc) => arc.circle(),
            Curve::Circle(circle) => circle,
        };
        if !circle.within_plane(plane, self.tolerance) {
            return Err(ReadError::Invalid);
        }
        let center = project(circle.center_mm)?;
        let clockwise = (dot(circle.normal, plane.normal) > 0.) != item.forward;
        if matches!(curve, Curve::Circle(_)) {
            Ok(ProfileEdge::Circle {
                center,
                radius_mm: circle.radius_mm,
                clockwise,
            })
        } else {
            Ok(ProfileEdge::Arc {
                center,
                a,
                b,
                clockwise,
            })
        }
    }
}

pub(super) fn face_area(outer: &Trim, holes: &[Trim], tolerance: f64) -> Result<f64> {
    let rings = std::iter::once(outer)
        .chain(holes)
        .map(profile_ring)
        .collect();
    let minimum = tolerance.next_up();
    let sweep = Sweep::new(rings, 1_f64.max(minimum), tolerance, minimum)
        .map_err(|_| ReadError::Invalid)?;
    Ok(sweep.area_mm2())
}

pub(super) fn duplicate_curve(a: Curve, b: Curve, tolerance: f64) -> bool {
    let middle = |curve| {
        let (center, radius, radial) = match curve {
            Curve::Semicircle(edge) => (edge.center_mm, edge.radius_mm, edge.radial_mid),
            Curve::Arc(edge) => (edge.center_mm, edge.radius_mm, edge.radial_mid()),
            _ => return None,
        };
        Some(std::array::from_fn(|i| center[i] + radial[i] * radius))
    };
    match (a, b) {
        (Curve::Line, Curve::Line) | (Curve::Circle(_), Curve::Circle(_)) => true,
        _ => match (middle(a), middle(b)) {
            (Some(a), Some(b)) => length(sub(a, b)) <= tolerance,
            _ => false,
        },
    }
}

fn profile_ring(trim: &Trim) -> Vec<ProfileEdge> {
    match trim {
        Trim::Polygon(ring) => edges(ring)
            .map(|[a, b]| ProfileEdge::Line { a, b })
            .collect(),
        Trim::Circle {
            center,
            radius,
            ccw,
        } => vec![ProfileEdge::Circle {
            center: *center,
            radius_mm: *radius,
            clockwise: !ccw,
        }],
        Trim::Stadium { profile, ccw } => {
            let [a, b, c, d] = profile.corners();
            let [low, high] = profile.centers();
            let mut ring = vec![
                ProfileEdge::Line { a, b },
                ProfileEdge::Arc {
                    center: high,
                    a: b,
                    b: c,
                    clockwise: false,
                },
                ProfileEdge::Line { a: c, b: d },
                ProfileEdge::Arc {
                    center: low,
                    a: d,
                    b: a,
                    clockwise: false,
                },
            ];
            if !ccw {
                ring = ring.iter().rev().map(|edge| edge.reversed()).collect();
            }
            ring
        }
        Trim::Mixed(ring) => ring.clone(),
    }
}

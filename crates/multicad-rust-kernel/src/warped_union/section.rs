//! Plane sections of exact planar and axially trimmed circular shell faces.
use super::*;
use crate::trim_curve::{Plane, PlaneCrossings};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SectionEdge {
    /// Shared endpoint positions, ordered with material to the left of the edge.
    pub ends_mm: [Point; 2],
    pub curve: Curve,
}

type Port = [usize; 2];

#[derive(Clone, Copy)]
struct Hit {
    port: Port,
    point: Point,
}

struct Section {
    origin: Point,
    normal: Point,
    tolerance: f64,
    limit: usize,
    hits: Vec<Vec<Hit>>,
    edges: BTreeMap<Port, (Port, SectionEdge)>,
}

const ANGULAR: f64 = 64. * f64::EPSILON;

impl Shell {
    /// Intersect the boundary with a transverse cutting plane. Cylindrical
    /// patches need circular cap trims, straight generators, and a section
    /// normal parallel or perpendicular to the cylinder axis.
    pub fn section(
        &self,
        origin_mm: Point,
        normal: Point,
        tolerance_mm: f64,
        segment_limit: usize,
    ) -> Result<Vec<Vec<SectionEdge>>, Error> {
        if !(1e-7..=1e-3).contains(&tolerance_mm)
            || origin_mm
                .into_iter()
                .any(|v| !v.is_finite() || v.abs() > 1e8)
        {
            return Err(Error::Invalid);
        }
        let normal = unit(normal)?;
        let plane = Plane::new(origin_mm, normal).map_err(|_| Error::Invalid)?;
        let signed = |p| dot(sub(p, origin_mm), normal);
        let distances: Vec<_> = self.vertices.iter().copied().map(signed).collect();
        if distances.iter().all(|&d| d > tolerance_mm)
            || distances.iter().all(|&d| d < -tolerance_mm)
        {
            // Ellipse extrema may lie outside the vertex projection.
            let outside = self.edges.iter().all(|edge| match edge.curve {
                Curve::Line => true,
                Curve::Ellipse(arc) => {
                    let [lo, hi] = arc.range_along(normal);
                    let cut = dot(origin_mm, normal);
                    lo > cut + tolerance_mm || hi < cut - tolerance_mm
                }
            });
            if outside {
                return Ok(Vec::new());
            }
        }
        // A vertex or an entire trim on the plane needs a regularized
        // contact rule, distinct from the transverse crossing proof below.
        if distances.iter().any(|d| d.abs() <= tolerance_mm) {
            return Err(Error::Unsupported);
        }
        let hits = self
            .edges
            .iter()
            .enumerate()
            .map(|(index, edge)| {
                let points = match edge.curve {
                    Curve::Line => {
                        let [da, db] = edge.vertices.map(|id| distances[id]);
                        if da.is_sign_positive() == db.is_sign_positive() {
                            Vec::new()
                        } else {
                            let [a, b] = edge.vertices.map(|id| self.vertices[id]);
                            vec![translate(a, sub(b, a), da / (da - db))]
                        }
                    }
                    Curve::Ellipse(arc) => arc_hits(arc, plane, tolerance_mm)?,
                };
                Ok(points
                    .into_iter()
                    .enumerate()
                    .map(|(ordinal, point)| Hit {
                        port: [index, ordinal],
                        point,
                    })
                    .collect())
            })
            .collect::<Result<_, _>>()?;
        let mut section = Section {
            origin: origin_mm,
            normal,
            tolerance: tolerance_mm,
            limit: segment_limit,
            hits,
            edges: BTreeMap::new(),
        };
        for face in &self.faces {
            match face.support {
                Support::Plane { normal } => section.planar(face, normal)?,
                Support::Cylinder {
                    axis_origin_mm,
                    axis_direction,
                    radius_mm,
                    outward,
                } => section.cylinder(
                    self,
                    face,
                    axis_origin_mm,
                    axis_direction,
                    radius_mm,
                    outward,
                )?,
            }
        }
        section.loops()
    }
}

fn translate(a: Point, b: Point, t: f64) -> Point {
    std::array::from_fn(|i| b[i].mul_add(t, a[i]))
}

fn unit(v: Point) -> Result<Point, Error> {
    let length = length3(v);
    if !length.is_finite() || length <= ANGULAR {
        return Err(Error::Invalid);
    }
    Ok(v.map(|value| value / length))
}

fn travel(arc: EllipseArc, angle: f64) -> f64 {
    ((angle - arc.start_rad()) * arc.sweep_rad().signum()).rem_euclid(std::f64::consts::TAU)
}

fn arc_hits(arc: EllipseArc, plane: Plane, tolerance: f64) -> Result<Vec<Point>, Error> {
    let ellipse = arc.ellipse();
    let within = |angle| travel(arc, angle) < arc.sweep_rad().abs();
    match ellipse
        .plane_crossings(plane, tolerance)
        .map_err(|_| Error::Unsupported)?
    {
        PlaneCrossings::None => Ok(Vec::new()),
        PlaneCrossings::Tangent(angle) if !within(angle) => Ok(Vec::new()),
        PlaneCrossings::Coincident | PlaneCrossings::Tangent(_) => Err(Error::Unsupported),
        PlaneCrossings::Crossing(angles) => Ok(angles
            .into_iter()
            .filter(|&angle| within(angle))
            .map(|angle| ellipse.point_at(angle))
            .collect()),
    }
}

fn axis_offset(point: Point, origin: Point, axis: Point) -> Point {
    let delta = sub(point, origin);
    translate(delta, axis, -dot(delta, axis))
}

struct CylinderPatch {
    arcs: [(usize, EllipseArc); 2],
    rails: [usize; 2],
}

impl CylinderPatch {
    fn new(
        shell: &Shell,
        face: &Face,
        origin: Point,
        axis: Point,
        radius: f64,
        tolerance: f64,
    ) -> Result<Self, Error> {
        let [a, b_rail, c, d] = face.boundary.as_slice() else {
            return Err(Error::Unsupported);
        };
        let arcs = [a.edge, c.edge].map(|edge| {
            let Curve::Ellipse(arc) = shell.edges[edge].curve else {
                return Err(Error::Unsupported);
            };
            let ellipse = arc.ellipse();
            let [major, minor] = ellipse.semiaxes_mm();
            if length3(cross(ellipse.normal(), axis)) > ANGULAR
                || length3(axis_offset(ellipse.center_mm(), origin, axis)) > tolerance
                || major != minor
                || (major - radius).abs() > tolerance
                || arc.sweep_rad().abs() > FRAC_PI_2 + ANGULAR
            {
                return Err(Error::Unsupported);
            }
            Ok((edge, arc))
        });
        let [a, b] = arcs;
        let arcs = [a?, b?];
        let rails = [b_rail.edge, d.edge];
        // The existing closed four-edge ring connects each circular endpoint
        // to the matching radial endpoint at the other cap. The short arc
        // between the pair is unique, so the wall is a rectangular chart.
        for edge in rails.map(|edge| shell.edges[edge]) {
            let [a, b] = edge.vertices.map(|i| shell.vertices[i]);
            let span = sub(b, a);
            if edge.curve != Curve::Line
                || length3(cross(span, axis)) > tolerance
                || dot(span, axis).abs() <= tolerance
            {
                return Err(Error::Unsupported);
            }
        }
        Ok(Self { arcs, rails })
    }
}

impl Section {
    fn planar(&mut self, face: &Face, normal: Point) -> Result<(), Error> {
        let tangent = cross(self.normal, normal);
        let mut hits: Vec<_> = std::iter::once(&face.boundary)
            .chain(&face.holes)
            .flatten()
            .flat_map(|edge| self.hits[edge.edge].iter().copied())
            .collect();
        hits.sort_unstable_by(|a, b| dot(a.point, tangent).total_cmp(&dot(b.point, tangent)));
        let (pairs, remainder) = hits.as_chunks::<2>();
        if !remainder.is_empty() {
            return Err(Error::Unsupported);
        }
        for &pair in pairs {
            self.add(pair, Curve::Line)?;
        }
        Ok(())
    }

    fn cylinder(
        &mut self,
        shell: &Shell,
        face: &Face,
        axis_origin: Point,
        axis: Point,
        radius: f64,
        outward: bool,
    ) -> Result<(), Error> {
        if face
            .boundary
            .iter()
            .all(|edge| self.hits[edge.edge].is_empty())
        {
            return Ok(());
        }
        let patch = CylinderPatch::new(shell, face, axis_origin, axis, radius, self.tolerance)?;
        let cosine = dot(axis, self.normal);
        if length3(cross(axis, self.normal)) <= ANGULAR {
            let (_, original) = patch.arcs[0];
            let cap = original.ellipse();
            let offset = dot(sub(self.origin, cap.center_mm()), self.normal) / cosine;
            let ellipse = Ellipse::new(
                translate(cap.center_mm(), axis, offset),
                cap.normal(),
                cap.major_axis(),
                cap.semiaxes_mm(),
            )
            .map_err(|_| Error::Unsupported)?;
            let arc = EllipseArc::new(ellipse, original.start_rad(), original.sweep_rad())
                .map_err(|_| Error::Unsupported)?;
            let [[first], [last]] = patch.rails.map(|id| self.hits[id].as_slice()) else {
                return Err(Error::Unsupported);
            };
            let ends = arc.endpoints();
            let mut pair = [*first, *last];
            if length3(sub(pair[0].point, ends[0])) > self.tolerance {
                pair.reverse();
            }
            if pair
                .iter()
                .zip(ends)
                .any(|(hit, end)| length3(sub(hit.point, end)) > self.tolerance)
            {
                return Err(Error::Unsupported);
            }
            let positive = dot(cap.normal(), self.normal) * arc.sweep_rad() > 0.;
            if positive == outward {
                self.add(pair, Curve::Ellipse(arc))
            } else {
                pair.reverse();
                let reverse =
                    EllipseArc::new(ellipse, arc.start_rad() + arc.sweep_rad(), -arc.sweep_rad())
                        .map_err(|_| Error::Unsupported)?;
                self.add(pair, Curve::Ellipse(reverse))
            }
        } else if cosine.abs() <= ANGULAR {
            let [a, b] = patch.arcs.map(|(id, _)| &self.hits[id]);
            if a.len() != b.len() {
                return Err(Error::Unsupported);
            }
            let mut used = [false; 2];
            let mut pairs = Vec::new();
            for &first in a {
                let (index, &last) = b
                    .iter()
                    .enumerate()
                    .find(|&(index, hit)| {
                        !used[index]
                            && length3(axis_offset(hit.point, first.point, axis)) <= self.tolerance
                    })
                    .ok_or(Error::Unsupported)?;
                used[index] = true;
                let radial = axis_offset(first.point, axis_origin, axis);
                let tangent = cross(self.normal, radial);
                let mut pair = [first, last];
                if (dot(sub(last.point, first.point), tangent) > 0.) != outward {
                    pair.reverse();
                }
                pairs.push(pair);
            }
            for pair in pairs {
                self.add(pair, Curve::Line)?;
            }
            Ok(())
        } else {
            Err(Error::Unsupported)
        }
    }

    fn add(&mut self, [a, b]: [Hit; 2], curve: Curve) -> Result<(), Error> {
        if length3(sub(a.point, b.point)) <= self.tolerance {
            return Err(Error::Unsupported);
        }
        self.limit = self.limit.checked_sub(1).ok_or(Error::Limit)?;
        let edge = SectionEdge {
            ends_mm: [a.point, b.point],
            curve,
        };
        if self.edges.insert(a.port, (b.port, edge)).is_some() {
            return Err(Error::Unsupported);
        }
        Ok(())
    }

    fn loops(mut self) -> Result<Vec<Vec<SectionEdge>>, Error> {
        let mut loops = Vec::new();
        while let Some((&start, _)) = self.edges.first_key_value() {
            let mut boundary = Vec::new();
            let mut next = start;
            loop {
                let (end, edge) = self.edges.remove(&next).ok_or(Error::Unsupported)?;
                boundary.push(edge);
                if end == start {
                    break;
                }
                next = end;
            }
            loops.push(boundary);
        }
        Ok(loops)
    }
}

#[cfg(test)]
mod tests;

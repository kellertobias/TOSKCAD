//! Produce physical HLR edges and exact blockers from a certified analytic shell.
use super::{
    cylinder_patch::{ANGULAR, CylinderPatch, axis_offset, tilted, trim_station},
    *,
};
use crate::{
    curved_projection,
    planar_projection::{
        AnalyticFace, FaceTrim, LineCandidate, ProjectionError, SpatialEllipseCandidate,
        SpatialRimCandidate,
    },
};
use std::f64::consts::TAU;

/// Drawing-space geometry of an orthographic shell elevation.
#[derive(Default)]
pub struct Elevation {
    pub blockers: Vec<AnalyticFace>,
    pub lines: Vec<LineCandidate>,
    pub rims: Vec<SpatialRimCandidate>,
    pub spatial_ellipses: Vec<SpatialEllipseCandidate>,
}

impl Shell {
    /// Prepare exact hidden-line inputs, without projecting mesh edges.
    ///
    /// The view axes are right-handed U, V and toward-viewer directions
    /// in shell coordinates. Cylindrical walls have straight generators and
    /// use circular rims or an exact tilted miter trim.
    pub fn elevation(
        &self,
        view_origin_mm: Point,
        view_axes: [Point; 3],
        source: usize,
        tolerance_mm: f64,
        tangent_edges: bool,
    ) -> Result<Elevation, Error> {
        let view = View::new(view_origin_mm, view_axes, tolerance_mm)?;
        let mut result = Elevation::default();
        let mut incidence = vec![[None; 2]; self.edges.len()];
        let mut silhouettes = vec![false; self.edges.len()];
        for face in &self.faces {
            for usage in face.boundary.iter().chain(face.holes.iter().flatten()) {
                let side = &mut incidence[usage.edge][usize::from(usage.forward)];
                if side.replace(face.support).is_some() {
                    return Err(Error::Invalid);
                }
            }
            let blocker = match face.support {
                Support::Plane { normal } => {
                    let normal = view.vector(normal);
                    if normal[2] <= ANGULAR {
                        continue;
                    }
                    let rings = std::iter::once(&face.boundary)
                        .chain(&face.holes)
                        .map(|wire| {
                            wire.iter()
                                .map(|usage| view.trim(self, *usage))
                                .collect::<Result<Vec<_>, _>>()
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    AnalyticFace::plane(view.point(face.point_mm), normal, rings, tolerance_mm)
                }
                Support::Cylinder {
                    axis_origin_mm,
                    axis_direction,
                    radius_mm,
                    outward,
                } => {
                    let patch = CylinderPatch::new(
                        self,
                        face,
                        axis_origin_mm,
                        axis_direction,
                        radius_mm,
                        tolerance_mm,
                    )?;
                    let [(_, cap), (_, end)] = patch.arcs;
                    let radial = patch.radial_chart(axis_direction)?;
                    let events = patch.silhouettes(
                        self,
                        radial,
                        view_axes[2],
                        axis_direction,
                        tolerance_mm,
                    )?;
                    for event in events.into_iter().flatten() {
                        match event {
                            Silhouette::Rail(edge) => silhouettes[edge] = true,
                            Silhouette::Interior(points) => result.lines.push(LineCandidate {
                                source,
                                points: points.map(|point| view.point(point)),
                            }),
                        }
                    }
                    let span = sub(end.ellipse().center_mm(), cap.ellipse().center_mm());
                    let height = dot(span, axis_direction);
                    let axial = view
                        .vector(axis_direction)
                        .map(|coordinate| coordinate * height.signum());
                    let chart = view.arc(radial, true)?;
                    if tilted(cap, axis_direction) {
                        AnalyticFace::cylinder_with_trims(
                            chart,
                            [view.arc(cap, true)?, view.arc(end, true)?],
                            axial,
                            outward,
                            tolerance_mm,
                        )
                    } else if tilted(end, axis_direction) {
                        AnalyticFace::tilted_cylinder(
                            chart,
                            view.arc(end, true)?,
                            axial,
                            outward,
                            tolerance_mm,
                        )
                    } else {
                        AnalyticFace::cylinder(chart, axial, height.abs(), outward, tolerance_mm)
                    }
                }
            }
            .map_err(projection_error)?;
            if let Some(blocker) = blocker {
                result.blockers.push(blocker);
            }
        }
        for (index, edge) in self.edges.iter().enumerate() {
            let [Some(left), Some(right)] = incidence[index] else {
                return Err(Error::Invalid);
            };
            let points = edge.vertices.map(|vertex| self.vertices[vertex]);
            if !silhouettes[index]
                && (same_support(left, right, tolerance_mm)
                    || !tangent_edges && smooth(left, right, points))
            {
                continue;
            }
            match edge.curve {
                Curve::Line => result.lines.push(LineCandidate {
                    source,
                    points: points.map(|point| view.point(point)),
                }),
                Curve::Ellipse(arc) if noncircular(arc) => {
                    result.spatial_ellipses.extend(view.ellipses(source, arc));
                }
                Curve::Ellipse(arc) => result.rims.extend(view.rims(source, arc)),
            }
        }
        Ok(result)
    }
}

fn noncircular(arc: EllipseArc) -> bool {
    let [a, b] = arc.ellipse().semiaxes_mm();
    a != b
}

fn projection_error(error: ProjectionError) -> Error {
    match error {
        ProjectionError::Invalid => Error::Invalid,
        ProjectionError::Limit => Error::Limit,
    }
}

struct View {
    origin: Point,
    axes: [Point; 3],
}

impl View {
    fn new(origin: Point, axes: [Point; 3], tolerance: f64) -> Result<Self, Error> {
        let [u, v, n] = axes;
        if !(1e-7..=1e-3).contains(&tolerance)
            || origin.into_iter().any(|x| !x.is_finite() || x.abs() > 1e8)
            || axes
                .iter()
                .any(|&axis| !(1. - FRAME_EPS..=1. + FRAME_EPS).contains(&dot(axis, axis)))
            || dot(u, v).abs() > FRAME_EPS
            || length3(sub(cross(u, v), n)) > FRAME_EPS
        {
            return Err(Error::Invalid);
        }
        Ok(Self { origin, axes })
    }

    fn point(&self, p: Point) -> Point {
        self.vector(sub(p, self.origin))
    }

    fn vector(&self, v: Point) -> Point {
        self.axes.map(|axis| dot(axis, v))
    }

    fn arc(&self, arc: EllipseArc, forward: bool) -> Result<EllipseArc, Error> {
        let ellipse = arc.ellipse();
        let projected = Ellipse::new(
            self.point(ellipse.center_mm()),
            self.vector(ellipse.normal()),
            self.vector(ellipse.major_axis()),
            ellipse.semiaxes_mm(),
        )
        .map_err(|_| Error::Invalid)?;
        let (start, sweep) = (arc.start_rad(), arc.sweep_rad());
        let (start, sweep) = if forward {
            (start, sweep)
        } else {
            (start + sweep, -sweep)
        };
        EllipseArc::new(projected, start, sweep).map_err(|_| Error::Invalid)
    }

    fn trim(&self, shell: &Shell, usage: OrientedEdge) -> Result<FaceTrim, Error> {
        let edge = shell.edges[usage.edge];
        match edge.curve {
            Curve::Line => {
                let mut vertices = edge.vertices;
                if !usage.forward {
                    vertices.reverse();
                }
                Ok(FaceTrim::Line(
                    vertices.map(|index| self.point(shell.vertices[index])),
                ))
            }
            Curve::Ellipse(arc) => self.arc(arc, usage.forward).map(FaceTrim::Arc),
        }
    }

    fn rims(&self, source: usize, arc: EllipseArc) -> impl Iterator<Item = SpatialRimCandidate> {
        let ellipse = arc.ellipse();
        let [radius, _] = ellipse.semiaxes_mm();
        let [u, v] = [ellipse.major_axis(), ellipse.minor_axis()].map(|basis| self.vector(basis));
        let inclination = u[2].hypot(v[2]);
        let [cos, sin] = if inclination <= ANGULAR {
            [1., 0.]
        } else {
            [v[2] / inclination, -u[2] / inclination]
        };
        let major: Point = std::array::from_fn(|i| cos.mul_add(u[i], sin * v[i]));
        let minor: Point = std::array::from_fn(|i| (-sin).mul_add(u[i], cos * v[i]));
        let screen_u = [major[0], major[1]];
        let projected = minor[0].hypot(minor[1]);
        let (screen_v, radius_v) = if projected <= ANGULAR {
            ([-screen_u[1], screen_u[0]], 0.)
        } else {
            (
                [minor[0] / projected, minor[1] / projected],
                radius * projected.min(1.),
            )
        };
        let [x, y, depth] = self.point(ellipse.center_mm());
        let sweep = arc.sweep_rad();
        let lo = (arc.start_rad() + sweep.min(0.) - sin.atan2(cos)).rem_euclid(TAU);
        let hi = lo + sweep.abs();
        [[lo, hi.min(TAU)], [0., hi - TAU]]
            .into_iter()
            .filter(|[a, b]| b - a > ANGULAR)
            .map(move |trim| SpatialRimCandidate {
                source,
                ellipse: curved_projection::Ellipse {
                    center: [x, y],
                    u: screen_u,
                    v: screen_v,
                    major: radius,
                    minor: radius_v,
                    trim,
                },
                depth,
                depth_sine: radius * minor[2],
            })
    }

    fn ellipses(
        &self,
        source: usize,
        arc: EllipseArc,
    ) -> impl Iterator<Item = SpatialEllipseCandidate> {
        let ellipse = arc.ellipse();
        let [u, v] = [ellipse.major_axis(), ellipse.minor_axis()].map(|axis| self.vector(axis));
        let [major, minor] = ellipse.semiaxes_mm();
        let [a, b] = [u.map(|x| major * x), v.map(|x| minor * x)];
        // Diagonalize the screen Gram matrix, applying the same phase to the
        // depth components so the projected trim retains its 3D position.
        let aa = a[0].mul_add(a[0], a[1] * a[1]);
        let bb = b[0].mul_add(b[0], b[1] * b[1]);
        let ab = a[0].mul_add(b[0], a[1] * b[1]);
        let phase = 0.5 * (2. * ab).atan2(aa - bb);
        let (sin, cos) = phase.sin_cos();
        let major_axis: Point = std::array::from_fn(|i| cos.mul_add(a[i], sin * b[i]));
        let minor_axis: Point = std::array::from_fn(|i| (-sin).mul_add(a[i], cos * b[i]));
        let major = major_axis[0].hypot(major_axis[1]);
        let u = [major_axis[0] / major, major_axis[1] / major];
        let perpendicular = [-u[1], u[0]];
        let signed = minor_axis[0].mul_add(perpendicular[0], minor_axis[1] * perpendicular[1]);
        let minor = if signed.abs() <= ANGULAR * major {
            0.
        } else {
            signed.abs().min(major)
        };
        let v = perpendicular.map(|value| value * signed.signum());
        let [x, y, depth] = self.point(ellipse.center_mm());
        let sweep = arc.sweep_rad();
        let lo = (arc.start_rad() + sweep.min(0.) - phase).rem_euclid(TAU);
        let hi = lo + sweep.abs();
        [[lo, hi.min(TAU)], [0., hi - TAU]]
            .into_iter()
            .filter(|[a, b]| b - a > ANGULAR)
            .map(move |trim| SpatialEllipseCandidate {
                source,
                ellipse: curved_projection::Ellipse {
                    center: [x, y],
                    u,
                    v,
                    major,
                    minor,
                    trim,
                },
                depth,
                depth_cosine: major_axis[2],
                depth_sine: minor_axis[2],
            })
    }
}

fn same_support(a: Support, b: Support, tolerance: f64) -> bool {
    match (a, b) {
        (Support::Plane { normal: a }, Support::Plane { normal: b }) => dot(a, b) > 1. - FRAME_EPS,
        (
            Support::Cylinder {
                axis_origin_mm: a,
                axis_direction: axis_a,
                radius_mm: radius_a,
                outward: outward_a,
            },
            Support::Cylinder {
                axis_origin_mm: b,
                axis_direction: axis_b,
                radius_mm: radius_b,
                outward: outward_b,
            },
        ) => {
            outward_a == outward_b
                && (radius_a - radius_b).abs() <= tolerance
                && length3(cross(axis_a, axis_b)) <= FRAME_EPS
                && length3(axis_offset(a, b, axis_b)) <= tolerance
        }
        _ => false,
    }
}

fn smooth(a: Support, b: Support, points: [Point; 2]) -> bool {
    let normal = |surface, point| match surface {
        Support::Plane { normal } => normal,
        Support::Cylinder {
            axis_origin_mm,
            axis_direction,
            radius_mm,
            outward,
        } => {
            let radial = axis_offset(point, axis_origin_mm, axis_direction);
            let factor = (if outward { 1. } else { -1. }) / radius_mm;
            radial.map(|coordinate| coordinate * factor)
        }
    };
    points
        .into_iter()
        .all(|point| dot(normal(a, point), normal(b, point)) > 1. - FRAME_EPS)
}

enum Silhouette {
    Rail(usize),
    Interior([Point; 2]),
}

impl CylinderPatch {
    fn silhouettes(
        &self,
        shell: &Shell,
        chart: EllipseArc,
        toward: Point,
        axis: Point,
        tolerance: f64,
    ) -> Result<[Option<Silhouette>; 2], Error> {
        let [(cap_edge, _), _] = self.arcs;
        let ellipse = chart.ellipse();
        let [radius, _] = ellipse.semiaxes_mm();
        let radial = cross(toward, axis);
        let length = length3(radial);
        if length <= ANGULAR {
            return Ok([None, None]);
        }
        // An oblique view reduces the cross product's magnitude without
        // changing its azimuth. The silhouette remains on the true cylinder.
        let radial = radial.map(|coordinate| coordinate / length);
        let silhouette = |sign| {
            let radial = radial.map(|coordinate| sign * coordinate);
            let point: Point =
                std::array::from_fn(|i| radius.mul_add(radial[i], ellipse.center_mm()[i]));
            let points = self.arcs.map(|(_, rim)| {
                let station = trim_station(rim, point, axis);
                std::array::from_fn(|i| axis[i].mul_add(station, point[i]))
            });
            // Reuse a real edge when the silhouette coincides with its
            // generator. Two smooth neighboring sectors can then publish it once.
            for vertex in shell.edges[cap_edge].vertices {
                if length3(sub(points[0], shell.vertices[vertex])) <= tolerance {
                    let edge = self
                        .rails
                        .into_iter()
                        .find(|&index| shell.edges[index].vertices.contains(&vertex))
                        .ok_or(Error::Invalid)?;
                    return Ok(Some(Silhouette::Rail(edge)));
                }
            }
            let phase = dot(radial, ellipse.minor_axis()).atan2(dot(radial, ellipse.major_axis()));
            let travel = ((phase - chart.start_rad()) * chart.sweep_rad().signum()).rem_euclid(TAU);
            Ok((travel < chart.sweep_rad().abs()).then_some(Silhouette::Interior(points)))
        };
        Ok([silhouette(-1.)?, silhouette(1.)?])
    }
}

#[cfg(test)]
mod tests;

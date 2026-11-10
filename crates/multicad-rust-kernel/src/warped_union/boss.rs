//! Attach a cylinder only when its swept disk crosses exactly one stock face.
use super::*;
use std::f64::consts::PI;

const MAX_BOSSES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Boss {
    pub start_mm: Point,
    pub end_mm: Point,
    pub radius_mm: f64,
    pub source: usize,
    pub wall_source_face: usize,
    pub cap_source_face: usize,
}

struct Attachment {
    tool: Boss,
    host: usize,
    mouth: Point,
    axes: [Point; 3],
    height: f64,
}

impl Shell {
    /// Join mutually separated bosses with an embedded start and one
    /// perpendicular exit through strictly interior planar material.
    pub fn attach_bosses(&self, bosses: &[Boss], tolerance_mm: f64) -> Result<Self, Error> {
        self.attach_bosses_inner(
            bosses.len(),
            bosses.iter().map(|&boss| (boss, None)),
            tolerance_mm,
        )
    }

    pub(crate) fn attach_bosses_with_reference(
        &self,
        bosses: &[(Boss, Point)],
        tolerance_mm: f64,
    ) -> Result<Self, Error> {
        self.attach_bosses_inner(
            bosses.len(),
            bosses.iter().map(|&(boss, radial)| (boss, Some(radial))),
            tolerance_mm,
        )
    }

    fn attach_bosses_inner(
        &self,
        count: usize,
        bosses: impl Iterator<Item = (Boss, Option<Point>)>,
        tolerance_mm: f64,
    ) -> Result<Self, Error> {
        if !(1e-10..=1e-3).contains(&tolerance_mm) || count == 0 {
            return Err(Error::Invalid);
        }
        if count > MAX_BOSSES {
            return Err(Error::Limit);
        }
        let mut attachments: Vec<Attachment> = Vec::with_capacity(count);
        for (boss, reference) in bosses {
            let next = Attachment::prove(self, boss, reference, tolerance_mm)?;
            if attachments
                .iter()
                .any(|old| !old.disjoint(&next, tolerance_mm))
            {
                return Err(Error::Unsupported);
            }
            attachments.push(next);
        }

        let mut shell = self.clone();
        let mut added = metrics::Sum::default();
        for attachment in attachments {
            added.add(attachment.append(&mut shell)?);
        }
        shell.volume_mm3 = positive_volume(shell.volume_mm3 + added.value())?;
        Ok(shell)
    }

    pub(super) fn oriented_vertices(&self, use_: OrientedEdge) -> [usize; 2] {
        let [a, b] = self.edges[use_.edge].vertices;
        if use_.forward { [a, b] } else { [b, a] }
    }
}

impl Attachment {
    fn prove(
        shell: &Shell,
        tool: Boss,
        reference: Option<Point>,
        tolerance: f64,
    ) -> Result<Self, Error> {
        if !tool.radius_mm.is_finite()
            || tool.radius_mm <= tolerance
            || tool
                .start_mm
                .into_iter()
                .chain(tool.end_mm)
                .any(|p| !p.is_finite())
        {
            return Err(Error::Invalid);
        }
        if tool.radius_mm > 1e8
            || tool
                .start_mm
                .into_iter()
                .chain(tool.end_mm)
                .any(|p| p.abs() > 1e8)
        {
            return Err(Error::Limit);
        }
        let delta = sub(tool.end_mm, tool.start_mm);
        let length = length3(delta);
        if length <= 2. * tolerance {
            return Err(Error::Invalid);
        }
        let axis = delta.map(|x| x / length);
        let mut hosts = shell.faces.iter().enumerate().filter_map(|(index, face)| {
            let mouth = mouth(shell, face, tool, axis, length, tolerance)?;
            Some((index, mouth))
        });
        let (host, mouth) = hosts.next().ok_or(Error::Unsupported)?;
        if hosts.next().is_some() {
            return Err(Error::Unsupported);
        }
        let [u, v] = if let Some(radial) = reference {
            let length = length3(radial);
            if !length.is_finite() || (length - 1.).abs() > 1e-10 || dot(radial, axis).abs() > 1e-10
            {
                return Err(Error::Invalid);
            }
            let side = cross(axis, radial);
            let v = side.map(|x| x / length3(side));
            [cross(v, axis), v]
        } else {
            perpendicular_axes(axis)
        };
        let attached = Self {
            tool,
            host,
            mouth,
            axes: [u, v, axis],
            height: length3(sub(tool.end_mm, mouth)),
        };
        if shell
            .faces
            .iter()
            .enumerate()
            .any(|(i, face)| i != host && !attached.clears_face(shell, face, tolerance))
        {
            return Err(Error::Unsupported);
        }
        Ok(attached)
    }

    fn clears_face(&self, shell: &Shell, face: &Face, tolerance: f64) -> bool {
        if let Support::Plane { normal } = face.support {
            let [lo, hi] = self.tool.range_along(normal, self.axes[2], face.point_mm);
            if lo > tolerance || hi < -tolerance {
                return true;
            }
        }
        let [u, v, axis] = self.axes;
        let [lo, hi] = face_range(shell, face, axis, self.tool.start_mm);
        let length = length3(sub(self.tool.end_mm, self.tool.start_mm));
        if hi < -tolerance || lo > length + tolerance {
            return true;
        }
        let radial_gap = [u, v].map(|direction| {
            let [lo, hi] = face_range(shell, face, direction, self.tool.start_mm);
            lo.max(-hi).max(0.)
        });
        radial_gap[0].hypot(radial_gap[1]) > self.tool.radius_mm + tolerance
    }

    fn disjoint(&self, other: &Self, tolerance: f64) -> bool {
        self.axes.into_iter().chain(other.axes).any(|direction| {
            let datum = self.tool.start_mm;
            let [a, b] = self.tool.range_along(direction, self.axes[2], datum);
            let [c, d] = other.tool.range_along(direction, other.axes[2], datum);
            b + tolerance < c || d + tolerance < a
        })
    }

    fn append(self, shell: &mut Shell) -> Result<f64, Error> {
        let [u, v, axis] = self.axes;
        let r = self.tool.radius_mm;
        let base = append_rim(shell, self.mouth, [u, v, axis], r)?;
        let top = append_rim(shell, self.tool.end_mm, [u, v, axis], r)?;
        let rails = std::array::from_fn::<_, 4, _>(|i| {
            append_edge(shell, [base.vertices[i], top.vertices[i]], Curve::Line)
        });
        let disk = PI * r * r;
        let base_face = &mut shell.faces[self.host];
        base_face.holes.push(
            base.edges
                .iter()
                .rev()
                .map(|&edge| oriented(edge, false))
                .collect(),
        );
        base_face.area_mm2 -= disk;
        if length3(sub(base_face.point_mm, self.mouth)) <= r {
            let edge = shell.edges[base_face.boundary[0].edge];
            base_face.point_mm = shell.vertices[edge.vertices[0]];
        }
        let cylinder = Support::Cylinder {
            axis_origin_mm: self.mouth,
            axis_direction: axis,
            radius_mm: r,
            outward: true,
        };
        for i in 0..4 {
            let next = (i + 1) % 4;
            let (sin, cos) = ((i as f64 + 0.5) * FRAC_PI_2).sin_cos();
            let point_mm = std::array::from_fn(|j| {
                self.mouth[j] + 0.5 * self.height * axis[j] + r * (cos * u[j] + sin * v[j])
            });
            shell.faces.push(Face {
                boundary: vec![
                    oriented(base.edges[i], true),
                    oriented(rails[next], true),
                    oriented(top.edges[i], false),
                    oriented(rails[i], false),
                ],
                holes: Vec::new(),
                support: cylinder,
                point_mm,
                area_mm2: FRAC_PI_2 * r * self.height,
                source: self.tool.source,
                source_face: self.tool.wall_source_face,
            });
        }
        shell.faces.push(Face {
            boundary: top.edges.map(|edge| oriented(edge, true)).into(),
            holes: Vec::new(),
            support: Support::Plane { normal: axis },
            point_mm: self.tool.end_mm,
            area_mm2: disk,
            source: self.tool.source,
            source_face: self.tool.cap_source_face,
        });
        for i in 0..3 {
            let direction = std::array::from_fn(|j| f64::from(i == j));
            let [lo, hi] = self.tool.range_along(direction, axis, [0.; 3]);
            shell.bounds_mm[0][i] = shell.bounds_mm[0][i].min(lo);
            shell.bounds_mm[1][i] = shell.bounds_mm[1][i].max(hi);
        }
        Ok(disk * self.height)
    }
}

impl Boss {
    fn range_along(self, direction: Point, axis: Point, datum: Point) -> [f64; 2] {
        let [a, b] = [self.start_mm, self.end_mm].map(|p| dot(sub(p, datum), direction));
        let radial = self.radius_mm * length3(cross(axis, direction));
        [a.min(b) - radial, a.max(b) + radial]
    }
}

fn mouth(
    shell: &Shell,
    face: &Face,
    tool: Boss,
    axis: Point,
    length: f64,
    tolerance: f64,
) -> Option<Point> {
    let Support::Plane { normal } = face.support else {
        return None;
    };
    let cosine = dot(axis, normal);
    let tilt = length3(cross(axis, normal));
    if cosine <= 0. || tilt > 64. * f64::EPSILON || tilt * tool.radius_mm > tolerance {
        return None;
    }
    let entry = dot(sub(face.point_mm, tool.start_mm), normal) / cosine;
    if entry <= tolerance || length - entry <= tolerance {
        return None;
    }
    let center = std::array::from_fn(|i| tool.start_mm[i] + entry * axis[i]);
    if !face.boundary.iter().all(|&use_| {
        let edge = shell.edges[use_.edge];
        let [a, b] = shell.oriented_vertices(use_).map(|i| shell.vertices[i]);
        let side = cross(normal, sub(b, a));
        edge.curve == Curve::Line
            && dot(sub(center, a), side) > length3(side) * (tool.radius_mm + tolerance)
    }) {
        return None;
    }
    if face.holes.iter().any(|hole| {
        let Curve::Ellipse(arc) = shell.edges[hole[0].edge].curve else {
            return true;
        };
        let old = arc.ellipse();
        length3(sub(old.center_mm(), center)) <= old.semiaxes_mm()[0] + tool.radius_mm + tolerance
    }) {
        return None;
    }
    Some(center)
}

// A plane reaches a directional extremum on its boundary. On a cylindrical
// patch, the objective is linear along each generator, so the same holds.
pub(super) fn face_range(shell: &Shell, face: &Face, direction: Point, datum: Point) -> [f64; 2] {
    face.boundary
        .iter()
        .map(|use_| {
            let edge = shell.edges[use_.edge];
            match edge.curve {
                Curve::Line => {
                    let [a, b] = edge
                        .vertices
                        .map(|i| dot(sub(shell.vertices[i], datum), direction));
                    [a.min(b), a.max(b)]
                }
                Curve::Ellipse(arc) => {
                    let anchor = dot(datum, direction);
                    arc.range_along(direction).map(|x| x - anchor)
                }
            }
        })
        .fold([f64::INFINITY, f64::NEG_INFINITY], |[a, b], [c, d]| {
            [a.min(c), b.max(d)]
        })
}

pub(super) fn perpendicular_axes(axis: Point) -> [Point; 2] {
    let least = (0..3)
        .min_by(|&a, &b| axis[a].abs().total_cmp(&axis[b].abs()))
        .expect("three axes");
    let reference = std::array::from_fn(|i| f64::from(i == least));
    let u = cross(axis, reference);
    let u = u.map(|x| x / length3(u));
    [u, cross(axis, u)]
}

pub(super) struct Rim {
    pub(super) vertices: [usize; 4],
    pub(super) edges: [usize; 4],
}

pub(super) fn append_rim(
    shell: &mut Shell,
    center: Point,
    [u, v, axis]: [Point; 3],
    r: f64,
) -> Result<Rim, Error> {
    let ellipse = Ellipse::new(center, axis, u, [r; 2]).map_err(|_| Error::Invalid)?;
    let mut vertices = [0; 4];
    let mut edges = [0; 4];
    let radial = [u, v, u.map(|x| -x), v.map(|x| -x)];
    for (i, direction) in radial.into_iter().enumerate() {
        vertices[i] = shell.vertices.len();
        shell
            .vertices
            .push(std::array::from_fn(|j| center[j] + r * direction[j]));
    }
    for i in 0..4 {
        let arc = EllipseArc::new(ellipse, i as f64 * FRAC_PI_2, FRAC_PI_2)
            .map_err(|_| Error::Invalid)?;
        edges[i] = append_edge(
            shell,
            [vertices[i], vertices[(i + 1) % 4]],
            Curve::Ellipse(arc),
        );
    }
    Ok(Rim { vertices, edges })
}

pub(super) fn append_edge(shell: &mut Shell, vertices: [usize; 2], curve: Curve) -> usize {
    let edge = shell.edges.len();
    shell.edges.push(Edge { vertices, curve });
    edge
}

pub(super) fn oriented(edge: usize, forward: bool) -> OrientedEdge {
    OrientedEdge { edge, forward }
}

#[cfg(test)]
mod tests;

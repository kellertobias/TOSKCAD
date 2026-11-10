//! Recover the exact tangent circle and rectangle of a verified axial stack.
use super::*;
use crate::axial_profile_stack::{Curve as StackCurve, Operand, Profile, ProfileStack, Support};
use std::f64::consts::PI;

const FRAME_EPS: f64 = 1e-10;

struct Frame {
    origin: Point,
    axes: [Point; 3],
}

impl Frame {
    fn project(&self, point: Point) -> Point {
        self.axes.map(|axis| dot(sub(point, self.origin), axis))
    }

    fn vector(&self, [x, y, z]: Point) -> Point {
        let [u, v, axis] = self.axes;
        std::array::from_fn(|i| x * u[i] + y * v[i] + z * axis[i])
    }

    fn point(&self, local: Point) -> Point {
        let delta = self.vector(local);
        std::array::from_fn(|i| self.origin[i] + delta[i])
    }
}

struct Levels {
    values: Vec<f64>,
    vertices: Vec<Vec<(u32, Point)>>,
    indices: BTreeMap<u32, usize>,
}

impl Levels {
    fn from_points(points: &BTreeMap<u32, Point>, frame: &Frame, tolerance: f64) -> Self {
        let mut points: Vec<_> = points
            .iter()
            .map(|(&id, &p)| (id, frame.project(p)))
            .collect();
        points.sort_unstable_by(|a, b| a.1[2].total_cmp(&b.1[2]));
        let mut result = Self {
            values: Vec::new(),
            vertices: Vec::new(),
            indices: BTreeMap::new(),
        };
        for (id, point) in points {
            if result
                .values
                .last()
                .is_none_or(|&z| point[2] - z > tolerance)
            {
                result.values.push(point[2]);
                result.vertices.push(Vec::new());
            }
            let level = result.vertices.len() - 1;
            result.vertices[level].push((id, point));
            result.indices.insert(id, level);
        }
        result
    }
}

impl Shell<'_, '_> {
    pub(super) fn axial_profile_stack(
        &self,
        faces: &[Face],
        readback: &Readback,
    ) -> Option<ImportSolid> {
        let (center, axis, radius) = if let Some(arc) = readback.arcs.first() {
            (arc.center_mm, arc.normal, arc.radius_mm)
        } else {
            let arc = readback.semicircles.first()?;
            (arc.center_mm, arc.normal, arc.radius_mm)
        };
        let axis = if preferred_axis(axis) {
            axis
        } else {
            axis.map(|v| -v)
        };
        let side = faces.iter().find_map(|face| match face {
            Face::Planar { normal, .. } if dot(*normal, axis).abs() <= FRAME_EPS => Some(*normal),
            _ => None,
        })?;
        let side = if preferred_axis(side) {
            side
        } else {
            side.map(|v| -v)
        };
        let [u, v] = frame_axes(axis, side);
        let low = self
            .points
            .values()
            .map(|&p| dot(sub(p, center), axis))
            .min_by(f64::total_cmp)?;
        let frame = Frame {
            origin: std::array::from_fn(|i| center[i] + low * axis[i]),
            axes: [u, v, axis],
        };
        let mut levels = Levels::from_points(&self.points, &frame, self.tolerance);
        levels.values[0] = 0.;
        let face_edges = self.indexed_edges(faces.len());
        let operands = self.stack_operands(faces, &face_edges, &levels, &frame, radius)?;
        let stack =
            ProfileStack::union(&operands, self.tolerance, self.tolerance.next_up()).ok()?;
        self.match_stack(&stack, &frame, &levels, faces, &face_edges)?;
        if !stack_metrics_match(&stack, &frame, readback, self.tolerance) {
            return None;
        }
        Some(ImportSolid {
            origin_mm: frame.origin.map(zero),
            u,
            v,
            height_mm: *levels.values.last()?,
            profile: ImportProfile::AxialProfileStack { stack },
        })
    }

    fn stack_operands(
        &self,
        faces: &[Face],
        face_edges: &[Vec<u32>],
        levels: &Levels,
        frame: &Frame,
        radius: f64,
    ) -> Option<Vec<Operand>> {
        let mut rect = [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]];
        let mut slabs = vec![None; levels.values.len().checked_sub(1)?];
        for (face, edges) in faces.iter().zip(face_edges) {
            let vertices = || edges.iter().flat_map(|id| self.edges[id].vertices);
            let (low, high) = vertices()
                .map(|id| levels.indices[&id])
                .fold((usize::MAX, 0), |(lo, hi), index| {
                    (lo.min(index), hi.max(index))
                });
            let is_rect = match face {
                Face::Planar { normal, .. } => {
                    if low == high && length(cross(*normal, frame.axes[2])) <= FRAME_EPS {
                        continue;
                    }
                    if dot(*normal, frame.axes[2]).abs() > FRAME_EPS {
                        return None;
                    }
                    for id in vertices() {
                        let p = frame.project(self.points[&id]);
                        for (i, value) in p.into_iter().take(2).enumerate() {
                            rect[0][i] = rect[0][i].min(value);
                            rect[1][i] = rect[1][i].max(value);
                        }
                    }
                    true
                }
                Face::HalfCylinder(_) | Face::PartialCylinder(_) => false,
                _ => return None,
            };
            if low.checked_add(1)? != high || slabs[low].is_some_and(|old| old != is_rect) {
                return None;
            }
            slabs[low] = Some(is_rect);
        }
        if !slabs.contains(&Some(true)) || !slabs.contains(&Some(false)) {
            return None;
        }
        // Projecting an oblique frame incurs floating-point roundoff. Only the
        // known analytic tangent coordinates may be substituted, and the full
        // oriented shell must still match within the import tolerance.
        for (corner, tangent) in rect.iter_mut().zip([-radius, radius]) {
            for value in corner {
                if (*value - tangent).abs() <= self.tolerance {
                    *value = tangent;
                }
            }
        }
        slabs
            .into_iter()
            .zip(levels.values.windows(2))
            .map(|(kind, span)| {
                let profile = if kind? {
                    Profile::Rectangle {
                        min: rect[0],
                        max: rect[1],
                    }
                } else {
                    Profile::Circle {
                        center: [0.; 2],
                        radius_mm: radius,
                    }
                };
                Some(Operand {
                    span_mm: [span[0], span[1]],
                    profile,
                })
            })
            .collect()
    }

    fn match_stack(
        &self,
        stack: &ProfileStack,
        frame: &Frame,
        levels: &Levels,
        faces: &[Face],
        face_edges: &[Vec<u32>],
    ) -> Option<()> {
        if self.points.len() != stack.vertices().len()
            || self.edges.len() != stack.edges().len()
            || faces.len() != stack.faces().len()
        {
            return None;
        }
        let mut used_vertices = BTreeSet::new();
        let source_vertices = stack
            .vertices()
            .iter()
            .map(|&expected| {
                let level = levels
                    .values
                    .binary_search_by(|z| z.total_cmp(&expected[2]))
                    .ok()?;
                let mut matches = levels.vertices[level]
                    .iter()
                    .filter(|(_, point)| length(sub(*point, expected)) <= self.tolerance);
                let id = matches.next()?.0;
                (matches.next().is_none() && used_vertices.insert(id)).then_some(id)
            })
            .collect::<Option<Vec<_>>>()?;
        let mut edge_index = BTreeMap::<_, Vec<_>>::new();
        for (&id, edge) in &self.edges {
            edge_index
                .entry(unordered(edge.vertices))
                .or_default()
                .push(id);
        }
        let mut source_edges = BTreeMap::new();
        for (i, expected) in stack.edges().iter().enumerate() {
            let endpoints = expected.vertices.map(|index| source_vertices[index]);
            let world = expected
                .vertices
                .map(|index| frame.point(stack.vertices()[index]));
            let mut matching = edge_index
                .get(&unordered(endpoints))?
                .iter()
                .filter(|&&id| {
                    curve_matches(
                        self.edges[&id].curve,
                        expected.curve,
                        frame,
                        world,
                        self.tolerance,
                    )
                });
            let id = *matching.next()?;
            let forward = self.edges[&id].vertices == endpoints;
            if matching.next().is_some() || source_edges.insert(id, (i, forward)).is_some() {
                return None;
            }
        }
        let mut expected_faces = BTreeMap::new();
        for face in stack.faces() {
            let mut key: Vec<_> = face
                .boundary
                .iter()
                .map(|edge| (edge.edge, edge.forward))
                .collect();
            key.sort_unstable();
            expected_faces.insert(key, face.support);
        }
        for (index, (face, edges)) in faces.iter().zip(face_edges).enumerate() {
            let mut key = Vec::with_capacity(edges.len());
            for id in edges {
                let (edge, forward) = source_edges[id];
                let use_ = self.edges[id].uses.iter().find(|use_| use_.face == index)?;
                key.push((edge, use_.forward == forward));
            }
            key.sort_unstable();
            let support = expected_faces.remove(&key)?;
            if !support_matches(face, support, frame) {
                return None;
            }
        }
        expected_faces.is_empty().then_some(())
    }
}

fn unordered(mut vertices: [u32; 2]) -> [u32; 2] {
    vertices.sort_unstable();
    vertices
}

fn curve_matches(
    source: Curve,
    expected: StackCurve,
    frame: &Frame,
    world: [Point; 2],
    tolerance: f64,
) -> bool {
    let StackCurve::Arc {
        center_mm,
        radius_mm,
        sweep_rad,
    } = expected
    else {
        return matches!(source, Curve::Line);
    };
    let arc = placed_arc(frame, world, center_mm, radius_mm, sweep_rad);
    let (center, normal, radius, sweep, middle) = match source {
        Curve::Arc(a) => (
            a.center_mm,
            a.normal,
            a.radius_mm,
            a.sweep_rad,
            arc_midpoint(a),
        ),
        Curve::Semicircle(a) => (
            a.center_mm,
            a.normal,
            a.radius_mm,
            PI,
            std::array::from_fn(|i| a.center_mm[i] + a.radius_mm * a.radial_mid[i]),
        ),
        _ => return false,
    };
    length(sub(center, arc.center_mm)) <= tolerance
        && radius * length(cross(normal, arc.normal)) <= tolerance
        && (radius - arc.radius_mm).abs() <= tolerance
        && radius * (sweep - arc.sweep_rad).abs() <= tolerance
        && length(sub(middle, arc_midpoint(arc))) <= tolerance
}

fn placed_arc(
    frame: &Frame,
    points: [Point; 2],
    center: Point,
    radius: f64,
    sweep: f64,
) -> ArcEdge {
    let center = frame.point(center);
    let [radial_start, radial_end] = points.map(|p| sub(p, center).map(|x| x / radius));
    ArcEdge {
        center_mm: center,
        normal: frame.axes[2].map(|x| x * sweep.signum()),
        radius_mm: radius,
        radial_start,
        radial_end,
        sweep_rad: sweep.abs(),
    }
}

fn arc_midpoint(arc: ArcEdge) -> Point {
    let (sin, cos) = (arc.sweep_rad * 0.5).sin_cos();
    let tangent = cross(arc.normal, arc.radial_start);
    std::array::from_fn(|i| {
        arc.center_mm[i] + arc.radius_mm * (cos * arc.radial_start[i] + sin * tangent[i])
    })
}

fn support_matches(face: &Face, support: Support, frame: &Frame) -> bool {
    match (face, support) {
        (Face::Planar { normal, .. }, Support::Plane { normal: expected }) => {
            length(sub(*normal, frame.vector(expected))) <= FRAME_EPS
        }
        // The STEP face validator already certifies these supports against the
        // exact circular rims and their generators. The oriented loop check
        // above also preserves the outward surface sense.
        (Face::HalfCylinder(_) | Face::PartialCylinder(_), Support::Cylinder { .. }) => true,
        _ => false,
    }
}

fn stack_metrics_match(
    stack: &ProfileStack,
    frame: &Frame,
    readback: &Readback,
    tolerance: f64,
) -> bool {
    let volume = stack.volume_mm3();
    let area = stack.faces().iter().map(|face| face.area_mm2).sum::<f64>();
    if (volume - readback.volume_mm3).abs() > 2. * tolerance * area + 64. * f64::EPSILON * volume {
        return false;
    }
    let vertices: Vec<_> = stack.vertices().iter().map(|&p| frame.point(p)).collect();
    let mut bounds = [vertices[0]; 2];
    for &point in &vertices {
        include_point(&mut bounds, point);
    }
    for edge in stack.edges() {
        if let StackCurve::Arc {
            center_mm,
            radius_mm,
            sweep_rad,
        } = edge.curve
        {
            let points = edge.vertices.map(|i| vertices[i]);
            placed_arc(frame, points, center_mm, radius_mm, sweep_rad).include_bounds(&mut bounds);
        }
    }
    bounds.into_iter().zip(readback.bounds_mm).all(|(a, b)| {
        a.into_iter()
            .zip(b)
            .all(|(a, b)| (a - b).abs() <= 2. * tolerance)
    })
}

#[cfg(test)]
mod tests;

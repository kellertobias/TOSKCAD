use super::*;
use crate::warped_union::boss::{Rim, append_edge, append_rim, oriented};
use std::f64::consts::TAU;

impl Plan<'_> {
    pub(super) fn append(self, shell: &mut Shell) -> Result<f64, Error> {
        let entry = self.path.entry.level;
        let exit = self.path.exit.map_or(self.tube.length, |mouth| mouth.level);
        let levels = match &self.recess {
            Some(recess) if recess.from_entry => [recess.shoulder, exit],
            Some(recess) => [entry, recess.shoulder],
            None => [entry, exit],
        };
        let round = append_round(shell, self.tube, levels, self.spec)?;
        let disk = PI * self.tube.radius * self.tube.radius;
        let mut volume = disk * (levels[1] - levels[0]);

        if let Some(recess) = &self.recess {
            let (wide, mouth, shoulder, round_shoulder) = if recess.from_entry {
                ([entry, recess.shoulder], self.path.entry.face, 1, &round[0])
            } else {
                (
                    [recess.shoulder, exit],
                    self.path.exit.ok_or(Error::Invalid)?.face,
                    0,
                    &round[1],
                )
            };
            volume += recess.area * (wide[1] - wide[0]);
            let rings = append_polygon(shell, self.tube, wide, recess);
            let edge_at_mouth = &rings[1 - shoulder].edges;
            let center = self.tube.point(wide[1 - shoulder]);
            open_face(
                shell,
                mouth,
                edge_at_mouth,
                recess.from_entry,
                center,
                self.envelope.radius,
                recess.area,
            );
            append_shoulder(
                shell,
                self.tube,
                recess,
                &rings[shoulder],
                round_shoulder,
                disk,
            );
        }

        if self.recess.as_ref().is_none_or(|recess| !recess.from_entry) {
            open_face(
                shell,
                self.path.entry.face,
                &round[0].edges,
                true,
                self.tube.point(entry),
                self.tube.radius,
                disk,
            );
        }
        if let Some(mouth) = self.path.exit {
            if self.recess.as_ref().is_none_or(|recess| recess.from_entry) {
                open_face(
                    shell,
                    mouth.face,
                    &round[1].edges,
                    false,
                    self.tube.point(exit),
                    self.tube.radius,
                    disk,
                );
            }
        } else {
            let normal = self.tube.axes[2].map(|x| -x);
            shell.faces.push(Face {
                boundary: loop_edges(&round[1].edges, false),
                holes: Vec::new(),
                support: Support::Plane { normal },
                point_mm: self.tube.point(exit),
                area_mm2: disk,
                source: self.spec.source,
                source_face: self.spec.cap_source_faces[usize::from(!self.path.reversed)],
            });
        }
        Ok(volume)
    }
}

fn append_round(
    shell: &mut Shell,
    tube: Tube,
    levels: [f64; 2],
    spec: &AxialCut,
) -> Result<[Rim; 2], Error> {
    let r = tube.radius;
    let [base, end] = levels.map(|q| tube.point(q));
    let low = append_rim(shell, base, tube.axes, r)?;
    let high = append_rim(shell, end, tube.axes, r)?;
    let rails: [usize; 4] = std::array::from_fn(|i| {
        append_edge(shell, [low.vertices[i], high.vertices[i]], Curve::Line)
    });
    let [u, v, axis] = tube.axes;
    let support = Support::Cylinder {
        axis_origin_mm: base,
        axis_direction: axis,
        radius_mm: r,
        outward: false,
    };
    let middle = levels[0].midpoint(levels[1]);
    for i in 0..4 {
        let next = (i + 1) % 4;
        let (sin, cos) = ((i as f64 + 0.5) * FRAC_PI_2).sin_cos();
        let center = tube.point(middle);
        let point_mm = std::array::from_fn(|j| center[j] + r * (u[j] * cos + v[j] * sin));
        shell.faces.push(Face {
            boundary: vec![
                oriented(low.edges[i], false),
                oriented(rails[i], true),
                oriented(high.edges[i], true),
                oriented(rails[next], false),
            ],
            holes: Vec::new(),
            support,
            point_mm,
            area_mm2: (TAU / 4.) * r * (levels[1] - levels[0]),
            source: spec.source,
            source_face: spec.wall_source_face,
        });
    }
    Ok([low, high])
}

struct PolygonRim {
    vertices: Vec<usize>,
    edges: Vec<usize>,
}

fn polygon_rim(shell: &mut Shell, tube: Tube, ring: &[XY], level: f64) -> PolygonRim {
    let first = shell.vertices.len();
    shell
        .vertices
        .extend(ring.iter().map(|&p| tube.lift(p, level)));
    let vertices: Vec<_> = (first..first + ring.len()).collect();
    let edges = (0..ring.len())
        .map(|i| {
            append_edge(
                shell,
                [vertices[i], vertices[(i + 1) % ring.len()]],
                Curve::Line,
            )
        })
        .collect();
    PolygonRim { vertices, edges }
}

fn append_polygon(
    shell: &mut Shell,
    tube: Tube,
    levels: [f64; 2],
    recess: &RecessPlan,
) -> [PolygonRim; 2] {
    let ring = &recess.polygon;
    let low = polygon_rim(shell, tube, ring, levels[0]);
    let high = polygon_rim(shell, tube, ring, levels[1]);
    let rails: Vec<_> = (0..ring.len())
        .map(|i| append_edge(shell, [low.vertices[i], high.vertices[i]], Curve::Line))
        .collect();
    let axis = tube.axes[2];
    for i in 0..ring.len() {
        let next = (i + 1) % ring.len();
        let [a, b] = [low.vertices[i], low.vertices[next]].map(|v| shell.vertices[v]);
        let tangent = sub(b, a);
        let length = length3(tangent);
        let normal = cross(axis, tangent).map(|x| x / length);
        let point_mm = tube.lift(
            std::array::from_fn(|j| ring[i][j].midpoint(ring[next][j])),
            levels[0].midpoint(levels[1]),
        );
        shell.faces.push(Face {
            boundary: vec![
                oriented(low.edges[i], false),
                oriented(rails[i], true),
                oriented(high.edges[i], true),
                oriented(rails[next], false),
            ],
            holes: Vec::new(),
            support: Support::Plane { normal },
            point_mm,
            area_mm2: length * (levels[1] - levels[0]),
            source: recess.source,
            source_face: recess.sides[i],
        });
    }
    [low, high]
}

fn append_shoulder(
    shell: &mut Shell,
    tube: Tube,
    recess: &RecessPlan,
    wide: &PolygonRim,
    narrow: &Rim,
    disk: f64,
) {
    let sign = if recess.from_entry { -1. } else { 1. };
    let normal = tube.axes[2].map(|x| x * sign);
    let point = recess.polygon[0];
    let length = point[0].hypot(point[1]);
    let fraction = length.midpoint(tube.radius) / length;
    shell.faces.push(Face {
        boundary: loop_edges(&wide.edges, !recess.from_entry),
        holes: vec![loop_edges(&narrow.edges, recess.from_entry)],
        support: Support::Plane { normal },
        point_mm: tube.lift(point.map(|x| x * fraction), recess.shoulder),
        area_mm2: recess.area - disk,
        source: recess.source,
        source_face: recess.shoulder_source,
    });
}

fn open_face(
    shell: &mut Shell,
    index: usize,
    edges: &[usize],
    entering: bool,
    center: Point,
    radius: f64,
    area: f64,
) {
    let face = &mut shell.faces[index];
    face.holes.push(loop_edges(edges, entering));
    face.area_mm2 -= area;
    if length3(sub(face.point_mm, center)) <= radius {
        face.point_mm = shell.vertices[shell.edges[face.boundary[0].edge].vertices[0]];
    }
}

fn loop_edges(edges: &[usize], forward: bool) -> Vec<OrientedEdge> {
    if forward {
        edges.iter().map(|&edge| oriented(edge, true)).collect()
    } else {
        edges
            .iter()
            .rev()
            .map(|&edge| oriented(edge, false))
            .collect()
    }
}

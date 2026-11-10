use super::{Curve, Edge, Error, Face, OrientedEdge, Point, Shell, Support, cross, dot, sub};
use crate::{
    normal_sweep::{Edge as ProfileEdge, Sweep},
    trim_curve::{Ellipse, EllipseArc},
};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub struct SheetWebEdge {
    pub ends: [[f64; 2]; 2],
    pub source: usize,
    pub flange: Option<usize>,
}

pub struct SheetFlange<'a> {
    /// Counterclockwise outward/thickness section, closed by a final root line.
    pub section: &'a [(ProfileEdge, usize)],
    pub ends: [usize; 2],
}

/// Join rectangular stock and up to two opposite flange sweeps at full shared
/// root faces. All edges and wall supports remain analytic.
pub fn formed_sheet(
    web: &[SheetWebEdge],
    holes: &[Vec<(ProfileEdge, usize)>],
    caps: [usize; 2],
    flanges: &[SheetFlange<'_>],
    signed_thickness: f64,
    tolerance: f64,
    minimum: f64,
) -> Result<Shell, Error> {
    if flanges.is_empty() || flanges.len() > 2 || web.len() > 32 {
        return Err(Error::Unsupported);
    }
    let thickness = signed_thickness.abs();
    let base_ring = web
        .iter()
        .map(|edge| {
            let [a, b] = edge.ends;
            (ProfileEdge::Line { a, b }, edge.source)
        })
        .collect::<Vec<_>>();
    let rings: Vec<_> = std::iter::once(base_ring.as_slice())
        .chain(holes.iter().map(Vec::as_slice))
        .collect();
    let base = checked_sweep(&rings, thickness, tolerance, minimum)?;
    let up = [0., 0., signed_thickness.signum()];
    let web_frame = SweepFrame {
        origin: [0.; 3],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
        direction: up,
    };
    let mut builder = Builder::new(tolerance);
    let mut volume = base.volume_mm3();
    let mut bounds = web_frame.bounds(&base);
    let mut hinges = Vec::new();
    for (index, flange) in flanges.iter().enumerate() {
        let mut matches = web.iter().filter(|edge| edge.flange == Some(index));
        let hinge = matches.next().ok_or(Error::Invalid)?;
        if matches.next().is_some() {
            return Err(Error::Invalid);
        }
        let [a, b] = hinge.ends.map(|[x, y]| [x, y, 0.]);
        let along = sub(b, a);
        let width = dot(along, along).sqrt();
        let along = scale(along, width.recip());
        let outward = [along[1], -along[0], 0.];
        if web.iter().any(|edge| {
            edge.ends
                .iter()
                .any(|&[x, y]| dot(sub([x, y, 0.], a), outward) > tolerance)
        }) || hinges.iter().any(|&(previous, normal)| {
            dot(normal, outward) > -1. + 1e-12 || dot(sub(previous, a), outward) >= -minimum
        }) {
            return Err(Error::Unsupported);
        }
        hinges.push((a, outward));
        let section = checked_sweep(&[flange.section], width, tolerance, minimum)?;
        let (&root, walls) = section.rings()[0].split_last().ok_or(Error::Invalid)?;
        let (root_a, root_b) = ([0., thickness], [0., 0.]);
        if root
            != (ProfileEdge::Line {
                a: root_a,
                b: root_b,
            })
            || walls.iter().any(|&edge| {
                let on_root = [edge.start(), edge.end()].map(|p| p == root_a || p == root_b);
                let extent = edge.support([-1., 0.]);
                extent > tolerance
                    || (!on_root[0] && !on_root[1] && extent > -minimum)
                    || (on_root[0] && on_root[1])
            })
        {
            return Err(Error::Unsupported);
        }
        let frame = SweepFrame {
            origin: a,
            u: outward,
            v: up,
            direction: along,
        };
        let omit = (0..flange.section.len()).map(|i| i == walls.len());
        builder.sweep(&[flange.section], &section, frame, flange.ends, omit)?;
        volume += section.volume_mm3();
        let flange_bounds = frame.bounds(&section);
        for i in 0..3 {
            bounds[0][i] = bounds[0][i].min(flange_bounds[0][i]);
            bounds[1][i] = bounds[1][i].max(flange_bounds[1][i]);
        }
    }
    if web
        .iter()
        .filter_map(|edge| edge.flange)
        .any(|index| index >= flanges.len())
    {
        return Err(Error::Invalid);
    }
    builder
        .sweep(
            &rings,
            &base,
            web_frame,
            caps,
            web.iter().map(|e| e.flange.is_some()),
        )
        .and_then(|()| builder.finish(bounds, volume))
}

fn checked_sweep(
    rings: &[&[(ProfileEdge, usize)]],
    height: f64,
    tolerance: f64,
    minimum: f64,
) -> Result<Sweep, Error> {
    let edges: Vec<Vec<_>> = rings
        .iter()
        .map(|ring| ring.iter().map(|&(edge, _)| edge).collect())
        .collect();
    Sweep::new(edges, height, tolerance, minimum).map_err(|error| match error {
        crate::normal_sweep::Error::Limit => Error::Limit,
        _ => Error::Invalid,
    })
}

#[derive(Clone, Copy)]
struct SweepFrame {
    origin: Point,
    u: Point,
    v: Point,
    direction: Point,
}

impl SweepFrame {
    fn point(self, [u, v]: [f64; 2], level: f64) -> Point {
        add(
            add(add(self.origin, scale(self.u, u)), scale(self.v, v)),
            scale(self.direction, level),
        )
    }

    fn bounds(self, sweep: &Sweep) -> [Point; 2] {
        [-1., 1.].map(|sign| {
            std::array::from_fn(|i| {
                self.origin[i]
                    + sign
                        * sweep.support([
                            sign * self.u[i],
                            sign * self.v[i],
                            sign * self.direction[i],
                        ])
            })
        })
    }
}

struct Builder {
    vertices: Vec<Point>,
    edges: Vec<Edge>,
    by_ends: BTreeMap<[usize; 2], usize>,
    faces: Vec<Face>,
    tolerance: f64,
}

impl Builder {
    fn new(tolerance: f64) -> Self {
        Self {
            vertices: Vec::new(),
            edges: Vec::new(),
            by_ends: BTreeMap::new(),
            faces: Vec::new(),
            tolerance,
        }
    }

    fn vertex(&mut self, point: Point) -> usize {
        if let Some(index) = self.vertices.iter().position(|&candidate| {
            let offset = sub(candidate, point);
            dot(offset, offset) <= self.tolerance.powi(2)
        }) {
            return index;
        }
        let index = self.vertices.len();
        self.vertices.push(point);
        index
    }

    fn edge(&mut self, points: [Point; 2], curve: Curve) -> Result<OrientedEdge, Error> {
        let vertices = points.map(|point| self.vertex(point));
        let [a, b] = vertices;
        if a == b {
            return Err(Error::Invalid);
        }
        let mut key = vertices;
        key.sort_unstable();
        if let Some(&edge) = self.by_ends.get(&key) {
            let prior = self.edges[edge];
            if curve != Curve::Line || prior.curve != Curve::Line {
                return Err(Error::Invalid);
            }
            return Ok(OrientedEdge {
                edge,
                forward: prior.vertices == vertices,
            });
        }
        let edge = self.edges.len();
        self.by_ends.insert(key, edge);
        self.edges.push(Edge { vertices, curve });
        Ok(OrientedEdge {
            edge,
            forward: true,
        })
    }

    fn sweep(
        &mut self,
        rings: &[&[(ProfileEdge, usize)]],
        sweep: &Sweep,
        frame: SweepFrame,
        caps: [usize; 2],
        omit: impl Iterator<Item = bool>,
    ) -> Result<(), Error> {
        let mut omit = omit.chain(std::iter::repeat(false));
        let height = sweep.height_mm();
        let reversed = dot(cross(frame.u, frame.v), frame.direction) < 0.;
        let mut cap_loops = [Vec::new(), Vec::new()];
        for (ring, authored) in sweep.rings().iter().zip(rings) {
            let mut end_loops = [Vec::new(), Vec::new()];
            for (&edge, &(_, source)) in ring.iter().zip(*authored) {
                let omit = omit.next().expect("unbounded omission flags");
                for arc in segments(edge) {
                    let lifts = [0., height].map(|level| {
                        let points = [arc.start(), arc.end()].map(|p| frame.point(p, level));
                        let curve = curve(arc, frame, level)?;
                        self.edge(points, curve)
                    });
                    let [bottom, top] = lifts;
                    let (bottom, top) = (bottom?, top?);
                    end_loops[0].push(bottom);
                    end_loops[1].push(top);
                    if !omit {
                        let rail = |p| [0., height].map(|level| frame.point(p, level));
                        let end = self.edge(rail(arc.end()), Curve::Line)?;
                        let start = self.edge(rail(arc.start()), Curve::Line)?;
                        let mut boundary = vec![bottom, end, reverse(top), reverse(start)];
                        if reversed {
                            reverse_loop(&mut boundary);
                            boundary.rotate_right(1);
                        }
                        self.faces.push(Face {
                            boundary,
                            holes: vec![],
                            support: side_support(arc, frame, reversed),
                            point_mm: frame.point(arc.start(), 0.),
                            area_mm2: arc.length_mm() * height,
                            source: 0,
                            source_face: source,
                        });
                    }
                }
            }
            for (i, (cap, mut ring)) in cap_loops.iter_mut().zip(end_loops).enumerate() {
                if (i == 0) != reversed {
                    reverse_loop(&mut ring);
                }
                cap.push(ring);
            }
        }
        for (i, mut rings) in cap_loops.into_iter().enumerate() {
            let direction = if i == 0 { -1. } else { 1. };
            self.faces.push(Face {
                boundary: rings.remove(0),
                holes: rings,
                support: Support::Plane {
                    normal: scale(frame.direction, direction),
                },
                point_mm: frame.point(sweep.centroid(), i as f64 * height),
                area_mm2: sweep.area_mm2(),
                source: 0,
                source_face: caps[i],
            });
        }
        Ok(())
    }

    fn finish(self, bounds_mm: [Point; 2], volume_mm3: f64) -> Result<Shell, Error> {
        let mut uses = vec![[0; 2]; self.edges.len()];
        for face in &self.faces {
            for ring in std::iter::once(&face.boundary).chain(&face.holes) {
                for edge in ring {
                    uses[edge.edge][usize::from(edge.forward)] += 1;
                }
            }
        }
        if uses.iter().any(|&count| count != [1, 1]) {
            return Err(Error::Invalid);
        }
        Ok(Shell {
            vertices: self.vertices,
            edges: self.edges,
            faces: self.faces,
            bounds_mm,
            volume_mm3,
            solid_count: 1,
            shell_count: 1,
            band: None,
            chart_w: vec![],
            chart_edges: 0,
        })
    }
}

fn reverse(edge: OrientedEdge) -> OrientedEdge {
    OrientedEdge {
        forward: !edge.forward,
        ..edge
    }
}

fn reverse_loop(ring: &mut [OrientedEdge]) {
    ring.reverse();
    for edge in ring {
        *edge = reverse(*edge);
    }
}

fn segments(edge: ProfileEdge) -> Vec<ProfileEdge> {
    if let ProfileEdge::Circle {
        center,
        radius_mm: r,
        clockwise,
    } = edge
    {
        let mut points =
            [[r, 0.], [0., r], [-r, 0.], [0., -r]].map(|[x, y]| [center[0] + x, center[1] + y]);
        if clockwise {
            points[1..].reverse();
        }
        return (0..4)
            .map(|i| ProfileEdge::Arc {
                center,
                a: points[i],
                b: points[(i + 1) % 4],
                clockwise,
            })
            .collect();
    }
    vec![edge]
}

fn curve(edge: ProfileEdge, frame: SweepFrame, level: f64) -> Result<Curve, Error> {
    let ProfileEdge::Arc { center, a, .. } = edge else {
        return Ok(Curve::Line);
    };
    let [u, v] = [a[0] - center[0], a[1] - center[1]];
    let radius = u.hypot(v);
    let ellipse = Ellipse::new(
        frame.point(center, level),
        cross(frame.u, frame.v),
        frame.u,
        [radius, radius],
    )
    .map_err(|_| Error::Invalid)?;
    EllipseArc::new(
        ellipse,
        v.atan2(u),
        edge.signed_sweep_rad().expect("arc sweep"),
    )
    .map(Curve::Ellipse)
    .map_err(|_| Error::Unsupported)
}

fn side_support(edge: ProfileEdge, frame: SweepFrame, reversed: bool) -> Support {
    if let ProfileEdge::Arc {
        center, clockwise, ..
    } = edge
    {
        return Support::Cylinder {
            axis_origin_mm: frame.point(center, 0.),
            axis_direction: frame.direction,
            radius_mm: edge.radius_mm().expect("arc radius"),
            outward: !clockwise,
        };
    }
    let delta = sub(frame.point(edge.end(), 0.), frame.point(edge.start(), 0.));
    let normal = cross(delta, frame.direction);
    let sign = if reversed { -1. } else { 1. };
    Support::Plane {
        normal: scale(normal, sign / dot(normal, normal).sqrt()),
    }
}

fn add(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] + b[i])
}

fn scale(point: Point, factor: f64) -> Point {
    point.map(|value| value * factor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::Tessellation;
    const TOL: f64 = 1e-7;
    const MIN: f64 = 1e-5;

    fn line(a: [f64; 2], b: [f64; 2]) -> ProfileEdge {
        ProfileEdge::Line { a, b }
    }

    fn web() -> Vec<SheetWebEdge> {
        let corners = [
            [0., 0.],
            [2., 0.],
            [8., 0.],
            [10., 0.],
            [10., 8.],
            [8., 8.],
            [2., 8.],
            [0., 8.],
        ];
        (0..corners.len())
            .map(|i| SheetWebEdge {
                ends: [corners[i], corners[(i + 1) % corners.len()]],
                source: i,
                flange: match i {
                    1 => Some(0),
                    5 => Some(1),
                    _ => None,
                },
            })
            .collect()
    }

    fn section() -> Vec<(ProfileEdge, usize)> {
        let arc = |a, b, clockwise| ProfileEdge::Arc {
            center: [0., -1.],
            a,
            b,
            clockwise,
        };
        [
            arc([0., 0.], [1., -1.], true),
            line([1., -1.], [1., -3.]),
            line([1., -3.], [2., -3.]),
            line([2., -3.], [2., -1.]),
            arc([2., -1.], [0., 1.], false),
            line([0., 1.], [0., 0.]),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, edge)| (edge, i + 10))
        .collect()
    }

    #[test]
    fn opposite_partial_bends_share_edges_and_keep_analytic_holes() {
        let web = web();
        let section = section();
        let flanges = [[20, 21], [22, 23]].map(|ends| SheetFlange {
            section: &section,
            ends,
        });
        let hole = ProfileEdge::Circle {
            center: [5., 4.],
            radius_mm: 1.,
            clockwise: true,
        };
        for thickness in [-1., 1.] {
            let shell = formed_sheet(
                &web,
                &[vec![(hole, 50)]],
                [30, 31],
                &flanges,
                thickness,
                TOL,
                MIN,
            )
            .unwrap();
            assert_eq!(shell.solid_count(), 1);
            assert_eq!(shell.shell_count(), 1);
            assert_eq!(shell.faces().len(), 26);
            assert_eq!(shell.bounds_mm()[0][1], -2.);
            let volume = 80. - std::f64::consts::PI + 12. * (2. + std::f64::consts::PI * 0.75);
            assert!((shell.volume_mm3() - volume).abs() < TOL);
            let mesh = shell
                .mesh(Tessellation {
                    deflection: 0.01,
                    angle: 0.1,
                    triangle_limit: 2000,
                })
                .unwrap();
            let mut incidence = BTreeMap::<_, [usize; 2]>::new();
            for t in mesh.indices.as_chunks::<3>().0 {
                for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    incidence.entry([a.min(b), a.max(b)]).or_default()[usize::from(a < b)] += 1;
                }
            }
            assert!(incidence.values().all(|counts| *counts == [1, 1]));
        }
    }

    #[test]
    fn malformed_spans_and_sections_are_refused() {
        let web = web();
        let section = section();
        let flange = || SheetFlange {
            section: &section,
            ends: [20, 21],
        };
        let build = |web: &[SheetWebEdge], flanges: &[SheetFlange<'_>]| {
            formed_sheet(web, &[], [30, 31], flanges, 1., TOL, MIN)
        };
        assert_eq!(build(&web, &[]), Err(Error::Unsupported));
        assert_eq!(build(&web, &[flange()]), Err(Error::Invalid)); // Unresolved hinge.
        let mut absent = web.clone();
        absent[1].flange = None;
        assert_eq!(build(&absent, &[flange(), flange()]), Err(Error::Invalid));
        let mut duplicate = web.clone();
        duplicate[0].flange = Some(0);
        assert_eq!(
            build(&duplicate, &[flange(), flange()]),
            Err(Error::Invalid)
        );
        let mut adjacent = web.clone();
        adjacent[5].flange = None;
        adjacent[3].flange = Some(1);
        assert_eq!(
            build(&adjacent, &[flange(), flange()]),
            Err(Error::Unsupported)
        );
        let mut diagonal = web.clone();
        diagonal[1].ends[0][1] = 1.;
        diagonal[0].ends[1][1] = 1.;
        assert_eq!(
            build(&diagonal, &[flange(), flange()]),
            Err(Error::Unsupported)
        );

        let mut one = web;
        one[5].flange = None;
        let wrong_root = &section[..5];
        assert_eq!(
            build(
                &one,
                &[SheetFlange {
                    section: wrong_root,
                    ends: [20, 21]
                }]
            ),
            Err(Error::Invalid)
        );
        let mut rotated = section;
        rotated.rotate_left(1);
        assert_eq!(
            build(
                &one,
                &[SheetFlange {
                    section: &rotated,
                    ends: [20, 21]
                }]
            ),
            Err(Error::Unsupported)
        );
        let mut huge = one;
        huge[1].ends[0][1] = 1e10;
        assert_eq!(
            build(
                &huge,
                &[SheetFlange {
                    section: &rotated,
                    ends: [20, 21]
                }]
            ),
            Err(Error::Limit)
        );
    }

    #[test]
    fn builder_rejects_open_or_ambiguous_edge_incidence() {
        let mut builder = Builder::new(TOL);
        let [a, b] = [[0.; 3], [1., 0., 0.]];
        let edge = builder.edge([a, b], Curve::Line).unwrap();
        assert_eq!(builder.edge([b, a], Curve::Line), Ok(reverse(edge)));
        assert_eq!(builder.edge([a, a], Curve::Line), Err(Error::Invalid));
        let circle = Ellipse::new([0.5, 0., 0.], [0., 0., 1.], [1., 0., 0.], [0.5; 2]).unwrap();
        let arc = EllipseArc::new(circle, std::f64::consts::PI, std::f64::consts::PI).unwrap();
        assert_eq!(
            builder.edge([a, b], Curve::Ellipse(arc)),
            Err(Error::Invalid)
        );
        assert_eq!(builder.finish([[0.; 3], [1.; 3]], 1.), Err(Error::Invalid));
    }
}

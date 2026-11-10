//! Exact ruled planar walls between corresponding straight sketch wires.

use super::*;
use crate::{
    CurvedEdge2d, CurvedProfile,
    rust_prism_topology::{Builder, polygon_planar_loops, position},
    taper,
};
use multicad_core::references::{
    AnalyticGeometry, AnchoredFrame, ReferenceFrame, TopologySnapshot,
};
use multicad_rust_kernel::{
    display::{DisplayMesh, MeshError, Tessellation},
    linear_sweep::{Error as SweepError, Sweep},
    linear_sweep_mesh,
    step::{StepError, Writer},
    trimmed_sweep::{Error as TrimError, TrimmedSweep},
};
use std::collections::BTreeSet;

type Point = [f64; 3];
type Point2 = [f64; 2];
const OPERATION: &str = "Rust linear extrusion";

pub(crate) struct LinearBody {
    geometry: Geometry,
    pub(super) feature: EntityId,
    edges: Vec<(Origin, bool)>,
    draft_face_origins: Option<Box<[Vec<Origin>; 6]>>,
    authored_offset: Point2,
    reverse: bool,
}

enum Geometry {
    Ruled(Sweep),
    Trimmed {
        sweep: Box<TrimmedSweep>,
        caps: [PlaneFrame; 2],
    },
}

enum PlaneTerminations {
    FromSketch {
        start: Option<PlaneFrame>,
        end: PlaneFrame,
    },
    Between([PlaneFrame; 2]),
}

struct Side {
    index: usize,
    previous: usize,
    next: usize,
    bottom: [Point; 2],
    top: [Point; 2],
    sketch: [Point2; 2],
}

fn subtract(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn length([x, y, z]: Point) -> f64 {
    x.hypot(y).hypot(z)
}

fn local_vector(frame: PlaneFrame, point: Point) -> Point {
    [frame.u, frame.v, frame.normal()].map(|axis| dot(axis, point))
}

fn world_vector(frame: PlaneFrame, point: Point) -> Point {
    let normal = frame.normal();
    std::array::from_fn(|i| frame.u[i] * point[0] + frame.v[i] * point[1] + normal[i] * point[2])
}

fn checked_edge_count(
    profile: &CurvedProfile,
    feature: EntityId,
    origins: &[Origin],
) -> Result<usize, GeometryError> {
    let count = profile.outer.len() + profile.holes.iter().map(Vec::len).sum::<usize>();
    if feature.is_nil()
        || origins.len() != count
        || origins.iter().collect::<BTreeSet<_>>().len() != count
        || origins.iter().any(|origin| {
            origin.source.is_nil()
                || origin.role.trim().is_empty()
                || format!("side/{}/{}", origin.source, origin.role).len() > 256
        })
    {
        return Err(input(
            "A linear extrusion needs distinct authored edge identities",
        ));
    }
    Ok(count)
}

fn flip_ring(points: &[Point2], outer: bool) -> bool {
    let anchor = points[0];
    let area = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| {
            (a[0] - anchor[0]) * (b[1] - anchor[1]) - (a[1] - anchor[1]) * (b[0] - anchor[0])
        })
        .sum::<f64>();
    (area > 0.) != outer
}

fn push_edge_origins(edges: &mut Vec<(Origin, bool)>, range: &[Origin], reverse: bool) {
    if reverse {
        edges.extend(range.iter().rev().cloned().map(|origin| (origin, true)));
    } else {
        edges.extend(range.iter().cloned().map(|origin| (origin, false)));
    }
}

impl Solid {
    /// Construct the unchanged straight section between real plane supports.
    /// The optional start must be strictly behind the sketch. The end must
    /// be strictly ahead along the requested direction for the entire wire.
    pub fn rust_extrude_to_planes_named(
        profile: &CurvedProfile,
        direction: Point,
        start: Option<PlaneFrame>,
        end: PlaneFrame,
        feature: EntityId,
        origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::rust_extrude_plane_caps(
            profile,
            direction,
            PlaneTerminations::FromSketch { start, end },
            feature,
            origins,
            policy,
        )
    }

    /// Use the section as the basis of a sweep between two ordered plane
    /// supports. The caps may lie on either side of the section frame, as
    /// long as the end is beyond the start along every sweep generator.
    pub fn rust_extrude_between_planes_named(
        profile: &CurvedProfile,
        direction: Point,
        planes: [PlaneFrame; 2],
        feature: EntityId,
        origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::rust_extrude_plane_caps(
            profile,
            direction,
            PlaneTerminations::Between(planes),
            feature,
            origins,
            policy,
        )
    }

    fn rust_extrude_plane_caps(
        profile: &CurvedProfile,
        direction: Point,
        terminations: PlaneTerminations,
        feature: EntityId,
        origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let (start, end) = match terminations {
            PlaneTerminations::FromSketch { start, end } => (start, end),
            PlaneTerminations::Between([start, end]) => (Some(start), end),
        };
        profile.checked(policy)?;
        let count = checked_edge_count(profile, feature, origins)?;
        let crossing = dot(direction, profile.frame.normal());
        if !crossing.is_finite() || crossing.abs() < 1e-10 {
            return Err(input("Extrusion direction must cross the sketch plane"));
        }
        let sign = crossing.signum();
        let frame = PlaneFrame {
            v: profile.frame.v.map(|v| v * sign),
            ..profile.frame
        };
        let caps = [start.unwrap_or(profile.frame), end].map(|plane| {
            plane.checked(policy)?;
            Ok::<_, GeometryError>(PlaneFrame {
                origin: local_vector(frame, subtract(plane.origin, frame.origin)),
                u: local_vector(frame, plane.u),
                v: local_vector(frame, plane.v),
            })
        });
        let [a, b] = caps;
        let caps = [a?, b?];
        let direction = local_vector(frame, direction);
        let [a, b] = caps.map(|plane| {
            let n = plane.normal();
            let crossing = dot(direction, n);
            if !crossing.is_finite() || crossing.abs() < 1e-10 {
                return Err(input(
                    "Extrusion direction must cross its termination plane",
                ));
            }
            Ok([
                dot(plane.origin, n) / crossing,
                -n[0] / crossing,
                -n[1] / crossing,
            ])
        });
        let travels = [if start.is_some() { a? } else { [0.; 3] }, b?];
        let mut rings = Vec::with_capacity(1 + profile.holes.len());
        let mut edges = Vec::with_capacity(count);
        for (wire, boundary) in std::iter::once(&profile.outer)
            .chain(&profile.holes)
            .enumerate()
        {
            let mut points = linear_ring(boundary, sign, [0.; 2])?;
            let reverse = flip_ring(&points, wire == 0);
            let origin_start = edges.len();
            push_edge_origins(
                &mut edges,
                &origins[origin_start..origin_start + boundary.len()],
                reverse,
            );
            if reverse {
                points[1..].reverse();
            }
            rings.push(points);
        }
        let sweep = TrimmedSweep::new(rings, direction, travels, policy.minimum_feature_mm)
            .map_err(|cause| match cause {
                TrimError::InvalidProfile => {
                    input("Plane-terminated extrusion needs simple straight wires")
                }
                TrimError::InvalidDirection => {
                    input("Extrusion direction must be finite, unit length, and transverse")
                }
                TrimError::InvalidExtent => {
                    input("Termination planes cross or leave insufficient material")
                }
                TrimError::Limit => error(
                    OPERATION,
                    GeometryErrorKind::Limit,
                    "Plane-terminated extrusion exceeds the analytic geometry budget",
                ),
            })?;
        if matches!(terminations, PlaneTerminations::FromSketch { .. }) {
            for &point in sweep.rings().iter().flatten() {
                if start.is_some() {
                    policy.length(-sweep.travel(false, point))?;
                }
                policy.length(sweep.travel(true, point))?;
            }
        }
        Self::from_trimmed_linear_sweep(sweep, frame, Some(caps), feature, edges, policy)
    }

    pub(super) fn from_trimmed_linear_sweep(
        sweep: TrimmedSweep,
        frame: PlaneFrame,
        caps: Option<[PlaneFrame; 2]>,
        feature: EntityId,
        edges: Vec<(Origin, bool)>,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if feature.is_nil() {
            return Err(input(
                "A linear extrusion needs an authored feature identity",
            ));
        }
        let caps = caps.unwrap_or_else(|| {
            let n = sweep.cap_normal(true);
            let axis = if n[0].abs() < 0.9 {
                [1., 0., 0.]
            } else {
                [0., 1., 0.]
            };
            let v = cross(n, axis);
            let v = v.map(|x| x / length(v));
            [
                PlaneFrame::default(),
                PlaneFrame {
                    origin: sweep.cap_centroid(true),
                    u: cross(v, n),
                    v,
                },
            ]
        });
        Self::from_linear_body(
            LinearBody {
                geometry: Geometry::Trimmed {
                    sweep: Box::new(sweep),
                    caps,
                },
                feature,
                edges,
                draft_face_origins: None,
                authored_offset: [0.; 2],
                reverse: false,
            },
            frame,
            policy,
        )
    }

    /// Extrude ordered straight sketch loops along an arbitrary crossing
    /// vector, with the native offset convention for nonzero draft.
    pub fn rust_extrude_linear_named(
        profile: &CurvedProfile,
        displacement_mm: Point,
        taper_rad: f64,
        feature: EntityId,
        origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let end = if taper_rad == 0. {
            profile.checked(policy)?;
            None
        } else {
            Some(taper::end_profile(
                profile,
                displacement_mm,
                taper_rad,
                policy,
            )?)
        };
        policy.point(displacement_mm)?;
        policy.length(length(displacement_mm))?;
        let count = checked_edge_count(profile, feature, origins)?;
        let vertical = dot(displacement_mm, profile.frame.normal());
        let height = vertical.abs();
        policy.length(height)?;
        let sign = vertical.signum();
        let frame = PlaneFrame {
            v: profile.frame.v.map(|v| sign * v),
            ..profile.frame
        };
        let offset = [frame.u, frame.v].map(|axis| dot(axis, displacement_mm));
        let top_profile = end.as_ref().unwrap_or(profile);
        let start_rings = std::iter::once(&profile.outer).chain(&profile.holes);
        let top_rings = std::iter::once(&top_profile.outer).chain(&top_profile.holes);
        let mut base = Vec::new();
        let mut top = Vec::new();
        let mut edges = Vec::with_capacity(count);
        let mut origin_start = 0;
        for (index, (a, b)) in start_rings.zip(top_rings).enumerate() {
            let mut a_points = linear_ring(a, sign, [0.; 2])?;
            let mut b_points = linear_ring(b, sign, offset)?;
            let reverse = flip_ring(&a_points, index == 0);
            let range = &origins[origin_start..origin_start + a.len()];
            origin_start += a.len();
            if reverse {
                a_points[1..].reverse();
                b_points[1..].reverse();
            }
            push_edge_origins(&mut edges, range, reverse);
            base.push(a_points);
            top.push(b_points);
        }
        let geometry = Sweep::new(base, top, height).map_err(|cause| match cause {
            SweepError::InvalidProfile => {
                input("Linear extrusion changes the profile topology or has invalid straight wires")
            }
            SweepError::InvalidHeight => {
                input("Linear extrusion must have a positive transverse height")
            }
            SweepError::Limit => error(
                OPERATION,
                GeometryErrorKind::Limit,
                "Linear extrusion exceeds the analytic geometry budget",
            ),
        })?;
        Self::from_linear_sweep(
            geometry,
            frame,
            feature,
            edges,
            offset,
            vertical < 0.,
            policy,
        )
    }

    pub(super) fn from_linear_sweep(
        geometry: Sweep,
        frame: PlaneFrame,
        feature: EntityId,
        edges: Vec<(Origin, bool)>,
        authored_offset: Point2,
        reverse: bool,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        if feature.is_nil() {
            return Err(input(
                "A linear extrusion needs an authored feature identity",
            ));
        }
        policy.length(geometry.height_mm())?;
        Self::from_linear_body(
            LinearBody {
                geometry: Geometry::Ruled(geometry),
                feature,
                edges,
                draft_face_origins: None,
                authored_offset,
                reverse,
            },
            frame,
            policy,
        )
    }

    pub(super) fn from_attributed_linear_draft(
        geometry: Sweep,
        frame: PlaneFrame,
        feature: EntityId,
        face_origins: [Vec<Origin>; 6],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let edges = face_origins[2..]
            .iter()
            .map(|origins| (origins[0].clone(), false))
            .collect();
        let mut solid =
            Self::from_linear_sweep(geometry, frame, feature, edges, [0.; 2], false, policy)?;
        let SolidBody::Linear(body, _) = &mut solid.body else {
            unreachable!("linear draft result");
        };
        Rc::get_mut(body)
            .expect("new linear body")
            .draft_face_origins = Some(Box::new(face_origins));
        Ok(solid)
    }

    fn from_linear_body(
        body: LinearBody,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        frame.checked(policy)?;
        let bounds = body.bounds(frame);
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(Self {
            body: SolidBody::Linear(Rc::new(body), frame),
            policy,
            provenance: Vec::new(),
            rust_origins: Vec::new(),
        })
    }

    pub(crate) fn linear_body(&self) -> Option<(&LinearBody, PlaneFrame)> {
        match &self.body {
            SolidBody::Linear(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }
}

fn linear_ring(
    wire: &[CurvedEdge2d],
    sign: f64,
    offset: Point2,
) -> Result<Vec<Point2>, GeometryError> {
    wire.iter()
        .zip(wire.iter().cycle().skip(1))
        .map(|(edge, next)| {
            let (CurvedEdge2d::Line { a, b }, CurvedEdge2d::Line { a: next, .. }) = (edge, next)
            else {
                return Err(error(
                    OPERATION,
                    GeometryErrorKind::Unsupported,
                    "Rust linear extrusion currently requires straight profile edges",
                ));
            };
            if b != next {
                return Err(input(
                    "Linear extrusion requires exactly shared boundary joins",
                ));
            }
            Ok([a[0] + offset[0], sign * a[1] + offset[1]])
        })
        .collect()
}

impl LinearBody {
    pub(super) fn volume_mm3(&self) -> f64 {
        match &self.geometry {
            Geometry::Ruled(sweep) => sweep.volume_mm3(),
            Geometry::Trimmed { sweep, .. } => sweep.volume_mm3(),
        }
    }

    pub(super) fn mesh(&self, options: Tessellation) -> Result<DisplayMesh, MeshError> {
        match &self.geometry {
            Geometry::Ruled(sweep) => linear_sweep_mesh::tessellate(sweep, options),
            Geometry::Trimmed { sweep, .. } => {
                linear_sweep_mesh::tessellate_trimmed(sweep, options)
            }
        }
    }

    pub(super) fn append_step(
        &self,
        writer: &mut Writer,
        frame: PlaneFrame,
    ) -> Result<(), StepError> {
        match &self.geometry {
            Geometry::Ruled(sweep) => {
                writer.append_linear_sweep(sweep, frame.origin, frame.u, frame.v)
            }
            Geometry::Trimmed { sweep, .. } => {
                writer.append_trimmed_sweep(sweep, frame.origin, frame.u, frame.v)
            }
        }
    }

    fn rings(&self, end: bool) -> &[Vec<Point2>] {
        match &self.geometry {
            Geometry::Ruled(sweep) if end => sweep.top(),
            Geometry::Ruled(sweep) => sweep.base(),
            Geometry::Trimmed { sweep, .. } => sweep.rings(),
        }
    }

    fn point(&self, frame: PlaneFrame, end: bool, point: Point2) -> Point {
        let local = match &self.geometry {
            Geometry::Ruled(sweep) => {
                [point[0], point[1], if end { sweep.height_mm() } else { 0. }]
            }
            Geometry::Trimmed { sweep, .. } => sweep.point(end, point),
        };
        position(frame, local)
    }

    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let normal = frame.normal();
        let support = |i: usize, sign: f64| {
            let direction = [frame.u[i], frame.v[i], normal[i]].map(|x| sign * x);
            match &self.geometry {
                Geometry::Ruled(sweep) => sweep.support(direction),
                Geometry::Trimmed { sweep, .. } => sweep.support(direction),
            }
        };
        Bounds {
            min: std::array::from_fn(|i| frame.origin[i] - support(i, -1.)),
            max: std::array::from_fn(|i| frame.origin[i] + support(i, 1.)),
        }
    }

    /// Exact local rectangle with an unchanged section. The caller must
    /// establish a cardinal placement before treating these bounds as stock.
    pub(super) fn orthogonal_box(&self, frame: PlaneFrame) -> Option<Bounds> {
        let Geometry::Ruled(sweep) = &self.geometry else {
            return None;
        };
        let [ring] = sweep.base() else {
            return None;
        };
        if ring.len() != 4 || sweep.base() != sweep.top() {
            return None;
        }
        let [x0, x1] = ring
            .iter()
            .fold([f64::INFINITY, f64::NEG_INFINITY], |[min, max], p| {
                [min.min(p[0]), max.max(p[0])]
            });
        let [y0, y1] = ring
            .iter()
            .fold([f64::INFINITY, f64::NEG_INFINITY], |[min, max], p| {
                [min.min(p[1]), max.max(p[1])]
            });
        [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
            .iter()
            .all(|corner| ring.contains(corner))
            .then(|| self.bounds(frame))
    }

    fn cap_frame(&self, frame: PlaneFrame, end: bool, authored: bool) -> ReferenceFrame {
        let sweep = match &self.geometry {
            Geometry::Ruled(sweep) => sweep,
            Geometry::Trimmed { sweep, caps } => {
                let cap = caps[usize::from(end)];
                let flip = !authored && dot(cap.normal(), sweep.cap_normal(end)) < 0.;
                return ReferenceFrame {
                    origin_mm: position(frame, cap.origin),
                    u: world_vector(frame, cap.u),
                    v: world_vector(frame, cap.v.map(|v| if flip { -v } else { v })),
                };
            }
        };
        let origin_mm = if end {
            let [u, v] = self.authored_offset;
            position(frame, [u, v, sweep.height_mm()])
        } else {
            frame.origin
        };
        let flip = if authored { self.reverse } else { !end };
        ReferenceFrame {
            origin_mm,
            u: frame.u,
            v: frame.v.map(|v| if flip { -v } else { v }),
        }
    }

    fn sides(&self, frame: PlaneFrame) -> impl Iterator<Item = Side> + '_ {
        let mut offset = 0;
        self.rings(false)
            .iter()
            .zip(self.rings(true))
            .flat_map(move |(a, b)| {
                let start = offset;
                offset += a.len();
                (0..a.len()).map(move |i| {
                    let next = (i + 1) % a.len();
                    Side {
                        index: start + i,
                        previous: start + (i + a.len() - 1) % a.len(),
                        next: start + next,
                        bottom: [a[i], a[next]].map(|uv| self.point(frame, false, uv)),
                        top: [b[i], b[next]].map(|uv| self.point(frame, true, uv)),
                        sketch: [a[i], a[next]],
                    }
                })
            })
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        let caps = [false, true].into_iter().enumerate().map(|(i, end)| {
            let support = self.cap_frame(frame, end, false);
            let (center_mm, area_mm2) = match &self.geometry {
                Geometry::Ruled(sweep) => (
                    self.point(frame, end, sweep.cap_centroid(end)),
                    sweep.cap_area_mm2(end),
                ),
                Geometry::Trimmed { sweep, .. } => (
                    position(frame, sweep.cap_centroid(end)),
                    sweep.cap_area_mm2(end),
                ),
            };
            Face {
                index: i as u32,
                plane: Some((support.origin_mm, support.normal())),
                center_mm,
                area_mm2,
            }
        });
        caps.chain(self.sides(frame).map(|side| {
            let support = side.support();
            let (area_mm2, center_mm) = side.area_and_center();
            Face {
                index: (side.index + 2) as u32,
                plane: Some((support.origin_mm, support.normal())),
                center_mm,
                area_mm2,
            }
        }))
        .collect()
    }

    fn side_anchor(
        &self,
        frame: PlaneFrame,
        side: &Side,
        support: ReferenceFrame,
    ) -> ReferenceFrame {
        let reverse = self.edges[side.index].1;
        if let Geometry::Trimmed { sweep, .. } = &self.geometry {
            let [a, b] = if reverse {
                [side.sketch[1], side.sketch[0]]
            } else {
                side.sketch
            };
            let edge = [b[0] - a[0], b[1] - a[1], 0.];
            let edge_length = length(edge);
            let u = edge.map(|x| x / edge_length);
            let direction = sweep.direction();
            let along = dot(direction, u);
            let v: Point = std::array::from_fn(|i| direction[i] - along * u[i]);
            let lateral_length = length(v);
            return ReferenceFrame {
                origin_mm: position(frame, [a[0], a[1], 0.]),
                u: world_vector(frame, u),
                v: world_vector(frame, v.map(|x| x / lateral_length)),
            };
        }
        if reverse {
            ReferenceFrame {
                origin_mm: side.bottom[1],
                u: support.u.map(|x| -x),
                ..support
            }
        } else {
            support
        }
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> TopologySnapshot {
        let face_origins = self.draft_face_origins.as_ref().map_or_else(
            || {
                ["cap.start", "cap.end"]
                    .into_iter()
                    .map(|role| {
                        vec![Origin {
                            source: self.feature,
                            role: role.into(),
                        }]
                    })
                    .chain(self.edges.iter().map(|(curve, _)| {
                        vec![
                            Origin {
                                source: self.feature,
                                role: format!("side/{}/{}", curve.source, curve.role),
                            },
                            curve.clone(),
                        ]
                    }))
                    .collect()
            },
            |origins| origins.to_vec(),
        );
        let mut build = Builder::new(face_origins);
        for (cap, end) in [false, true].into_iter().enumerate() {
            build.face(
                cap,
                Some(self.cap_frame(frame, end, false)),
                Some(self.cap_frame(frame, end, true)),
                2..self.edges.len() + 2,
            );
            for side in self.sides(frame) {
                let [a, b] = if end { side.top } else { side.bottom };
                build.non_face(
                    AnalyticGeometry::Line { a_mm: a, b_mm: b },
                    &[cap, side.index + 2],
                );
                build.non_face(
                    AnalyticGeometry::Point { position_mm: a },
                    &[cap, side.previous + 2, side.index + 2],
                );
            }
        }
        for side in self.sides(frame) {
            let support = side.support();
            let anchor = self.side_anchor(frame, &side, support);
            build.face(
                side.index + 2,
                Some(support),
                Some(anchor),
                [0, 1, side.previous + 2, side.next + 2],
            );
            build.non_face(
                AnalyticGeometry::Line {
                    a_mm: side.bottom[0],
                    b_mm: side.top[0],
                },
                &[side.previous + 2, side.index + 2],
            );
        }
        let mut elements = build.into_elements();
        if self.draft_face_origins.is_some() {
            for element in &mut elements {
                if let Some(AnalyticGeometry::Plane { frame }) = element.geometry {
                    element.frames = element
                        .origins
                        .iter()
                        .cloned()
                        .map(|origin| AnchoredFrame { origin, frame })
                        .collect();
                }
            }
        }
        TopologySnapshot {
            owner,
            revision,
            elements,
        }
    }

    pub(crate) fn planar_loops(&self, face: u32) -> Vec<Vec<(u32, bool)>> {
        polygon_planar_loops(self.rings(false), 0, face)
    }
}

impl Side {
    fn support(&self) -> ReferenceFrame {
        let edge = subtract(self.bottom[1], self.bottom[0]);
        let a = length(edge);
        let u = edge.map(|x| x / a);
        let rail = subtract(self.top[0], self.bottom[0]);
        let along = dot(u, rail);
        let perpendicular: Point = std::array::from_fn(|i| rail[i] - along * u[i]);
        let height = length(perpendicular);
        let v = perpendicular.map(|x| x / height);
        ReferenceFrame {
            origin_mm: self.bottom[0],
            u,
            v,
        }
    }

    fn area_and_center(&self) -> (f64, Point) {
        let origin = self.bottom[0];
        let [b, c, d] = [self.bottom[1], self.top[1], self.top[0]].map(|p| subtract(p, origin));
        let weights = [length(cross(b, c)), length(cross(c, d))];
        let twice_area = weights[0] + weights[1];
        let ratio = weights[0] / twice_area;
        let centroid =
            std::array::from_fn(|i| origin[i] + (ratio * (b[i] - d[i]) + c[i] + d[i]) / 3.);
        (twice_area / 2., centroid)
    }
}

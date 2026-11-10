//! Exact solids formed by revolving one-sided polygonal meridians.

use super::*;
use crate::{CurvedEdge2d, CurvedProfile, RevolutionAxis, rust_prism_topology::position};
use multicad_core::references::{AnchoredFrame, ReferenceFrame};
use multicad_rust_kernel::{
    revolution::{Error as RevolutionError, Revolution},
    revolution_mesh,
};
use std::collections::BTreeSet;

mod topology;
mod union;

type Point = [f64; 3];
type MeridianPoint = [f64; 2];
const OPERATION: &str = "Rust revolution";

pub(crate) struct RevolutionBody {
    pub(crate) geometry: Revolution,
    pub(super) feature: EntityId,
    sketch: PlaneFrame,
    segments: Vec<Vec<MeridianSegment>>,
    rings: Vec<topology::RingEdges>,
}

struct MeridianSegment {
    origin: Origin,
    inherited: Vec<Origin>,
    inherited_frames: Vec<AnchoredFrame>,
    authored_start: MeridianPoint,
}

fn difference(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn norm([x, y, z]: Point) -> f64 {
    x.hypot(y).hypot(z)
}

fn unit(vector: Point) -> Point {
    let length = norm(vector);
    vector.map(|value| value / length)
}

fn spun([r, z]: MeridianPoint, angle: f64) -> Point {
    let (sin, cos) = angle.sin_cos();
    [r * cos, r * sin, z]
}

fn local(frame: PlaneFrame, point: Point) -> Point {
    let delta = difference(point, frame.origin);
    [frame.u, frame.v, frame.normal()].map(|axis| dot(axis, delta))
}

fn world_direction(frame: PlaneFrame, direction: Point) -> Point {
    let basis = [frame.u, frame.v, frame.normal()];
    std::array::from_fn(|i| dot(direction, basis.map(|axis| axis[i])))
}

impl Solid {
    /// Construct the solid directly from straight, closed sketch wires.
    pub fn rust_revolve_curved_profile_named(
        profile: &CurvedProfile,
        axis: RevolutionAxis,
        angle_rad: f64,
        feature: EntityId,
        origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        profile.checked(policy)?;
        axis.validate(profile, angle_rad, policy)?;
        if feature.is_nil()
            || origins.len()
                != profile.outer.len() + profile.holes.iter().map(Vec::len).sum::<usize>()
            || origins.iter().collect::<BTreeSet<_>>().len() != origins.len()
            || origins.iter().any(|origin| {
                origin.source.is_nil()
                    || origin.role.trim().is_empty()
                    || format!("side/{}/{}", origin.source, origin.role).len() > 256
            })
        {
            return Err(input("Revolution needs distinct authored profile origins"));
        }

        let n = unit(axis.direction);
        let mut u = unit(cross(n, profile.frame.normal()));
        let radial = |uv| dot(difference(profile.frame.point(uv), axis.origin_mm), u);
        let mut rings = Vec::with_capacity(profile.holes.len() + 1);
        let mut farthest = 0_f64;
        for ring in std::iter::once(&profile.outer).chain(&profile.holes) {
            let mut ring_points = Vec::with_capacity(ring.len());
            for (edge, next) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                let (CurvedEdge2d::Line { a, b }, CurvedEdge2d::Line { a: next, .. }) =
                    (edge, next)
                else {
                    return Err(error(
                        OPERATION,
                        GeometryErrorKind::Unsupported,
                        "Rust revolution currently requires straight meridian edges",
                    ));
                };
                if (b[0] - next[0]).hypot(b[1] - next[1]) > policy.linear_tolerance_mm {
                    return Err(input(
                        "Revolution profile edges must form ordered closed wires",
                    ));
                }
                let r = radial(*a);
                if r.abs() > farthest.abs() {
                    farthest = r;
                }
                let z = dot(difference(profile.frame.point(*a), axis.origin_mm), n);
                ring_points.push([r, z]);
            }
            rings.push(ring_points);
        }
        if farthest < 0. {
            u = u.map(|value| -value);
        }
        let frame = PlaneFrame {
            origin: axis.origin_mm,
            u,
            v: cross(n, u),
        };
        let sketch = PlaneFrame {
            origin: local(frame, profile.frame.origin),
            u: [frame.u, frame.v, n].map(|axis| dot(axis, profile.frame.u)),
            v: [frame.u, frame.v, n].map(|axis| dot(axis, profile.frame.v)),
        };
        let mut next_origin = origins.iter();
        let mut segments = Vec::with_capacity(rings.len());
        for (index, ring) in rings.iter_mut().enumerate() {
            for [r, _] in ring.iter_mut() {
                *r *= if farthest < 0. { -1. } else { 1. };
                if r.abs() <= policy.linear_tolerance_mm {
                    *r = 0.;
                }
            }
            let mut edges: Vec<_> = ring
                .iter()
                .map(|&authored_start| MeridianSegment {
                    authored_start,
                    inherited: vec![],
                    inherited_frames: vec![],
                    origin: next_origin
                        .next()
                        .expect("profile origin count checked")
                        .clone(),
                })
                .collect();
            let anchor = ring[0];
            let signed = ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .map(|(a, b)| {
                    (a[0] - anchor[0]) * (b[1] - anchor[1])
                        - (a[1] - anchor[1]) * (b[0] - anchor[0])
                })
                .sum::<f64>();
            if (signed > 0.) != (index == 0) {
                ring.reverse();
                edges.reverse();
                edges.rotate_left(1);
            }
            segments.push(edges);
        }
        let geometry = Revolution::new(rings, angle_rad).map_err(|cause| match cause {
            RevolutionError::InvalidAngle => input("Invalid Rust revolution angle"),
            RevolutionError::InvalidProfile => {
                input("Rust revolution needs simple one-sided wires without pinched axis contacts")
            }
            RevolutionError::Limit => error(
                OPERATION,
                GeometryErrorKind::Limit,
                "Revolution exceeds the analytic geometry budget",
            ),
        })?;
        let rings = topology::ring_edges(&geometry);
        let body = RevolutionBody {
            geometry,
            feature,
            sketch,
            segments,
            rings,
        };
        let bounds = body.bounds(frame);
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(Self {
            body: SolidBody::Revolved(Rc::new(body), frame),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }

    pub(crate) fn revolution_body(&self) -> Option<(&RevolutionBody, PlaneFrame)> {
        match &self.body {
            SolidBody::Revolved(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }

    pub fn rust_revolution_contains(&self, point_mm: Point) -> Result<bool, GeometryError> {
        self.policy.point(point_mm)?;
        let (body, frame) = self.revolution_body().ok_or_else(|| {
            error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "Select a Rust revolution",
            )
        })?;
        Ok(body
            .geometry
            .contains(local(frame, point_mm), self.policy.linear_tolerance_mm))
    }
}

impl RevolutionBody {
    /// Some full revolutions are exactly the circular sweep already supported
    /// by the exchange and drawing paths. Check all four meridian corners.
    pub(crate) fn circular_sweep(
        &self,
        frame: PlaneFrame,
    ) -> Option<(PlaneFrame, f64, f64, Option<f64>)> {
        let [ring] = self.geometry.rings() else {
            return None;
        };
        let [a, b, c, d] = ring.as_slice() else {
            return None;
        };
        if !self.geometry.is_full() {
            return None;
        }
        let points = [*a, *b, *c, *d];
        let extent = |axis: usize| {
            points
                .iter()
                .fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], p| {
                    [lo.min(p[axis]), hi.max(p[axis])]
                })
        };
        let [inner, outer] = extent(0);
        let [low, high] = extent(1);
        if ![[inner, low], [outer, low], [outer, high], [inner, high]]
            .into_iter()
            .all(|corner| points.contains(&corner))
        {
            return None;
        }
        let base = PlaneFrame {
            origin: position(frame, [0., 0., low]),
            ..frame
        };
        Some((base, outer, high - low, (inner > 0.).then_some(inner)))
    }

    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let axis = frame.normal();
        let support = |i: usize, sign: f64| {
            self.geometry
                .support([frame.u[i], frame.v[i], axis[i]].map(|x| sign * x))
        };
        Bounds {
            min: std::array::from_fn(|i| frame.origin[i] - support(i, -1.)),
            max: std::array::from_fn(|i| frame.origin[i] + support(i, 1.)),
        }
    }

    fn cap_plane(&self, frame: PlaneFrame, index: usize) -> ReferenceFrame {
        let sweep = self.geometry.angle_rad();
        let angle = if index == 0 { 0. } else { sweep };
        let sign = if index == 0 {
            sweep.signum()
        } else {
            -sweep.signum()
        };
        ReferenceFrame {
            origin_mm: frame.origin,
            u: world_direction(frame, spun([1., 0.], angle)),
            v: frame.normal().map(|x| sign * x),
        }
    }

    fn planar_wall(&self, frame: PlaneFrame, ring: usize, edge: usize) -> Option<ReferenceFrame> {
        let points = &self.geometry.rings()[ring];
        let [a, b] = [points[edge], points[(edge + 1) % points.len()]];
        (a[1] == b[1]).then(|| ReferenceFrame {
            origin_mm: position(frame, [0., 0., a[1]]),
            u: frame.u,
            v: frame.v.map(|v| v * (a[0] - b[0]).signum()),
        })
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        let sweep = self.geometry.angle_rad();
        let mut faces = Vec::new();
        if !self.geometry.is_full() {
            for (i, angle) in [0., sweep].into_iter().enumerate() {
                let plane = self.cap_plane(frame, i);
                faces.push(Face {
                    index: i as u32,
                    plane: Some((plane.origin_mm, plane.normal())),
                    center_mm: position(frame, spun(self.geometry.section_centroid(), angle)),
                    area_mm2: self.geometry.section_area_mm2(),
                });
            }
        }
        let mean_angular = if self.geometry.is_full() {
            0.
        } else {
            (sweep / 2.).sin() / (sweep / 2.)
        };
        for (ring, points) in self.geometry.rings().iter().enumerate() {
            for (edge, (&[a, z0], &[b, z1])) in
                points.iter().zip(points.iter().cycle().skip(1)).enumerate()
            {
                let Some(index) = self.geometry.wall_face(ring, edge) else {
                    continue;
                };
                let radius_sum = a + b;
                let center_z = z0 + (z1 - z0) * (a + 2. * b) / (3. * radius_sum);
                let center_r = 2. * (a * a + a * b + b * b) / (3. * radius_sum);
                let plane = self
                    .planar_wall(frame, ring, edge)
                    .map(|plane| (plane.origin_mm, plane.normal()));
                faces.push(Face {
                    index,
                    plane,
                    center_mm: position(
                        frame,
                        spun([center_r * mean_angular, center_z], sweep / 2.),
                    ),
                    area_mm2: sweep.abs() * (a - b).hypot(z1 - z0) * radius_sum / 2.,
                });
            }
        }
        faces
    }

    pub(super) fn mesh(
        &self,
        frame: PlaneFrame,
        options: multicad_rust_kernel::display::Tessellation,
    ) -> Result<Mesh, GeometryError> {
        crate::rust_display::placed_mesh(
            revolution_mesh::tessellate(&self.geometry, options),
            frame,
            false,
        )
    }
}

//! Exact ruled planar walls between corresponding straight sketch wires.

use super::*;
use crate::{
    CurvedEdge2d, CurvedProfile,
    rust_prism_topology::{Builder, polygon_planar_loops, position},
    taper,
};
use multicad_core::references::{AnalyticGeometry, ReferenceFrame, TopologySnapshot};
use multicad_rust_kernel::linear_sweep::{Error as SweepError, Sweep};
use std::collections::BTreeSet;

type Point = [f64; 3];
type Point2 = [f64; 2];
const OPERATION: &str = "Rust linear extrusion";

pub(crate) struct LinearBody {
    pub(super) geometry: Sweep,
    pub(super) feature: EntityId,
    edges: Vec<(Origin, bool)>,
    authored_offset: Point2,
    reverse: bool,
}

struct Side {
    index: usize,
    previous: usize,
    next: usize,
    bottom: [Point; 2],
    top: [Point; 2],
}

fn subtract(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn length([x, y, z]: Point) -> f64 {
    x.hypot(y).hypot(z)
}

impl Solid {
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
            let anchor = a_points[0];
            let signed_area = a_points
                .iter()
                .zip(a_points.iter().cycle().skip(1))
                .map(|(a, b)| {
                    (a[0] - anchor[0]) * (b[1] - anchor[1])
                        - (a[1] - anchor[1]) * (b[0] - anchor[0])
                })
                .sum::<f64>();
            let reverse = (signed_area > 0.) != (index == 0);
            let range = &origins[origin_start..origin_start + a.len()];
            origin_start += a.len();
            if reverse {
                a_points[1..].reverse();
                b_points[1..].reverse();
                edges.extend(range.iter().rev().cloned().map(|origin| (origin, true)));
            } else {
                edges.extend(range.iter().cloned().map(|origin| (origin, false)));
            }
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
        frame.checked(policy)?;
        if feature.is_nil() {
            return Err(input(
                "A linear extrusion needs an authored feature identity",
            ));
        }
        policy.length(geometry.height_mm())?;
        let body = LinearBody {
            geometry,
            feature,
            edges,
            authored_offset,
            reverse,
        };
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
    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let normal = frame.normal();
        let support = |i: usize, sign: f64| {
            self.geometry
                .support([frame.u[i], frame.v[i], normal[i]].map(|x| sign * x))
        };
        Bounds {
            min: std::array::from_fn(|i| frame.origin[i] - support(i, -1.)),
            max: std::array::from_fn(|i| frame.origin[i] + support(i, 1.)),
        }
    }

    fn cap_frame(&self, frame: PlaneFrame, end: bool, authored: bool) -> ReferenceFrame {
        let origin_mm = if end {
            let [u, v] = self.authored_offset;
            position(frame, [u, v, self.geometry.height_mm()])
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
        let height = self.geometry.height_mm();
        let point = move |[u, v]: Point2, height| position(frame, [u, v, height]);
        self.geometry
            .base()
            .iter()
            .zip(self.geometry.top())
            .flat_map(move |(a, b)| {
                let start = offset;
                offset += a.len();
                (0..a.len()).map(move |i| {
                    let next = (i + 1) % a.len();
                    Side {
                        index: start + i,
                        previous: start + (i + a.len() - 1) % a.len(),
                        next: start + next,
                        bottom: [point(a[i], 0.), point(a[next], 0.)],
                        top: [point(b[i], height), point(b[next], height)],
                    }
                })
            })
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        let caps = [false, true].into_iter().enumerate().map(|(i, end)| {
            let support = self.cap_frame(frame, end, false);
            let [u, v] = self.geometry.cap_centroid(end);
            let z = if end { self.geometry.height_mm() } else { 0. };
            Face {
                index: i as u32,
                plane: Some((support.origin_mm, support.normal())),
                center_mm: position(frame, [u, v, z]),
                area_mm2: self.geometry.cap_area_mm2(end),
            }
        });
        caps.chain(self.sides(frame).map(|side| {
            let (support, a, b, height) = side.support();
            let t = (a + 2. * b) / (3. * (a + b));
            Face {
                index: (side.index + 2) as u32,
                plane: Some((support.origin_mm, support.normal())),
                center_mm: std::array::from_fn(|i| {
                    let lo = side.bottom[0][i].midpoint(side.bottom[1][i]);
                    let hi = side.top[0][i].midpoint(side.top[1][i]);
                    t.mul_add(hi - lo, lo)
                }),
                area_mm2: (a + b) * height / 2.,
            }
        }))
        .collect()
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> TopologySnapshot {
        let face_origins = ["cap.start", "cap.end"]
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
            .collect();
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
            let support = side.support().0;
            let anchor = if self.edges[side.index].1 {
                ReferenceFrame {
                    origin_mm: side.bottom[1],
                    u: support.u.map(|x| -x),
                    ..support
                }
            } else {
                support
            };
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
        TopologySnapshot {
            owner,
            revision,
            elements: build.into_elements(),
        }
    }

    pub(crate) fn planar_loops(&self, face: u32) -> Vec<Vec<(u32, bool)>> {
        polygon_planar_loops(self.geometry.base(), 0, face)
    }
}

impl Side {
    fn support(&self) -> (ReferenceFrame, f64, f64, f64) {
        let edge = subtract(self.bottom[1], self.bottom[0]);
        let a = length(edge);
        let b = length(subtract(self.top[1], self.top[0]));
        let u = edge.map(|x| x / a);
        let rail = subtract(self.top[0], self.bottom[0]);
        let along = dot(u, rail);
        let perpendicular: Point = std::array::from_fn(|i| rail[i] - along * u[i]);
        let height = length(perpendicular);
        let v = perpendicular.map(|x| x / height);
        (
            ReferenceFrame {
                origin_mm: self.bottom[0],
                u,
                v,
            },
            a,
            b,
            height,
        )
    }
}

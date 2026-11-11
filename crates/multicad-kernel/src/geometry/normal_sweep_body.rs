//! Rigid placement, authored references and exact boundary for normal sweeps.
use super::*;
use crate::{
    CurvedEdge2d, CurvedProfile,
    rust_prism_topology::{Builder, place_frame, position},
};
use multicad_core::references::{
    AnalyticGeometry, AnchoredFrame, ReferenceFrame, TopologySnapshot,
};
use multicad_rust_kernel::normal_sweep::{self, Edge, Sweep};
use std::{collections::BTreeSet, f64::consts::TAU};

mod fillet;
mod piercing;
pub use piercing::SweepOpening;

#[derive(Clone)]
pub(crate) struct NormalSweepBody {
    sweep: Sweep,
    reverse: bool,
    pub(super) feature: EntityId,
    sides: Vec<Side>,
    rings: Vec<std::ops::Range<usize>>,
    openings: Option<Rc<piercing::Openings>>,
    path_origin: Option<Origin>,
    stock_history: Option<Rc<fillet::StockHistory>>,
    cap_anchors: Option<[ReferenceFrame; 2]>,
}

#[derive(Clone)]
struct Side {
    edge: Edge,
    origin: Origin,
    reversed: bool,
    previous: usize,
    next: usize,
    rail: Option<u32>,
}

impl Solid {
    /// Retain the named cap frames of a normal profile sweep without changing
    /// its geometric supports. The anchors follow later rigid placements.
    pub fn with_rust_sweep_cap_anchors(
        mut self,
        caps: [PlaneFrame; 2],
    ) -> Result<Self, GeometryError> {
        let policy = self.policy;
        let local = |placement, heights| local_cap_anchors(caps, placement, heights, policy);
        match &mut self.body {
            SolidBody::NormalSweep(body, placement) => {
                let anchors = local(*placement, [0., body.z(true)])?;
                Rc::make_mut(body).cap_anchors = Some(anchors);
            }
            SolidBody::AxialCylinder(cylinder, placement, Some(sketch)) => {
                let anchors = local(*placement, [0., cylinder.height().as_f64()])?;
                Rc::make_mut(sketch).cap_anchors = Some(anchors);
            }
            _ => {
                return Err(input(
                    "Authored cap frames require a Rust normal profile sweep",
                ));
            }
        }
        Ok(self)
    }

    /// Extrude an analytic line-and-arc profile along the signed frame normal.
    pub fn rust_extrude_normal_profile_named(
        profile: &CurvedProfile,
        signed_height_mm: f64,
        feature: EntityId,
        origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::rust_extrude_normal_profile_with_path(
            profile,
            signed_height_mm,
            feature,
            origins,
            None,
            policy,
        )
    }

    pub(crate) fn rust_extrude_normal_profile_with_path(
        profile: &CurvedProfile,
        signed_height_mm: f64,
        feature: EntityId,
        origins: &[Origin],
        path_origin: Option<Origin>,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        profile.frame.checked(policy)?;
        let height = signed_height_mm.abs();
        policy.length(height)?;
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
                "Normal extrusion needs distinct authored edge identities",
            ));
        }
        let mut rings = Vec::new();
        let mut sides = Vec::with_capacity(count);
        let mut ranges = Vec::new();
        let mut rail = 2 * count as u32;
        for (index, wire) in std::iter::once(&profile.outer)
            .chain(&profile.holes)
            .enumerate()
        {
            let mut edges = wire
                .iter()
                .map(convert_edge)
                .collect::<Result<Vec<_>, _>>()?;
            let reversed = (normal_sweep::signed_area(&edges) > 0.) != (index == 0);
            let first = sides.len();
            let len = edges.len();
            let curve_origins = &origins[first..first + len];
            if reversed {
                edges.reverse();
                edges.iter_mut().for_each(|edge| *edge = edge.reversed());
            }
            for (i, &edge) in edges.iter().enumerate() {
                let origin = &curve_origins[if reversed { len - 1 - i } else { i }];
                let id = (!matches!(edge, Edge::Circle { .. })).then(|| {
                    let id = rail;
                    rail += 1;
                    id
                });
                sides.push(Side {
                    edge,
                    origin: origin.clone(),
                    reversed,
                    previous: first + (i + len - 1) % len,
                    next: first + (i + 1) % len,
                    rail: id,
                });
            }
            ranges.push(first..first + len);
            rings.push(edges);
        }
        let sweep = Sweep::new(
            rings,
            height,
            policy.linear_tolerance_mm,
            policy.minimum_feature_mm,
        )
        .map_err(|cause| {
            error(
                "normal extrusion",
                if matches!(cause, normal_sweep::Error::Limit) {
                    GeometryErrorKind::Limit
                } else {
                    GeometryErrorKind::Input
                },
                format!("Invalid analytic normal extrusion: {cause:?}"),
            )
        })?;
        let body = NormalSweepBody {
            sweep,
            reverse: signed_height_mm < 0.,
            feature,
            sides,
            rings: ranges,
            openings: None,
            path_origin,
            stock_history: None,
            cap_anchors: None,
        };
        profile
            .frame
            .checked_metric_bounds(body.bounds(PlaneFrame::default()), policy)?;
        let bounds = body.bounds(profile.frame);
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(Self {
            body: SolidBody::NormalSweep(Rc::new(body), profile.frame),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }

    pub(crate) fn normal_sweep_body(&self) -> Option<(&NormalSweepBody, PlaneFrame)> {
        match &self.body {
            SolidBody::NormalSweep(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }
}

fn local_cap_anchors(
    caps: [PlaneFrame; 2],
    placement: PlaneFrame,
    heights: [f64; 2],
    policy: GeometryPolicy,
) -> Result<[ReferenceFrame; 2], GeometryError> {
    let axes = [placement.u, placement.v, placement.normal()];
    let local = |point| axes.map(|axis| dot(axis, point));
    let convert = |cap: PlaneFrame, height: f64| {
        cap.checked(policy)?;
        let delta = std::array::from_fn(|i| cap.origin[i] - placement.origin[i]);
        let anchor = ReferenceFrame {
            origin_mm: local(delta),
            u: local(cap.u),
            v: local(cap.v),
        };
        if (anchor.origin_mm[2] - height).abs() > policy.linear_tolerance_mm
            || anchor.u[2].abs().max(anchor.v[2].abs()) > 1e-10
        {
            return Err(input("An authored cap frame must lie on its sweep cap"));
        }
        Ok(anchor)
    };
    Ok([convert(caps[0], heights[0])?, convert(caps[1], heights[1])?])
}

fn convert_edge(edge: &CurvedEdge2d) -> Result<Edge, GeometryError> {
    match *edge {
        CurvedEdge2d::Line { a, b } => Ok(Edge::Line { a, b }),
        CurvedEdge2d::Arc {
            center,
            a,
            b,
            clockwise,
        } => Ok(Edge::Arc {
            center,
            a,
            b,
            clockwise,
        }),
        CurvedEdge2d::Circle { center, radius_mm } => Ok(Edge::Circle {
            center,
            radius_mm,
            clockwise: false,
        }),
        CurvedEdge2d::BSpline(_) => Err(error(
            "normal extrusion",
            GeometryErrorKind::Unsupported,
            "Rust normal extrusion requires straight or circular profile edges",
        )),
    }
}

impl NormalSweepBody {
    pub(super) fn unpierced_profile(&self) -> Option<(&[Vec<Edge>], f64)> {
        self.openings.is_none().then(|| {
            let height = self.sweep.height_mm();
            (
                self.sweep.rings(),
                if self.reverse { -height } else { height },
            )
        })
    }
    pub(crate) fn drawing_section(&self) -> (&[Vec<Edge>], f64) {
        (self.sweep.rings(), self.z(true))
    }

    pub(crate) fn drawing_shell(
        &self,
        tolerance: f64,
    ) -> Result<multicad_rust_kernel::warped_union::Shell, multicad_rust_kernel::warped_union::Error>
    {
        use multicad_rust_kernel::warped_union::{normal_sweep_shell, pierced_sweep_shell};

        match &self.openings {
            Some(openings) => pierced_sweep_shell(&openings.geometry, self.reverse, tolerance),
            None => normal_sweep_shell(&self.sweep, self.reverse, tolerance),
        }
    }

    pub(crate) fn drawing_rims(
        &self,
        frame: PlaneFrame,
        end: bool,
    ) -> impl Iterator<Item = AnalyticGeometry> + '_ {
        self.sides
            .iter()
            .map(move |side| self.rim(frame, side, end))
    }

    fn z(&self, end: bool) -> f64 {
        if !end {
            0.
        } else if self.reverse {
            -self.sweep.height_mm()
        } else {
            self.sweep.height_mm()
        }
    }

    fn cap(&self, frame: PlaneFrame, end: bool, outward: bool) -> ReferenceFrame {
        let flip = outward && (end == self.reverse);
        ReferenceFrame {
            origin_mm: position(frame, [0., 0., self.z(end)]),
            u: frame.u,
            v: frame.v.map(|v| if flip { -v } else { v }),
        }
    }

    fn point(&self, frame: PlaneFrame, [x, y]: [f64; 2], end: bool) -> [f64; 3] {
        position(frame, [x, y, self.z(end)])
    }

    fn plane(&self, frame: PlaneFrame, side: &Side, authored: bool) -> Option<ReferenceFrame> {
        let Edge::Line { mut a, mut b } = side.edge else {
            return None;
        };
        if authored && side.reversed {
            std::mem::swap(&mut a, &mut b);
        }
        let [dx, dy] = [b[0] - a[0], b[1] - a[1]];
        let length = dx.hypot(dy);
        let flip = authored && self.reverse;
        Some(ReferenceFrame {
            origin_mm: frame.point(a),
            u: std::array::from_fn(|i| (dx * frame.u[i] + dy * frame.v[i]) / length),
            v: frame.normal().map(|n| if flip { -n } else { n }),
        })
    }

    fn rim(&self, frame: PlaneFrame, side: &Side, end: bool) -> AnalyticGeometry {
        rim_in_frame(side.edge, self.cap(frame, end, false))
    }

    pub(super) fn volume_mm3(&self) -> f64 {
        self.pierced_volume()
            .unwrap_or_else(|| self.sweep.volume_mm3())
    }

    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let normal = frame.normal();
        let support = |i: usize, sign: f64| {
            let axis = if self.reverse { -normal[i] } else { normal[i] };
            self.sweep
                .support([sign * frame.u[i], sign * frame.v[i], sign * axis])
        };
        Bounds {
            min: std::array::from_fn(|i| frame.origin[i] - support(i, -1.)),
            max: std::array::from_fn(|i| frame.origin[i] + support(i, 1.)),
        }
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        let mut faces = [false, true]
            .into_iter()
            .enumerate()
            .map(|(index, end)| {
                let support = self.cap(frame, end, true);
                Face {
                    index: index as u32,
                    plane: Some((support.origin_mm, support.normal())),
                    center_mm: self.point(frame, self.sweep.centroid(), end),
                    area_mm2: self.sweep.area_mm2(),
                }
            })
            .chain(self.sides.iter().enumerate().map(|(index, side)| {
                let [x, y] = side.edge.centroid();
                Face {
                    index: index as u32 + 2,
                    plane: self
                        .plane(frame, side, false)
                        .map(|support| (support.origin_mm, support.normal())),
                    center_mm: position(frame, [x, y, self.z(true) / 2.]),
                    area_mm2: side.edge.length_mm() * self.sweep.height_mm(),
                }
            }))
            .collect();
        self.apply_cut_faces(frame, &mut faces);
        faces
    }

    pub(super) fn mesh(
        &self,
        frame: PlaneFrame,
        options: multicad_rust_kernel::display::Tessellation,
    ) -> Result<Mesh, GeometryError> {
        if let Some(result) = self.pierced_mesh(frame, options) {
            return result;
        }
        crate::rust_display::placed_mesh(
            multicad_rust_kernel::display::normal_sweep(&self.sweep, options),
            frame,
            self.reverse,
        )
    }

    pub(super) fn append_step(
        &self,
        writer: &mut multicad_rust_kernel::step::Writer,
        frame: PlaneFrame,
    ) -> Result<(), multicad_rust_kernel::step::StepError> {
        if let Some(openings) = &self.openings {
            return writer.append_pierced_normal_sweep(
                &openings.geometry,
                frame.origin,
                frame.u,
                frame.v,
                self.reverse,
            );
        }
        writer.append_normal_sweep(&self.sweep, frame.origin, frame.u, frame.v, self.reverse)
    }

    pub(crate) fn planar_loops(&self, face: u32) -> Vec<Vec<(u32, bool)>> {
        let count = self.sides.len() as u32;
        if face < 2 {
            return self
                .rings
                .iter()
                .map(|ring| {
                    ring.clone()
                        .map(|edge| (face * count + edge as u32, false))
                        .collect()
                })
                .collect();
        }
        let index = face - 2;
        let Some(side) = self.sides.get(index as usize) else {
            return self.extra_loops(face, vec![]);
        };
        let Some(rail) = side.rail else { return vec![] };
        self.extra_loops(
            face,
            vec![vec![
                (index, false),
                (
                    self.sides[side.next].rail.expect("closed piecewise ring"),
                    false,
                ),
                (count + index, true),
                (rail, true),
            ]],
        )
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> TopologySnapshot {
        let side_origins = |origin: &Origin| {
            [
                Origin {
                    source: self.feature,
                    role: format!("side/{}/{}", origin.source, origin.role),
                },
                origin.clone(),
            ]
        };
        let path_origins = self.path_origin.as_ref().map(side_origins);
        let mut origins: Vec<Vec<Origin>> = ["cap.start", "cap.end"]
            .into_iter()
            .map(|role| {
                vec![Origin {
                    source: self.feature,
                    role: role.into(),
                }]
            })
            .chain(self.sides.iter().map(|side| {
                side_origins(&side.origin)
                    .into_iter()
                    .chain(path_origins.iter().flatten().cloned())
                    .collect()
            }))
            .chain(self.cut_face_origins())
            .collect();
        if let Some(history) = &self.stock_history {
            history.add_face_origins(&mut origins);
        }
        let mut build = Builder::new(origins);
        for (cap, end) in [false, true].into_iter().enumerate() {
            let anchor = self.cap_anchors.map_or_else(
                || self.cap(frame, end, false),
                |anchors| place_frame(frame, anchors[cap]),
            );
            build.face(
                cap,
                Some(self.cap(frame, end, true)),
                Some(anchor),
                2..self.sides.len() + 2,
            );
            for (index, side) in self.sides.iter().enumerate() {
                build.non_face(self.rim(frame, side, end), &[cap, index + 2]);
                if side.rail.is_some() {
                    build.non_face(
                        AnalyticGeometry::Point {
                            position_mm: self.point(frame, side.edge.start(), end),
                        },
                        &[cap, side.previous + 2, index + 2],
                    );
                }
            }
        }
        for (index, side) in self.sides.iter().enumerate() {
            let ordinal = index + 2;
            if side.rail.is_none() {
                build.face(ordinal, None, None, [0, 1]);
                continue;
            }
            build.face(
                ordinal,
                self.plane(frame, side, false),
                self.plane(frame, side, true),
                [0, 1, side.previous + 2, side.next + 2]
                    .into_iter()
                    .chain(self.cut_wall_neighbors(index)),
            );
            let corner = side.edge.start();
            build.non_face(
                AnalyticGeometry::Line {
                    a_mm: self.point(frame, corner, false),
                    b_mm: self.point(frame, corner, true),
                },
                &[side.previous + 2, ordinal],
            );
        }
        self.append_cut_topology(frame, &mut build);
        let mut elements = build.into_elements();
        if let Some(origins) = path_origins {
            for element in &mut elements {
                if element.origins.contains(&origins[0])
                    && let Some(AnalyticGeometry::Plane { frame }) = element.geometry
                {
                    element.frames.extend(
                        origins
                            .iter()
                            .cloned()
                            .map(|origin| AnchoredFrame { origin, frame }),
                    );
                }
            }
        }
        if let Some(history) = &self.stock_history {
            history.restore(frame, &mut elements);
        }
        TopologySnapshot {
            owner,
            revision,
            elements,
        }
    }
}

pub(super) fn rim_in_frame(edge: Edge, frame: ReferenceFrame) -> AnalyticGeometry {
    let at = |p| super::rounded_slot_body::point(frame, p);
    match edge {
        Edge::Line { a, b } => AnalyticGeometry::Line {
            a_mm: at(a),
            b_mm: at(b),
        },
        Edge::Arc { center, a, .. } => {
            let [x, y] = std::array::from_fn(|i| a[i] - center[i]);
            AnalyticGeometry::CircularArc {
                frame: ReferenceFrame {
                    origin_mm: at(center),
                    ..frame
                },
                radius_mm: x.hypot(y),
                start_rad: y.atan2(x),
                sweep_rad: edge.signed_sweep_rad().expect("circular arc"),
            }
        }
        Edge::Circle {
            center,
            radius_mm,
            clockwise,
        } => AnalyticGeometry::CircularArc {
            frame: ReferenceFrame {
                origin_mm: at(center),
                ..frame
            },
            radius_mm,
            start_rad: 0.,
            sweep_rad: if clockwise { -TAU } else { TAU },
        },
    }
}

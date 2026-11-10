//! Analytic loose-tenon solids and shared rounded-slot boundary geometry.

use super::*;
use crate::rust_prism_topology::{Builder, position};
use multicad_core::references::{AnalyticGeometry, ReferenceFrame, TopologySnapshot};
use multicad_rust_kernel::stadium::Stadium;
use std::{collections::BTreeSet, f64::consts::PI};

type Point = [f64; 3];

pub(crate) struct RoundedSlotBody {
    profile: Stadium,
    height: f64,
    reverse: bool,
    pub(super) feature: EntityId,
    origins: [Origin; 4],
}

pub(super) fn point(frame: ReferenceFrame, [u, v]: [f64; 2]) -> Point {
    std::array::from_fn(|i| frame.origin_mm[i] + u * frame.u[i] + v * frame.v[i])
}

pub(super) fn rim_edge(profile: &Stadium, edge: usize, frame: ReferenceFrame) -> AnalyticGeometry {
    let corners = profile.corners();
    if edge.is_multiple_of(2) {
        AnalyticGeometry::Line {
            a_mm: point(frame, corners[edge]),
            b_mm: point(frame, corners[(edge + 1) % 4]),
        }
    } else {
        let [u, v] = profile.centers()[usize::from(edge == 1)];
        AnalyticGeometry::CircularArc {
            frame: ReferenceFrame {
                origin_mm: point(frame, [u, v]),
                ..frame
            },
            radius_mm: profile.radius_mm(),
            start_rad: (corners[edge][1] - v).atan2(corners[edge][0] - u),
            sweep_rad: PI,
        }
    }
}

pub(super) fn wall_support(
    profile: &Stadium,
    edge: usize,
    frame: ReferenceFrame,
    inward: bool,
) -> Option<ReferenceFrame> {
    if !edge.is_multiple_of(2) {
        return None;
    }
    let [a, b] = profile.centers();
    let sign = if edge == 0 { 1. } else { -1. };
    let length = profile.spine_length_mm();
    let u = (b[0] - a[0]) * sign / length;
    let v = (b[1] - a[1]) * sign / length;
    Some(ReferenceFrame {
        origin_mm: point(frame, profile.corners()[edge]),
        u: std::array::from_fn(|i| u * frame.u[i] + v * frame.v[i]),
        v: frame.normal().map(|x| if inward { -x } else { x }),
    })
}

pub(super) fn wall_measure(profile: &Stadium, edge: usize) -> (f64, [f64; 2]) {
    let corners = profile.corners();
    let radius = profile.radius_mm();
    let length = profile.spine_length_mm();
    if edge.is_multiple_of(2) {
        let center = std::array::from_fn(|i| corners[edge][i].midpoint(corners[(edge + 1) % 4][i]));
        (length, center)
    } else {
        let [a, b] = profile.centers();
        let (center, opposite) = if edge == 1 { (b, a) } else { (a, b) };
        let center = std::array::from_fn(|i| {
            center[i] + (center[i] - opposite[i]) * (2. * radius / (PI * length))
        });
        (PI * radius, center)
    }
}

impl Solid {
    /// Extrude a closed two-line, two-semicircle profile along its normal.
    pub fn rust_extrude_rounded_slot(
        frame: PlaneFrame,
        centers_uv: [[f64; 2]; 2],
        radius_mm: f64,
        signed_height_mm: f64,
        feature: EntityId,
        origins: [Origin; 4],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        frame.checked(policy)?;
        let profile =
            Stadium::new(centers_uv, radius_mm).map_err(circular_pocket_body::pocket_error)?;
        for size in [radius_mm, signed_height_mm.abs(), profile.spine_length_mm()] {
            policy.length(size)?;
        }
        if feature.is_nil()
            || origins.iter().collect::<BTreeSet<_>>().len() != origins.len()
            || origins.iter().any(|origin| {
                origin.source.is_nil()
                    || origin.role.trim().is_empty()
                    || format!("side/{}/{}", origin.source, origin.role).len() > 256
            })
        {
            return Err(input(
                "Rounded extrusion needs distinct authored boundary identities",
            ));
        }
        let body = RoundedSlotBody {
            profile,
            height: signed_height_mm.abs(),
            reverse: signed_height_mm < 0.,
            feature,
            origins,
        };
        let bounds = body.bounds(frame);
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(Self {
            body: SolidBody::RoundedSlot(Rc::new(body), frame),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }

    pub(crate) fn rounded_slot_body(&self) -> Option<(&RoundedSlotBody, PlaneFrame)> {
        match &self.body {
            SolidBody::RoundedSlot(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }
}

impl RoundedSlotBody {
    fn cap(&self, frame: PlaneFrame, end: bool, outward: bool) -> ReferenceFrame {
        let height = if end { self.height } else { 0. };
        let origin_mm = position(frame, [0., 0., if self.reverse { -height } else { height }]);
        let flip = outward && (end == self.reverse);
        ReferenceFrame {
            origin_mm,
            u: frame.u,
            v: frame.v.map(|v| if flip { -v } else { v }),
        }
    }

    pub(super) fn volume_mm3(&self) -> f64 {
        self.profile.area_mm2() * self.height
    }

    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let normal = frame.normal();
        let support = |axis: usize, sign: f64| {
            let extent = self
                .profile
                .support([sign * frame.u[axis], sign * frame.v[axis]]);
            let z = if self.reverse {
                -normal[axis]
            } else {
                normal[axis]
            };
            extent + (sign * z * self.height).max(0.)
        };
        Bounds {
            min: std::array::from_fn(|i| frame.origin[i] - support(i, -1.)),
            max: std::array::from_fn(|i| frame.origin[i] + support(i, 1.)),
        }
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        let [start, end] = [false, true].map(|end| self.cap(frame, end, false));
        let mut faces: Vec<_> = [false, true]
            .into_iter()
            .enumerate()
            .map(|(i, end)| {
                let surface = self.cap(frame, end, true);
                let center = point(self.cap(frame, end, false), self.profile.centroid());
                Face {
                    index: i as u32,
                    plane: Some((surface.origin_mm, surface.normal())),
                    center_mm: center,
                    area_mm2: self.profile.area_mm2(),
                }
            })
            .collect();
        faces.extend((0..4).map(|edge| {
            let (length, center) = wall_measure(&self.profile, edge);
            let [a, b] = [start, end].map(|frame| point(frame, center));
            Face {
                index: edge as u32 + 2,
                plane: wall_support(&self.profile, edge, start, false)
                    .map(|support| (support.origin_mm, support.normal())),
                center_mm: std::array::from_fn(|i| a[i].midpoint(b[i])),
                area_mm2: length * self.height,
            }
        }));
        faces
    }

    pub(super) fn mesh(
        &self,
        frame: PlaneFrame,
        options: multicad_rust_kernel::display::Tessellation,
    ) -> Result<ffi::Mesh, GeometryError> {
        crate::rust_display::placed_mesh(
            multicad_rust_kernel::display::stadium_prism(&self.profile, self.height, options),
            frame,
            self.reverse,
        )
    }

    pub(super) fn step(
        &self,
        frame: PlaneFrame,
    ) -> Result<String, multicad_rust_kernel::step::StepError> {
        multicad_rust_kernel::step::write_stadium_prism(
            &self.profile,
            self.height,
            frame.origin,
            frame.u,
            frame.v,
            self.reverse,
        )
    }

    pub(crate) fn planar_loops(&self, face: u32) -> Vec<Vec<(u32, bool)>> {
        if face < 2 {
            let start = face * 4;
            return vec![(start..start + 4).map(|edge| (edge, false)).collect()];
        }
        let side = face - 2;
        vec![vec![
            (side, false),
            (8 + (side + 1) % 4, false),
            (4 + side, true),
            (8 + side, true),
        ]]
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
            .chain(self.origins.iter().map(|curve| {
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
        let corners = self.profile.corners();
        let [start, end] = [false, true].map(|end| self.cap(frame, end, false));
        for (i, end) in [false, true].into_iter().enumerate() {
            let basis = self.cap(frame, end, false);
            build.face(i, Some(self.cap(frame, end, true)), Some(basis), 2..6);
            for (edge, corner) in corners.into_iter().enumerate() {
                build.non_face(rim_edge(&self.profile, edge, basis), &[i, edge + 2]);
                build.non_face(
                    AnalyticGeometry::Point {
                        position_mm: point(basis, corner),
                    },
                    &[i, edge + 2, (edge + 3) % 4 + 2],
                );
            }
        }
        for (edge, corner) in corners.into_iter().enumerate() {
            let side = wall_support(&self.profile, edge, start, false);
            let anchor = side.map(|basis| ReferenceFrame {
                v: basis.v.map(|v| if self.reverse { -v } else { v }),
                ..basis
            });
            build.face(
                edge + 2,
                side,
                anchor,
                [0, 1, (edge + 3) % 4 + 2, (edge + 1) % 4 + 2],
            );
            build.non_face(
                AnalyticGeometry::Line {
                    a_mm: point(start, corner),
                    b_mm: point(end, corner),
                },
                &[edge + 2, (edge + 3) % 4 + 2],
            );
        }
        TopologySnapshot {
            owner,
            revision,
            elements: build.into_elements(),
        }
    }
}

//! Exact circular through-bores and stable cap/rim topology.

use super::*;
use crate::rust_prism_topology::position;
use multicad_core::references::{
    AnalyticGeometry, AnchoredFrame, ReferenceFrame, TopologyElement, TopologyKey, TopologyKind,
    TopologySnapshot,
};
use multicad_rust_kernel::annulus::Annulus;
use std::{collections::BTreeSet, f64::consts::TAU};

mod boolean;
mod mirror;

pub(crate) struct AnnulusBody {
    pub(crate) geometry: Annulus,
    pub(super) feature: EntityId,
    // The wall orders are inner, outer, as in the sketch-facing interface.
    wall_origins: [Origin; 2],
    center_uv: [f64; 2],
    reverse: bool,
    cap_anchors: Option<[ReferenceFrame; 2]>,
}

impl Solid {
    /// Construct concentric circular hardware in a frame at the bore center.
    pub fn rust_annulus(
        inner_radius_mm: f64,
        outer_radius_mm: f64,
        height_mm: f64,
        feature: EntityId,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.length(height_mm)?;
        Self::rust_annulus_from_sketch(
            [inner_radius_mm, outer_radius_mm],
            height_mm,
            feature,
            frame,
            [0.; 2],
            ["hardware.bore/0/0", "hardware.outer/0"].map(|role| Origin {
                source: feature,
                role: role.into(),
            }),
            policy,
        )
    }

    /// Sweep two concentric authored circles. Radii and origins are ordered
    /// [inner, outer]. The supplied frame is at the start plane's UV origin.
    pub fn rust_annulus_from_sketch(
        radii: [f64; 2],
        signed_height_mm: f64,
        feature: EntityId,
        profile_frame: PlaneFrame,
        center_uv: [f64; 2],
        origins: [Origin; 2],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        profile_frame.checked(policy)?;
        let frame = PlaneFrame {
            origin: profile_frame.point(center_uv),
            u: profile_frame.u,
            v: profile_frame
                .v
                .map(|value| if signed_height_mm < 0. { -value } else { value }),
        };
        Self::build_rust_annulus(
            radii,
            signed_height_mm,
            feature,
            frame,
            center_uv,
            origins,
            policy,
        )
    }

    fn build_rust_annulus(
        [inner, outer]: [f64; 2],
        signed_height_mm: f64,
        feature: EntityId,
        frame: PlaneFrame,
        center_uv: [f64; 2],
        origins: [Origin; 2],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.point([center_uv[0], center_uv[1], 0.])?;
        let height = signed_height_mm.abs();
        for length in [inner, outer, height, outer - inner] {
            policy.length(length)?;
        }
        if feature.is_nil()
            || origins[0] == origins[1]
            || origins.iter().any(|origin| {
                origin.source.is_nil()
                    || origin.role.trim().is_empty()
                    || format!("side/{}/{}", origin.source, origin.role).len() > 256
            })
        {
            return Err(input("An annulus needs two distinct valid curve origins"));
        }
        let reverse = signed_height_mm < 0.;
        let geometry = Annulus::new(inner, outer, height)
            .map_err(|cause| input(format!("Invalid concentric circular solid: {cause:?}")))?;
        let body = AnnulusBody {
            geometry,
            feature,
            wall_origins: origins,
            center_uv,
            reverse,
            cap_anchors: None,
        };
        let bounds = body.bounds(frame);
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(Self {
            body: SolidBody::Annulus(Rc::new(body), frame),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }

    pub(crate) fn annulus_body(&self) -> Option<(&AnnulusBody, PlaneFrame)> {
        match &self.body {
            SolidBody::Annulus(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }
}

impl AnnulusBody {
    fn cap_anchor(
        &self,
        frame: PlaneFrame,
        cap: usize,
        center: [f64; 3],
        authored_v: [f64; 3],
    ) -> ReferenceFrame {
        self.cap_anchors.map_or_else(
            || ReferenceFrame {
                origin_mm: std::array::from_fn(|i| {
                    center[i] - frame.u[i] * self.center_uv[0] - authored_v[i] * self.center_uv[1]
                }),
                u: frame.u,
                v: authored_v,
            },
            |anchors| {
                let axes = [frame.u, frame.v, frame.normal()];
                let vector = |direction: [f64; 3]| {
                    std::array::from_fn(|i| (0..3).map(|j| axes[j][i] * direction[j]).sum())
                };
                let anchor = anchors[cap];
                ReferenceFrame {
                    origin_mm: position(frame, anchor.origin_mm),
                    u: vector(anchor.u),
                    v: vector(anchor.v),
                }
            },
        )
    }
    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let normal = frame.normal();
        let support = |i: usize, sign: f64| {
            self.geometry
                .support([frame.u[i], frame.v[i], normal[i]].map(|value| sign * value))
        };
        Bounds {
            min: std::array::from_fn(|i| frame.origin[i] - support(i, -1.)),
            max: std::array::from_fn(|i| frame.origin[i] + support(i, 1.)),
        }
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        let height = self.geometry.height_mm();
        let inner = self.geometry.inner_radius_mm();
        let outer = self.geometry.outer_radius_mm();
        let area = self.geometry.cross_section_area_mm2();
        let center = |z| position(frame, [0., 0., z]);
        vec![
            Face {
                index: 0,
                plane: Some((frame.origin, frame.normal().map(|x| -x))),
                center_mm: frame.origin,
                area_mm2: area,
            },
            Face {
                index: 1,
                plane: Some((center(height), frame.normal())),
                center_mm: center(height),
                area_mm2: area,
            },
            Face {
                index: 2,
                plane: None,
                center_mm: center(height / 2.),
                area_mm2: TAU * outer * height,
            },
            Face {
                index: 3,
                plane: None,
                center_mm: center(height / 2.),
                area_mm2: TAU * inner * height,
            },
        ]
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> TopologySnapshot {
        let caps = ["cap.start", "cap.end"].map(|role| Origin {
            source: self.feature,
            role: role.into(),
        });
        let walls = [1, 0].map(|index| {
            let curve = &self.wall_origins[index];
            BTreeSet::from([
                curve.clone(),
                Origin {
                    source: self.feature,
                    role: format!("side/{}/{}", curve.source, curve.role),
                },
            ])
        });
        let mut elements = Vec::with_capacity(8);
        let authored_v = frame
            .v
            .map(|value| if self.reverse { -value } else { value });
        let centers = [
            frame.origin,
            position(frame, [0., 0., self.geometry.height_mm()]),
        ];
        for (cap, center) in centers.into_iter().enumerate() {
            let anchor = self.cap_anchor(frame, cap, center, authored_v);
            let outward = frame.v.map(|value| if cap == 0 { -value } else { value });
            elements.push(TopologyElement {
                key: TopologyKey {
                    kind: TopologyKind::Face,
                    ordinal: cap as u32,
                },
                origins: BTreeSet::from([caps[cap].clone()]),
                boundary: walls.iter().flatten().cloned().collect(),
                frames: vec![AnchoredFrame {
                    origin: caps[cap].clone(),
                    frame: anchor,
                }],
                geometry: Some(AnalyticGeometry::Plane {
                    frame: ReferenceFrame {
                        origin_mm: center,
                        u: frame.u,
                        v: outward,
                    },
                }),
            });
        }
        for (wall, radius) in [
            self.geometry.outer_radius_mm(),
            self.geometry.inner_radius_mm(),
        ]
        .into_iter()
        .enumerate()
        {
            elements.push(TopologyElement {
                key: TopologyKey {
                    kind: TopologyKind::Face,
                    ordinal: 2 + wall as u32,
                },
                origins: walls[wall].clone(),
                boundary: caps.iter().cloned().collect(),
                frames: vec![],
                geometry: None,
            });
            for (cap, center) in centers.into_iter().enumerate() {
                let mut origins = walls[wall].clone();
                origins.insert(caps[cap].clone());
                elements.push(TopologyElement {
                    key: TopologyKey {
                        kind: TopologyKind::Edge,
                        ordinal: (2 * wall + cap) as u32,
                    },
                    boundary: origins.clone(),
                    origins,
                    frames: vec![],
                    geometry: Some(AnalyticGeometry::CircularArc {
                        frame: ReferenceFrame {
                            origin_mm: center,
                            u: frame.u,
                            v: authored_v,
                        },
                        radius_mm: radius,
                        start_rad: 0.,
                        sweep_rad: TAU,
                    }),
                });
            }
        }
        TopologySnapshot {
            owner,
            revision,
            elements,
        }
    }
}

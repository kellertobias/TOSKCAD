//! Exact involute spur solids, with authored face names and rigid placement.
use super::*;
use crate::{
    GearSweep, InvoluteGear,
    rust_prism_topology::{Builder, position},
};
use multicad_core::references::{AnalyticGeometry, ReferenceFrame, TopologySnapshot};
use multicad_rust_kernel::{
    normal_sweep,
    spur_gear::{Edge, Error, Hub, Profile, Spec, ToothRole},
};
use std::f64::consts::{PI, TAU};

mod clearance;

pub(crate) struct SpurGearBody {
    pub(super) profile: Profile,
    pub(super) feature: EntityId,
}

impl Solid {
    pub fn rust_spur_gear_named(
        spec: InvoluteGear,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        const OPERATION: &str = "spur gear";
        policy.validate()?;
        if spec.sweep != GearSweep::Spur || spec.twist_rad_per_mm != 0. {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "Rust spur gears require a straight external tooth band",
            ));
        }
        if feature.is_nil() {
            return Err(input("Spur gear feature identity must not be nil"));
        }
        let profile = Profile::new(
            Spec {
                teeth: spec.teeth,
                pitch_radius_mm: spec.pitch_radius_mm,
                base_radius_mm: spec.base_radius_mm,
                tip_radius_mm: spec.tip_radius_mm,
                root_radius_mm: spec.root_radius_mm,
                root_fillet_radius_mm: spec.root_fillet_radius_mm,
                half_base_angle_rad: spec.half_base_angle_rad,
                face_width_mm: spec.face_width_mm,
                bore_radius_mm: spec.bore_radius_mm,
            },
            policy.minimum_feature_mm,
            policy.maximum_coordinate_mm,
        )
        .and_then(|profile| {
            if let Some(hub) = spec.hub {
                profile.with_hub(
                    Hub {
                        radius_mm: hub.radius_mm,
                        axial_start_mm: hub.axial_start_mm,
                        axial_end_mm: hub.axial_end_mm,
                    },
                    policy.minimum_feature_mm,
                    policy.maximum_coordinate_mm,
                )
            } else {
                Ok(profile)
            }
        })
        .map_err(|cause| {
            error(
                OPERATION,
                if cause == Error::Limit {
                    GeometryErrorKind::Limit
                } else {
                    GeometryErrorKind::Input
                },
                format!("Invalid analytic spur profile: {cause:?}"),
            )
        })?;
        Ok(Self {
            body: SolidBody::SpurGear(
                Rc::new(SpurGearBody { profile, feature }),
                PlaneFrame::default(),
            ),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }

    pub(crate) fn spur_gear_body(&self) -> Option<(&SpurGearBody, PlaneFrame)> {
        match &self.body {
            SolidBody::SpurGear(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }
}

impl SpurGearBody {
    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let normal = frame.normal();
        let support = |index: usize, sign: f64| {
            self.profile
                .support([frame.u[index], frame.v[index], normal[index]].map(|value| value * sign))
        };
        Bounds {
            min: std::array::from_fn(|i| frame.origin[i] - support(i, -1.)),
            max: std::array::from_fn(|i| frame.origin[i] + support(i, 1.)),
        }
    }

    fn sides(&self) -> impl Iterator<Item = Edge> + '_ {
        self.profile
            .outline()
            .iter()
            .map(|edge| edge.edge)
            .chain(self.profile.bore().map(Edge::Planar))
    }

    pub(crate) fn end_on_rims(
        &self,
        frame: PlaneFrame,
    ) -> impl Iterator<Item = (Edge, AnalyticGeometry)> + '_ {
        let cap = ReferenceFrame {
            v: frame.v,
            ..self.cap(frame, 0)
        };
        self.sides().map(move |edge| (edge, rim(edge, cap)))
    }

    fn cap(&self, frame: PlaneFrame, index: usize) -> ReferenceFrame {
        let z = index as f64 * self.profile.height_mm();
        Self::cap_at(frame, index, z)
    }

    fn cap_at(frame: PlaneFrame, index: usize, z: f64) -> ReferenceFrame {
        ReferenceFrame {
            origin_mm: position(frame, [0., 0., z]),
            u: frame.u,
            v: frame.v.map(|v| if index == 0 { -v } else { v }),
        }
    }

    pub(crate) fn hub_end_on_rim(
        &self,
        frame: PlaneFrame,
        facing_upper_cap: bool,
    ) -> Option<(AnalyticGeometry, bool)> {
        let extensions = self.profile.hub_extensions();
        let near = usize::from(facing_upper_cap);
        let far = 1 - near;
        let (hub, hidden) = extensions[near]
            .map(|hub| (hub, false))
            .or_else(|| extensions[far].map(|hub| (hub, true)))?;
        let outward = Self::cap_at(frame, near, hub.end_z_mm);
        Some((hub_circle(outward, hub.radius_mm), hidden))
    }

    fn hub_end_area(&self, radius: f64) -> f64 {
        let bore = self
            .profile
            .bore()
            .and_then(normal_sweep::Edge::radius_mm)
            .unwrap_or(0.);
        PI * (radius - bore) * (radius + bore)
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        let height = self.profile.height_mm();
        let extensions = self.profile.hub_extensions();
        extensions
            .into_iter()
            .enumerate()
            .map(|(index, extension)| {
                let cap = self.cap(frame, index);
                Face {
                    index: index as u32,
                    plane: Some((cap.origin_mm, cap.normal())),
                    center_mm: cap.origin_mm,
                    area_mm2: self.profile.area_mm2()
                        - extension.map_or(0., |hub| self.hub_end_area(hub.radius_mm)),
                }
            })
            .chain(self.sides().enumerate().map(|(i, edge)| {
                let [x, y] = edge.centroid();
                let [start, end] = if i < self.profile.outline().len() {
                    [0., height]
                } else {
                    self.profile.axial_bounds()
                };
                Face {
                    index: i as u32 + 2,
                    plane: side_plane(frame, edge).map(|plane| (plane.origin_mm, plane.normal())),
                    center_mm: position(frame, [x, y, (start + end) / 2.]),
                    area_mm2: edge.length_mm() * (end - start),
                }
            }))
            .chain(
                extensions
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, extension)| {
                        let hub = extension?;
                        let cap = Self::cap_at(frame, index, hub.end_z_mm);
                        Some([
                            Face {
                                index: hub.wall_face,
                                plane: None,
                                center_mm: position(
                                    frame,
                                    [0., 0., (hub.gear_z_mm + hub.end_z_mm) / 2.],
                                ),
                                area_mm2: TAU
                                    * hub.radius_mm
                                    * (hub.gear_z_mm - hub.end_z_mm).abs(),
                            },
                            Face {
                                index: hub.cap_face,
                                plane: Some((cap.origin_mm, cap.normal())),
                                center_mm: cap.origin_mm,
                                area_mm2: self.hub_end_area(hub.radius_mm),
                            },
                        ])
                    })
                    .flatten(),
            )
            .collect()
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> TopologySnapshot {
        let outline = self.profile.outline();
        let count = outline.len();
        let bore = self.profile.bore().map(|_| count + 2);
        let extensions = self.profile.hub_extensions();
        let origin = |role| {
            vec![Origin {
                source: self.feature,
                role,
            }]
        };
        let origins = [
            origin("gear/end.start".into()),
            origin("gear/end.end".into()),
        ]
        .into_iter()
        .chain(outline.iter().map(|edge| {
            origin(format!(
                "gear/whole/tooth/{}/{}",
                edge.tooth,
                side_role(edge.role)
            ))
        }))
        .chain(self.profile.bore().map(|_| origin("gear/bore".into())))
        .chain(
            extensions
                .into_iter()
                .enumerate()
                .filter_map(|(i, hub)| {
                    hub.map(|_| {
                        [
                            origin("gear/hub.side".into()),
                            origin(
                                if i == 0 {
                                    "gear/hub.end.start"
                                } else {
                                    "gear/hub.end.end"
                                }
                                .into(),
                            ),
                        ]
                    })
                })
                .flatten(),
        )
        .collect();
        let mut build = Builder::new(origins);
        let at = |xy: [f64; 2], cap: usize| {
            position(frame, [xy[0], xy[1], cap as f64 * self.profile.height_mm()])
        };
        for (cap, extension) in extensions.into_iter().enumerate() {
            let plane = self.cap(frame, cap);
            let cap_neighbor = extension.map(|hub| hub.wall_face as usize).or(bore);
            build.face(
                cap,
                Some(plane),
                Some(plane),
                (2..2 + count).chain(cap_neighbor),
            );
            let rim_frame = ReferenceFrame {
                v: frame.v,
                ..plane
            };
            for (i, edge) in self.sides().enumerate() {
                if i < count {
                    build.non_face(rim(edge, rim_frame), &[cap, i + 2]);
                    let prev = (i + count - 1) % count;
                    build.non_face(
                        AnalyticGeometry::Point {
                            position_mm: at(edge.point(0.), cap),
                        },
                        &[cap, prev + 2, i + 2],
                    );
                } else {
                    let (bore_cap, bore_frame) = extension.map_or((cap, rim_frame), |hub| {
                        (
                            hub.cap_face as usize,
                            ReferenceFrame {
                                v: frame.v,
                                ..Self::cap_at(frame, cap, hub.end_z_mm)
                            },
                        )
                    });
                    build.non_face(rim(edge, bore_frame), &[bore_cap, i + 2]);
                }
            }
            if let Some(hub) = extension {
                let wall = hub.wall_face as usize;
                let end = hub.cap_face as usize;
                let outer = Self::cap_at(frame, cap, hub.end_z_mm);
                build.face(wall, None, None, [cap, end]);
                build.face(
                    end,
                    Some(outer),
                    Some(outer),
                    std::iter::once(wall).chain(bore),
                );
                build.non_face(hub_circle(plane, hub.radius_mm), &[cap, wall]);
                build.non_face(hub_circle(outer, hub.radius_mm), &[wall, end]);
            }
        }
        for (i, edge) in self.sides().enumerate() {
            let side = i + 2;
            let plane = side_plane(frame, edge);
            if i < count {
                let prev = (i + count - 1) % count + 2;
                let next = (i + 1) % count + 2;
                build.face(side, plane, plane, [0, 1, prev, next]);
                let p = edge.point(0.);
                build.non_face(
                    AnalyticGeometry::Line {
                        a_mm: at(p, 0),
                        b_mm: at(p, 1),
                    },
                    &[prev, side],
                );
            } else {
                build.face(
                    side,
                    None,
                    None,
                    extensions
                        .into_iter()
                        .enumerate()
                        .map(|(i, hub)| hub.map_or(i, |hub| hub.cap_face as usize)),
                );
            }
        }
        TopologySnapshot {
            owner,
            revision,
            elements: build.into_elements(),
        }
    }
}

fn hub_circle(frame: ReferenceFrame, radius_mm: f64) -> AnalyticGeometry {
    AnalyticGeometry::CircularArc {
        frame,
        radius_mm,
        start_rad: 0.,
        sweep_rad: TAU,
    }
}

pub(super) fn rim(edge: Edge, frame: ReferenceFrame) -> AnalyticGeometry {
    match edge {
        Edge::Planar(edge) => super::normal_sweep_body::rim_in_frame(edge, frame),
        Edge::Involute(curve) => {
            let [u, v] = curve
                .basis()
                .map(|[x, y]| std::array::from_fn(|i| frame.u[i] * x + frame.v[i] * y));
            AnalyticGeometry::InvoluteArc {
                frame: ReferenceFrame { u, v, ..frame },
                base_radius_mm: curve.base_radius_mm(),
                parameters: curve.interval(),
            }
        }
    }
}

fn side_plane(frame: PlaneFrame, edge: Edge) -> Option<ReferenceFrame> {
    let Edge::Planar(normal_sweep::Edge::Line { a, .. }) = edge else {
        return None;
    };
    let [x, y] = edge.tangent(0.);
    Some(ReferenceFrame {
        origin_mm: frame.point(a),
        u: std::array::from_fn(|i| frame.u[i] * x + frame.v[i] * y),
        v: frame.normal(),
    })
}

fn side_role(role: ToothRole) -> &'static str {
    match role {
        ToothRole::LeftFillet => "fillet.left",
        ToothRole::LeftRadial => "continuation.left",
        ToothRole::LeftFlank => "involute.left",
        ToothRole::Tip => "tip",
        ToothRole::RightFlank => "involute.right",
        ToothRole::RightRadial => "continuation.right",
        ToothRole::RightFillet => "fillet.right",
        ToothRole::Root => "root",
    }
}

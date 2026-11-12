//! Exact internal ring gears: an outside cylinder minus the native involute
//! cavity, with the native persistent tooth, cap and rim roles.
use super::*;
use crate::{
    GearSweep, InvoluteGear,
    rust_prism_topology::{Builder, position},
};
use multicad_core::references::{AnalyticGeometry, ReferenceFrame};
use multicad_rust_kernel::spur_gear::{
    Edge, Error,
    internal::{Ring, RingSpec},
};

mod clearance;

const OPERATION: &str = "internal ring gear";

pub(crate) struct InternalRingBody {
    pub(crate) ring: Ring,
    pub(super) feature: EntityId,
}

impl Solid {
    /// Build the native internal-ring envelope from exact Rust geometry.
    /// Refusals mirror the native adapter and its ring construction checks.
    pub fn rust_internal_ring_named(
        spec: InvoluteGear,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        let GearSweep::InternalSpur { outer_radius_mm } = spec.sweep else {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "Rust internal rings require an internal spur sweep",
            ));
        };
        if feature.is_nil() {
            return Err(input("Gear feature identity must not be nil"));
        }
        if spec.hub.is_some() {
            return Err(input(
                "External gear hub needs a positive axial overlap, a bore wall and clearance below the tooth root",
            ));
        }
        let envelope = "Internal spur needs base < inward tip < pitch < outward root < outside rim, zero twist and no separate bore";
        if spec.twist_rad_per_mm != 0. || spec.bore_radius_mm != 0. {
            return Err(input(envelope));
        }
        let ring = Ring::new(
            RingSpec {
                teeth: spec.teeth,
                pitch_radius_mm: spec.pitch_radius_mm,
                base_radius_mm: spec.base_radius_mm,
                tip_radius_mm: spec.tip_radius_mm,
                root_radius_mm: spec.root_radius_mm,
                root_fillet_radius_mm: spec.root_fillet_radius_mm,
                half_base_angle_rad: spec.half_base_angle_rad,
                face_width_mm: spec.face_width_mm,
                outer_radius_mm,
            },
            policy.minimum_feature_mm,
            policy.maximum_coordinate_mm,
            policy.linear_tolerance_mm,
        )
        .map_err(|cause| match cause {
            Error::InvalidDimensions => input(format!(
                "{envelope}; the inward tooth and space must also resolve"
            )),
            Error::InvalidProfile => {
                input("Internal fillet leaves an unresolved root space, flank or tooth tip")
            }
            Error::Limit => error(
                OPERATION,
                GeometryErrorKind::Limit,
                "Internal involute flank exceeds its exact half-turn domain",
            ),
        })?;
        // The native ring refuses flanks whose certified cubic exceeds the
        // bounded pole budget at the policy tolerance; so does Rust.
        let budget = ring.cavity().iter().all(|entry| match entry.edge {
            Edge::Involute(flank) => flank.cubic_spline(policy.linear_tolerance_mm).is_ok(),
            Edge::Planar(_) => true,
        });
        if !budget {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Limit,
                "Involute spline exceeds the bounded 256-pole budget",
            ));
        }
        Ok(Self {
            body: SolidBody::InternalRing(
                Rc::new(InternalRingBody { ring, feature }),
                PlaneFrame::default(),
            ),
            policy,
            provenance: vec![],
            rust_origins: vec![],
            retained_placement: None,
        })
    }

    pub(crate) fn internal_ring_body(&self) -> Option<(&InternalRingBody, PlaneFrame)> {
        match &self.body {
            SolidBody::InternalRing(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }
}

impl InternalRingBody {
    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let normal = frame.normal();
        let support = |index: usize, sign: f64| {
            self.ring
                .support([frame.u[index], frame.v[index], normal[index]].map(|value| value * sign))
        };
        Bounds {
            min: std::array::from_fn(|i| frame.origin[i] - support(i, -1.)),
            max: std::array::from_fn(|i| frame.origin[i] + support(i, 1.)),
        }
    }

    /// Cavity walls in authored order, then the outside rim.
    fn sides(&self) -> impl Iterator<Item = Edge> + '_ {
        self.ring
            .cavity()
            .iter()
            .map(|entry| entry.edge)
            .chain(std::iter::once(Edge::Planar(self.ring.rim())))
    }

    fn cap(&self, frame: PlaneFrame, index: usize) -> ReferenceFrame {
        let z = index as f64 * self.ring.height_mm();
        ReferenceFrame {
            origin_mm: position(frame, [0., 0., z]),
            u: frame.u,
            v: frame.v.map(|v| if index == 0 { -v } else { v }),
        }
    }

    /// Exact rims on the start cap, as drawn in an axial view.
    pub(crate) fn end_on_rims(
        &self,
        frame: PlaneFrame,
    ) -> impl Iterator<Item = (Edge, AnalyticGeometry)> + '_ {
        let cap = ReferenceFrame {
            v: frame.v,
            ..self.cap(frame, 0)
        };
        self.sides()
            .map(move |edge| (edge, super::spur_gear_body::rim(edge, cap)))
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        let height = self.ring.height_mm();
        (0..2)
            .map(|index| {
                let cap = self.cap(frame, index);
                Face {
                    index: index as u32,
                    plane: Some((cap.origin_mm, cap.normal())),
                    center_mm: cap.origin_mm,
                    area_mm2: self.ring.area_mm2(),
                }
            })
            .chain(self.sides().enumerate().map(|(i, edge)| {
                let [x, y] = edge.centroid();
                // Cavity walls are cylinders or involute extrusions.
                Face {
                    index: i as u32 + 2,
                    plane: None,
                    center_mm: position(frame, [x, y, height / 2.]),
                    area_mm2: edge.length_mm() * height,
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
        let cavity = self.ring.cavity();
        let count = cavity.len();
        let rim = count + 2;
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
        .chain(cavity.iter().map(|entry| {
            origin(format!(
                "gear/whole/tooth/{}/{}",
                entry.tooth,
                super::spur_gear_body::side_role(entry.role)
            ))
        }))
        .chain(std::iter::once(origin("gear/rim.outer".into())))
        .collect();
        let mut build = Builder::new(origins);
        let height = self.ring.height_mm();
        let at = |xy: [f64; 2], cap: usize| position(frame, [xy[0], xy[1], cap as f64 * height]);
        for cap in 0..2 {
            let plane = self.cap(frame, cap);
            build.face(cap, Some(plane), Some(plane), 2..=rim);
            let rim_frame = ReferenceFrame {
                v: frame.v,
                ..plane
            };
            for (i, edge) in self.sides().enumerate() {
                build.non_face(super::spur_gear_body::rim(edge, rim_frame), &[cap, i + 2]);
                if i < count {
                    let prev = (i + count - 1) % count;
                    build.non_face(
                        AnalyticGeometry::Point {
                            position_mm: at(edge.point(0.), cap),
                        },
                        &[cap, prev + 2, i + 2],
                    );
                }
            }
        }
        for (i, edge) in cavity.iter().enumerate() {
            let side = i + 2;
            let prev = (i + count - 1) % count + 2;
            let next = (i + 1) % count + 2;
            build.face(side, None, None, [0, 1, prev, next]);
            let p = edge.edge.point(0.);
            build.non_face(
                AnalyticGeometry::Line {
                    a_mm: at(p, 0),
                    b_mm: at(p, 1),
                },
                &[prev, side],
            );
        }
        build.face(rim, None, None, [0, 1]);
        TopologySnapshot {
            owner,
            revision,
            elements: build.into_elements(),
        }
    }
}

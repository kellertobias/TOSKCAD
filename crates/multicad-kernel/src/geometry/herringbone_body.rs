//! Continuous herringbone geometry and persistent named tooth topology.
use super::*;
use crate::{
    GearSweep, InvoluteGear,
    rust_prism_topology::{Builder, position},
};
use multicad_core::references::{AnalyticGeometry, ReferenceFrame};
use multicad_rust_kernel::{
    display::{MeshError, Tessellation},
    normal_sweep,
    spur_gear::{
        Edge, Error, Spec,
        herringbone::{Curve, Face as GearFace, Herringbone, Ring, Topology},
    },
};
use std::collections::BTreeSet;

mod clearance;
mod metrics;

pub(crate) struct HerringboneBody {
    pub(super) geometry: Herringbone,
    pub(super) feature: EntityId,
    topology: Topology,
    faces: Vec<Face>,
}

impl Solid {
    pub fn rust_herringbone_named(
        spec: InvoluteGear,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::rust_twisted_gear_named(spec, feature, policy, GearSweep::Chevron)
    }

    pub fn rust_helical_gear_named(
        spec: InvoluteGear,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        Self::rust_twisted_gear_named(spec, feature, policy, GearSweep::Helical)
    }

    fn rust_twisted_gear_named(
        spec: InvoluteGear,
        feature: EntityId,
        policy: GeometryPolicy,
        required_sweep: GearSweep,
    ) -> Result<Self, GeometryError> {
        const OPERATION: &str = "twisted gear";
        policy.validate()?;
        if spec.sweep != required_sweep {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "Rust twisted gear requires the requested continuous tooth bands",
            ));
        }
        if feature.is_nil() {
            return Err(input("Twisted gear feature identity must not be nil"));
        }
        let construct = if required_sweep == GearSweep::Helical {
            Herringbone::new_helical
        } else {
            Herringbone::new
        };
        let geometry = construct(
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
            spec.twist_rad_per_mm,
            policy.minimum_feature_mm,
            policy.maximum_coordinate_mm,
        )
        .and_then(|gear| match spec.hub {
            Some(hub) => gear.with_hub(
                multicad_rust_kernel::spur_gear::Hub {
                    radius_mm: hub.radius_mm,
                    axial_start_mm: hub.axial_start_mm,
                    axial_end_mm: hub.axial_end_mm,
                },
                policy.minimum_feature_mm,
                policy.maximum_coordinate_mm,
            ),
            None => Ok(gear),
        })
        .map_err(|cause| {
            error(
                OPERATION,
                if cause == Error::Limit {
                    GeometryErrorKind::Limit
                } else {
                    GeometryErrorKind::Input
                },
                format!("Invalid analytic twisted gear profile: {cause:?}"),
            )
        })?;
        let topology = geometry.topology();
        let faces = metrics::local_faces(&geometry, &topology);
        if faces
            .iter()
            .any(|f| !f.area_mm2.is_finite() || f.area_mm2 <= 0.)
        {
            return Err(error(
                OPERATION,
                GeometryErrorKind::InvalidResult,
                "Invalid twisted gear face area",
            ));
        }
        Ok(Self {
            body: SolidBody::Herringbone(
                Rc::new(HerringboneBody {
                    geometry,
                    feature,
                    topology,
                    faces,
                }),
                PlaneFrame::default(),
            ),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }

    pub(crate) fn herringbone_body(&self) -> Option<(&HerringboneBody, PlaneFrame)> {
        match &self.body {
            SolidBody::Herringbone(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }
    /// Validate the authored herringbone shell and integrate its decoded
    /// STEP volume in mm³.
    pub fn verify_herringbone_step(
        &self,
        source: &str,
    ) -> Option<Result<f64, multicad_rust_kernel::step_read::ReadError>> {
        let (body, frame) = self.herringbone_body()?;
        Some(multicad_rust_kernel::step_read::verify_herringbone(
            source,
            &body.geometry,
            frame.origin,
            frame.u,
            frame.v,
            self.policy.linear_tolerance_mm,
        ))
    }
}

impl HerringboneBody {
    pub(super) fn supports_step(&self, frame: PlaneFrame, policy: GeometryPolicy) -> bool {
        multicad_rust_kernel::step::supports_herringbone(
            &self.geometry,
            frame.origin,
            frame.u,
            frame.v,
            policy.linear_tolerance_mm,
        )
    }

    pub(super) fn append_step(
        &self,
        writer: &mut multicad_rust_kernel::step::Writer,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<(), multicad_rust_kernel::step::StepError> {
        writer.append_herringbone(
            &self.geometry,
            frame.origin,
            frame.u,
            frame.v,
            policy.linear_tolerance_mm,
        )
    }

    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let n = frame.normal();
        let support = |i: usize, sign: f64| {
            self.geometry
                .support([frame.u[i], frame.v[i], n[i]].map(|x| x * sign))
        };
        Bounds {
            min: std::array::from_fn(|i| frame.origin[i] - support(i, -1.)),
            max: std::array::from_fn(|i| frame.origin[i] + support(i, 1.)),
        }
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        let n = frame.normal();
        self.faces
            .iter()
            .map(|f| Face {
                index: f.index,
                plane: f
                    .plane
                    .map(|(p, outward)| (position(frame, p), n.map(|x| x * outward[2]))),
                center_mm: position(frame, f.center_mm),
                area_mm2: f.area_mm2,
            })
            .collect()
    }

    pub(crate) fn section_rims(
        &self,
        frame: ReferenceFrame,
    ) -> impl Iterator<Item = (Edge, AnalyticGeometry)> + '_ {
        self.geometry
            .profile()
            .outline()
            .iter()
            .map(|edge| edge.edge)
            .chain(self.geometry.profile().bore().map(Edge::Planar))
            .map(move |edge| (edge, super::spur_gear_body::rim(edge, frame)))
    }

    pub(crate) fn profile(&self) -> &multicad_rust_kernel::spur_gear::Profile {
        self.geometry.profile()
    }

    pub(crate) fn phase_at(&self, z: f64) -> f64 {
        self.geometry.phase_at(z)
    }

    pub(crate) fn is_helical(&self) -> bool {
        self.geometry.is_helical()
    }

    pub(crate) fn section(&self, frame: PlaneFrame, z: f64, outward: f64) -> ReferenceFrame {
        let (sin, cos) = self.geometry.phase_at(z).sin_cos();
        ReferenceFrame {
            origin_mm: position(frame, [0., 0., z]),
            u: std::array::from_fn(|i| cos * frame.u[i] + sin * frame.v[i]),
            v: std::array::from_fn(|i| outward * (-sin * frame.u[i] + cos * frame.v[i])),
        }
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> TopologySnapshot {
        let geometry = &self.geometry;
        let height = geometry.profile().height_mm();
        let t = &self.topology;
        let mut build = Builder::new(
            t.faces
                .iter()
                .map(|face| {
                    vec![Origin {
                        source: self.feature,
                        role: face.role(),
                    }]
                })
                .collect(),
        );
        let mut neighbors = vec![BTreeSet::new(); t.faces.len()];
        let mut incident = vec![BTreeSet::new(); t.vertices_mm.len()];
        for edge in &t.edges {
            let [a, b] = edge.faces.map(|i| i as usize);
            if a != b {
                neighbors[a].insert(b);
                neighbors[b].insert(a);
            }
            for vertex in edge.vertices {
                incident[vertex as usize].extend([a, b]);
            }
            let described = match edge.curve {
                Curve::Rim { ring, edge } => {
                    let z = match ring {
                        Ring::Start => 0.,
                        Ring::Apex => height / 2.,
                        Ring::End => height,
                    };
                    Some(spur_gear_body::rim(edge, self.section(frame, z, 1.)))
                }
                Curve::BoreRim { end, radius_mm } => Some(spur_gear_body::rim(
                    Edge::Planar(normal_sweep::Edge::Circle {
                        center: [0.; 2],
                        radius_mm,
                        clockwise: true,
                    }),
                    ReferenceFrame {
                        origin_mm: position(
                            frame,
                            [0., 0., self.profile().axial_bounds()[usize::from(end)]],
                        ),
                        u: frame.u,
                        v: frame.v,
                    },
                )),
                Curve::HubRim { radius_mm, z_mm } => Some(spur_gear_body::rim(
                    Edge::Planar(normal_sweep::Edge::Circle {
                        center: [0.; 2],
                        radius_mm,
                        clockwise: false,
                    }),
                    ReferenceFrame {
                        origin_mm: position(frame, [0., 0., z_mm]),
                        u: frame.u,
                        v: frame.v,
                    },
                )),
                Curve::BoreSeam { .. } | Curve::HubSeam { .. } => Some(AnalyticGeometry::Line {
                    a_mm: position(frame, t.vertices_mm[edge.vertices[0] as usize]),
                    b_mm: position(frame, t.vertices_mm[edge.vertices[1] as usize]),
                }),
                Curve::Rail { .. } => None,
            };
            if let Some(curve) = described {
                build.non_face(curve, &[a, b]);
            } else {
                build.non_analytic_edge(&[a, b]);
            }
        }
        for (i, face) in t.faces.iter().enumerate() {
            let plane = match face {
                GearFace::Cap { end } => Some(self.section(
                    frame,
                    if *end { height } else { 0. },
                    if *end { 1. } else { -1. },
                )),
                GearFace::HubCap { extension } => Some(ReferenceFrame {
                    origin_mm: position(frame, [0., 0., extension.end_z_mm]),
                    u: frame.u,
                    v: frame.v.map(|x| {
                        if extension.end_z_mm > extension.gear_z_mm {
                            x
                        } else {
                            -x
                        }
                    }),
                }),
                _ => None,
            };
            build.face(i, plane, plane, neighbors[i].iter().copied());
        }
        for (point, faces) in t.vertices_mm.iter().zip(incident) {
            build.non_face(
                AnalyticGeometry::Point {
                    position_mm: position(frame, *point),
                },
                &faces.into_iter().collect::<Vec<_>>(),
            );
        }
        TopologySnapshot {
            owner,
            revision,
            elements: build.into_elements(),
        }
    }

    pub(super) fn sample_rail(
        &self,
        frame: PlaneFrame,
        ordinal: u32,
        options: Tessellation,
    ) -> Option<Result<Vec<[f64; 3]>, MeshError>> {
        let curve = self.topology.edges.get(ordinal as usize)?.curve;
        let Curve::Rail { point: [x, y], .. } = curve else {
            return None;
        };
        let height = self.geometry.profile().height_mm();
        let sweep = self.geometry.phase_at(height / 2.).abs();
        let max_span = (8. * options.deflection / x.hypot(y))
            .sqrt()
            .min(options.angle);
        let count = (sweep / max_span).ceil().max(1.);
        if !count.is_finite() || count > options.triangle_limit as f64 {
            return Some(Err(MeshError::BudgetExceeded));
        }
        let count = count as usize;
        Some(Ok((0..=count)
            .map(|i| position(frame, curve.point(&self.geometry, i as f64 / count as f64)))
            .collect()))
    }
}

//! Rust spherical bevel solids with the native persistent tooth roles.
use super::*;
use crate::{
    SphericalBevelGear,
    rust_prism_topology::{Builder, position},
};
use multicad_core::references::{AnalyticGeometry, ReferenceFrame};
use multicad_rust_kernel::{
    bevel_gear::{
        Bevel, Error, Face as BevelFace, Shape, Spec, Topology,
        clearance::{self, Placement, ResultKind, Witness},
    },
    display::{MeshError, Tessellation},
};
use std::collections::BTreeSet;

const OPERATION: &str = "spherical bevel gear";

pub(crate) struct BevelBody {
    pub(crate) geometry: Bevel,
    pub(super) feature: EntityId,
    topology: Topology,
}

impl Solid {
    pub fn rust_spherical_bevel_gear_named(
        spec: SphericalBevelGear,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        if feature.is_nil() {
            return Err(input("Bevel feature identity must not be nil"));
        }
        let geometry = Bevel::new(
            Spec {
                teeth: spec.teeth,
                pitch_cone_angle_rad: spec.pitch_cone_angle_rad,
                base_cone_angle_rad: spec.base_cone_angle_rad,
                tip_cone_angle_rad: spec.tip_cone_angle_rad,
                root_cone_angle_rad: spec.root_cone_angle_rad,
                outer_cone_distance_mm: spec.outer_cone_distance_mm,
                face_width_mm: spec.face_width_mm,
                bore_radius_mm: spec.bore_radius_mm,
                half_base_tooth_angle_rad: spec.half_base_tooth_angle_rad,
                root_fillet_radius_mm: spec.root_fillet_radius_mm,
            },
            policy.minimum_feature_mm,
            policy.maximum_coordinate_mm,
        )
        .map_err(|cause| {
            error(
                OPERATION,
                if cause == Error::Limit {
                    GeometryErrorKind::Limit
                } else {
                    GeometryErrorKind::Input
                },
                format!("Invalid spherical bevel profile: {cause:?}"),
            )
        })?;
        let topology = geometry.topology();
        Ok(Self {
            body: SolidBody::Bevel(
                Rc::new(BevelBody {
                    geometry,
                    feature,
                    topology,
                }),
                PlaneFrame::default(),
            ),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }

    /// Validate the decoded bevel shell and integrate its STEP volume in mm³.
    pub fn verify_bevel_step(
        &self,
        source: &str,
    ) -> Option<Result<f64, multicad_rust_kernel::step_read::ReadError>> {
        let (body, frame) = self.bevel_body()?;
        Some(multicad_rust_kernel::step_read::verify_bevel(
            source,
            &body.geometry,
            frame.origin,
            frame.u,
            frame.v,
            self.policy.linear_tolerance_mm,
        ))
    }

    pub(crate) fn bevel_body(&self) -> Option<(&BevelBody, PlaneFrame)> {
        match &self.body {
            SolidBody::Bevel(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }

    /// Prove positive clearance or a strict common interior point for two
    /// placed Rust bevels. A nearest flank witness within `reach_mm` uses
    /// actual points on both solids. Separate apexes with disjoint bounds
    /// are clear without engagement.
    pub fn certify_rust_bevel_pair(
        &self,
        other: &Self,
        policy: GeometryPolicy,
        reach_mm: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<(ResultKind, Option<Witness>), GeometryError> {
        const PAIR: &str = "bevel pair interference";
        policy.validate()?;
        let unsupported = |message| error(PAIR, GeometryErrorKind::Unsupported, message);
        let (Some((a, af)), Some((b, bf))) = (self.bevel_body(), other.bevel_body()) else {
            return Err(unsupported(
                "Rust bevel interference requires two Rust bevel solids",
            ));
        };
        if self.policy != policy || other.policy != policy {
            return Err(unsupported(
                "Rust bevel interference requires a shared policy",
            ));
        }
        let apart = (0..3)
            .map(|i| (af.origin[i] - bf.origin[i]).powi(2))
            .sum::<f64>()
            .sqrt();
        if apart > policy.linear_tolerance_mm {
            let [p, q] = [a.bounds(af), b.bounds(bf)];
            if (0..3).any(|i| p.max[i] < q.min[i] || q.max[i] < p.min[i]) {
                return Ok((ResultKind::Clear, None));
            }
            return Err(unsupported(
                "Rust bevel interference requires a common pitch apex or separated bounds",
            ));
        }
        let placement = |frame: PlaneFrame| Placement {
            apex_mm: frame.origin,
            axes: [frame.u, frame.v, frame.normal()],
        };
        clearance::certify(
            [&a.geometry, &b.geometry],
            [placement(af), placement(bf)],
            policy.linear_tolerance_mm,
            reach_mm,
            interrupt,
        )
        .map(|certificate| (certificate.result, certificate.nearest))
        .map_err(|cause| {
            let kind = match cause {
                clearance::Error::Invalid => GeometryErrorKind::Unsupported,
                clearance::Error::Limit => GeometryErrorKind::Limit,
                clearance::Error::Cancelled => GeometryErrorKind::Cancelled,
            };
            error(
                PAIR,
                kind,
                format!("Rust bevel interference certificate: {cause:?}"),
            )
        })
    }
}

impl BevelBody {
    pub(super) fn supports_step(&self, frame: PlaneFrame, policy: GeometryPolicy) -> bool {
        multicad_rust_kernel::step::supports_bevel(
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
        writer.append_bevel(
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

    fn direction(frame: PlaneFrame, local: [f64; 3]) -> [f64; 3] {
        let n = frame.normal();
        std::array::from_fn(|i| frame.u[i] * local[0] + frame.v[i] * local[1] + n[i] * local[2])
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        self.topology
            .faces
            .iter()
            .enumerate()
            .map(|(index, face)| {
                let (center, area_mm2, plane) = self.geometry.face_metrics(*face);
                Face {
                    index: index as u32,
                    plane: plane
                        .map(|(p, normal)| (position(frame, p), Self::direction(frame, normal))),
                    center_mm: position(frame, center),
                    area_mm2,
                }
            })
            .collect()
    }

    pub(super) fn mesh(
        &self,
        frame: PlaneFrame,
        options: Tessellation,
    ) -> Result<Mesh, GeometryError> {
        crate::rust_display::placed_mesh(self.geometry.mesh(options), frame, false)
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> TopologySnapshot {
        let t = &self.topology;
        let role = |face: &BevelFace| Origin {
            source: self.feature,
            role: face.role(),
        };
        let mut build = Builder::new(t.faces.iter().map(|face| vec![role(face)]).collect());
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
            match self.geometry.curve_shape(t, edge) {
                Shape::Line([p, q]) => build.non_face(
                    AnalyticGeometry::Line {
                        a_mm: position(frame, p),
                        b_mm: position(frame, q),
                    },
                    &[a, b],
                ),
                Shape::Arc {
                    z_mm,
                    radius_mm,
                    start_rad,
                    sweep_rad,
                } => build.non_face(
                    AnalyticGeometry::CircularArc {
                        frame: ReferenceFrame {
                            origin_mm: position(frame, [0., 0., z_mm]),
                            u: frame.u,
                            v: frame.v,
                        },
                        radius_mm,
                        start_rad,
                        sweep_rad,
                    },
                    &[a, b],
                ),
                Shape::Sampled { .. } => build.non_analytic_edge(&[a, b]),
            }
        }
        for (i, face) in t.faces.iter().enumerate() {
            // A meridian plane contains the gear axis.
            let plane = self.geometry.face_metrics(*face).2.map(|(_, normal)| {
                let u = frame.normal();
                let n = Self::direction(frame, normal);
                ReferenceFrame {
                    origin_mm: frame.origin,
                    u,
                    v: cross(n, u),
                }
            });
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

    pub(super) fn sample_edge(
        &self,
        frame: PlaneFrame,
        ordinal: u32,
        options: Tessellation,
    ) -> Option<Result<Vec<[f64; 3]>, MeshError>> {
        let edge = self.topology.edges.get(ordinal as usize)?;
        let Shape::Sampled { piece, .. } = self.geometry.curve_shape(&self.topology, edge) else {
            return None;
        };
        Some(self.geometry.piece_segments(piece, options).map(|count| {
            (0..=count)
                .map(|i| {
                    let local =
                        self.geometry
                            .curve_point(&self.topology, edge, i as f64 / count as f64);
                    position(frame, local)
                })
                .collect()
        }))
    }
}

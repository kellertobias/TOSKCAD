//! Rust ZA worms and generated worm wheels with the native face roles.
use super::*;
use crate::{
    GeneratedZaWormWheel, ZaWorm,
    rust_prism_topology::{Builder, position},
};
use multicad_core::references::{AnalyticGeometry, ReferenceFrame};
use multicad_rust_kernel::{
    display::{MeshError, Tessellation},
    worm_gear::{
        Error, Shape, Topology, Wheel, WheelSpec, Worm, WormSpec,
        clearance::{self, Placement, ResultKind, Witness},
    },
};
use std::collections::BTreeSet;

const OPERATION: &str = "Rust ZA worm gearing";

type CurvePoint<'a> = dyn Fn(f64) -> Option<[f64; 3]> + 'a;

pub(crate) enum WormGeometry {
    Worm(Worm),
    Wheel(Wheel),
}

pub(crate) struct WormBody {
    pub(super) geometry: WormGeometry,
    pub(super) feature: EntityId,
    topology: Topology,
}

fn failure(cause: Error) -> GeometryError {
    error(
        OPERATION,
        if cause == Error::Limit {
            GeometryErrorKind::Limit
        } else {
            GeometryErrorKind::Input
        },
        format!("Invalid ZA worm gearing profile: {cause:?}"),
    )
}

impl Solid {
    fn rust_worm_body(
        geometry: WormGeometry,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let topology = match &geometry {
            WormGeometry::Worm(worm) => worm.topology(),
            WormGeometry::Wheel(wheel) => wheel.topology(),
        };
        let body = WormBody {
            geometry,
            feature,
            topology,
        };
        let bounds = body.bounds(PlaneFrame::default());
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(Self {
            body: SolidBody::Worm(Rc::new(body), PlaneFrame::default()),
            policy,
            provenance: vec![],
            rust_origins: vec![],
        })
    }

    /// Right single-start q10 ZA worm: axis local +Z, ends at ±L/2 and the
    /// phase-zero crest facing -X at Z=0, like the native worm.
    pub fn rust_za_worm_named(
        spec: ZaWorm,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        if feature.is_nil() {
            return Err(input("Worm feature identity must not be nil"));
        }
        policy.length(spec.axial_module_mm)?;
        if spec.bore_radius_mm > 0. {
            policy.length(spec.bore_radius_mm)?;
        }
        let worm = Worm::new(
            WormSpec {
                axial_module_mm: spec.axial_module_mm,
                threaded_length_mm: spec.threaded_length_mm,
                bore_radius_mm: spec.bore_radius_mm,
                axial_thinning_mm: spec.axial_thinning_mm,
            },
            policy.minimum_feature_mm,
        )
        .map_err(failure)?;
        Self::rust_worm_body(WormGeometry::Worm(worm), feature, policy)
    }

    /// Generated wheel with axis local +Z, width centered at Z=0 and the
    /// phase-zero tooth space centered on +X, like the native wheel.
    pub fn rust_generated_za_worm_wheel_named(
        spec: GeneratedZaWormWheel,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        if feature.is_nil() {
            return Err(input("Wheel feature identity must not be nil"));
        }
        policy.length(spec.axial_module_mm)?;
        policy.length(spec.face_width_mm)?;
        if spec.bore_radius_mm > 0. {
            policy.length(spec.bore_radius_mm)?;
        }
        let wheel = Wheel::new(
            WheelSpec {
                teeth: spec.teeth,
                axial_module_mm: spec.axial_module_mm,
                face_width_mm: spec.face_width_mm,
                bore_radius_mm: spec.bore_radius_mm,
                pitch_thinning_mm: spec.pitch_thinning_mm,
            },
            policy.minimum_feature_mm,
            policy.maximum_coordinate_mm,
        )
        .map_err(failure)?;
        Self::rust_worm_body(WormGeometry::Wheel(wheel), feature, policy)
    }

    /// The placed Rust worm of this solid.
    pub(crate) fn rust_worm(&self) -> Option<(&Worm, PlaneFrame)> {
        match self.worm_body()? {
            (
                WormBody {
                    geometry: WormGeometry::Worm(worm),
                    ..
                },
                frame,
            ) => Some((worm, frame)),
            _ => None,
        }
    }

    /// The placed Rust generated wheel of this solid.
    pub(crate) fn rust_worm_wheel(&self) -> Option<(&Wheel, PlaneFrame)> {
        match self.worm_body()? {
            (
                WormBody {
                    geometry: WormGeometry::Wheel(wheel),
                    ..
                },
                frame,
            ) => Some((wheel, frame)),
            _ => None,
        }
    }

    pub(crate) fn worm_body(&self) -> Option<(&WormBody, PlaneFrame)> {
        match &self.body {
            SolidBody::Worm(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }

    /// Prove positive clearance or a strict common interior point for a
    /// placed Rust worm and Rust generated wheel, in either order. A
    /// nearest flank witness within `reach_mm` uses actual points on both.
    pub fn certify_rust_worm_pair(
        &self,
        other: &Self,
        policy: GeometryPolicy,
        reach_mm: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<(ResultKind, Option<Witness>), GeometryError> {
        const PAIR: &str = "worm pair interference";
        policy.validate()?;
        let unsupported = |message| error(PAIR, GeometryErrorKind::Unsupported, message);
        let (Some((a, af)), Some((b, bf))) = (self.worm_body(), other.worm_body()) else {
            return Err(unsupported(
                "Rust worm interference requires a Rust worm and a Rust wheel",
            ));
        };
        if self.policy != policy || other.policy != policy {
            return Err(unsupported(
                "Rust worm interference requires a shared policy",
            ));
        }
        let ((worm, wf), (wheel, gf)) = match (&a.geometry, &b.geometry) {
            (WormGeometry::Worm(w), WormGeometry::Wheel(g)) => ((w, af), (g, bf)),
            (WormGeometry::Wheel(g), WormGeometry::Worm(w)) => ((w, bf), (g, af)),
            _ => {
                return Err(unsupported(
                    "Rust worm interference requires one worm and one wheel",
                ));
            }
        };
        let placement = |frame: PlaneFrame| Placement {
            origin_mm: frame.origin,
            axes: [frame.u, frame.v, frame.normal()],
        };
        clearance::certify(
            worm,
            placement(wf),
            wheel,
            placement(gf),
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
                format!("Rust worm interference certificate: {cause:?}"),
            )
        })
    }
}

impl WormBody {
    pub(super) fn volume_mm3(&self) -> f64 {
        match &self.geometry {
            WormGeometry::Worm(worm) => worm.volume_mm3(),
            WormGeometry::Wheel(wheel) => wheel.volume_mm3(),
        }
    }

    fn support(&self, direction: [f64; 3]) -> f64 {
        match &self.geometry {
            WormGeometry::Worm(worm) => worm.support(direction),
            WormGeometry::Wheel(wheel) => wheel.support(direction),
        }
    }

    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let n = frame.normal();
        let support =
            |i: usize, sign: f64| self.support([frame.u[i], frame.v[i], n[i]].map(|x| x * sign));
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
            .map(|(index, face)| Face {
                index: index as u32,
                plane: face
                    .plane
                    .map(|(p, normal)| (position(frame, p), Self::direction(frame, normal))),
                center_mm: position(frame, face.center_mm),
                area_mm2: face.area_mm2,
            })
            .collect()
    }

    pub(super) fn mesh(
        &self,
        frame: PlaneFrame,
        options: Tessellation,
    ) -> Result<Mesh, GeometryError> {
        let mesh = match &self.geometry {
            WormGeometry::Worm(worm) => worm.mesh(options),
            WormGeometry::Wheel(wheel) => wheel.mesh(options),
        };
        crate::rust_display::placed_mesh(mesh, frame, false)
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> TopologySnapshot {
        let t = &self.topology;
        let role = |role: &str| Origin {
            source: self.feature,
            role: role.into(),
        };
        let mut build = Builder::new(t.faces.iter().map(|face| vec![role(&face.role)]).collect());
        let mut neighbors = vec![BTreeSet::new(); t.faces.len()];
        let mut incident = vec![BTreeSet::new(); t.vertices_mm.len()];
        for edge in &t.edges {
            let [a, b] = edge.faces.map(|i| i as usize);
            neighbors[a].insert(b);
            neighbors[b].insert(a);
            for vertex in edge.vertices {
                incident[vertex as usize].extend([a, b]);
            }
            match edge.shape {
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
                Shape::Sampled(_) => build.non_analytic_edge(&[a, b]),
            }
        }
        for (i, face) in t.faces.iter().enumerate() {
            let plane = face.plane.map(|(p, normal)| {
                let n = Self::direction(frame, normal);
                // End planes keep the authored in-plane axes, reversed on
                // the outward -Z start face.
                let v = if normal[2] < 0. {
                    frame.v.map(|x| -x)
                } else {
                    frame.v
                };
                debug_assert!((dot(cross(frame.u, v), n) - 1.).abs() < 1e-9);
                ReferenceFrame {
                    origin_mm: position(frame, p),
                    u: frame.u,
                    v,
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
        let Shape::Sampled(curve) = edge.shape else {
            return None;
        };
        let (count, point): (_, Box<CurvePoint<'_>>) = match &self.geometry {
            WormGeometry::Worm(worm) => (
                worm.curve_segments(curve, options),
                Box::new(move |t| worm.curve_point(curve, t)),
            ),
            WormGeometry::Wheel(wheel) => (
                wheel.curve_segments(curve, options),
                Box::new(move |t| wheel.curve_point(curve, t)),
            ),
        };
        Some(count.map(|count| {
            (0..=count)
                .filter_map(|i| point(i as f64 / count as f64))
                .map(|local| position(frame, local))
                .collect()
        }))
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl Solid {
    /// Validate the decoded Rust worm or generated wheel shell and integrate
    /// its STEP volume in mm³.
    pub fn verify_worm_step(
        &self,
        source: &str,
    ) -> Option<Result<f64, multicad_rust_kernel::step_read::ReadError>> {
        let (body, frame) = self.worm_body()?;
        let tolerance = self.policy.linear_tolerance_mm;
        let [origin, u, v] = [frame.origin, frame.u, frame.v];
        Some(match &body.geometry {
            WormGeometry::Worm(worm) => multicad_rust_kernel::step_read::verify_za_worm(
                source, worm, origin, u, v, tolerance,
            ),
            WormGeometry::Wheel(wheel) => multicad_rust_kernel::step_read::verify_worm_wheel(
                source, wheel, origin, u, v, tolerance,
            ),
        })
    }
}

impl Solid {
    /// The authored compound slot of a placed Rust worm or wheel.
    pub(crate) fn expected_worm_member(
        &self,
    ) -> Option<multicad_rust_kernel::step_read::ExpectedStepMember<'_>> {
        use multicad_rust_kernel::step_read::ExpectedStepMember;
        let (body, frame) = self.worm_body()?;
        let [origin, u, v] = [frame.origin, frame.u, frame.v];
        Some(match &body.geometry {
            WormGeometry::Worm(worm) => ExpectedStepMember::Worm { worm, origin, u, v },
            WormGeometry::Wheel(wheel) => ExpectedStepMember::WormWheel {
                wheel,
                origin,
                u,
                v,
            },
        })
    }
}

impl WormBody {
    pub(super) fn supports_step(&self, frame: PlaneFrame, policy: GeometryPolicy) -> bool {
        match &self.geometry {
            WormGeometry::Worm(worm) => multicad_rust_kernel::step::supports_za_worm(
                worm,
                frame.origin,
                frame.u,
                frame.v,
                policy.linear_tolerance_mm,
            ),
            WormGeometry::Wheel(wheel) => multicad_rust_kernel::step::supports_worm_wheel(
                wheel,
                frame.origin,
                frame.u,
                frame.v,
                policy.linear_tolerance_mm,
            ),
        }
    }

    pub(super) fn append_step(
        &self,
        writer: &mut multicad_rust_kernel::step::Writer,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<(), multicad_rust_kernel::step::StepError> {
        match &self.geometry {
            WormGeometry::Worm(worm) => writer.append_za_worm(
                worm,
                frame.origin,
                frame.u,
                frame.v,
                policy.linear_tolerance_mm,
            ),
            WormGeometry::Wheel(wheel) => writer.append_worm_wheel(
                wheel,
                frame.origin,
                frame.u,
                frame.v,
                policy.linear_tolerance_mm,
            ),
        }
    }
}

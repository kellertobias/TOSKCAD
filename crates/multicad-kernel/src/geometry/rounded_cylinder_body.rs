//! Exact outer cylinder rim fillets with retained authored cap attachments.
use super::*;
use crate::{
    TreatmentEdge,
    rust_prism_topology::{place_frame, position},
};
use multicad_core::references::{
    AnalyticGeometry, AnchoredFrame, TopologyElement, TopologyKey, TopologyKind,
};
use multicad_rust_kernel::rounded_cylinder::{Face as Role, RoundedCylinder, Surface};
use std::{
    collections::BTreeSet,
    f64::consts::{FRAC_PI_2, TAU},
};
mod query;

#[derive(Clone)]
struct Labels {
    origins: BTreeSet<Origin>,
    anchors: Vec<AnchoredFrame>,
}
pub(crate) struct RoundedCylinderBody {
    pub(crate) geometry: RoundedCylinder,
    pub(super) feature: EntityId,
    labels: Vec<Option<Labels>>,
    reverse_rims: bool,
}
fn failure(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    error("cylinder fillet", kind, message)
}

impl Solid {
    pub(crate) fn rounded_cylinder_body(&self) -> Option<(&RoundedCylinderBody, PlaneFrame)> {
        match &self.body {
            SolidBody::RoundedCylinder(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }
    pub(crate) fn fillet_cylinder_rims(
        &self,
        edges: &[TreatmentEdge],
        radius: f64,
        feature: EntityId,
        policy: GeometryPolicy,
        snapshot: &TopologySnapshot,
    ) -> Option<Result<Self, GeometryError>> {
        let (stock_radius, height, mut radii, frame, reverse) =
            if let Some((body, frame)) = self.rounded_cylinder_body() {
                (
                    body.geometry.radius_mm(),
                    body.geometry.height_mm(),
                    body.geometry.fillet_radii_mm(),
                    frame,
                    body.reverse_rims,
                )
            } else {
                let (cylinder, frame) = self.axial_cylinder_frame()?;
                (
                    cylinder.circle().radius().as_f64(),
                    cylinder.height().as_f64(),
                    [None; 2],
                    frame,
                    self.cylinder_sketch().is_some_and(|s| s.reverse),
                )
            };
        Some((|| {
            if self.policy != policy {
                return Err(failure(
                    GeometryErrorKind::Unsupported,
                    "Rust Fillet requires the source geometry policy",
                ));
            }
            let source = self.rounded_cylinder_body();
            let mut labels = vec![None; 5];
            for role in [
                Role::StartCap,
                Role::EndCap,
                Role::Wall,
                Role::StartFillet,
                Role::EndFillet,
            ] {
                let ordinal = source.map_or_else(
                    || ((role as u32) < 3).then_some(role as u32),
                    |(body, _)| body.geometry.face_index(role),
                );
                if let Some(ordinal) = ordinal {
                    let face = snapshot
                        .elements
                        .iter()
                        .find(|e| e.key.kind == TopologyKind::Face && e.key.ordinal == ordinal)
                        .expect("validated source face");
                    let anchors = face
                        .frames
                        .iter()
                        .filter(|a| !self.rust_origins.contains(&a.origin))
                        .map(|a| {
                            Ok(AnchoredFrame {
                                origin: a.origin.clone(),
                                frame: super::revolution_body::local_attachment(
                                    frame, a.frame, policy,
                                )?,
                            })
                        })
                        .collect::<Result<Vec<_>, GeometryError>>()?;
                    labels[role as usize] = Some(Labels {
                        origins: face
                            .origins
                            .iter()
                            .filter(|origin| !self.rust_origins.contains(origin))
                            .cloned()
                            .collect(),
                        anchors,
                    });
                }
            }
            for edge in edges {
                let end = if let Some((body, _)) = source {
                    let rim = body
                        .geometry
                        .rims()
                        .get(edge.key.ordinal as usize)
                        .copied()
                        .ok_or_else(|| {
                            failure(GeometryErrorKind::Input, "Missing rounded-cylinder edge")
                        })?;
                    if rim.ordinal >= 2 {
                        return Err(failure(
                            GeometryErrorKind::Unsupported,
                            "Fillet requires a remaining sharp cylinder rim",
                        ));
                    }
                    rim.ordinal as usize
                } else {
                    let i = edge.key.ordinal as usize;
                    if i > 1 {
                        return Err(failure(
                            GeometryErrorKind::Unsupported,
                            "Fillet requires the circular cylinder cap rims",
                        ));
                    }
                    i
                };
                if radii[end].is_some() {
                    return Err(failure(
                        GeometryErrorKind::Unsupported,
                        "Cylinder rim is already rounded",
                    ));
                }
                radii[end] = Some(radius);
                labels[3 + end] = Some(Labels {
                    origins: BTreeSet::from([edge.origin.clone()]),
                    anchors: vec![],
                });
            }
            let geometry=RoundedCylinder::new(stock_radius,height,radii,policy.minimum_feature_mm,policy.maximum_coordinate_mm).map_err(|cause|failure(GeometryErrorKind::InvalidResult,format!("Fillet reaches a cap center or removes the intervening wall: {cause:?}; reduce its radius")))?;
            Self::finish_rounded(
                geometry,
                frame,
                feature,
                labels,
                reverse,
                self.rust_origins.clone(),
                policy,
            )
        })())
    }
    fn finish_rounded(
        geometry: RoundedCylinder,
        frame: PlaneFrame,
        feature: EntityId,
        labels: Vec<Option<Labels>>,
        reverse_rims: bool,
        rust_origins: Vec<Origin>,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        frame.checked(policy)?;
        if feature.is_nil() {
            return Err(failure(
                GeometryErrorKind::Input,
                "Rounded cylinder needs a feature identity",
            ));
        }
        let body = RoundedCylinderBody {
            geometry,
            feature,
            labels,
            reverse_rims,
        };
        frame.checked_metric_bounds(body.bounds(PlaneFrame::default()), policy)?;
        let bounds = body.bounds(frame);
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        let result = Self {
            body: SolidBody::RoundedCylinder(Rc::new(body), frame),
            policy,
            provenance: vec![],
            rust_origins,
            retained_placement: None,
        };
        result
            .topology_snapshot(feature, 0)?
            .validate()
            .map_err(|message| failure(GeometryErrorKind::InvalidResult, message))?;
        Ok(result)
    }
    pub(super) fn import_rounded_cylinder(
        geometry: RoundedCylinder,
        frame: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let mut labels = vec![None; 5];
        for role in geometry.active_faces() {
            // STEP charts carry no authored end identities. Stamp finite physical
            // regions, never the decoder's arbitrary axis direction or face order.
            let mut axis = frame.normal();
            if axis.into_iter().find(|&n| n != 0.).is_some_and(|n| n < 0.) {
                axis = axis.map(|n| -n);
            }
            let mut rings: Vec<[f64; 4]> = geometry
                .rims()
                .into_iter()
                .filter(|rim| rim.faces.contains(&role))
                .map(|rim| {
                    let p = position(frame, [0., 0., rim.z_mm]);
                    [p[0], p[1], p[2], rim.radius_mm]
                })
                .collect();
            rings.sort_unstable_by(|a, b| {
                a.iter()
                    .zip(b)
                    .find_map(|(a, b)| (a != b).then(|| a.total_cmp(b)))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let (kind, support, cap) = match geometry.surface(role).expect("active surface") {
                Surface::Plane {
                    radius_mm,
                    z_mm,
                    normal_z,
                } => {
                    let normal = frame.normal().map(|n| n * normal_z);
                    let center = position(frame, [0., 0., z_mm]);
                    (
                        "rounded.cap",
                        center
                            .into_iter()
                            .chain(normal)
                            .chain([radius_mm])
                            .collect::<Vec<_>>(),
                        Some((center, normal)),
                    )
                }
                Surface::Cylinder { radius_mm, .. } => (
                    "rounded.wall",
                    axis.into_iter().chain([radius_mm]).collect(),
                    None,
                ),
                Surface::Toroid {
                    major_radius_mm,
                    minor_radius_mm,
                    center_z_mm,
                    ..
                } => (
                    "rounded.patch",
                    position(frame, [0., 0., center_z_mm])
                        .into_iter()
                        .chain(axis)
                        .chain([major_radius_mm, minor_radius_mm])
                        .collect(),
                    None,
                ),
            };
            let origin = super::step_import::feature_origin(
                feature,
                kind,
                support.into_iter().chain(rings.into_iter().flatten()),
            );
            let anchors = if let Some((center, normal)) = cap {
                // A deterministic outward world chart prevents datum rotation
                // when an equivalent STEP record reverses its cylinder axis.
                let norm = |v: Point| v[0].hypot(v[1]).hypot(v[2]);
                let normal = normal.map(|v| v / norm(normal));
                let basis = (0..3)
                    .min_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
                    .expect("axis");
                let mut unit = [0.; 3];
                unit[basis] = 1.;
                let cross = |a: Point, b: Point| {
                    [
                        a[1] * b[2] - a[2] * b[1],
                        a[2] * b[0] - a[0] * b[2],
                        a[0] * b[1] - a[1] * b[0],
                    ]
                };
                let u = cross(unit, normal);
                let u = u.map(|v| v / norm(u));
                let v = cross(normal, u);
                vec![AnchoredFrame {
                    origin: origin.clone(),
                    frame: super::revolution_body::local_attachment(
                        frame,
                        ReferenceFrame {
                            origin_mm: center,
                            u,
                            v,
                        },
                        policy,
                    )?,
                }]
            } else {
                vec![]
            };
            labels[role as usize] = Some(Labels {
                origins: BTreeSet::from([origin]),
                anchors,
            });
        }
        Self::finish_rounded(geometry, frame, feature, labels, false, vec![], policy)
    }
}

impl RoundedCylinderBody {
    pub(crate) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let n = frame.normal();
        let support = |i, sign: f64| {
            self.geometry
                .support([frame.u[i] * sign, frame.v[i] * sign, n[i] * sign])
        };
        Bounds {
            min: std::array::from_fn(|i| frame.origin[i] - support(i, -1.)),
            max: std::array::from_fn(|i| frame.origin[i] + support(i, 1.)),
        }
    }
    pub(crate) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        self.geometry
            .active_faces()
            .into_iter()
            .enumerate()
            .map(|(index, role)| {
                let surface = self.geometry.surface(role).expect("active surface");
                let (z, plane) = match surface {
                    Surface::Plane { z_mm, normal_z, .. } => (
                        z_mm,
                        Some((
                            position(frame, [0., 0., z_mm]),
                            frame.normal().map(|n| n * normal_z),
                        )),
                    ),
                    Surface::Cylinder { span_mm, .. } => (span_mm[0].midpoint(span_mm[1]), None),
                    Surface::Toroid {
                        major_radius_mm: a,
                        minor_radius_mm: f,
                        center_z_mm: z,
                        meridian_rad,
                        ..
                    } => {
                        let sign = if meridian_rad[0] < 0. { -1. } else { 1. };
                        (z + sign * f * (a + f / 2.) / (a * FRAC_PI_2 + f), None)
                    }
                };
                Face {
                    index: index as u32,
                    plane,
                    center_mm: position(frame, [0., 0., z]),
                    area_mm2: self.geometry.face_area_mm2(role).expect("active area"),
                }
            })
            .collect()
    }
    pub(crate) fn planar_loops(&self, face: u32) -> Vec<Vec<(u32, bool)>> {
        let Some(role) = self.geometry.face_role(face).filter(|r| (*r as u32) < 2) else {
            return vec![];
        };
        self.geometry
            .rims()
            .iter()
            .position(|r| r.faces.contains(&role))
            .map_or_else(Vec::new, |i| vec![vec![(i as u32, false)]])
    }
    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> TopologySnapshot {
        let mut elements = Vec::new();
        let rims = self.geometry.rims();
        let origins = |role: Role| {
            &self.labels[role as usize]
                .as_ref()
                .expect("active labels")
                .origins
        };
        for role in self.geometry.active_faces() {
            let labels = self.labels[role as usize].as_ref().expect("active labels");
            let geometry = match self.geometry.surface(role).expect("active surface") {
                Surface::Plane { z_mm, normal_z, .. } => Some(AnalyticGeometry::Plane {
                    frame: ReferenceFrame {
                        origin_mm: position(frame, [0., 0., z_mm]),
                        u: frame.u,
                        v: frame.v.map(|x| x * normal_z),
                    },
                }),
                _ => None,
            };
            let boundary = rims
                .iter()
                .filter(|r| r.faces.contains(&role))
                .flat_map(|r| r.faces)
                .filter(|&r| r != role)
                .flat_map(|r| origins(r).iter().cloned())
                .collect();
            elements.push(TopologyElement {
                key: TopologyKey {
                    kind: TopologyKind::Face,
                    ordinal: self.geometry.face_index(role).expect("active face"),
                },
                origins: labels.origins.clone(),
                boundary,
                frames: labels
                    .anchors
                    .iter()
                    .map(|a| AnchoredFrame {
                        origin: a.origin.clone(),
                        frame: place_frame(frame, a.frame),
                    })
                    .collect(),
                geometry,
            });
        }
        for (ordinal, rim) in rims.into_iter().enumerate() {
            let origins: BTreeSet<_> = rim
                .faces
                .into_iter()
                .flat_map(|r| origins(r).iter().cloned())
                .collect();
            elements.push(TopologyElement {
                key: TopologyKey {
                    kind: TopologyKind::Edge,
                    ordinal: ordinal as u32,
                },
                boundary: origins.clone(),
                origins,
                frames: vec![],
                geometry: Some(AnalyticGeometry::CircularArc {
                    frame: ReferenceFrame {
                        origin_mm: position(frame, [0., 0., rim.z_mm]),
                        u: frame.u,
                        v: frame.v.map(|x| if self.reverse_rims { -x } else { x }),
                    },
                    radius_mm: rim.radius_mm,
                    start_rad: 0.,
                    sweep_rad: TAU,
                }),
            });
        }
        TopologySnapshot {
            owner,
            revision,
            elements,
        }
    }
    pub(crate) fn mesh(
        &self,
        frame: PlaneFrame,
        options: multicad_rust_kernel::display::Tessellation,
    ) -> Result<Mesh, GeometryError> {
        let raw = self.geometry.mesh(options);
        if frame != PlaneFrame::default()
            && let Ok(raw) = &raw
        {
            let magnitude = frame.origin.into_iter().map(f64::abs).fold(
                self.geometry
                    .radius_mm()
                    .max(self.geometry.height_mm())
                    .max(1.),
                f64::max,
            );
            let roundoff = 64. * f64::EPSILON * magnitude;
            let points = raw.vertices.as_chunks::<3>().0;
            let mut altitude = f64::INFINITY;
            for triangle in raw.indices.as_chunks::<3>().0 {
                let [a, b, c] = triangle.map(|i| points[i as usize]);
                let delta = |a: Point, b: Point| std::array::from_fn(|i| a[i] - b[i]);
                let [ab, ac, bc] = [delta(b, a), delta(c, a), delta(c, b)];
                let norm = |p: Point| p[0].hypot(p[1]).hypot(p[2]);
                let n = [
                    ab[1] * ac[2] - ab[2] * ac[1],
                    ab[2] * ac[0] - ab[0] * ac[2],
                    ab[0] * ac[1] - ab[1] * ac[0],
                ];
                altitude = altitude.min(norm(n) / norm(ab).max(norm(ac)).max(norm(bc)));
            }
            if roundoff > options.deflection / 4. || roundoff > altitude * options.angle / 64. {
                return Err(failure(
                    GeometryErrorKind::Unsupported,
                    "Placed rounded-cylinder mesh cannot meet its precision budget",
                ));
            }
        }
        crate::rust_display::placed_mesh(raw, frame, false)
    }
}
type Point = [f64; 3];

#[cfg(test)]
mod metric_frame_tests {
    use super::*;
    #[test]
    fn rounded_constructor_checks_long_axial_extent_even_with_small_radius() {
        let policy = GeometryPolicy::default();
        let frame = PlaneFrame {
            u: [1. + 4e-11, 0., 0.],
            v: [0., -1., 0.],
            ..Default::default()
        };
        for height in [3., 1_000_000.] {
            let geometry = RoundedCylinder::new(
                1.,
                height,
                [Some(0.25), None],
                policy.minimum_feature_mm,
                policy.maximum_coordinate_mm,
            )
            .unwrap();
            let result = Solid::import_rounded_cylinder(geometry, frame, EntityId::new(), policy);
            if height > 10. {
                assert_eq!(
                    result.err().expect("large metric displacement").kind,
                    GeometryErrorKind::Unsupported
                );
            } else {
                result.unwrap();
            }
        }
    }
}

//! Placement and persistent references for an exact axial profile union.
use super::*;
use crate::rust_prism_topology::{Builder, position};
use multicad_core::references::{AnalyticGeometry, ReferenceFrame, TopologyKind, TopologySnapshot};
use multicad_rust_kernel::{
    axial_profile_stack::{self, Curve, Operand, Profile, ProfileStack, Support},
    display, step,
};
use std::collections::BTreeSet;

type Point = [f64; 3];

pub(crate) struct AxialStackBody {
    pub(super) feature: EntityId,
    geometry: ProfileStack,
    origins: Vec<Vec<Origin>>,
}

fn local(frame: PlaneFrame, point: Point) -> Point {
    let delta: Point = std::array::from_fn(|i| point[i] - frame.origin[i]);
    [frame.u, frame.v, frame.normal()].map(|axis| dot(delta, axis))
}

fn vector(frame: PlaneFrame, point: Point) -> Point {
    position(
        PlaneFrame {
            origin: [0.; 3],
            ..frame
        },
        point,
    )
}

fn rectangular_operand(solid: &Solid, frame: PlaneFrame) -> Option<Operand> {
    let (box_, placement) = solid.rust_prism_frame()?;
    if box_.rounded_end().is_some() || box_.inner_loops().len() != 0 {
        return None;
    }
    let target_axes = [frame.u, frame.v, frame.normal()];
    let source_axes = [placement.u, placement.v, placement.normal()];
    let dimensions = box_.dimensions().map(Rational::as_f64);
    let mut extent = [0.; 3];
    for (length, axis) in dimensions.into_iter().zip(source_axes) {
        let mapped = target_axes.map(|target| dot(axis, target));
        let coordinate = mapped.iter().position(|d| (d.abs() - 1.).abs() <= 1e-12)?;
        if extent[coordinate] != 0.
            || mapped
                .iter()
                .enumerate()
                .any(|(i, d)| i != coordinate && d.abs() > 1e-12)
        {
            return None;
        }
        extent[coordinate] = mapped[coordinate].signum() * length;
    }
    let start = local(frame, placement.origin);
    let end: Point = std::array::from_fn(|i| start[i] + extent[i]);
    let lo: Point = std::array::from_fn(|i| start[i].min(end[i]));
    let hi: Point = std::array::from_fn(|i| start[i].max(end[i]));
    Some(Operand {
        span_mm: [lo[2], hi[2]],
        profile: Profile::Rectangle {
            min: [lo[0], lo[1]],
            max: [hi[0], hi[1]],
        },
    })
}

impl Solid {
    pub fn union_round_rectangular_stack(
        solids: &[&Self],
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Option<Result<Self, GeometryError>> {
        if solids.len() < 2
            || policy.boolean_fuzzy_mm != 0.
            || solids.iter().any(|s| s.policy != policy)
        {
            return None;
        }
        let mut rounds = solids.iter().enumerate().filter_map(|(i, solid)| {
            solid
                .axial_cylinder_frame()
                .map(|(cylinder, frame)| (i, cylinder, frame))
        });
        let (round, cylinder, frame) = rounds.next()?;
        if rounds.next().is_some() {
            return None;
        }
        let operands = solids
            .iter()
            .enumerate()
            .map(|(i, solid)| {
                if i == round {
                    Some(Operand {
                        span_mm: [0., cylinder.height().as_f64()],
                        profile: Profile::Circle {
                            center: [0.; 2],
                            radius_mm: cylinder.circle().radius().as_f64(),
                        },
                    })
                } else {
                    rectangular_operand(solid, frame)
                }
            })
            .collect::<Option<Vec<_>>>()?;
        let geometry = match ProfileStack::union(
            &operands,
            policy.linear_tolerance_mm,
            policy.minimum_feature_mm,
        ) {
            Ok(geometry) => geometry,
            Err(axial_profile_stack::Error::Limit) => {
                return Some(Err(error(
                    "axial profile union",
                    GeometryErrorKind::Limit,
                    "The union exceeds its analytic profile budget",
                )));
            }
            Err(_) => return None,
        };
        Some(AxialStackBody::place(
            geometry, frame, solids, feature, policy,
        ))
    }

    pub(crate) fn axial_stack_body(&self) -> Option<(&AxialStackBody, PlaneFrame)> {
        match &self.body {
            SolidBody::AxialStack(body, frame) => Some((body, *frame)),
            _ => None,
        }
    }

    pub(super) fn from_imported_axial_stack(
        geometry: ProfileStack,
        frame: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let origins = (0..geometry.faces().len())
            .map(|index| {
                vec![Origin {
                    source: feature,
                    role: format!("step.axial-face.{index}"),
                }]
            })
            .collect();
        AxialStackBody::place_with_origins(geometry, frame, feature, origins, policy)
    }
}

impl AxialStackBody {
    pub(super) fn append_step(
        &self,
        writer: &mut step::Writer,
        frame: PlaneFrame,
    ) -> Result<(), step::StepError> {
        writer.append_axial_profile_stack(&self.geometry, frame.origin, frame.u, frame.v)
    }

    fn place(
        geometry: ProfileStack,
        frame: PlaneFrame,
        solids: &[&Solid],
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Solid, GeometryError> {
        if feature.is_nil() {
            return Err(input("Axial union requires a source feature"));
        }
        let snapshots = solids
            .iter()
            .map(|solid| solid.rust_topology().expect("Rust operand"))
            .collect::<Result<Vec<_>, _>>()?;
        let mut origins = Vec::with_capacity(geometry.faces().len());
        for (i, face) in geometry.faces().iter().enumerate() {
            let mut ancestry = BTreeSet::new();
            let center = position(frame, face.center_mm);
            for &source in &face.sources {
                for original in &snapshots[source].elements {
                    if original.key.kind != TopologyKind::Face {
                        continue;
                    }
                    let coplanar = match (face.support, original.geometry) {
                        (
                            Support::Plane { normal },
                            Some(AnalyticGeometry::Plane { frame: plane }),
                        ) => {
                            let normal = vector(frame, normal);
                            let delta: Point =
                                std::array::from_fn(|j| center[j] - plane.origin_mm[j]);
                            dot(normal, plane.normal()) > 1. - 1e-10
                                && dot(delta, normal).abs() <= policy.linear_tolerance_mm
                        }
                        (Support::Cylinder { .. }, None) => true,
                        _ => false,
                    };
                    if coplanar {
                        ancestry.extend(original.origins.iter().cloned());
                    }
                }
            }
            origins.push(
                std::iter::once(Origin {
                    source: feature,
                    role: format!("axial.face.{i}"),
                })
                .chain(ancestry)
                .collect(),
            );
        }
        Self::place_with_origins(geometry, frame, feature, origins, policy)
    }

    fn place_with_origins(
        geometry: ProfileStack,
        frame: PlaneFrame,
        feature: EntityId,
        origins: Vec<Vec<Origin>>,
        policy: GeometryPolicy,
    ) -> Result<Solid, GeometryError> {
        let body = Self {
            feature,
            geometry,
            origins,
        };
        frame.checked_metric_bounds(body.bounds(PlaneFrame::default()), policy)?;
        let bounds = body.bounds(frame);
        policy.point(bounds.min)?;
        policy.point(bounds.max)?;
        Ok(Solid {
            body: SolidBody::AxialStack(Rc::new(body), frame),
            policy,
            provenance: vec![],
            rust_origins: vec![],
            retained_placement: None,
        })
    }

    pub(super) fn volume_mm3(&self) -> f64 {
        self.geometry.volume_mm3()
    }

    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let mut bounds = Bounds {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        };
        let mut include = |point| {
            let point = position(frame, point);
            for (i, value) in point.into_iter().enumerate() {
                bounds.min[i] = bounds.min[i].min(value);
                bounds.max[i] = bounds.max[i].max(value);
            }
        };
        for &point in self.geometry.vertices() {
            include(point);
        }
        // A coordinate extremum can occur between the endpoints of a rim.
        for edge in self.geometry.edges() {
            let Curve::Arc {
                center_mm,
                radius_mm,
                sweep_rad,
            } = edge.curve
            else {
                continue;
            };
            let a = self.geometry.vertices()[edge.vertices[0]];
            let phase = (a[1] - center_mm[1]).atan2(a[0] - center_mm[0]);
            for i in 0..3 {
                let phase_max = frame.v[i].atan2(frame.u[i]);
                for angle in [phase_max, phase_max + std::f64::consts::PI] {
                    let progress =
                        ((angle - phase) * sweep_rad.signum()).rem_euclid(std::f64::consts::TAU);
                    if progress <= sweep_rad.abs() {
                        include([
                            center_mm[0] + radius_mm * angle.cos(),
                            center_mm[1] + radius_mm * angle.sin(),
                            center_mm[2],
                        ]);
                    }
                }
            }
        }
        bounds
    }

    fn plane(&self, face: usize, frame: PlaneFrame) -> Option<ReferenceFrame> {
        let face = &self.geometry.faces()[face];
        let Support::Plane { normal } = face.support else {
            return None;
        };
        let edge = &self.geometry.edges()[face.boundary[0].edge];
        let origin = self.geometry.vertices()[edge.vertices[0]];
        let basis = normal.iter().position(|coordinate| *coordinate != 0.)?;
        let u = std::array::from_fn(|i| f64::from(i == (basis + 1) % 3));
        Some(ReferenceFrame {
            origin_mm: position(frame, origin),
            u: vector(frame, u),
            v: vector(frame, cross(normal, u)),
        })
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        self.geometry
            .faces()
            .iter()
            .enumerate()
            .map(|(index, face)| Face {
                index: index as u32,
                plane: self
                    .plane(index, frame)
                    .map(|plane| (plane.origin_mm, plane.normal())),
                center_mm: position(frame, face.center_mm),
                area_mm2: face.area_mm2,
            })
            .collect()
    }

    pub(crate) fn planar_loops(&self, face: u32) -> Vec<Vec<(u32, bool)>> {
        self.geometry
            .faces()
            .get(face as usize)
            .and_then(|face| {
                matches!(face.support, Support::Plane { .. }).then(|| {
                    face.boundary
                        .iter()
                        .map(|use_| (use_.edge as u32, !use_.forward))
                        .collect()
                })
            })
            .into_iter()
            .collect()
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> TopologySnapshot {
        let mut builder = Builder::new(self.origins.clone());
        let mut edge_faces = vec![Vec::new(); self.geometry.edges().len()];
        let mut vertex_faces = vec![BTreeSet::new(); self.geometry.vertices().len()];
        for (i, face) in self.geometry.faces().iter().enumerate() {
            for use_ in &face.boundary {
                edge_faces[use_.edge].push(i);
                for vertex in self.geometry.edges()[use_.edge].vertices {
                    vertex_faces[vertex].insert(i);
                }
            }
        }
        for (i, edge) in self.geometry.edges().iter().enumerate() {
            let [a, b] = edge.vertices.map(|j| self.geometry.vertices()[j]);
            let geometry = match edge.curve {
                Curve::Line => AnalyticGeometry::Line {
                    a_mm: position(frame, a),
                    b_mm: position(frame, b),
                },
                Curve::Arc {
                    center_mm,
                    radius_mm,
                    sweep_rad,
                } => {
                    let u = [
                        (a[0] - center_mm[0]) / radius_mm,
                        (a[1] - center_mm[1]) / radius_mm,
                        0.,
                    ];
                    let v = [-u[1], u[0], 0.];
                    AnalyticGeometry::CircularArc {
                        frame: ReferenceFrame {
                            origin_mm: position(frame, center_mm),
                            u: vector(frame, u),
                            v: vector(frame, v),
                        },
                        radius_mm,
                        start_rad: 0.,
                        sweep_rad,
                    }
                }
            };
            builder.non_face(geometry, &edge_faces[i]);
        }
        for (i, &vertex) in self.geometry.vertices().iter().enumerate() {
            let incident: Vec<_> = vertex_faces[i].iter().copied().collect();
            builder.non_face(
                AnalyticGeometry::Point {
                    position_mm: position(frame, vertex),
                },
                &incident,
            );
        }
        for (i, face) in self.geometry.faces().iter().enumerate() {
            let plane = self.plane(i, frame);
            builder.face(
                i,
                plane,
                plane,
                face.boundary
                    .iter()
                    .flat_map(|use_| edge_faces[use_.edge].iter().copied())
                    .filter(|neighbor| *neighbor != i),
            );
        }
        TopologySnapshot {
            owner,
            revision,
            elements: builder.into_elements(),
        }
    }

    pub(super) fn mesh(
        &self,
        frame: PlaneFrame,
        options: display::Tessellation,
    ) -> Result<Mesh, GeometryError> {
        crate::rust_display::placed_mesh(
            display::axial_profile_stack(&self.geometry, options),
            frame,
            false,
        )
    }
}

#[cfg(test)]
mod metric_frame_tests {
    use super::*;
    #[test]
    fn axial_stack_constructor_checks_negative_local_levels() {
        let policy = GeometryPolicy::default();
        let frame = PlaneFrame {
            u: [1. + 4e-11, 0., 0.],
            ..Default::default()
        };
        for length in [10., 1_000_000.] {
            let geometry = ProfileStack::union(
                &[
                    Operand {
                        span_mm: [-length, 1.],
                        profile: Profile::Rectangle {
                            min: [-1.; 2],
                            max: [1.; 2],
                        },
                    },
                    Operand {
                        span_mm: [0., 2.],
                        profile: Profile::Circle {
                            center: [0.; 2],
                            radius_mm: 1.,
                        },
                    },
                ],
                policy.linear_tolerance_mm,
                policy.minimum_feature_mm,
            )
            .unwrap();
            let result = Solid::from_imported_axial_stack(geometry, frame, EntityId::new(), policy);
            if length > 100. {
                assert_eq!(
                    result
                        .err()
                        .expect("remote negative level metric displacement")
                        .kind,
                    GeometryErrorKind::Unsupported
                );
            } else {
                result.unwrap();
            }
        }
    }
}

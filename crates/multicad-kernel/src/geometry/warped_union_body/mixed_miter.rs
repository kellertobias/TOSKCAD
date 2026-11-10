//! Place analytic line-and-arc miter walls and attach their authored identities.
use super::*;
use crate::{CurvedEdge2d, CurvedProfile};
use multicad_rust_kernel::normal_sweep::Edge;

const OPERATION: &str = "Rust line-and-arc miter sweep";

fn unit(point: Point) -> Point {
    let length = norm(point);
    point.map(|v| v / length)
}

impl Solid {
    pub(crate) fn rust_sweep_mixed_miter_named(
        profile: &CurvedProfile,
        path_mm: &[Point],
        feature: EntityId,
        profile_origins: &[Origin],
        path_origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let start = path_mm[0];
        let first = subtract(path_mm[1], start);
        let frame = profile.frame;
        let u = unit(frame.u);
        let reversed = dot(frame.normal(), first) < 0.;
        let forward = unit(frame.normal()).map(|v| if reversed { -v } else { v });
        let v = cross(forward, u);
        let placement = PlaneFrame {
            origin: start,
            u,
            v,
        };
        let local = |point: Point| [dot(point, u), dot(point, v), dot(point, forward)];
        let sketch = local(subtract(frame.origin, start));
        let mut path = path_mm
            .iter()
            .map(|&point| {
                policy.point(point)?;
                Ok(local(subtract(point, start)))
            })
            .collect::<Result<Vec<_>, GeometryError>>()?;
        path[1] = [0., 0., norm(first)];
        let profile_u = local(frame.u);
        let profile_v = local(frame.v);
        let point = |[x, y]: [f64; 2]| {
            std::array::from_fn(|i| profile_u[i].mul_add(x, profile_v[i].mul_add(y, sketch[i])))
        };
        let edges = profile
            .outer
            .iter()
            .map(|edge| {
                edge.data(policy, 0)?;
                Ok(match *edge {
                    CurvedEdge2d::Line { a, b } => Edge::Line {
                        a: point(a),
                        b: point(b),
                    },
                    CurvedEdge2d::Arc {
                        center,
                        a,
                        b,
                        clockwise,
                    } => Edge::Arc {
                        center: point(center),
                        a: point(a),
                        b: point(b),
                        clockwise: clockwise ^ reversed,
                    },
                    _ => {
                        return Err(error(
                            OPERATION,
                            GeometryErrorKind::Unsupported,
                            "A mixed miter needs straight edges or circular arcs",
                        ));
                    }
                })
            })
            .collect::<Result<Vec<_>, GeometryError>>()?;
        let (geometry, [end_u, end_v]) =
            warped_union::mixed_miter_with_end_frame(&edges, &path, policy.linear_tolerance_mm)
                .map_err(|cause| {
                    let (kind, message) = match cause {
                        warped_union::Error::Invalid => (
                            GeometryErrorKind::InvalidResult,
                            "The mixed sweep has an invalid boundary",
                        ),
                        warped_union::Error::Unsupported => (
                            GeometryErrorKind::Unsupported,
                            "The mixed sweep could not certify its joints and path clearance",
                        ),
                        warped_union::Error::Limit => (
                            GeometryErrorKind::Limit,
                            "The mixed sweep exceeds the geometry range",
                        ),
                    };
                    error(OPERATION, kind, message)
                })?;

        let sketch = [sketch[0], sketch[1], 0.];
        let start_u = [1., 0., 0.];
        let end = path[path.len() - 1];
        let cap_frames = [
            (sketch, start_u),
            (
                std::array::from_fn(|i| end[i] + sketch[0] * end_u[i] + sketch[1] * end_v[i]),
                end_u,
            ),
        ];
        let mut origins = Vec::with_capacity(geometry.faces().len());
        let mut anchors = Vec::with_capacity(geometry.faces().len());
        let mut sectors = BTreeMap::new();
        for face in geometry.faces() {
            if face.source_face < 2 {
                let cap = face.source_face;
                let Support::Plane { normal } = face.support else {
                    return Err(error(
                        OPERATION,
                        GeometryErrorKind::InvalidResult,
                        "The mixed sweep cap has no plane",
                    ));
                };
                let origin = Origin {
                    source: feature,
                    role: ["cap.start", "cap.end"][cap].into(),
                };
                let (origin_mm, u) = cap_frames[cap];
                let frame = ReferenceFrame {
                    origin_mm,
                    u,
                    v: cross(normal, u),
                };
                origins.push(vec![origin.clone()]);
                anchors.push(vec![AnchoredFrame { origin, frame }]);
                continue;
            }
            let edge = face.source_face - 2;
            let path = &path_origins[face.source];
            let sector = sectors.entry((face.source, edge)).or_insert(0);
            let mut labels = vec![Origin {
                source: feature,
                role: format!("sweep.wall/{}/{edge}/{sector}", path.source),
            }];
            *sector += 1;
            for origin in [&profile_origins[edge], path] {
                labels.push(origin.clone());
                labels.push(Origin {
                    source: feature,
                    role: format!("side/{}/{}", origin.source, origin.role),
                });
            }
            let wall_anchors = match face.support {
                Support::Plane { normal } => {
                    let edge = face.boundary[0];
                    let [mut a, mut b] = geometry.edges()[edge.edge].vertices;
                    if !edge.forward {
                        std::mem::swap(&mut a, &mut b);
                    }
                    let origin_mm = geometry.vertices()[a];
                    let u = unit(subtract(geometry.vertices()[b], origin_mm));
                    let frame = ReferenceFrame {
                        origin_mm,
                        u,
                        v: cross(normal, u),
                    };
                    labels
                        .iter()
                        .cloned()
                        .map(|origin| AnchoredFrame { origin, frame })
                        .collect()
                }
                Support::Cylinder { .. } => vec![],
            };
            origins.push(labels);
            anchors.push(wall_anchors);
        }
        Self::from_boundary_shell(geometry, placement, feature, origins, anchors, policy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_miter_reverses_arc_phase_with_the_sweep_direction() {
        let [a, b, c, d] = [[-3., -1.], [3., -1.], [3., 1.], [-3., 1.]];
        let profile = CurvedProfile {
            frame: PlaneFrame {
                origin: [90., 30., 20.],
                ..Default::default()
            },
            outer: vec![
                CurvedEdge2d::Line { a, b },
                CurvedEdge2d::Arc {
                    center: [3., 0.],
                    a: b,
                    b: c,
                    clockwise: false,
                },
                CurvedEdge2d::Line { a: c, b: d },
                CurvedEdge2d::Arc {
                    center: [-3., 0.],
                    a: d,
                    b: a,
                    clockwise: false,
                },
            ],
            holes: vec![],
        };
        let path = [[90., 30., 20.], [90., 30., 0.], [110., 30., 0.]];
        let curve = |role: &str| Origin {
            source: EntityId::new(),
            role: role.into(),
        };
        let origins = [
            curve("curve"),
            curve("curve"),
            curve("curve"),
            curve("curve"),
        ];
        let path_origins = [curve("path.edge"), curve("path.edge")];
        let feature = EntityId::new();
        let solid = Solid::rust_sweep_polyline_named(
            &profile,
            &path,
            feature,
            &origins,
            &path_origins,
            GeometryPolicy::default(),
        )
        .unwrap();
        assert!((solid.volume_mm3().unwrap() - 40. * (12. + std::f64::consts::PI)).abs() < 1e-7);
        let topology = solid.topology_snapshot(feature, 0).unwrap();
        for (role, origin_mm, normal) in [
            ("cap.start", path[0], [0., 0., 1.]),
            ("cap.end", path[2], [1., 0., 0.]),
        ] {
            let frame = topology
                .elements
                .iter()
                .filter(|element| element.key.kind == TopologyKind::Face)
                .flat_map(|element| &element.frames)
                .find(|anchor| anchor.origin.role == role)
                .unwrap()
                .frame;
            assert_eq!(frame.origin_mm, origin_mm);
            assert_eq!(frame.normal(), normal);
        }
        #[cfg(feature = "occt")]
        {
            let reference = Solid::sweep_polyline_named(
                &profile,
                &path,
                feature,
                &origins,
                &path_origins,
                GeometryPolicy::default(),
            )
            .unwrap();
            assert!((reference.volume_mm3().unwrap() - solid.volume_mm3().unwrap()).abs() < 1e-5);
        }
    }
}

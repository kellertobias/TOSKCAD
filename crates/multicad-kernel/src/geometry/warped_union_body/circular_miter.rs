//! Exact circular sweep bands with shared elliptical miters.
use super::*;

const OPERATION: &str = "Rust circular miter sweep";

fn normalized(vector: Point) -> Point {
    let length = norm(vector);
    vector.map(|component| component / length)
}

fn in_plane(vector: Point, normal: Point) -> Point {
    let height = dot(vector, normal);
    std::array::from_fn(|i| vector[i] - normal[i] * height)
}

impl Solid {
    pub(crate) fn rust_sweep_circular_miter_named(
        frame: PlaneFrame,
        center: [f64; 2],
        radius_mm: f64,
        path_mm: &[Point],
        feature: EntityId,
        (profile_origin, path_origins): (&Origin, &[Origin]),
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.point([center[0], center[1], 0.])?;
        policy.length(radius_mm)?;
        for &point in path_mm {
            policy.point(point)?;
        }
        let directions = path_mm
            .windows(2)
            .map(|pair| {
                let direction = subtract(pair[1], pair[0]);
                policy.length(norm(direction))?;
                Ok(normalized(direction))
            })
            .collect::<Result<Vec<_>, GeometryError>>()?;
        let start = path_mm[0];
        let incoming = directions[0];
        let sketch = in_plane(subtract(frame.origin, start), incoming);
        let start_u = normalized(in_plane(frame.u, incoming));
        let mut center = in_plane(
            std::array::from_fn(|i| {
                frame.u[i].mul_add(center[0], frame.v[i].mul_add(center[1], sketch[i]))
            }),
            incoming,
        );
        let mut path = Vec::with_capacity(path_mm.len());
        path.push(center);
        let (mut end_sketch, mut end_u) = (sketch, start_u);
        for (&corner, pair) in path_mm[1..].iter().zip(directions.windows(2)) {
            let (incoming, outgoing) = (pair[0], pair[1]);
            let bisector = std::array::from_fn(|i| incoming[i] + outgoing[i]);
            let slope = dot(bisector, incoming);
            if slope <= 64. * f64::EPSILON {
                return Err(error(
                    OPERATION,
                    GeometryErrorKind::Input,
                    "Sweep path reverses at a corner",
                ));
            }
            // Intersect the incoming generator with the bisector and project
            // to the outgoing disk to transport the authored section offsets.
            let on_miter = |point: Point| {
                let travel = -dot(bisector, point) / slope;
                std::array::from_fn(|i| incoming[i].mul_add(travel, point[i]))
            };
            let miter_center = on_miter(center);
            let corner = subtract(corner, start);
            path.push(std::array::from_fn(|i| corner[i] + miter_center[i]));
            center = in_plane(miter_center, outgoing);
            end_sketch = in_plane(on_miter(end_sketch), outgoing);
            end_u = normalized(in_plane(on_miter(end_u), outgoing));
        }
        let end = subtract(path_mm[path_mm.len() - 1], start);
        path.push(std::array::from_fn(|i| end[i] + center[i]));
        for (pair, &axis) in path.windows(2).zip(&directions) {
            let adjusted = subtract(pair[1], pair[0]);
            if dot(adjusted, axis) <= policy.minimum_feature_mm
                || norm(cross(adjusted, axis)) > policy.linear_tolerance_mm
            {
                return Err(error(
                    OPERATION,
                    GeometryErrorKind::Unsupported,
                    "The offset circular section collapses or reverses a sweep leg",
                ));
            }
        }
        let geometry = warped_union::circular_miter_chain(
            &path,
            radius_mm,
            policy.linear_tolerance_mm,
        )
        .map_err(|cause| {
            let (kind, message) = match cause {
                warped_union::Error::Invalid => (
                    GeometryErrorKind::InvalidResult,
                    "The circular miter has an invalid closed boundary",
                ),
                warped_union::Error::Unsupported => (
                    GeometryErrorKind::Unsupported,
                    "The circular sweep cannot certify clearance between its trims and path legs",
                ),
                warped_union::Error::Limit => (
                    GeometryErrorKind::Limit,
                    "The circular miter exceeds the geometry range",
                ),
            };
            error(OPERATION, kind, message)
        })?;
        let cap_frames = [
            (sketch, start_u),
            (std::array::from_fn(|i| end[i] + end_sketch[i]), end_u),
        ];
        let mut origins = Vec::with_capacity(geometry.faces().len());
        let mut anchors = Vec::with_capacity(geometry.faces().len());
        for face in geometry.faces() {
            match face.support {
                Support::Plane { normal } => {
                    let cap = face.source_face;
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
                }
                Support::Cylinder { .. } => {
                    let path = &path_origins[face.source];
                    let mut labels = vec![Origin {
                        source: feature,
                        role: format!("sweep.sector/{}/{}", path.source, face.source_face - 2),
                    }];
                    for source in [profile_origin, path] {
                        labels.push(source.clone());
                        labels.push(Origin {
                            source: feature,
                            role: format!("side/{}/{}", source.source, source.role),
                        });
                    }
                    origins.push(labels);
                    anchors.push(vec![]);
                }
            }
        }
        let placement = PlaneFrame {
            origin: start,
            ..Default::default()
        };
        Self::from_boundary_shell(geometry, placement, feature, origins, anchors, policy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CurvedEdge2d, CurvedProfile};

    #[test]
    fn circular_miter_transports_an_offset_section_and_the_sketch_cap_anchor() {
        let feature = EntityId::new();
        let origin = |role: &str| Origin {
            source: EntityId::new(),
            role: role.into(),
        };
        let profile = CurvedProfile {
            frame: PlaneFrame {
                origin: [100., 200., 300.],
                ..Default::default()
            },
            outer: vec![CurvedEdge2d::Circle {
                center: [2., 1.],
                radius_mm: 1.5,
            }],
            holes: vec![],
        };
        let path = [[100., 200., 300.], [100., 200., 280.], [120., 200., 280.]];
        let authored = [origin("curve"), origin("path.edge"), origin("path.edge")];
        let solid = Solid::rust_sweep_polyline_named(
            &profile,
            &path,
            feature,
            &authored[..1],
            &authored[1..],
            GeometryPolicy::default(),
        )
        .unwrap();
        let near = |actual: f64, expected: f64| {
            assert!((actual - expected).abs() < 1e-7, "{actual} != {expected}");
        };
        near(
            solid.volume_mm3().unwrap(),
            std::f64::consts::PI * 1.5_f64.powi(2) * 36.,
        );
        let bounds = solid.bounds().unwrap();
        for (actual, expected) in bounds
            .min
            .into_iter()
            .chain(bounds.max)
            .zip([100.5, 199.5, 280.5, 120., 202.5, 300.])
        {
            near(actual, expected);
        }
        let topology = solid.topology_snapshot(feature, 3).unwrap();
        for (role, origin_mm, normal) in [
            ("cap.start", path[0], [0., 0., 1.]),
            ("cap.end", path[2], [1., 0., 0.]),
        ] {
            let anchor = topology
                .elements
                .iter()
                .filter(|element| element.key.kind == TopologyKind::Face)
                .flat_map(|element| &element.frames)
                .find(|anchor| anchor.origin.role == role)
                .unwrap();
            assert_eq!(anchor.frame.origin_mm, origin_mm);
            assert_eq!(anchor.frame.normal(), normal);
        }
        #[cfg(feature = "occt")]
        {
            let native = Solid::sweep_polyline_named(
                &profile,
                &path,
                feature,
                &authored[..1],
                &authored[1..],
                GeometryPolicy::default(),
            )
            .unwrap();
            near(native.volume_mm3().unwrap(), solid.volume_mm3().unwrap());
        }
    }

    #[test]
    fn circular_chain_transports_an_offset_and_rejects_a_reversed_leg() {
        let profile = CurvedProfile {
            frame: PlaneFrame::default(),
            outer: vec![CurvedEdge2d::Circle {
                center: [2., 1.],
                radius_mm: 1.5,
            }],
            holes: vec![],
        };
        let feature = EntityId::new();
        let path_origins = std::array::from_fn::<_, 3, _>(|_| Origin {
            source: EntityId::new(),
            role: "path.edge".into(),
        });
        let section = Origin {
            source: EntityId::new(),
            role: "curve".into(),
        };
        let sweep = |path| {
            Solid::rust_sweep_polyline_named(
                &profile,
                path,
                feature,
                std::slice::from_ref(&section),
                &path_origins,
                GeometryPolicy::default(),
            )
        };
        let path = [[0.; 3], [0., 0., 20.], [20., 0., 20.], [20., 20., 20.]];
        let solid = sweep(&path).unwrap();
        let expected_volume = std::f64::consts::PI * 1.5_f64.powi(2) * 54.;
        assert!((solid.volume_mm3().unwrap() - expected_volume).abs() < 1e-7);
        let topology = solid.topology_snapshot(feature, 5).unwrap();
        let anchor = topology
            .elements
            .iter()
            .filter(|element| element.key.kind == TopologyKind::Face)
            .flat_map(|element| &element.frames)
            .find(|anchor| anchor.origin.role == "cap.end")
            .unwrap();
        assert_eq!(anchor.frame.origin_mm, path[3]);
        assert_eq!(anchor.frame.u, [0., 0., -1.]);
        assert_eq!(anchor.frame.normal(), [0., 1., 0.]);

        let reversal = [[0.; 3], [0., 0., 20.], [1., 0., 20.], [1., 0., 0.]];
        let error = sweep(&reversal).err().unwrap();
        assert_eq!(error.kind, GeometryErrorKind::Unsupported);
        assert!(error.message.contains("reverses a sweep leg"));
    }
}

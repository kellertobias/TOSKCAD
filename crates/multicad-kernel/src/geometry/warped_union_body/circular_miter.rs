//! Exact circular sweep bands with one shared elliptical miter.
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
        [start, corner, end]: [Point; 3],
        feature: EntityId,
        [profile_origin, first_origin, last_origin]: [&Origin; 3],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.point([center[0], center[1], 0.])?;
        policy.point(end)?;
        policy.length(radius_mm)?;
        policy.length(norm(subtract(end, corner)))?;
        let outgoing = normalized(subtract(end, corner));
        let incoming = normalized(subtract(corner, start));
        let bisector = std::array::from_fn(|i| incoming[i] + outgoing[i]);
        let slope = dot(bisector, incoming);
        if slope <= 64. * f64::EPSILON {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Input,
                "Sweep path reverses at its corner",
            ));
        }
        // Intersect the incoming generators with the bisector, then project
        // those offsets onto the outgoing disk. This is the minimum rotation
        // of the authored section, including any offset from the path center.
        let on_miter = |point: Point| {
            let travel = -dot(bisector, point) / slope;
            std::array::from_fn(|i| incoming[i].mul_add(travel, point[i]))
        };
        let outgoing_offset = |point| in_plane(on_miter(point), outgoing);
        let sketch = in_plane(subtract(frame.origin, start), incoming);
        let start_center = in_plane(
            std::array::from_fn(|i| {
                frame.u[i].mul_add(center[0], frame.v[i].mul_add(center[1], sketch[i]))
            }),
            incoming,
        );
        let corner = subtract(corner, start);
        let end = subtract(end, start);
        let miter_offset = on_miter(start_center);
        let end_offset = outgoing_offset(start_center);
        let path = [
            start_center,
            std::array::from_fn(|i| corner[i] + miter_offset[i]),
            std::array::from_fn(|i| end[i] + end_offset[i]),
        ];
        let geometry = warped_union::circular_miter(path, radius_mm, policy.linear_tolerance_mm)
            .map_err(|cause| {
                let (kind, message) = match cause {
                    warped_union::Error::Invalid => (
                        GeometryErrorKind::InvalidResult,
                        "The circular miter has an invalid closed boundary",
                    ),
                    warped_union::Error::Unsupported => (
                        GeometryErrorKind::Unsupported,
                        "The circular section cannot clear the miter and both end caps",
                    ),
                    warped_union::Error::Limit => (
                        GeometryErrorKind::Limit,
                        "The circular miter exceeds the geometry range",
                    ),
                };
                error(OPERATION, kind, message)
            })?;
        let outgoing_sketch = outgoing_offset(sketch);
        let cap_frames = [
            (sketch, normalized(in_plane(frame.u, incoming))),
            (
                std::array::from_fn(|i| end[i] + outgoing_sketch[i]),
                normalized(outgoing_offset(in_plane(frame.u, incoming))),
            ),
        ];
        let mut origins = Vec::with_capacity(geometry.faces().len());
        let mut anchors = Vec::with_capacity(geometry.faces().len());
        for face in geometry.faces() {
            match face.support {
                Support::Plane { normal } => {
                    let cap = face.source;
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
                    let path = [first_origin, last_origin][face.source];
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
}

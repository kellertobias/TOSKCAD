//! Exact miter walls with the source frames of the authored section and path.
use super::*;
use crate::{CurvedEdge2d, CurvedProfile};
use multicad_rust_kernel::planar_boolean::miter_sweep;

const OPERATION: &str = "Rust miter sweep";

fn normalized(vector: Point) -> Point {
    let length = norm(vector);
    vector.map(|component| component / length)
}

impl Solid {
    pub(crate) fn rust_sweep_miter_named(
        profile: &CurvedProfile,
        path_mm: &[Point],
        feature: EntityId,
        profile_origins: &[Origin],
        path_origins: &[Origin],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let frame = profile.frame;
        frame.checked(policy)?;
        let start = path_mm[0];
        let first = subtract(path_mm[1], start);
        let length = norm(first);
        policy.length(length)?;
        let u = normalized(frame.u);
        let normal = normalized(frame.normal());
        let forward = normal.map(|axis| axis * dot(first, normal).signum());
        let v = cross(forward, u);
        let placement = PlaneFrame {
            origin: start,
            u,
            v,
        };
        let local = |p| [dot(p, u), dot(p, v), dot(p, forward)];
        let profile_origin = local(subtract(frame.origin, start));
        let tolerance = policy.linear_tolerance_mm;
        let roundoff = 64.
            * f64::EPSILON
            * start
                .into_iter()
                .chain(path_mm[1])
                .map(f64::abs)
                .fold(length.max(1.), f64::max);
        let [x, y, _] = local(first);
        if profile_origin[2].abs() > tolerance || x.hypot(y) > roundoff.min(tolerance) {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Input,
                "Sweep profile plane must contain the path start and be perpendicular to its first segment",
            ));
        }
        let mut path: Vec<_> = path_mm
            .iter()
            .map(|&point| {
                policy.point(point)?;
                Ok(local(subtract(point, start)))
            })
            .collect::<Result<_, GeometryError>>()?;
        // The pure constructor uses a path-local chart. These values are
        // exact in that chart once the plane relation has passed the check.
        path[1] = [0., 0., length];
        for segment in path.windows(2).skip(1) {
            policy.length(norm(subtract(segment[1], segment[0])))?;
        }

        let profile_u = local(frame.u);
        let profile_v = local(frame.v);
        let mut section = Vec::with_capacity(profile.outer.len());
        for (edge, next) in profile
            .outer
            .iter()
            .zip(profile.outer.iter().cycle().skip(1))
        {
            let (CurvedEdge2d::Line { a, b }, CurvedEdge2d::Line { a: next, .. }) = (edge, next)
            else {
                return Err(error(
                    OPERATION,
                    GeometryErrorKind::Unsupported,
                    "Rust miter sweep requires a straight-edged section",
                ));
            };
            for &p in [a, b] {
                policy.point([p[0], p[1], 0.])?;
                policy.point(frame.point(p))?;
            }
            policy.length((b[0] - a[0]).hypot(b[1] - a[1]))?;
            if (b[0] - next[0]).hypot(b[1] - next[1]) > tolerance {
                return Err(error(
                    OPERATION,
                    GeometryErrorKind::Input,
                    "Sweep section wire is disconnected",
                ));
            }
            section.push(std::array::from_fn(|i| {
                profile_u[i].mul_add(a[0], profile_v[i].mul_add(a[1], profile_origin[i]))
            }));
        }

        let (geometry, material) = miter_sweep(&section, &path, tolerance).map_err(|cause| {
            let (kind, message) = match cause {
                Error::Invalid => (
                    GeometryErrorKind::Unsupported,
                    "Rust miter sweep requires a simple polygon section and a path with separated, uncollapsed segments",
                ),
                Error::Unresolved => (
                    GeometryErrorKind::InvalidResult,
                    "Rust miter sweep could not certify the closed shell",
                ),
                Error::Limit => (GeometryErrorKind::Limit, "Sweep exceeds the planar geometry budget"),
            };
            error(OPERATION, kind, message)
        })?;
        let material = Material::profile_cells(
            &geometry,
            material
                .into_iter()
                .map(|cell| (cell.piece, cell.segment, cell.source_faces)),
        );
        let count = section.len();
        let initial = geometry.vertices();
        let first_edge = normalized(subtract(initial[1], initial[0]));
        let second_axis = cross([0., 0., 1.], first_edge);
        let sketch_offset = subtract([profile_origin[0], profile_origin[1], 0.], initial[0]);
        let cap_anchor = |a: Point, b: Point, normal: Point, end: bool| {
            // Recover the section's transported coordinate system from the
            // first authored edge of each exact cap and its travel direction.
            let along = normalized(subtract(b, a));
            let across = cross(normal.map(|n| if end { n } else { -n }), along);
            let rotate = |vector| -> Point {
                let x = dot(vector, first_edge);
                let y = dot(vector, second_axis);
                std::array::from_fn(|i| along[i] * x + across[i] * y)
            };
            let u = normalized(rotate([1., 0., 0.]));
            let offset = rotate(sketch_offset);
            ReferenceFrame {
                origin_mm: std::array::from_fn(|i| a[i] + offset[i]),
                u,
                v: cross(normal, u),
            }
        };
        PlanarBooleanBody::build_labeled(
            geometry,
            feature,
            policy,
            Some(material),
            placement,
            |_, face, vertices| {
                let segment = face.source;
                let normal = face.normal;
                let (origins, frame) = if face.source_face < 2 {
                    let end = face.source_face == 1;
                    let ring = (segment + usize::from(end)) * count;
                    let origin = Origin {
                        source: feature,
                        role: if end { "cap.end" } else { "cap.start" }.into(),
                    };
                    (
                        vec![origin],
                        cap_anchor(vertices[ring], vertices[ring + 1], normal, end),
                    )
                } else {
                    let edge = face.source_face - 2;
                    let ring = segment * count;
                    let a = vertices[ring + edge];
                    let b = vertices[ring + (edge + 1) % count];
                    let tangent = normalized(subtract(path[segment + 1], path[segment]));
                    let across = subtract(b, a);
                    let along_tangent = dot(across, tangent);
                    let u = normalized(std::array::from_fn(|i| {
                        across[i] - along_tangent * tangent[i]
                    }));
                    let origins = [&profile_origins[edge], &path_origins[segment]]
                        .into_iter()
                        .flat_map(|authored| {
                            [
                                Origin {
                                    source: feature,
                                    role: format!("side/{}/{}", authored.source, authored.role),
                                },
                                authored.clone(),
                            ]
                        })
                        .collect();
                    (
                        origins,
                        ReferenceFrame {
                            origin_mm: a,
                            u,
                            v: cross(normal, u),
                        },
                    )
                };
                let anchors = origins
                    .iter()
                    .cloned()
                    .map(|origin| AnchoredFrame { origin, frame })
                    .collect();
                (origins, anchors)
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::references::{Attachment, GeometryIntent, Resolution, resolve};

    #[test]
    fn reversed_placed_sweep_preserves_cap_and_wall_attachment_frames() {
        let feature = EntityId::new();
        let source = EntityId::new();
        let origins = |prefix, count| {
            (0..count)
                .map(|edge| Origin {
                    source,
                    role: format!("{prefix}/{edge}"),
                })
                .collect::<Vec<_>>()
        };
        let curves = origins("profile", 4);
        let path = origins("path", 2);
        let section = [[-2., -1.], [2., -1.], [2., 1.], [-2., 1.]];
        let profile = CurvedProfile {
            frame: PlaneFrame {
                origin: [11., 6., 23.],
                u: [0., 0., 1.],
                v: [0., 1., 0.],
            },
            outer: (0..4)
                .map(|i| CurvedEdge2d::Line {
                    a: section[i],
                    b: section[(i + 1) % 4],
                })
                .collect(),
            holes: vec![],
        };
        let policy = GeometryPolicy::default();
        let solid = Solid::rust_sweep_polyline_named(
            &profile,
            &[[11., 6., 23.], [31., 6., 23.], [31., 6., 43.]],
            feature,
            &curves,
            &path,
            policy,
        )
        .unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        assert!((solid.volume_mm3().unwrap() - 320.).abs() < 1e-7);
        let topology = solid.topology_snapshot(feature, 2).unwrap();
        topology.validate().unwrap();
        let attach = |origins, anchor| {
            let intent = GeometryIntent {
                id: EntityId::new(),
                kind: TopologyKind::Face,
                origins,
                boundary: None,
                attachment: Some(Attachment {
                    anchor,
                    offset_mm: [0.; 3],
                    angle_rad: 0.,
                }),
            };
            let Resolution::Resolved {
                key,
                frame: Some(frame),
                ..
            } = resolve(&intent, &topology, 2)
            else {
                panic!("miter face attachment must resolve uniquely");
            };
            (key, frame)
        };
        let cap = |role: &str| {
            let origin = Origin {
                source: feature,
                role: role.into(),
            };
            attach(BTreeSet::from([origin.clone()]), origin)
        };
        let (start, frame) = cap("cap.start");
        assert_eq!(frame.origin_mm, [11., 6., 23.]);
        assert_eq!(frame.normal(), [-1., 0., 0.]);
        let (end, frame) = cap("cap.end");
        assert_eq!(frame.origin_mm, [31., 6., 43.]);
        assert_eq!(frame.normal(), [0., 0., 1.]);
        let (wall, frame) = attach(
            BTreeSet::from([curves[0].clone(), path[1].clone()]),
            path[1].clone(),
        );
        assert_eq!(frame.origin_mm, [33., 5., 21.]);
        assert_eq!(frame.normal(), [0., -1., 0.]);
        let pieces = solid.planar_convex_parts(&topology, policy).unwrap();
        let mapped =
            |piece: usize, face: usize| pieces[piece].1[face].map(|i| topology.elements[i].key);
        assert_eq!(mapped(0, 0), Some(start));
        assert_eq!(mapped(0, 1), None);
        assert_eq!(mapped(1, 0), None);
        assert_eq!(mapped(1, 1), Some(end));
        assert_eq!(mapped(1, 2), Some(wall));
    }

    #[test]
    fn subtracting_a_notch_from_a_concave_sweep_retains_the_authored_recess_wall() {
        let policy = GeometryPolicy::default();
        let feature = EntityId::new();
        let section = [[0., 0.], [4., 0.], [4., 1.], [1., 1.], [1., 3.], [0., 3.]];
        let profile = CurvedProfile {
            frame: PlaneFrame::default(),
            outer: (0..section.len())
                .map(|i| CurvedEdge2d::Line {
                    a: section[i],
                    b: section[(i + 1) % section.len()],
                })
                .collect(),
            holes: Vec::new(),
        };
        let origin = |role: &str| Origin {
            source: EntityId::new(),
            role: role.into(),
        };
        let curves = section.map(|_| origin("profile"));
        let path = [origin("path"), origin("path")];
        let stock = Solid::rust_sweep_polyline_named(
            &profile,
            &[[0.; 3], [0., 0., 20.], [20., 0., 20.]],
            feature,
            &curves,
            &path,
            policy,
        )
        .unwrap();
        let tool = Solid::rust_box(
            EntityId::new().uuid_bits(),
            [1, 1, 2].map(|size| Rational::new(size, 1).unwrap()),
            PlaneFrame {
                origin: [0.5, 1.5, 5.],
                ..Default::default()
            },
            policy,
        )
        .unwrap();
        let cut_feature = EntityId::new();
        let result = stock
            .rust_subtract_planar_tools(&[&tool], cut_feature, policy)
            .unwrap()
            .unwrap();
        assert_eq!(result.solid_count().unwrap(), 1);
        assert!((result.volume_mm3().unwrap() - 221.).abs() < 1e-7);

        let topology = result.topology_snapshot(cut_feature, 1).unwrap();
        topology.validate().unwrap();
        let faces = result.faces().unwrap();
        let mut wall_area = 0.;
        for face in topology.elements.iter().filter(|face| {
            face.key.kind == TopologyKind::Face
                && face.origins.contains(&curves[3])
                && face.origins.contains(&path[0])
        }) {
            let physical = &faces[face.key.ordinal as usize];
            let (point, normal) = physical.plane.unwrap();
            assert!((point[0] - 1.).abs() < 1e-7);
            assert!(dot(normal, [1., 0., 0.]) > 1. - 1e-10);
            assert!(face.frames.iter().any(|frame| frame.origin == curves[3]));
            wall_area += physical.area_mm2;
        }
        assert!((wall_area - 36.).abs() < 1e-7);
    }
}

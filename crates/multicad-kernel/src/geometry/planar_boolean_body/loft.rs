//! Ruled lofts with authored correspondence and stable planar attachments.
use super::*;
use crate::{CurvedEdge2d, CurvedProfile};
use multicad_core::references::GeometryIntent;
use multicad_rust_kernel::planar_boolean::ruled_loft;

const OPERATION: &str = "Rust ruled polygon loft";
const ANGULAR_ROUNDOFF: f64 = 64. * f64::EPSILON;

fn unsupported(message: &str) -> GeometryError {
    error(OPERATION, GeometryErrorKind::Unsupported, message)
}

fn normalized(vector: Point) -> Point {
    let magnitude = norm(vector);
    vector.map(|value| value / magnitude)
}

fn side_origin(feature: EntityId, edge: &Origin) -> Origin {
    Origin {
        source: feature,
        role: format!("side/{}/{}", edge.source, edge.role),
    }
}

fn validate_origins(
    feature: EntityId,
    section_origins: &[Vec<Origin>],
    caps: &[Origin; 2],
) -> Result<(), GeometryError> {
    let mut authored = BTreeSet::new();
    let mut generated: BTreeSet<_> = caps.iter().cloned().collect();
    for (section, edges) in section_origins.iter().enumerate() {
        for edge in edges {
            if !authored.insert(edge.clone()) {
                return Err(input("Loft needs distinct authored curve origins"));
            }
            let mut origins = BTreeSet::from([edge.clone()]);
            if section + 1 < section_origins.len() {
                let side = side_origin(feature, edge);
                origins.insert(side.clone());
                generated.insert(side);
            }
            GeometryIntent {
                id: feature,
                kind: TopologyKind::Face,
                origins,
                boundary: None,
                attachment: None,
            }
            .validate()
            .map_err(|message| error(OPERATION, GeometryErrorKind::Input, message))?;
        }
    }
    if !authored.is_disjoint(&generated) {
        return Err(input(
            "Authored loft curve collides with a generated face identity",
        ));
    }
    Ok(())
}

impl Solid {
    /// Loft ordered matching polygon sections with planar ruled sides.
    pub fn rust_loft_ruled_polygons_named(
        profiles: &[CurvedProfile],
        feature: EntityId,
        section_origins: &[Vec<Origin>],
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        if !(2..=16).contains(&profiles.len()) || profiles.len() != section_origins.len() {
            return Err(input(
                "Loft needs 2 to 16 ordered sections and their authored origins",
            ));
        }
        let count = profiles[0].outer.len();
        if count < 3
            || profiles
                .iter()
                .zip(section_origins)
                .any(|(profile, origins)| {
                    !profile.holes.is_empty()
                        || profile.outer.len() != count
                        || origins.len() != count
                })
        {
            return Err(unsupported(
                "Rust ruled loft requires matching straight polygon sections without holes",
            ));
        }
        if count > 64 {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Limit,
                "Loft profile exceeds 64 edges",
            ));
        }
        let caps = ["cap.start", "cap.end"].map(|role| Origin {
            source: feature,
            role: role.into(),
        });
        validate_origins(feature, section_origins, &caps)?;

        let first = profiles[0].frame;
        let last = profiles[profiles.len() - 1].frame;
        first.checked(policy)?;
        last.checked(policy)?;
        let u = normalized(first.u);
        let forward = normalized(first.normal());
        let span = dot(subtract(last.origin, first.origin), forward);
        policy.length(span.abs())?;
        let normal = forward.map(|value| value * span.signum());
        // Flipping the normal also flips V. Section and edge indices stay
        // authored even when their winding in the local chart is clockwise.
        let v = cross(normal, u);
        let placement = PlaneFrame {
            origin: first.origin,
            u,
            v,
        };
        let project = |vector| [dot(u, vector), dot(v, vector), dot(normal, vector)];
        let mut sections = Vec::with_capacity(profiles.len());
        let mut frames = Vec::with_capacity(profiles.len());
        let mut heights = Vec::with_capacity(profiles.len());
        let tolerance = policy.linear_tolerance_mm;
        for profile in profiles {
            profile.frame.checked(policy)?;
            if norm(cross(normal, normalized(profile.frame.normal()))) > ANGULAR_ROUNDOFF {
                return Err(unsupported(
                    "Rust ruled loft requires parallel section planes",
                ));
            }
            let [x, y, height] = project(subtract(profile.frame.origin, first.origin));
            if let Some(previous) = heights.last() {
                policy.length(height - previous)?;
            }
            let [ux, uy, uz] = project(profile.frame.u);
            let [vx, vy, vz] = project(profile.frame.v);
            let mut ring = Vec::with_capacity(count);
            for (edge, next) in profile
                .outer
                .iter()
                .zip(profile.outer.iter().cycle().skip(1))
            {
                let (CurvedEdge2d::Line { a, b }, CurvedEdge2d::Line { a: next, .. }) =
                    (edge, next)
                else {
                    return Err(unsupported(
                        "Rust ruled loft requires straight section edges",
                    ));
                };
                for &point in [a, b] {
                    policy.point([point[0], point[1], 0.])?;
                    policy.point(profile.frame.point(point))?;
                }
                policy.length((b[0] - a[0]).hypot(b[1] - a[1]))?;
                policy.length((next[0] - a[0]).hypot(next[1] - a[1]))?;
                if (b[0] - next[0]).hypot(b[1] - next[1]) > tolerance {
                    return Err(input("Loft section wire is disconnected"));
                }
                if uz.mul_add(a[0], vz * a[1]).abs() > tolerance {
                    return Err(unsupported(
                        "Loft section does not fit its parallel support plane",
                    ));
                }
                ring.push([
                    ux.mul_add(a[0], vx.mul_add(a[1], x)),
                    uy.mul_add(a[0], vy.mul_add(a[1], y)),
                ]);
            }
            sections.push(ring);
            frames.push(ReferenceFrame {
                origin_mm: [x, y, height],
                u: normalized([ux, uy, 0.]),
                v: [0.; 3],
            });
            heights.push(height);
        }
        let (geometry, material) = ruled_loft(&sections, &heights, tolerance).map_err(|cause| {
            match cause {
                Error::Invalid => unsupported(
                    "Rust ruled loft requires consistently wound simple sections and planar noncrossing sides",
                ),
                Error::Unresolved => error(
                    OPERATION,
                    GeometryErrorKind::InvalidResult,
                    "Rust ruled loft could not certify the closed shell",
                ),
                Error::Limit => error(OPERATION, GeometryErrorKind::Limit, "Loft exceeds the planar geometry budget"),
            }
        })?;
        let material = Material::profile_cells(
            &geometry,
            material
                .into_iter()
                .map(|cell| (cell.piece, cell.segment, cell.source_faces)),
        );
        PlanarBooleanBody::build_labeled(
            geometry,
            feature,
            policy,
            Some(material),
            placement,
            |_, face, _| {
                let normal = face.normal;
                if face.source_face < 2 {
                    let cap = face.source_face;
                    let section = if cap == 0 { 0 } else { frames.len() - 1 };
                    let origin = caps[cap].clone();
                    let mut frame = frames[section];
                    frame.v = cross(normal, frame.u);
                    return (vec![origin.clone()], vec![AnchoredFrame { origin, frame }]);
                }
                let edge = face.source_face - 2;
                let lower = face.source;
                let upper = lower + 1;
                let side_frame = |section: usize| {
                    let [a, b] = [edge, (edge + 1) % count].map(|i| sections[section][i]);
                    let along = [b[0] - a[0], b[1] - a[1], 0.];
                    let normal_offset = dot(normal, along);
                    let u = normalized(std::array::from_fn(|i| {
                        along[i] - normal[i] * normal_offset
                    }));
                    ReferenceFrame {
                        origin_mm: [a[0], a[1], heights[section]],
                        u,
                        v: cross(normal, u),
                    }
                };
                let start = &section_origins[lower][edge];
                let end = &section_origins[upper][edge];
                // An interior section is both an end and a start. The
                // generated side identity follows the slab that starts there.
                let origins = vec![side_origin(feature, start), start.clone(), end.clone()];
                let first = side_frame(lower);
                let anchors = origins
                    .iter()
                    .cloned()
                    .zip([first, first, side_frame(upper)])
                    .map(|(origin, frame)| AnchoredFrame { origin, frame })
                    .collect();
                (origins, anchors)
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::references::{Attachment, Resolution, resolve};

    fn rectangle(x: f64, width: f64) -> CurvedProfile {
        let points = [[0., 0.], [width, 0.], [width, 3.], [0., 3.]];
        CurvedProfile {
            frame: PlaneFrame {
                origin: [x, -2., 17.],
                u: [0., 0., 1.],
                v: [0., 1., 0.],
            },
            outer: (0..4)
                .map(|i| CurvedEdge2d::Line {
                    a: points[i],
                    b: points[(i + 1) % 4],
                })
                .collect(),
            holes: vec![],
        }
    }

    #[test]
    fn descending_loft_keeps_authored_attachment_frames_and_material_face_mapping() {
        let feature = EntityId::new();
        let origins: Vec<Vec<_>> = (0..3)
            .map(|_| {
                let source = EntityId::new();
                (0..4)
                    .map(|edge| Origin {
                        source,
                        role: format!("edge.{edge}"),
                    })
                    .collect()
            })
            .collect();
        let profiles = [rectangle(13., 4.), rectangle(18., 6.), rectangle(23., 8.)];
        let policy = GeometryPolicy::default();
        let solid =
            Solid::rust_loft_ruled_polygons_named(&profiles, feature, &origins, policy).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        assert!((solid.volume_mm3().unwrap() - 180.).abs() < 1e-7);
        let topology = solid.topology_snapshot(feature, 7).unwrap();
        topology.validate().unwrap();
        let attach = |anchor: Origin| {
            let intent = GeometryIntent {
                id: EntityId::new(),
                kind: TopologyKind::Face,
                origins: BTreeSet::from([anchor.clone()]),
                boundary: None,
                attachment: Some(Attachment {
                    anchor,
                    offset_mm: [2., 0., 0.],
                    angle_rad: 0.,
                }),
            };
            let Resolution::Resolved {
                key,
                frame: Some(frame),
                ..
            } = resolve(&intent, &topology, 7)
            else {
                panic!("authored loft attachment must resolve uniquely");
            };
            (key, frame)
        };
        let (start, frame) = attach(Origin {
            source: feature,
            role: "cap.start".into(),
        });
        assert_eq!(frame.origin_mm, [13., -2., 19.]);
        assert_eq!(frame.normal(), [-1., 0., 0.]);
        let (end, frame) = attach(Origin {
            source: feature,
            role: "cap.end".into(),
        });
        assert_eq!(frame.origin_mm, [23., -2., 19.]);
        assert_eq!(frame.normal(), [1., 0., 0.]);
        let (side, frame) = attach(side_origin(feature, &origins[1][0]));
        assert_eq!(frame.origin_mm, [18., -2., 19.]);
        assert_eq!(frame.normal(), [0., -1., 0.]);
        let parts = solid.planar_convex_parts(&topology, policy).unwrap();
        assert_eq!(parts.len(), 2);
        let mapped = |piece: usize, face: usize| {
            parts[piece].1[face].map(|index| topology.elements[index].key)
        };
        assert_eq!(mapped(0, 0), Some(start));
        assert_eq!(mapped(0, 1), None);
        assert_eq!(mapped(1, 0), None);
        assert_eq!(mapped(1, 1), Some(end));
        assert_eq!(mapped(1, 2), Some(side));
    }
}

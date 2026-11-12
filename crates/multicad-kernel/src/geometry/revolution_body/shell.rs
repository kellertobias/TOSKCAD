//! Signed circular-cylinder walls with one or both planar end faces open.
use super::*;
use crate::TreatmentFace;
use multicad_core::references::{AnalyticGeometry, TopologyElement, TopologyKind};
use std::f64::consts::TAU;

fn shell_error(kind: GeometryErrorKind, message: impl Into<String>) -> GeometryError {
    error("shell", kind, message)
}

struct Surface {
    source: usize,
    role: &'static str,
    offset: bool,
    plane: Option<(f64, f64)>,
    opening: Option<Origin>,
}

impl Solid {
    pub(crate) fn shell_cylindrical(
        &self,
        openings: &[TreatmentFace],
        thickness: f64,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Option<Result<Self, GeometryError>> {
        let (cylinder, frame) = self.axial_cylinder_frame()?;
        Some(build(
            self,
            frame,
            [
                cylinder.circle().radius().as_f64(),
                cylinder.height().as_f64(),
            ],
            openings,
            thickness,
            feature,
            policy,
        ))
    }
}

fn build(
    source: &Solid,
    frame: PlaneFrame,
    [radius, height]: [f64; 2],
    openings: &[TreatmentFace],
    thickness: f64,
    feature: EntityId,
    policy: GeometryPolicy,
) -> Result<Solid, GeometryError> {
    if source.policy != policy {
        return Err(shell_error(
            GeometryErrorKind::Unsupported,
            "Rust Shell requires the source geometry policy",
        ));
    }
    let mut opened = [None, None];
    for opening in openings {
        let Some(slot) = opened.get_mut(opening.key.ordinal as usize) else {
            return Err(shell_error(
                GeometryErrorKind::Unsupported,
                "Rust cylindrical Shell requires one or both planar cap openings",
            ));
        };
        *slot = Some(opening);
    }
    if opened.iter().all(Option::is_none) {
        return Err(shell_error(
            GeometryErrorKind::Unsupported,
            "Rust cylindrical Shell requires at least one planar cap opening",
        ));
    }
    let shifted = [
        if opened[0].is_some() { 0. } else { thickness },
        if opened[1].is_some() {
            height
        } else {
            height - thickness
        },
    ];
    let inward = thickness > 0.;
    let (outer_radius, inner_radius, outer_span, inner_span) = if inward {
        (radius, radius - thickness, [0., height], shifted)
    } else {
        (radius - thickness, radius, shifted, [0., height])
    };
    if inner_radius < policy.minimum_feature_mm
        || inner_span[1] - inner_span[0] < policy.minimum_feature_mm
    {
        return Err(shell_error(
            GeometryErrorKind::InvalidResult,
            "Shell thickness crosses the cylinder or leaves no cavity; reduce thickness",
        ));
    }
    for length in [
        outer_radius,
        inner_radius,
        outer_radius - inner_radius,
        outer_span[1] - outer_span[0],
        inner_span[1] - inner_span[0],
    ] {
        policy.length(length)?;
    }
    let mut points = vec![[outer_radius, outer_span[0]], [outer_radius, outer_span[1]]];
    if opened[1].is_none() {
        points.extend([[0., outer_span[1]], [0., inner_span[1]]]);
    }
    points.extend([[inner_radius, inner_span[1]], [inner_radius, inner_span[0]]]);
    if opened[0].is_none() {
        points.extend([[0., inner_span[0]], [0., outer_span[0]]]);
    }
    let geometry = Revolution::new(vec![points.clone()], TAU).map_err(|cause| {
        shell_error(
            GeometryErrorKind::InvalidResult,
            format!("Invalid cylindrical Shell meridian: {cause:?}"),
        )
    })?;
    let snapshot = source.topology_snapshot(feature, 0)?;
    let faces = snapshot
        .elements
        .iter()
        .filter(|element| element.key.kind == TopologyKind::Face)
        .collect::<Vec<_>>();
    let surface = |[a, z]: MeridianPoint, [b, next_z]: MeridianPoint| {
        if a == b {
            let offset = if a == outer_radius { !inward } else { inward };
            return (a != 0.).then_some(Surface {
                source: 2,
                role: if offset {
                    "shell.wall.offset"
                } else {
                    "shell.wall.retained"
                },
                offset,
                plane: None,
                opening: None,
            });
        }
        debug_assert_eq!(z, next_z);
        let end = usize::from(z != outer_span[0] && z != inner_span[0]);
        let opening = opened[end].map(|opening| opening.origin.clone());
        let offset = opening.is_none()
            && if z == outer_span[end] {
                !inward
            } else {
                inward
            };
        Some(Surface {
            source: end,
            role: if opening.is_some() {
                ["shell.opening.start", "shell.opening.end"][end]
            } else if offset {
                ["shell.offset.start", "shell.offset.end"][end]
            } else {
                ["shell.retained.start", "shell.retained.end"][end]
            },
            offset,
            plane: Some((z, (a - b).signum())),
            opening,
        })
    };
    let segments = points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
        .map(|(a, b)| segment(surface(a, b), a, &faces, feature, frame, height, policy))
        .collect::<Result<Vec<_>, GeometryError>>()?;
    let body = RevolutionBody {
        rings: topology::ring_edges(&geometry),
        geometry,
        feature,
        sketch: PlaneFrame {
            origin: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 0., 1.],
        },
        segments: vec![segments],
        reverse_rims: source
            .cylinder_sketch()
            .is_some_and(|sketch| sketch.reverse),
        retained_rim_boundaries: vec![],
    };
    let bounds = body.bounds(frame);
    policy.point(bounds.min)?;
    policy.point(bounds.max)?;
    let result = Solid {
        body: SolidBody::Revolved(Rc::new(body), frame),
        policy,
        provenance: vec![],
        rust_origins: vec![],
        retained_placement: None,
    };
    result
        .topology_snapshot(feature, 0)?
        .validate()
        .map_err(|cause| shell_error(GeometryErrorKind::InvalidResult, cause))?;
    Ok(result)
}

fn segment(
    surface: Option<Surface>,
    authored_start: MeridianPoint,
    faces: &[&TopologyElement],
    feature: EntityId,
    frame: PlaneFrame,
    height: f64,
    policy: GeometryPolicy,
) -> Result<MeridianSegment, GeometryError> {
    let mut result = MeridianSegment {
        origin: Origin {
            source: feature,
            role: "shell.axis".into(),
        },
        direct_identity: false,
        inherited: vec![],
        inherited_frames: vec![],
        authored_start,
    };
    let Some(surface) = surface else {
        return Ok(result);
    };
    result.origin.role = surface.role.into();
    result.direct_identity = surface.plane.is_some();
    let source = faces[surface.source];
    for origin in &source.origins {
        let derived = if surface.offset {
            Origin {
                source: feature,
                role: format!("offset/{}/{}", origin.source, origin.role),
            }
        } else {
            origin.clone()
        };
        if let Some((z, outward)) = surface.plane {
            let Some(AnalyticGeometry::Plane { frame: support }) = source.geometry else {
                unreachable!("cylinder cap geometry");
            };
            let anchor = source
                .frames
                .iter()
                .find(|anchor| &anchor.origin == origin)
                .map_or(support, |anchor| anchor.frame);
            let shift = z - if surface.source == 0 { 0. } else { height };
            let mut anchor = ReferenceFrame {
                origin_mm: std::array::from_fn(|i| anchor.origin_mm[i] + frame.normal()[i] * shift),
                ..anchor
            };
            if dot(anchor.normal(), frame.normal()) * outward < 0. {
                anchor.v = anchor.v.map(|v| -v);
            }
            result.inherited_frames.push(AnchoredFrame {
                origin: derived.clone(),
                frame: local_attachment(frame, anchor, policy)?,
            });
        }
        result.inherited.push(derived);
    }
    if let Some(opening) = surface.opening {
        let Some(AnalyticGeometry::Plane { frame: support }) = source.geometry else {
            unreachable!("cylinder opening plane");
        };
        result.inherited_frames.push(AnchoredFrame {
            origin: opening.clone(),
            frame: local_attachment(frame, support, policy)?,
        });
        result.inherited.push(opening);
    }
    Ok(result)
}

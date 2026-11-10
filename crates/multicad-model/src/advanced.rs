//! Exact advanced geometry from canonical selection and bound source parameters.
use crate::{GeometryBackend, ModelError, error};
use multicad_core::{
    document::EntityId,
    model::{self, AdvancedDefinition, ResolvedAdvanced},
    parameters::EvaluatedParameters,
    references::{self, GeometryIntent, Origin, Resolution, TopologyKind, TopologySnapshot},
};
use multicad_kernel::{
    BooleanOperation, CurvedProfile, GeometryErrorKind, GeometryPolicy, PlaneFrame, RigidTransform,
    Solid, TreatmentFace,
};
use std::collections::BTreeMap;
pub(crate) type Section = (CurvedProfile, Vec<Origin>);
pub(crate) type Path = (Vec<[f64; 3]>, Vec<Origin>);
#[derive(Default)]
pub(crate) struct ProfileInputs {
    pub sections: Vec<Section>,
    pub path: Option<Path>,
}
pub(crate) struct Inputs<'a> {
    pub backend: GeometryBackend,
    pub revision: u64,
    pub parameters: &'a EvaluatedParameters,
    pub solids: &'a BTreeMap<EntityId, Solid>,
    pub topologies: &'a BTreeMap<EntityId, TopologySnapshot>,
    pub policy: GeometryPolicy,
    pub sections: Vec<Section>,
    pub path: Option<Path>,
}
fn frame(value: multicad_core::sketch::Frame) -> PlaneFrame {
    PlaneFrame {
        origin: value.origin,
        u: value.u,
        v: value.v,
    }
}
pub(crate) fn build(
    inputs: Inputs<'_>,
    feature: EntityId,
    source: &AdvancedDefinition,
    captures: &[GeometryIntent],
) -> Result<Solid, ModelError> {
    let resolved = source
        .parameters
        .resolve(inputs.parameters)
        .map_err(|e| error(Some(feature), e))?;
    if let ResolvedAdvanced::Sweep = resolved {
        let (profile, origins) = inputs
            .sections
            .first()
            .ok_or_else(|| error(Some(feature), "Sweep profile is unavailable"))?;
        let (points, path_origins) = inputs
            .path
            .as_ref()
            .ok_or_else(|| error(Some(feature), "Sweep path is unavailable"))?;
        let sweep = match inputs.backend {
            GeometryBackend::Rust => Solid::rust_sweep_straight_named,
            GeometryBackend::OpenCascade => Solid::sweep_polyline_named,
        };
        return sweep(
            profile,
            points,
            feature,
            origins,
            path_origins,
            inputs.policy,
        )
        .map_err(|e| error(Some(feature), e));
    }
    if let ResolvedAdvanced::Loft { ruled } = resolved {
        let profiles = inputs
            .sections
            .iter()
            .map(|s| s.0.clone())
            .collect::<Vec<_>>();
        let origins = inputs
            .sections
            .iter()
            .map(|s| s.1.clone())
            .collect::<Vec<_>>();
        let solid = match inputs.backend {
            GeometryBackend::Rust => {
                Solid::rust_loft_profiles_named(&profiles, feature, &origins, ruled, inputs.policy)
            }
            GeometryBackend::OpenCascade => {
                Solid::loft_profiles_named(&profiles, feature, &origins, ruled, inputs.policy)
            }
        };
        return solid.map_err(|e| error(Some(feature), e));
    }
    let input = source
        .input
        .as_ref()
        .ok_or_else(|| error(Some(feature), "Advanced Part source is unavailable"))?;
    let solid = inputs.solids.get(&input.producer).ok_or_else(|| {
        error(
            Some(feature),
            "Advanced Part source has no current native solid",
        )
    })?;
    let snapshot = inputs
        .topologies
        .get(&input.producer)
        .ok_or_else(|| error(Some(feature), "Advanced source topology is unavailable"))?;
    let faces = captures
        .iter()
        .map(|capture| {
            let key = match references::resolve(capture, snapshot, inputs.revision) {
                Resolution::Resolved { key, .. } if key.kind == TopologyKind::Face => key,
                diagnostic => {
                    return Err(error(
                        Some(feature),
                        format!("Selected face {} needs repair: {diagnostic:?}", capture.id),
                    ));
                }
            };
            Ok(TreatmentFace {
                key,
                origin: Origin {
                    source: feature,
                    role: format!("face/{}", capture.id),
                },
            })
        })
        .collect::<Result<Vec<_>, ModelError>>()?;
    match resolved {
        ResolvedAdvanced::Shell { thickness_mm } => {
            solid.shell_named(&faces, thickness_mm, feature, inputs.policy)
        }
        ResolvedAdvanced::Draft {
            angle_rad,
            neutral,
            pull_direction,
        } => solid.draft_faces_named(
            &faces,
            pull_direction,
            frame(neutral),
            angle_rad,
            feature,
            inputs.policy,
        ),
        ResolvedAdvanced::Mirror { plane } => {
            solid.mirrored_named(frame(plane), feature, inputs.policy)
        }
        ResolvedAdvanced::LinearPattern {
            count,
            step_mm,
            merge,
        } => {
            let transforms = (0..count)
                .map(|i| RigidTransform {
                    rotation: model::RigidPose::default().rotation,
                    translation_mm: step_mm.map(|v| v * i as f64),
                })
                .collect::<Vec<_>>();
            return pattern(solid, snapshot, &transforms, merge, feature, inputs.policy);
        }
        ResolvedAdvanced::CircularPattern {
            count,
            step_angle_rad,
            origin_mm,
            axis,
            merge,
        } => {
            let transforms = (0..count)
                .map(|i| {
                    let control = model::NumericTransform {
                        pivot: origin_mm.map(model::length),
                        axis,
                        angle: multicad_core::constraint_sketch::Dimension::Literal {
                            value: step_angle_rad * i as f64,
                            unit: "rad".into(),
                        },
                        ..Default::default()
                    };
                    let pose = control
                        .resolve(inputs.parameters)
                        .map_err(|e| error(Some(feature), e))?;
                    Ok(RigidTransform {
                        rotation: pose.rotation,
                        translation_mm: pose.translation_mm,
                    })
                })
                .collect::<Result<Vec<_>, ModelError>>()?;
            return pattern(solid, snapshot, &transforms, merge, feature, inputs.policy);
        }
        ResolvedAdvanced::Sweep | ResolvedAdvanced::Loft { .. } => {
            unreachable!("sketch-based features returned above")
        }
    }
    .map_err(|e| error(Some(feature), e))
}
fn pattern(
    solid: &Solid,
    snapshot: &TopologySnapshot,
    transforms: &[RigidTransform],
    merge: bool,
    feature: EntityId,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let count = solid.solid_count().map_err(|e| error(Some(feature), e))? as usize;
    let Some(max_solids) = count.checked_mul(transforms.len()).filter(|&n| n <= 1024) else {
        return Err(error(
            Some(feature),
            "Pattern exceeds the 1024-solid budget; reduce count or simplify the source",
        ));
    };
    if snapshot
        .elements
        .len()
        .checked_mul(transforms.len())
        .is_none_or(|n| n > 1_000_000)
    {
        return Err(error(
            Some(feature),
            "Pattern exceeds the native topology-work budget; reduce count or simplify the source",
        ));
    }
    let mut members = vec![];
    for (index, transform) in transforms.iter().enumerate() {
        members.push(
            solid
                .transformed(*transform)
                .and_then(|s| s.with_member_origin(feature, index as u32))
                .map_err(|e| error(Some(feature), e))?,
        );
    }
    if !merge {
        return Solid::compound_named(&members.iter().collect::<Vec<_>>(), feature, policy)
            .map_err(|e| error(Some(feature), e));
    }
    if solid.uses_rust_geometry()
        && let Some((first, rest)) = members.split_first()
        && !rest.is_empty()
    {
        match first.rust_union_orthogonal_tools(rest, policy) {
            Ok(result) => {
                return result
                    .ok_or_else(|| error(Some(feature), "Pattern merge removed every solid"));
            }
            Err(cause) if cause.kind == GeometryErrorKind::Unsupported => {}
            Err(cause) => return Err(error(Some(feature), cause)),
        }
        let solids: Vec<_> = members.iter().collect();
        let merged = Solid::union_planar_pieces(&solids, feature, policy)
            .ok_or_else(|| {
                error(
                    Some(feature),
                    "Rust pattern merge requires certified straight-sided material",
                )
            })?
            .map_err(|cause| error(Some(feature), cause))?;
        // A union cannot split an input solid. If the certified count did not
        // decrease, preserve the independent members and their STEP shells.
        if merged
            .solid_count()
            .map_err(|cause| error(Some(feature), cause))? as usize
            == max_solids
        {
            return Solid::compound_named(&solids, feature, policy)
                .map_err(|cause| error(Some(feature), cause));
        }
        return Ok(merged);
    }
    let mut members = members.into_iter();
    let mut result = members
        .next()
        .ok_or_else(|| error(Some(feature), "Pattern has no members"))?;
    for member in members {
        result = result
            .boolean(&member, BooleanOperation::Union, policy)
            .map_err(|e| error(Some(feature), e))?
            .ok_or_else(|| error(Some(feature), "Pattern merge removed every solid"))?;
    }
    Ok(result)
}

//! Thread solids and receiving cuts use the same versioned source for UI/MCP.
use crate::{ModelError, error};
use multicad_core::{
    document::EntityId,
    model::{ResolvedThread, ThreadDefinition, ThreadHand, ThreadMode, ThreadRepresentation},
    parameters::EvaluatedParameters,
    references::Origin,
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, HelicalThread, PlaneFrame,
    RigidTransform, Solid,
};
use std::collections::BTreeMap;

fn envelope(
    feature: EntityId,
    radius: f64,
    resolved: &ResolvedThread,
    role: &str,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let profile = CurvedProfile {
        frame: PlaneFrame {
            origin: resolved.frame.origin,
            u: resolved.frame.u,
            v: resolved.frame.v,
        },
        outer: vec![CurvedEdge2d::Circle {
            center: [0., 0.],
            radius_mm: radius,
        }],
        holes: vec![],
    };
    Solid::extrude_curved_profile_vector_tapered_named(
        &profile,
        resolved.frame.n.map(|n| n * resolved.length_mm),
        resolved.profile.radial_taper.atan(),
        feature,
        &[Origin {
            source: feature,
            role: role.into(),
        }],
        policy,
    )
    .map_err(|e| error(Some(feature), e))
}
fn boolean(
    feature: EntityId,
    a: &Solid,
    b: &Solid,
    operation: BooleanOperation,
    policy: GeometryPolicy,
) -> Result<Option<Solid>, ModelError> {
    a.boolean(b, operation, policy)
        .map_err(|e| error(Some(feature), e))
}
fn require_covered(
    feature: EntityId,
    required: &Solid,
    target: &Solid,
    policy: GeometryPolicy,
    message: &str,
) -> Result<(), ModelError> {
    if boolean(
        feature,
        required,
        target,
        BooleanOperation::Subtract,
        policy,
    )
    .map_err(|e| error(Some(feature), format!("{message}: {e}")))?
    .is_some()
    {
        return Err(error(Some(feature), message));
    }
    Ok(())
}
pub(crate) fn build(
    feature: EntityId,
    source: &ThreadDefinition,
    parameters: &EvaluatedParameters,
    solids: &BTreeMap<EntityId, Solid>,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let resolved = source
        .parameters
        .resolve(parameters, source.mode)
        .map_err(|e| error(Some(feature), e))?;
    let target = source
        .target
        .as_ref()
        .map(|input| {
            solids.get(&input.producer).ok_or_else(|| {
                error(
                    Some(feature),
                    "Thread receiving geometry is unavailable; repair its predecessor",
                )
            })
        })
        .transpose()?;
    build_resolved(feature, resolved, source.mode, target, policy)
}
/// Reuse the same native thread and receiving-wall checks inside a typed
/// mechanical Part, without inventing a second canonical thread definition.
pub(crate) fn build_resolved(
    feature: EntityId,
    resolved: ResolvedThread,
    mode: ThreadMode,
    target: Option<&Solid>,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let tool = if resolved.representation == ThreadRepresentation::Modeled {
        let profile = resolved.profile;
        let local = Solid::helical_thread_named(
            HelicalThread {
                major_radius_mm: resolved.major_radius_mm,
                pitch_mm: resolved.pitch_mm,
                length_mm: resolved.length_mm,
                depth_mm: profile.depth_mm,
                crest_width_mm: profile.crest_width_mm,
                lower_flank_deg: profile.lower_flank_deg,
                upper_flank_deg: profile.upper_flank_deg,
                radial_taper: profile.radial_taper,
                starts: resolved.starts,
                left_hand: resolved.hand == ThreadHand::Left,
            },
            feature,
            policy,
        )
        .map_err(|e| error(Some(feature), e))?;
        local
            .transformed(RigidTransform {
                rotation: std::array::from_fn(|i| {
                    [
                        resolved.frame.u[i],
                        resolved.frame.v[i],
                        resolved.frame.n[i],
                    ]
                }),
                translation_mm: resolved.frame.origin,
            })
            .map_err(|e| error(Some(feature), e))?
    } else {
        // Cosmetic internal threads show the unthreaded minor-diameter bore.
        // Manufacturing exporters must refuse this representation explicitly.
        let radius = resolved.major_radius_mm
            - if mode == ThreadMode::CutInternal {
                resolved.profile.depth_mm
            } else {
                0.
            };
        envelope(
            feature,
            radius,
            &resolved,
            "thread.cosmetic.envelope",
            policy,
        )?
    };
    if mode == ThreadMode::NewExternal {
        return Ok(tool);
    }
    let target =
        target.ok_or_else(|| error(Some(feature), "Receiving thread has no target predecessor"))?;
    if target.solid_count().map_err(|e| error(Some(feature), e))? != 1 {
        return Err(error(
            Some(feature),
            "Thread receiving target must be one connected solid",
        ));
    }
    let cutter = match mode {
        ThreadMode::CutExternal => {
            let nominal = envelope(
                feature,
                resolved.nominal_major_radius_mm,
                &resolved,
                "thread.nominal.envelope",
                policy,
            )?;
            require_covered(
                feature,
                &nominal,
                target,
                policy,
                "External receiving surface is incomplete or does not contain the full nominal cylinder/cone",
            )?;
            let mask = envelope(
                feature,
                resolved.nominal_major_radius_mm + resolved.pitch_mm * 0.25,
                &resolved,
                "thread.cut.limit",
                policy,
            )?;
            if let Some(local) =
                boolean(feature, target, &mask, BooleanOperation::Intersect, policy)?
                && boolean(
                    feature,
                    &local,
                    &nominal,
                    BooleanOperation::Subtract,
                    policy,
                )?
                .is_some()
            {
                return Err(error(
                    Some(feature),
                    "External receiving surface must match the nominal cylinder/cone over the complete thread length",
                ));
            }
            // The receiving band is proven to be the complete nominal solid.
            // Replace that band directly; building its thin complementary
            // groove tool can fail native adaptive volume integration.
            let remainder = boolean(
                feature,
                target,
                &nominal,
                BooleanOperation::Subtract,
                policy,
            )?;
            let result = match remainder {
                None => tool,
                Some(outside) => {
                    boolean(feature, &outside, &tool, BooleanOperation::Union, policy)?.ok_or_else(
                        || error(Some(feature), "External thread replacement is empty"),
                    )?
                }
            };
            if result.solid_count().map_err(|e| error(Some(feature), e))? != 1 {
                return Err(error(
                    Some(feature),
                    "External thread disconnected the receiving body",
                ));
            }
            return Ok(result);
        }
        ThreadMode::CutInternal => {
            let outer = envelope(
                feature,
                resolved.major_radius_mm + resolved.minimum_wall_mm,
                &resolved,
                "thread.wall.limit",
                policy,
            )?;
            let bore = envelope(
                feature,
                resolved.major_radius_mm - resolved.profile.depth_mm,
                &resolved,
                "thread.minor.envelope",
                policy,
            )?;
            let required = boolean(feature, &outer, &bore, BooleanOperation::Subtract, policy)?
                .ok_or_else(|| {
                    error(
                        Some(feature),
                        "Internal thread has no positive receiving wall",
                    )
                })?;
            require_covered(
                feature,
                &required,
                target,
                policy,
                "Internal receiving body has an oversized/incomplete pilot hole or insufficient minimum wall around the full thread length",
            )?;
            tool
        }
        ThreadMode::NewExternal => unreachable!(),
    };
    if resolved.representation == ThreadRepresentation::Modeled
        && boolean(
            feature,
            target,
            &cutter,
            BooleanOperation::Intersect,
            policy,
        )
        .map_err(|e| error(Some(feature), format!("Receiving material query: {e}")))?
        .is_none()
    {
        return Err(error(
            Some(feature),
            "Requested thread removes no receiving material",
        ));
    }
    let result = boolean(feature, target, &cutter, BooleanOperation::Subtract, policy)
        .map_err(|e| error(Some(feature), format!("Receiving thread cut: {e}")))?
        .ok_or_else(|| {
            error(
                Some(feature),
                "Thread cut removed the entire receiving body",
            )
        })?;
    if result.solid_count().map_err(|e| error(Some(feature), e))? != 1 {
        return Err(error(
            Some(feature),
            "Thread cut disconnected the receiving body",
        ));
    }
    Ok(result)
}

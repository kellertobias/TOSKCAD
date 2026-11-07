//! Authored custom connection hardware, built from actual helical solids.
use super::*;
use multicad_core::model::{ResolvedCustomConnectionHardware, ScrewLengthConvention};
fn stage(
    feature: EntityId,
    thread: multicad_core::model::ResolvedThread,
    index: u32,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    crate::threads::build_resolved(feature, thread, ThreadMode::NewExternal, None, policy)?
        .with_member_origin(feature, index)
        .map_err(|e| error(Some(feature), e))
}
fn join(
    feature: EntityId,
    a: &Solid,
    b: &Solid,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let shape = screw_boolean(feature, a, b, BooleanOperation::Union, policy)?;
    if shape.solid_count().map_err(|e| error(Some(feature), e))? != 1 {
        return Err(error(
            Some(feature),
            "Custom hardware components must form one connected actual solid",
        ));
    }
    Ok(shape)
}
pub(super) fn build(
    feature: EntityId,
    p: ResolvedCustomConnectionHardware,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let result = match p {
        ResolvedCustomConnectionHardware::WoodScrew {
            mut wood_thread,
            plain_length_mm,
            plain_radius_mm,
            head,
            drive,
            underhead_frame,
            ..
        } => {
            // This is the existing headed-thread builder using the actual authored
            // Custom profile. No MachineScrew source/provider or ISO wood alias exists.
            wood_thread.frame = Frame {
                origin: underhead_frame.origin_mm,
                u: underhead_frame.u,
                v: underhead_frame.v,
                n: underhead_frame.normal(),
            };
            let length = plain_length_mm + wood_thread.length_mm;
            super::build_resolved(
                feature,
                ResolvedMechanicalPart::MachineScrew {
                    underhead_length_mm: length,
                    authored_length_mm: length,
                    length_convention: ScrewLengthConvention::Underhead,
                    shank_radius_mm: plain_radius_mm,
                    head,
                    drive,
                    thread: wood_thread,
                },
                policy,
            )?
        }
        ResolvedCustomConnectionHardware::WoodInsert {
            outer_thread,
            inner_thread,
            ..
        } => {
            let wall = outer_thread.major_radius_mm
                - outer_thread.profile.depth_mm
                - inner_thread.major_radius_mm;
            let floor = outer_thread.length_mm - inner_thread.length_mm;
            if wall < policy.minimum_feature_mm || (floor > 0. && floor < policy.minimum_feature_mm)
            {
                return Err(error(
                    Some(feature),
                    "Custom insert radial wall or blind floor is below native minimum feature size",
                ));
            }
            let outer = stage(feature, *outer_thread, 0, policy)?;
            crate::threads::build_resolved(
                feature,
                *inner_thread,
                ThreadMode::CutInternal,
                Some(&outer),
                policy,
            )?
        }
        ResolvedCustomConnectionHardware::HangerBolt {
            wood_thread,
            metric_thread,
            plain_length_mm,
            plain_radius_mm,
            frame,
            ..
        } => {
            let wood = stage(feature, *wood_thread, 0, policy)?;
            let metal = stage(feature, *metric_thread, 1, policy)?;
            let mut result = if plain_length_mm > 0. {
                if plain_length_mm < policy.minimum_feature_mm
                    || plain_radius_mm < policy.minimum_feature_mm
                {
                    return Err(error(
                        Some(feature),
                        "Custom hanger plain section is below native minimum feature size",
                    ));
                }
                let plane = PlaneFrame {
                    origin: std::array::from_fn(|i| frame.origin[i] - plain_length_mm * frame.n[i]),
                    u: frame.u,
                    v: frame.v,
                };
                let profile = CurvedProfile {
                    frame: plane,
                    outer: vec![CurvedEdge2d::Circle {
                        center: [0., 0.],
                        radius_mm: plain_radius_mm,
                    }],
                    holes: vec![],
                };
                let plain = Solid::extrude_curved_profile_named(
                    &profile,
                    plain_length_mm,
                    feature,
                    &[Origin {
                        source: feature,
                        role: "custom.hanger/plain".into(),
                    }],
                    policy,
                )
                .map_err(|e| error(Some(feature), e))?;
                join(feature, &wood, &plain, policy)?
            } else {
                wood
            };
            result = join(feature, &result, &metal, policy)?;
            result
        }
    };
    if result.solid_count().map_err(|e| error(Some(feature), e))? != 1
        || result
            .topology_snapshot(feature, 0)
            .map_err(|e| error(Some(feature), e))?
            .elements
            .iter()
            .any(|e| {
                e.key.kind == multicad_core::references::TopologyKind::Face && e.origins.is_empty()
            })
    {
        return Err(error(
            Some(feature),
            "Custom connection hardware needs one valid solid with complete native face ancestry",
        ));
    }
    Ok(result)
}

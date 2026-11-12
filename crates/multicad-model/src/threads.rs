//! Thread solids and receiving cuts use the same versioned source for UI/MCP.
mod cosmetic;
mod end_treatments;
use crate::{ModelError, error};
use multicad_core::{
    document::EntityId,
    model::{
        ResolvedThread, ThreadDefinition, ThreadFamily, ThreadHand, ThreadMode,
        ThreadRepresentation,
    },
    parameters::EvaluatedParameters,
    references::Origin,
};
use multicad_kernel::{
    AxialFlatHelicalThread, BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy,
    HelicalThread, KnuckleHelicalThread, PlaneFrame, RigidTransform, RootRoundedHelicalThread,
    RoundedHelicalThread, Solid,
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
    let target = receiving_target(feature, source, solids)?;
    build_resolved(feature, resolved, source.mode, target, policy)
}

fn receiving_target<'a>(
    feature: EntityId,
    source: &ThreadDefinition,
    solids: &'a BTreeMap<EntityId, Solid>,
) -> Result<Option<&'a Solid>, ModelError> {
    source
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
        .transpose()
}

fn helical_spec(thread: &ResolvedThread) -> HelicalThread {
    let profile = thread.profile;
    HelicalThread {
        major_radius_mm: thread.major_radius_mm,
        pitch_mm: thread.pitch_mm,
        length_mm: thread.length_mm,
        depth_mm: profile.depth_mm,
        crest_width_mm: profile.crest_width_mm,
        lower_flank_deg: profile.lower_flank_deg,
        upper_flank_deg: profile.upper_flank_deg,
        radial_taper: profile.radial_taper,
        starts: thread.starts,
        left_hand: thread.hand == ThreadHand::Left,
    }
}

pub(crate) fn build_rust(
    feature: EntityId,
    source: &ThreadDefinition,
    parameters: &EvaluatedParameters,
    solids: &BTreeMap<EntityId, Solid>,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let fail = |message| error(Some(feature), message);
    let thread = source
        .parameters
        .resolve(parameters, source.mode)
        .map_err(|cause| error(Some(feature), cause))?;
    if thread.representation == ThreadRepresentation::CosmeticEnvelope
        && source.mode == ThreadMode::NewExternal
    {
        return cosmetic::shaft(feature, &thread, policy);
    }
    if thread.representation == ThreadRepresentation::Modeled
        && rounded_family(source.parameters.family)
    {
        if !matches!(
            source.mode,
            ThreadMode::NewExternal | ThreadMode::CutInternal
        ) {
            return Err(fail(
                "Rust double-rounded external receiving closures are not certified",
            ));
        }
        if thread.profile.radial_taper != 0. || thread.starts != 1 {
            return Err(fail(
                "Rust double-rounded threads require a straight single-start profile",
            ));
        }
        if thread
            .end_treatments
            .as_ref()
            .is_some_and(|e| e.start_chamfer_radial_depth_mm != 0. || e.end_relief_length_mm != 0.)
        {
            return Err(fail("Rounded thread end treatments are not yet certified"));
        }
        let (crest, root) = if let Some(round) = &thread.round {
            (round.cutter_crest_radius_mm, round.cutter_root_radius_mm)
        } else {
            let radius = thread
                .profile
                .rounding_radius_mm
                .ok_or_else(|| fail("Canonical rounded thread has no circular profile"))?;
            (radius, radius)
        };
        let spec = multicad_kernel::rust_kernel::rounded_thread::Spec {
            major_radius_mm: thread.major_radius_mm,
            pitch_mm: thread.pitch_mm,
            length_mm: thread.length_mm,
            depth_mm: thread.profile.depth_mm,
            crest_radius_mm: crest,
            root_radius_mm: root,
            flank_angle_deg: thread.profile.lower_flank_deg,
            starts: thread.starts,
            left_hand: thread.hand == ThreadHand::Left,
        };
        let frame = PlaneFrame {
            origin: thread.frame.origin,
            u: thread.frame.u,
            v: thread.frame.v,
        };
        return if source.mode == ThreadMode::CutInternal {
            let target = receiving_target(feature, source, solids)?
                .ok_or_else(|| fail("Receiving rounded thread has no target predecessor"))?;
            target.rust_cut_rounded_internal_thread(
                spec,
                thread.minimum_wall_mm,
                feature,
                frame,
                policy,
            )
        } else {
            Solid::rust_rounded_thread(spec, feature, frame, policy)
        }
        .map_err(|cause| error(Some(feature), cause));
    }
    validate_rust_family(feature, source.parameters.family)?;
    if thread.representation == ThreadRepresentation::CosmeticEnvelope
        && source.mode == ThreadMode::CutExternal
    {
        return Err(fail(
            "Rust external receiving cuts require a modeled thread; cosmetic receiving envelopes are not yet certified",
        ));
    }
    let terminal = thread
        .end_treatments
        .as_ref()
        .filter(|ends| ends.start_chamfer_radial_depth_mm != 0. || ends.end_relief_length_mm != 0.);
    if let Some(ends) = terminal {
        if source.mode != ThreadMode::NewExternal
            || source.parameters.family != ThreadFamily::IsoMetric
            || ends.start_chamfer_radial_depth_mm != 0.
        {
            return Err(fail(
                "Rust end treatments currently support only ISO metric NewExternal terminal cylindrical relief with a flat entry",
            ));
        }
        let (spec, root_radius, frame) = rust_profile_geometry(feature, &thread)?;
        return Solid::rust_helical_thread_end_relief(
            spec,
            root_radius,
            ends.end_relief_length_mm,
            ends.end_relief_radial_depth_mm,
            feature,
            frame,
            policy,
        )
        .map_err(|cause| error(Some(feature), cause));
    }
    let (spec, root_radius, frame) = rust_profile(feature, &thread)?;
    let solid = if source.mode == ThreadMode::CutInternal {
        let target = receiving_target(feature, source, solids)?
            .ok_or_else(|| fail("Receiving thread has no target predecessor"))?;
        target.rust_cut_internal_thread(spec, thread.minimum_wall_mm, feature, frame, policy)
    } else if source.mode == ThreadMode::CutExternal {
        let target = receiving_target(feature, source, solids)?
            .ok_or_else(|| fail("Receiving thread has no target predecessor"))?;
        target.rust_cut_external_thread(spec, root_radius, feature, frame, policy)
    } else {
        Solid::rust_helical_thread(spec, root_radius, feature, frame, policy)
    };
    solid.map_err(|cause| error(Some(feature), cause))
}

fn rounded_family(family: ThreadFamily) -> bool {
    matches!(
        family,
        ThreadFamily::Bsw | ThreadFamily::Bsf | ThreadFamily::Bspp | ThreadFamily::Round
    )
}
/// Thread-tool mode availability; dimensions and closures still require proof.
pub fn rust_thread_mode_supported(family: ThreadFamily, mode: ThreadMode) -> bool {
    rust_thread_family_supported(family)
        || (matches!(mode, ThreadMode::NewExternal | ThreadMode::CutInternal)
            && rounded_family(family))
}

pub(crate) fn validate_rust_family(
    feature: EntityId,
    family: ThreadFamily,
) -> Result<(), ModelError> {
    if !rust_thread_family_supported(family) {
        return Err(error(
            Some(feature),
            "Rust thread geometry supports ISO metric, sourced UNC/UNF/Trapezoidal/Acme/Buttress and straight custom profiles",
        ));
    }
    Ok(())
}

/// Families implemented by the owned kernel; individual source dimensions,
/// representations and receiving stock still undergo geometry validation.
pub fn rust_thread_family_supported(family: ThreadFamily) -> bool {
    matches!(
        family,
        ThreadFamily::IsoMetric
            | ThreadFamily::Unc
            | ThreadFamily::Unf
            | ThreadFamily::Trapezoidal
            | ThreadFamily::Acme
            | ThreadFamily::Buttress
            | ThreadFamily::Custom
    )
}

pub(crate) fn rust_profile(
    feature: EntityId,
    thread: &ResolvedThread,
) -> Result<(HelicalThread, Option<f64>, PlaneFrame), ModelError> {
    let fail = |message| error(Some(feature), message);
    if thread.representation != ThreadRepresentation::Modeled {
        return Err(fail(
            "Rust thread geometry currently requires the Modeled representation",
        ));
    }
    if thread.end_treatments.as_ref().is_some_and(|ends| {
        ends.start_chamfer_radial_depth_mm != 0. || ends.end_relief_length_mm != 0.
    }) {
        return Err(fail(
            "This Rust thread workflow does not support nontrivial end treatments",
        ));
    }
    rust_profile_geometry(feature, thread)
}
fn rust_profile_geometry(
    feature: EntityId,
    thread: &ResolvedThread,
) -> Result<(HelicalThread, Option<f64>, PlaneFrame), ModelError> {
    let fail = |message| error(Some(feature), message);
    if thread.representation != ThreadRepresentation::Modeled {
        return Err(fail(
            "Rust thread geometry currently requires the Modeled representation",
        ));
    }
    if thread.profile.radial_taper != 0. || thread.profile.rounding_radius_mm.is_some() {
        return Err(fail(
            "Rust thread geometry requires an untapered profile without crest rounding",
        ));
    }
    let phase = thread
        .buttress
        .as_ref()
        .map_or(0., |buttress| buttress.phase_rotation_rad);
    let (sin, cos) = phase.sin_cos();
    Ok((
        helical_spec(thread),
        thread
            .buttress
            .as_ref()
            .and_then(|buttress| buttress.root_circle.as_ref())
            .map(|circle| circle.radius_mm)
            .or_else(|| {
                thread
                    .metric_fit
                    .as_ref()
                    .and_then(|fit| fit.external_root_radius_mm)
            }),
        PlaneFrame {
            origin: thread.frame.origin,
            u: std::array::from_fn(|i| thread.frame.u[i] * cos + thread.frame.v[i] * sin),
            v: std::array::from_fn(|i| thread.frame.v[i] * cos - thread.frame.u[i] * sin),
        },
    ))
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
        let local = if let Some(root_radius_mm) = resolved
            .buttress
            .as_ref()
            .and_then(|b| b.root_circle.as_ref())
            .map(|c| c.radius_mm)
            .or_else(|| {
                resolved
                    .metric_fit
                    .as_ref()
                    .and_then(|m| m.external_root_radius_mm)
            }) {
            Solid::root_rounded_helical_thread_named(
                RootRoundedHelicalThread {
                    profile: HelicalThread {
                        radial_taper: 0.,
                        ..helical_spec(&resolved)
                    },
                    root_radius_mm,
                },
                feature,
                policy,
            )
        } else if let Some(npt) = &resolved.npt {
            Solid::axial_flat_helical_thread_named(
                AxialFlatHelicalThread {
                    profile: helical_spec(&resolved),
                    crest_radius_limit_mm: npt.stock_crest_radius_limit_mm,
                },
                feature,
                policy,
            )
        } else if let Some(round) = &resolved.round {
            Solid::knuckle_helical_thread_named(
                KnuckleHelicalThread {
                    major_radius_mm: resolved.major_radius_mm,
                    pitch_mm: resolved.pitch_mm,
                    length_mm: resolved.length_mm,
                    crest_radius_mm: round.cutter_crest_radius_mm,
                    root_radius_mm: round.cutter_root_radius_mm,
                    left_hand: resolved.hand == ThreadHand::Left,
                },
                feature,
                policy,
            )
        } else if let Some(rounding_radius_mm) = profile.rounding_radius_mm {
            Solid::rounded_helical_thread_named(
                RoundedHelicalThread {
                    major_radius_mm: resolved.major_radius_mm,
                    pitch_mm: resolved.pitch_mm,
                    length_mm: resolved.length_mm,
                    depth_mm: profile.depth_mm,
                    rounding_radius_mm,
                    flank_angle_deg: profile.lower_flank_deg,
                    radial_taper: profile.radial_taper,
                    starts: resolved.starts,
                    left_hand: resolved.hand == ThreadHand::Left,
                },
                feature,
                policy,
            )
        } else {
            Solid::helical_thread_named(helical_spec(&resolved), feature, policy)
        }
        .map_err(|e| error(Some(feature), e))?;
        let local = end_treatments::apply(feature, local, &resolved, mode, policy)?;
        let phase = resolved
            .buttress
            .as_ref()
            .map_or(0., |b| b.phase_rotation_rad);
        let (sin, cos) = phase.sin_cos();
        local
            .transformed(RigidTransform {
                rotation: std::array::from_fn(|i| {
                    [
                        resolved.frame.u[i] * cos + resolved.frame.v[i] * sin,
                        resolved.frame.v[i] * cos - resolved.frame.u[i] * sin,
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
        let cosmetic = envelope(
            feature,
            radius,
            &resolved,
            "thread.cosmetic.envelope",
            policy,
        )?;
        if let Some(limit) = resolved
            .npt
            .as_ref()
            .and_then(|npt| npt.stock_crest_radius_limit_mm)
        {
            let mut stock_context = resolved.clone();
            stock_context.profile.radial_taper = 0.;
            let stock = envelope(
                feature,
                limit,
                &stock_context,
                "thread.stock.envelope",
                policy,
            )?;
            boolean(
                feature,
                &cosmetic,
                &stock,
                BooleanOperation::Intersect,
                policy,
            )?
            .ok_or_else(|| error(Some(feature), "NPT cosmetic stock envelope is empty"))?
        } else {
            cosmetic
        }
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
            // NPT is formed from its sourced pipe-OD cylinder, including the
            // conical reduction before the imperfect-crest region.
            let mut receiving_context = resolved.clone();
            let nominal_radius = if let Some(limit) = resolved
                .npt
                .as_ref()
                .and_then(|npt| npt.stock_crest_radius_limit_mm)
            {
                receiving_context.profile.radial_taper = 0.;
                limit
            } else {
                resolved.nominal_major_radius_mm
            };
            let nominal = envelope(
                feature,
                nominal_radius,
                &receiving_context,
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
                nominal_radius + resolved.pitch_mm * 0.25,
                &receiving_context,
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
                resolved
                    .end_treatments
                    .as_ref()
                    .map_or(resolved.major_radius_mm, |t| t.maximum_cutter_radius_mm)
                    + resolved.minimum_wall_mm,
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

//! Positive-clearance certificates for unshifted external rotary pairs and
//! internal ring/spur pinion pairs.
use super::*;
use multicad_core::model::GearFamily;
use multicad_kernel::rust_kernel::spur_gear::clearance::ResultKind;

fn check(
    pair: EntityId,
    [a, b]: [&Solid; 2],
    drive: f64,
    family: GearFamily,
    policy: GeometryPolicy,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<(), ModelError> {
    let result = if matches!(family, GearFamily::Herringbone | GearFamily::Helical) {
        a.certify_rust_herringbone_pair(b, policy, interrupt)
    } else if family == GearFamily::InternalRing {
        a.certify_rust_internal_pair(b, policy, interrupt)
    } else {
        a.certify_rust_spur_pair(b, policy, interrupt)
    }
    .map_err(|cause| error(Some(pair), cause))?;
    let reason = match result {
        ResultKind::Clear => return Ok(()),
        ResultKind::CommonInterior => "overlaps with a certified common-interior point",
        ResultKind::Unresolved => "does not have certified positive clearance",
    };
    Err(error(
        Some(pair),
        format!(
            "Rust gear pair {reason} at tested drive {:.6} degrees; repair its phase or clearance",
            drive.to_degrees()
        ),
    ))
}

pub(crate) fn validate(
    inputs: ValidationInputs<'_>,
) -> Result<BTreeMap<EntityId, GearPairCheck>, ModelError> {
    let ValidationInputs {
        document,
        evaluated,
        solids,
        policy,
        interrupt,
        ..
    } = inputs;
    let mut reports = BTreeMap::new();
    for entity in document.entities() {
        if !matches!(&entity.data, EntityData::Extension { definition, .. } if definition.type_id == model::GEAR_PAIR_TYPE)
        {
            continue;
        }
        checkpoint(interrupt)?;
        let pair = entity.id;
        let source = model::read_gear_pair(document, pair).map_err(|e| error(Some(pair), e))?;
        let resolved = model::resolve_gear_pair_parameters(document, evaluated, &source.parameters)
            .map_err(|e| error(Some(pair), e))?;
        let family = resolved.gears[0].family();
        if family == GearFamily::Worm {
            reports.insert(
                pair,
                super::rust_worm::validate(&inputs, pair, &source, &resolved)?,
            );
            continue;
        }
        if family == GearFamily::Bevel {
            reports.insert(
                pair,
                super::rust_bevel::validate(&inputs, pair, &source, &resolved)?,
            );
            continue;
        }
        // A ring/pinion pair is certified with the ring as the first operand.
        let families = resolved.gears.each_ref().map(|gear| gear.family());
        let ring = families
            .iter()
            .position(|&member| member == GearFamily::InternalRing);
        let internal = ring.is_some();
        let order = if ring == Some(1) { [1, 0] } else { [0, 1] };
        let expected = |index: usize| {
            if internal && index == order[0] {
                GearFamily::InternalRing
            } else if internal {
                GearFamily::Spur
            } else {
                family
            }
        };
        if shifted_engagement(document, source.parameters.gears)
            || !(internal
                || matches!(
                    family,
                    GearFamily::Spur | GearFamily::Herringbone | GearFamily::Helical
                ))
            || resolved.gears.iter().enumerate().any(|(index, gear)| {
                !gear.rotary().is_some_and(|rotary| {
                    rotary.family == expected(index)
                        && rotary.profile_shift_coefficient == 0.
                        && (rotary.hub.is_none() || rotary.family != GearFamily::Spur)
                })
            })
        {
            return Err(error(
                Some(pair),
                "Rust gear-pair interference requires unshifted external spur, helical or continuous herringbone members, or an internal ring with a spur pinion; spur hubs require an additional certificate",
            ));
        }
        let family = if internal {
            GearFamily::InternalRing
        } else {
            family
        };
        let features = order.map(|index| source.parameters.gears[index]);
        let local = features.map(|feature| solids.get(&feature));
        let [Some(a), Some(b)] = local else {
            return Err(error(
                Some(pair),
                "Rust gear-pair interference requires both exact member solids",
            ));
        };
        let phases: Vec<_> = (0..SAMPLES)
            .map(|i| TAU / f64::from(resolved.gears[0].teeth()) * i as f64 / (SAMPLES - 1) as f64)
            .collect();
        let start = std::time::Instant::now();
        for &drive in &phases {
            checkpoint(interrupt)?;
            let sample = sample_document(document, pair, &source, drive)?;
            let poses = order.map(|index| {
                model::resolve_instance_pose(&sample, source.outputs.instances[index], evaluated)
                    .map_err(|e| error(Some(pair), e))
            });
            let [pa, pb] = poses;
            let placed_a = placed(a, pa?, pair)?;
            let placed_b = placed(b, pb?, pair)?;
            check(
                pair,
                [&placed_a, &placed_b],
                drive,
                family,
                policy,
                interrupt,
            )?;
        }
        let sampled_cycle_ms = start.elapsed().as_secs_f64() * 1000.;
        checkpoint(interrupt)?;
        let current = source
            .parameters
            .drive_angle
            .value_in(multicad_core::parameters::Unit::Radian, Some(evaluated))
            .map_err(|e| error(Some(pair), e))?;
        let start = std::time::Instant::now();
        check(
            pair,
            [
                inputs.current_member(pair, &source, order[0])?,
                inputs.current_member(pair, &source, order[1])?,
            ],
            current,
            family,
            policy,
            interrupt,
        )?;
        let current_pose_ms = start.elapsed().as_secs_f64() * 1000.;
        reports.insert(pair, GearPairCheck {
            pair,
            center_distance_mm: resolved.center_distance_mm,
            bevel: None,
            worm: None,
            current_engagement: None,
            cycle_drive_angles_rad: phases,
            cycle_reused: false,
            sampled_cycle_ms,
            current_pose_ms,
            current_drive_angle_rad: current,
            current_pose_checked: true,
            maximum_overlap_volume_mm3: 0.,
            scope: if family == GearFamily::Herringbone {
                "Rust full-axial positive-clearance certificates for the current pose and nine sampled phases of one unshifted continuous herringbone tooth cycle. Continuous motion, strength and manufacturing fit are outside this check."
            } else if family == GearFamily::Helical {
                "Rust full-axial positive-clearance certificates for the current pose and nine sampled phases of one unshifted helical tooth cycle. Continuous motion, strength and manufacturing fit are outside this check."
            } else if internal {
                "Rust positive-clearance certificates for the current pose and nine sampled phases over one tooth cycle of an unshifted internal ring and spur pinion, including nesting of the pinion inside the ring cavity. Continuous motion, strength and manufacturing fit are outside this check."
            } else {
                "Rust positive-clearance certificates for the current pose and nine sampled phases over one unshifted external spur tooth cycle. Continuous motion, strength and manufacturing fit are outside this check."
            },
        });
    }
    Ok(reports)
}

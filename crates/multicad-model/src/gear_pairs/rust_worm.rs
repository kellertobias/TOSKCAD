//! Crossed-axis positive-clearance certificates and flank witnesses for a
//! ZA worm and its generated wheel.
use super::*;
use multicad_kernel::rust_kernel::worm_gear::clearance::ResultKind;

fn check(
    pair: EntityId,
    [a, b]: [&Solid; 2],
    drive: f64,
    clearance_mm: f64,
    inputs: &ValidationInputs<'_>,
) -> Result<GearEngagementWitness, ModelError> {
    let reach = clearance_mm + inputs.policy.linear_tolerance_mm * 10.;
    let (result, witness) = a
        .certify_rust_worm_pair(b, inputs.policy, reach, inputs.interrupt)
        .map_err(|cause| error(Some(pair), cause))?;
    let at = drive.to_degrees();
    let reason = match result {
        ResultKind::Clear => {
            return witness
                .map(|w| GearEngagementWitness {
                    distance_mm: w.distance_mm,
                    kind: "rust_worm_flank_witness",
                    faces: None,
                    points_mm: w.points_mm,
                })
                .ok_or_else(|| {
                    error(
                        Some(pair),
                        format!(
                            "Worm members have no Rust engagement proximity within authored total thinning {clearance_mm} mm at tested drive {at:.6} degrees"
                        ),
                    )
                });
        }
        ResultKind::CommonInterior => "overlaps with a certified common-interior point",
        ResultKind::Unresolved => "does not have certified positive clearance",
    };
    Err(error(
        Some(pair),
        format!(
            "Rust worm pair {reason} at tested drive {at:.6} degrees; repair its phase, center distance or thinning"
        ),
    ))
}

pub(super) fn validate(
    inputs: &ValidationInputs<'_>,
    pair: EntityId,
    source: &GearPairDefinition,
    resolved: &model::ResolvedGearPair,
) -> Result<GearPairCheck, ModelError> {
    let document = inputs.document;
    let evaluated = inputs.evaluated;
    let members = source.parameters.gears.map(|gear| inputs.solids.get(&gear));
    let [Some(a), Some(b)] = members else {
        return Err(error(
            Some(pair),
            "Rust worm-pair interference requires both exact member solids",
        ));
    };
    let worm = resolved
        .worm
        .as_ref()
        .ok_or_else(|| error(Some(pair), "Worm pairs require a resolved worm and wheel"))?;
    let clearance = resolved
        .gears
        .iter()
        .map(|g| {
            g.worm()
                .map(|w| w.thinning_mm)
                .ok_or_else(|| error(Some(pair), "Worm pairs require two worm members"))
        })
        .sum::<Result<f64, _>>()?;
    let phases: Vec<_> = (0..SAMPLES)
        .map(|i| worm.worm_period_rad * i as f64 / (SAMPLES - 1) as f64)
        .collect();
    let start = std::time::Instant::now();
    for &drive in &phases {
        checkpoint(inputs.interrupt)?;
        let sample = sample_document(document, pair, source, drive)?;
        let [pa, pb] = source.outputs.instances.map(|instance| {
            model::resolve_instance_pose(&sample, instance, evaluated)
                .map_err(|e| error(Some(pair), e))
        });
        let placed_a = placed(a, pa?, pair)?;
        let placed_b = placed(b, pb?, pair)?;
        check(pair, [&placed_a, &placed_b], drive, clearance, inputs)?;
    }
    let sampled_cycle_ms = start.elapsed().as_secs_f64() * 1000.;
    checkpoint(inputs.interrupt)?;
    let current = source
        .parameters
        .drive_angle
        .value_in(multicad_core::parameters::Unit::Radian, Some(evaluated))
        .map_err(|e| error(Some(pair), e))?;
    let start = std::time::Instant::now();
    let witness = check(
        pair,
        [
            inputs.current_member(pair, source, 0)?,
            inputs.current_member(pair, source, 1)?,
        ],
        current,
        clearance,
        inputs,
    )?;
    Ok(GearPairCheck {
        pair,
        center_distance_mm: resolved.center_distance_mm,
        bevel: None,
        worm: resolved.worm.clone(),
        current_engagement: Some(witness),
        cycle_drive_angles_rad: phases,
        cycle_reused: false,
        sampled_cycle_ms,
        current_pose_ms: start.elapsed().as_secs_f64() * 1000.,
        current_drive_angle_rad: current,
        current_pose_checked: true,
        maximum_overlap_volume_mm3: 0.,
        scope: "Rust crossed-axis positive-clearance certificates for the current pose and nine sampled phases over one full ZA worm turn, each with an actual flank witness within the authored total thinning. Continuous motion, strength and manufacturing fit are outside this check.",
    })
}

use super::*;

pub(super) fn check(
    input: &connection_receivers::Inputs<'_>,
    source: &model::ScrewConnectionRequest,
    evaluated: &Model,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<ScrewConnectionCheck, ModelError> {
    let id = source.connection;
    let fail = |message: &str| error(Some(id), message);
    let unsupported = || {
        fail(
            "Rust blind tapped connections require one generated metric screw, no grip stack and one unseated blind tap",
        )
    };
    let ([receiver], [row]) = (source.receivers.as_slice(), source.hardware.as_slice()) else {
        return Err(unsupported());
    };
    let ScrewConnectionHardwareSource::Generated { part } = &row.source else {
        return Err(unsupported());
    };
    if row.role != Role::Screw
        || !evaluated
            .snapshot()
            .leaf_occurrences
            .values()
            .any(|leaf| leaf.path.root == row.instance && leaf.producer == part.feature)
    {
        return Err(unsupported());
    }
    let screw = hardware(input.document, input.parameters, evaluated, row, id)?;
    let ResolvedMechanicalPart::MachineScrew {
        head,
        shank_radius_mm,
        underhead_length_mm,
        thread: external,
        ..
    } = &screw.part
    else {
        return Err(unsupported());
    };
    if !matches!(
        head,
        ResolvedScrewHead::SocketCap { .. } | ResolvedScrewHead::ExternalHex { .. }
    ) {
        return Err(unsupported());
    }

    let physical = evaluated
        .snapshot()
        .leaf_occurrences
        .get(&receiver.source.occurrence)
        .filter(|leaf| leaf.producer == receiver.feature)
        .ok_or_else(|| fail("Blind tap must be the current physical receiving output"))?;
    let receiving = evaluated
        .leaf_solid(&physical.path)
        .ok_or_else(|| fail("Blind tap has no current physical solid"))?;
    let proof = screw
        .solid
        .rust_screw_blind_thread(receiving)
        .ok_or_else(unsupported)?;
    if !proof.entry_is_support_plane {
        return Err(fail(
            "Rust cannot prove the entire screw head clears the blind tap entry plane",
        ));
    }
    let grip = model::resolve_screw_connection_grip(
        input.document,
        input.parameters,
        source,
        input.revision,
        input.snapshots,
    )
    .map_err(|cause| error(Some(id), cause))?;
    let tolerance = input.policy.linear_tolerance_mm;
    if !grip.is_finite() || grip.abs() > tolerance {
        return Err(unsupported());
    }
    let minimum = mm(&source.parameters.minimum_engagement, input.parameters, id)?;
    let blind_clearance = mm(&source.parameters.blind_tip_clearance, input.parameters, id)?;
    if !blind_clearance.is_finite() || blind_clearance < 0. {
        return Err(fail("Blind tip clearance must be finite and nonnegative"));
    }
    let axis = source
        .axis
        .source
        .resolve(
            input.document,
            input.parameters,
            &source.axis.face,
            input.revision,
            input.snapshots,
        )
        .map_err(|cause| error(Some(id), cause))?
        .frame(ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        });
    let [(screw_thread, screw_frame), (tap_thread, tap_frame)] = proof.threads;
    let reference = |frame: PlaneFrame| ReferenceFrame {
        origin_mm: frame.origin,
        u: frame.u,
        v: frame.v,
    };
    let screw_frame = reference(screw_frame);
    let tap_frame = reference(tap_frame);
    at(
        axis,
        placed_frame(&screw.part, screw.pose),
        0.,
        tolerance,
        id,
    )?;
    at(axis, screw_frame, 0., tolerance, id)?;
    at(axis, tap_frame, 0., tolerance, id)?;
    let (receiver_station, _) = station(axis, tap_frame.origin_mm);
    let (screw_station, _) = station(axis, screw_frame.origin_mm);
    if (*underhead_length_mm - screw_thread.spec().length_mm).abs() > tolerance {
        return Err(fail(
            "The inserted screw must be threaded from its head bearing plane",
        ));
    }
    if screw_station + screw_thread.spec().length_mm
        > receiver_station + tap_thread.spec().length_mm - blind_clearance + tolerance
    {
        return Err(fail("Screw tip violates the declared blind-hole clearance"));
    }
    let shaft = shank_radius_mm.max(screw_thread.spec().major_radius_mm);
    let margin = input.policy.minimum_feature_mm.max(16. * tolerance);
    if head.minimum_radius_mm().min(proof.protected_radius_mm)
        - tap_thread.spec().major_radius_mm.max(shaft)
        < 5. * margin
    {
        return Err(fail(
            "The screw head has no complete overlapping flat bearing land on the blind tap stock",
        ));
    }
    let resolved = receiver
        .source
        .resolve_current(
            input.document,
            input.parameters,
            input.revision,
            input.snapshots,
        )
        .map_err(|cause| error(Some(id), cause))?;
    let internal = resolved.thread.ok_or_else(unsupported)?;
    let mating = model::resolve_metric_thread_mating(
        external,
        screw_frame,
        &internal,
        tap_frame,
        minimum,
        tolerance,
    )
    .map_err(|cause| error(Some(id), cause))?;
    no_phase_correction(&mating, id)?;
    check_radial_clearance(
        id,
        [(screw_thread, screw_frame), (tap_thread, tap_frame)],
        mating.phase.axial_station_mm,
        shaft,
        tolerance,
    )?;
    let bounds = screw
        .solid
        .bounds()
        .map_err(|cause| error(Some(id), cause))?;
    for leaf in evaluated.snapshot().leaf_occurrences.values() {
        checkpoint(interrupt)?;
        if leaf.path == receiver.source.occurrence || leaf.path.root == row.instance {
            continue;
        }
        if !(0..3).any(|i| {
            bounds.max[i] + tolerance < leaf.bounds.min[i]
                || leaf.bounds.max[i] + tolerance < bounds.min[i]
        }) {
            return Err(fail(
                "Rust cannot certify clearance to another nearby physical component",
            ));
        }
    }
    Ok(ScrewConnectionCheck {
        connection:id,
        mode:source.mode,
        hardware_instances:vec![row.instance],
        receiver_features:vec![receiver.feature],
        grip_mm:grip,
        minimum_engagement_mm:minimum,
        blind_tip_clearance_mm:blind_clearance,
        actual_engagement_mm:Some(mating.overlap_length_mm),
        actual_wood_engagement_mm:None,
        native_component_collision_checked:true,
        scope:"Exact placed metric profiles, blind-floor and tip clearance, flat seating and separation from other components".into(),
    })
}

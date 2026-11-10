//! Exact checks for generated metric hardware and one receiving bore.
use super::*;
use crate::connection_receivers::{self, rust_through};
use multicad_core::model::{ResolvedScrewHead, ScrewConnectionHardwareSource};
use multicad_kernel::rust_kernel::helical_thread::Thread;
mod blind;

pub(crate) fn validate(
    document: &Document,
    revision: u64,
    parameters: &EvaluatedParameters,
    evaluated: &Model,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Vec<ScrewConnectionCheck>, ModelError> {
    let mut connections = document.entities().filter(|entity| {
        matches!(&entity.data, EntityData::Extension { definition, .. }
            if definition.type_id == model::SCREW_CONNECTION_TYPE)
    });
    let Some(first) = connections.next() else {
        return Ok(vec![]);
    };
    let topologies = connection_topologies(evaluated);
    let input = connection_receivers::Inputs {
        document,
        revision,
        parameters,
        snapshots: &topologies,
        solids: &evaluated.solids,
        policy: evaluated.policy,
    };
    let mut checks = Vec::new();
    for entity in std::iter::once(first).chain(connections) {
        checkpoint(interrupt)?;
        if checks.len() >= 64 {
            return Err(error(
                Some(entity.id),
                "Rust screw connection validation exceeds its budget",
            ));
        }
        let source = model::read_screw_connection_request(document, entity.id)
            .map_err(|e| error(Some(entity.id), e))?;
        checks.push(if source.mode == Mode::TappedReceiver {
            blind::check(&input, &source, evaluated, interrupt)?
        } else {
            through_nut(&input, &source, evaluated, interrupt)?
        });
    }
    Ok(checks)
}

fn through_nut(
    input: &connection_receivers::Inputs<'_>,
    source: &model::ScrewConnectionRequest,
    evaluated: &Model,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<ScrewConnectionCheck, ModelError> {
    let id = source.connection;
    let fail = |message: &str| error(Some(id), message);
    let unsupported = || {
        fail(
            "Rust screw connection requires one unseated through bore and a generated screw and nut",
        )
    };
    let [receiver] = source.receivers.as_slice() else {
        return Err(unsupported());
    };
    if source.mode != Mode::ThroughNut || source.hardware.len() != 2 {
        return Err(unsupported());
    }
    let mut placed = Vec::with_capacity(2);
    for row in &source.hardware {
        let ScrewConnectionHardwareSource::Generated { part } = &row.source else {
            return Err(unsupported());
        };
        let matches = evaluated
            .snapshot()
            .leaf_occurrences
            .values()
            .any(|leaf| leaf.path.root == row.instance && leaf.producer == part.feature);
        if !matches {
            return Err(fail(
                "Generated hardware must retain its original physical geometry",
            ));
        }
        placed.push(hardware(
            input.document,
            input.parameters,
            evaluated,
            row,
            id,
        )?);
    }
    let screw = placed
        .iter()
        .find(|row| row.role == Role::Screw)
        .ok_or_else(unsupported)?;
    let nut = placed
        .iter()
        .find(|row| row.role == Role::Nut)
        .ok_or_else(unsupported)?;
    let ResolvedMechanicalPart::MachineScrew {
        head,
        shank_radius_mm,
        thread: external,
        ..
    } = &screw.part
    else {
        return Err(unsupported());
    };
    let ResolvedMechanicalPart::HexNut {
        across_flats_mm,
        thread: internal,
    } = &nut.part
    else {
        return Err(unsupported());
    };
    if !matches!(
        head,
        ResolvedScrewHead::SocketCap { .. } | ResolvedScrewHead::ExternalHex { .. }
    ) {
        return Err(unsupported());
    }
    let [(screw_thread, screw_frame), (nut_thread, nut_frame)] = screw
        .solid
        .rust_screw_nut_threads(nut.solid)
        .ok_or_else(unsupported)?;

    let physical = evaluated
        .snapshot()
        .leaf_occurrences
        .get(&receiver.source.occurrence)
        .ok_or_else(|| fail("Receiving target must be a current physical leaf occurrence"))?;
    if physical.producer != receiver.feature {
        return Err(fail(
            "The drilled receiving part must be the current physical output",
        ));
    }
    let grip = model::resolve_screw_connection_grip(
        input.document,
        input.parameters,
        source,
        input.revision,
        input.snapshots,
    )
    .map_err(|e| error(Some(id), e))?;
    let minimum = mm(&source.parameters.minimum_engagement, input.parameters, id)?;
    let blind_clearance = mm(&source.parameters.blind_tip_clearance, input.parameters, id)?;
    if !grip.is_finite() || grip <= 0. || !blind_clearance.is_finite() || blind_clearance < 0. {
        return Err(fail(
            "Through-nut grip must be positive and blind clearance nonnegative",
        ));
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
        .map_err(|e| error(Some(id), e))?
        .frame(ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        });
    let tolerance = input.policy.linear_tolerance_mm;
    let (_, bore) = rust_through::build(
        connection_receivers::Inputs {
            document: input.document,
            parameters: input.parameters,
            revision: input.revision,
            snapshots: input.snapshots,
            solids: input.solids,
            policy: input.policy,
        },
        receiver.feature,
        &receiver.source,
    )?;
    let physical_frame = |frame: PlaneFrame| ReferenceFrame {
        origin_mm: frame.origin,
        u: frame.u,
        v: frame.v,
    };
    let screw_frame = physical_frame(screw_frame);
    let nut_frame = physical_frame(nut_frame);
    let n = screw_frame.normal();
    let stock_tilt = cross(n, axis.normal());
    let stock_tilt = dot(stock_tilt, stock_tilt).sqrt();
    if stock_tilt > 1e-10 || dot(n, axis.normal()) < 0. {
        return Err(fail(
            "The actual screw axis must be parallel to the certified bore",
        ));
    }
    for (mouth, target) in bore
        .mouths_mm
        .map(|point| physical.pose.point(point))
        .into_iter()
        .zip([0., grip])
    {
        let (position, radial) = station(axis, mouth);
        if (position - target).abs() > tolerance || radial > tolerance {
            return Err(fail(
                "The certified through bore does not align with the grip and connection axis",
            ));
        }
    }
    let proof_margin = input.policy.minimum_feature_mm.max(16. * tolerance);
    let shaft = shank_radius_mm.max(screw_thread.spec().major_radius_mm);
    if shaft + proof_margin + stock_tilt * grip >= bore.radius_mm {
        return Err(fail(
            "The actual screw shaft has insufficient clearance in the through bore",
        ));
    }
    for (inner, outer) in [
        (shaft, head.minimum_radius_mm()),
        (nut_thread.spec().major_radius_mm, across_flats_mm / 2.),
    ] {
        if outer.min(bore.protected_radius_mm) - inner.max(bore.radius_mm) < 5. * proof_margin {
            return Err(fail(
                "The hardware has no complete overlapping flat bearing land on the receiving part",
            ));
        }
    }

    at(
        axis,
        placed_frame(&screw.part, screw.pose),
        0.,
        tolerance,
        id,
    )?;
    at(axis, nut_frame, grip, tolerance, id)?;
    let (thread_start, thread_offset) = station(axis, screw_frame.origin_mm);
    if thread_start > grip + tolerance || thread_offset > tolerance {
        return Err(fail(
            "The nut would intersect the plain shank before reaching the modeled thread",
        ));
    }
    let mating = model::resolve_metric_thread_mating(
        external,
        screw_frame,
        internal,
        nut_frame,
        minimum,
        tolerance,
    )
    .map_err(|e| error(Some(id), e))?;
    no_phase_correction(&mating, id)?;

    check_radial_clearance(
        id,
        [(screw_thread, screw_frame), (nut_thread, nut_frame)],
        mating.phase.axial_station_mm,
        shaft,
        tolerance,
    )?;

    for leaf in evaluated.snapshot().leaf_occurrences.values() {
        checkpoint(interrupt)?;
        if leaf.path == receiver.source.occurrence
            || placed
                .iter()
                .any(|hardware| hardware.instance == leaf.path.root)
        {
            continue;
        }
        for row in [screw, nut] {
            let bounds = row.solid.bounds().map_err(|e| error(Some(id), e))?;
            if !(0..3).any(|i| {
                bounds.max[i] + tolerance < leaf.bounds.min[i]
                    || leaf.bounds.max[i] + tolerance < bounds.min[i]
            }) {
                return Err(fail(
                    "Rust cannot certify clearance to another nearby physical component",
                ));
            }
        }
    }
    Ok(ScrewConnectionCheck {
        connection: id,
        mode: source.mode,
        hardware_instances: placed.iter().map(|row| row.instance).collect(),
        receiver_features: vec![receiver.feature],
        grip_mm: grip,
        minimum_engagement_mm: minimum,
        blind_tip_clearance_mm: blind_clearance,
        actual_engagement_mm: Some(mating.overlap_length_mm),
        actual_wood_engagement_mm: None,
        native_component_collision_checked: true,
        scope: "Exact placed metric profiles, through-bore clearance and flat seating with separation from other components".into(),
    })
}

fn check_radial_clearance(
    id: EntityId,
    [(screw_thread, screw_frame), (nut_thread, nut_frame)]: [(Thread, ReferenceFrame); 2],
    axial_station_mm: f64,
    shaft: f64,
    tolerance: f64,
) -> Result<(), ModelError> {
    let fail = |message: &str| error(Some(id), message);
    let n = screw_frame.normal();
    let q = nut_frame.normal();
    let tilt = cross(n, q);
    let tilt = dot(tilt, tilt).sqrt();
    if tilt > 1e-10 || dot(n, q) < 0. {
        return Err(fail(
            "This Rust connection requires the screw and receiving thread axes to have the same direction",
        ));
    }
    let alpha = dot(nut_frame.u, screw_frame.v).atan2(dot(nut_frame.u, screw_frame.u));
    let rate = screw_thread.helical_rate();
    let offset = rate * alpha - axial_station_mm;
    let clearance = screw_thread
        .minimum_radial_clearance(&nut_thread, offset, false)
        .map_err(|e| {
            error(
                Some(id),
                format!("Unsupported analytic metric profile clearance: {e:?}"),
            )
        })?;
    let extents = screw_thread.spec().length_mm + nut_thread.spec().length_mm + shaft;
    // Reserve clearance for axis drift and the corresponding angular and axial
    // shift in the bore profile. Evaluate the ideal profiles over a full period.
    let drift = 4. * tolerance + 4. * tilt * extents;
    let phase_drift = drift * (1. + 4. * rate.abs() / nut_thread.root_radius_mm());
    if clearance <= drift + nut_thread.maximum_profile_slope() * phase_drift {
        return Err(fail(
            "The placed modeled metric thread profiles interfere or have insufficient clearance",
        ));
    }

    Ok(())
}

fn cross([x, y, z]: [f64; 3], [u, v, w]: [f64; 3]) -> [f64; 3] {
    [y * w - z * v, z * u - x * w, x * v - y * u]
}

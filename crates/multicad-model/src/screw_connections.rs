//! Native connection validation over reconstructed canonical sources and physical leaves.
use crate::{Model, ModelError, compiler::checkpoint, error};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{
        self, ResolvedMechanicalPart, RigidPose, ScrewConnectionHardwareRole as Role,
        ScrewConnectionMode as Mode,
    },
    parameters::{EvaluatedParameters, Unit},
    references::{ReferenceFrame, TopologySnapshot},
};
use multicad_kernel::{BooleanOperation, PlaneFrame, Solid};
use std::collections::{BTreeMap, BTreeSet};

pub use multicad_core::model::ScrewConnectionCheck;

fn mm(
    value: &multicad_core::constraint_sketch::Dimension,
    p: &EvaluatedParameters,
    id: EntityId,
) -> Result<f64, ModelError> {
    value
        .value_in(Unit::Millimeter, Some(p))
        .map_err(|e| error(Some(id), e))
}
fn frame(frame: multicad_core::sketch::Frame) -> ReferenceFrame {
    ReferenceFrame {
        origin_mm: frame.origin,
        u: frame.u,
        v: frame.v,
    }
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn station(axis: ReferenceFrame, point: [f64; 3]) -> (f64, f64) {
    let d = std::array::from_fn(|i| point[i] - axis.origin_mm[i]);
    let axial = dot(d, axis.normal());
    let radial: [f64; 3] = std::array::from_fn(|i| d[i] - axial * axis.normal()[i]);
    (axial, dot(radial, radial).sqrt())
}
fn placed_frame(part: &ResolvedMechanicalPart, pose: RigidPose) -> ReferenceFrame {
    let local = match part {
        ResolvedMechanicalPart::MachineScrew { thread, .. }
        | ResolvedMechanicalPart::HexNut { thread, .. } => thread.frame,
        ResolvedMechanicalPart::Washer { frame, .. }
        | ResolvedMechanicalPart::BearingEnvelope { frame, .. } => *frame,
        ResolvedMechanicalPart::WoodScrew { parameters } => parameters.frame,
    };
    pose.frame(frame(local))
}
fn at(
    axis: ReferenceFrame,
    actual: ReferenceFrame,
    expected: f64,
    tolerance: f64,
    id: EntityId,
) -> Result<(), ModelError> {
    let (value, radial) = station(axis, actual.origin_mm);
    if !value.is_finite()
        || radial > tolerance
        || (value - expected).abs() > tolerance
        || dot(axis.normal(), actual.normal()) < 1. - 1e-9
    {
        return Err(error(
            Some(id),
            "Placed hardware disagrees with the connection axis or derived stack station",
        ));
    }
    Ok(())
}
fn collision(
    a: &Solid,
    b: &Solid,
    model: &Model,
    id: EntityId,
    queries: &mut usize,
) -> Result<(), ModelError> {
    let ab = a.bounds().map_err(|e| error(Some(id), e))?;
    let bb = b.bounds().map_err(|e| error(Some(id), e))?;
    if (0..3).any(|i| ab.max[i] < bb.min[i] || bb.max[i] < ab.min[i]) {
        return Ok(());
    }
    if *queries >= 4096 {
        return Err(error(
            Some(id),
            "Connection collision validation exceeds the 4096 native candidate-query budget",
        ));
    }
    *queries += 1;
    if a.boolean(b, BooleanOperation::Intersect, model.policy)
        .map_err(|e| error(Some(id), e))?
        .is_some()
    {
        return Err(error(
            Some(id),
            "Native screw connection has unintended component interference",
        ));
    }
    Ok(())
}
fn no_phase_correction(
    mating: &model::ResolvedMetricThreadMating,
    id: EntityId,
) -> Result<(), ModelError> {
    if mating.phase.roll_rad.abs() > 1e-9 {
        return Err(error(
            Some(id),
            "Placed metric threads have incompatible native helix phase; repair hardware roll instead of moving geometry during validation",
        ));
    }
    Ok(())
}
struct Hardware<'a> {
    role: Role,
    instance: EntityId,
    part: ResolvedMechanicalPart,
    pose: RigidPose,
    solid: &'a Solid,
}
fn hardware<'a>(
    document: &Document,
    p: &EvaluatedParameters,
    model: &'a Model,
    row: &model::ScrewConnectionHardwareRequest,
    id: EntityId,
) -> Result<Hardware<'a>, ModelError> {
    let part = model::resolve_screw_connection_hardware(document, p, row)
        .map_err(|e| error(Some(id), e))?;
    let leaves = model
        .snapshot()
        .leaf_occurrences
        .iter()
        .filter(|(path, _)| path.root == row.instance)
        .collect::<Vec<_>>();
    let [(path, leaf)] = leaves.as_slice() else {
        return Err(error(
            Some(id),
            "Each connection hardware role requires exactly one current physical leaf",
        ));
    };
    let solid = model.leaf_solid(path).ok_or_else(|| {
        error(
            Some(id),
            "Connection hardware leaf has no exact native solid",
        )
    })?;
    if solid.solid_count().map_err(|e| error(Some(id), e))? != 1 {
        return Err(error(
            Some(id),
            "Connection hardware must be one connected native solid",
        ));
    }
    Ok(Hardware {
        role: row.role,
        instance: row.instance,
        part,
        pose: leaf.pose,
        solid,
    })
}
fn require_contiguous(
    intervals: &mut [(f64, f64)],
    end: f64,
    tol: f64,
    id: EntityId,
) -> Result<(), ModelError> {
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut station = 0.;
    for &(a, b) in intervals.iter() {
        if !a.is_finite() || !b.is_finite() || b <= a || (a - station).abs() > tol {
            return Err(error(
                Some(id),
                "Receiving material does not form the declared contiguous grip stack",
            ));
        }
        station = b;
    }
    if (station - end).abs() > tol {
        return Err(error(
            Some(id),
            "Actual receiving stack thickness disagrees with the declared grip",
        ));
    }
    Ok(())
}
/// Runs once after physical occurrence expansion and before publishing a model.
/// No source pose or recipe is changed to make an invalid candidate pass.
pub(crate) fn validate(
    document: &Document,
    revision: u64,
    p: &EvaluatedParameters,
    model: &Model,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Vec<ScrewConnectionCheck>, ModelError> {
    if !document.entities().any(|entity| matches!(&entity.data, EntityData::Extension { definition, .. } if definition.type_id == model::SCREW_CONNECTION_TYPE)) {
        return Ok(vec![]);
    }
    let mut topologies: BTreeMap<EntityId, TopologySnapshot> = model
        .snapshot()
        .shapes
        .iter()
        .map(|(id, s)| (*id, s.topology.clone()))
        .collect();
    topologies.extend(
        model
            .snapshot()
            .occurrences
            .iter()
            .map(|(id, s)| (*id, s.topology.clone())),
    );
    let mut checks = vec![];
    let mut collision_queries = 0;
    for entity in document.entities() {
        if !matches!(&entity.data,EntityData::Extension{definition,..}if definition.type_id==model::SCREW_CONNECTION_TYPE)
        {
            continue;
        }
        checkpoint(interrupt)?;
        if checks.len() >= 64 {
            return Err(error(
                Some(entity.id),
                "Native screw connection validation exceeds the 64-connection budget",
            ));
        }
        let id = entity.id;
        let source =
            model::read_screw_connection_request(document, id).map_err(|e| error(Some(id), e))?;
        if source
            .receivers
            .iter()
            .any(|receiver| receiver.source.seat.is_some())
        {
            return Err(error(
                Some(id),
                "Connection head seating requires a verified bearing/contact and remaining-thread interval; recessed connection seats are not ready yet",
            ));
        }
        let grip = mm(&source.parameters.grip, p, id)?;
        let engagement = mm(&source.parameters.minimum_engagement, p, id)?;
        let clearance = mm(&source.parameters.blind_tip_clearance, p, id)?;
        if !grip.is_finite()
            || grip < 0.
            || !engagement.is_finite()
            || engagement <= 0.
            || !clearance.is_finite()
            || clearance < 0.
        {
            return Err(error(
                Some(id),
                "Connection grip, engagement and blind clearance must be finite nonnegative lengths with positive required engagement",
            ));
        }
        let axis_pose = source
            .axis
            .source
            .resolve(document, p, &source.axis.face, revision, &topologies)
            .map_err(|e| error(Some(id), e))?;
        let axis = axis_pose.frame(ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        });
        let tolerance = model.policy.linear_tolerance_mm;
        let hardware = source
            .hardware
            .iter()
            .map(|row| hardware(document, p, model, row, id))
            .collect::<Result<Vec<_>, _>>()?;
        if hardware.iter().any(|row| match source.mode {
            Mode::ThroughNut => !matches!(
                row.role,
                Role::Screw | Role::HeadWasher | Role::NutWasher | Role::Nut
            ),
            Mode::TappedReceiver => !matches!(row.role, Role::Screw | Role::HeadWasher),
            _ => false,
        }) {
            return Err(error(
                Some(id),
                "Hardware role is incompatible with this connection mode",
            ));
        }
        let mut head_thickness = 0.;
        let mut nut_thickness = 0.;
        for row in &hardware {
            if matches!(row.role, Role::HeadWasher | Role::NutWasher) {
                let ResolvedMechanicalPart::Washer { thickness_mm, .. } = row.part else {
                    return Err(error(
                        Some(id),
                        "Washer connection roles must resolve to actual washer geometry",
                    ));
                };
                if row.role == Role::HeadWasher {
                    head_thickness += thickness_mm;
                } else {
                    nut_thickness += thickness_mm;
                }
            }
        }
        let mut head_station = -head_thickness;
        let mut nut_station = grip;
        for row in &hardware {
            if let ResolvedMechanicalPart::Washer { thickness_mm, .. } = row.part {
                let expected = match row.role {
                    Role::HeadWasher => {
                        let v = head_station;
                        head_station += thickness_mm;
                        v
                    }
                    Role::NutWasher => {
                        let v = nut_station;
                        nut_station += thickness_mm;
                        v
                    }
                    _ => continue,
                };
                at(
                    axis,
                    placed_frame(&row.part, row.pose),
                    expected,
                    tolerance,
                    id,
                )?;
            }
        }
        let screws = hardware
            .iter()
            .filter(|row| row.role == Role::Screw)
            .collect::<Vec<_>>();
        let [screw] = screws.as_slice() else {
            return Err(error(
                Some(id),
                "A verified metric connection requires exactly one screw role",
            ));
        };
        let ResolvedMechanicalPart::MachineScrew {
            underhead_length_mm,
            thread: external,
            ..
        } = &screw.part
        else {
            return Err(error(
                Some(id),
                "This connection has no known modeled metric screw profile; manufacturer wood envelope thread/point engagement cannot be certified",
            ));
        };
        at(
            axis,
            placed_frame(&screw.part, screw.pose),
            -head_thickness,
            tolerance,
            id,
        )?;
        let tip = -head_thickness + underhead_length_mm;
        if tip <= grip {
            return Err(error(
                Some(id),
                "Screw length does not reach beyond the clamped grip stack",
            ));
        }
        let external_start = model::machine_screw_thread_start(&screw.part, screw.pose)
            .map_err(|e| error(Some(id), e))?;
        let mut intervals = vec![];
        let mut tapped = vec![];
        for receiver in &source.receivers {
            checkpoint(interrupt)?;
            let resolved = receiver.source.resolve(p).map_err(|e| error(Some(id), e))?;
            let predecessor = model.solid(receiver.source.input.producer).ok_or_else(|| {
                error(
                    Some(id),
                    "Receiving predecessor is missing from native evaluation",
                )
            })?;
            let local_pose = receiver
                .source
                .local_pose(document, p, revision, &topologies)
                .map_err(|e| error(Some(id), e))?;
            let local = local_pose.frame(ReferenceFrame {
                origin_mm: [0.; 3],
                u: [1., 0., 0.],
                v: [0., 1., 0.],
            });
            let depth = if let Some(depth) = resolved.depth_mm {
                depth
            } else {
                let reach = predecessor
                    .transfer_through_depth(
                        PlaneFrame {
                            origin: local.origin_mm,
                            u: local.u,
                            v: local.v,
                        },
                        false,
                        model.policy,
                    )
                    .map_err(|e| error(Some(id), e))?;
                reach
                    - model
                        .policy
                        .minimum_feature_mm
                        .max(8. * tolerance.max(predecessor.geometry_policy().linear_tolerance_mm))
            };
            if let Some(thread) = resolved.thread {
                let occurrence = model::resolve_occurrence_pose(
                    document,
                    &receiver.source.occurrence,
                    receiver.source.input.body,
                    p,
                    revision,
                    &topologies,
                )
                .map_err(|e| error(Some(id), e))?;
                let world_pose = local_pose
                    .then(occurrence)
                    .map_err(|e| error(Some(id), e))?;
                let internal_start = world_pose.frame(frame(thread.frame));
                tapped.push((resolved.station_mm, thread, internal_start));
            } else {
                intervals.push((resolved.station_mm, resolved.station_mm + depth));
            }
            if resolved.depth_mm.is_some()
                && tip > resolved.station_mm + depth - clearance + tolerance
            {
                return Err(error(
                    Some(id),
                    "Screw tip violates the declared blind-hole clearance",
                ));
            }
        }
        require_contiguous(&mut intervals, grip, tolerance, id)?;
        let actual_engagement = match source.mode {
            Mode::ThroughNut => {
                if grip <= 0. || !tapped.is_empty() {
                    return Err(error(
                        Some(id),
                        "Through-nut mode requires a positive clearance grip without a tapped receiver",
                    ));
                }
                let nuts = hardware
                    .iter()
                    .filter(|row| row.role == Role::Nut)
                    .collect::<Vec<_>>();
                let [nut] = nuts.as_slice() else {
                    return Err(error(Some(id), "Through-nut mode requires exactly one nut"));
                };
                let ResolvedMechanicalPart::HexNut {
                    thread: internal, ..
                } = &nut.part
                else {
                    return Err(error(
                        Some(id),
                        "The nut role must resolve to a known internal metric nut profile",
                    ));
                };
                let internal_start = placed_frame(&nut.part, nut.pose);
                at(axis, internal_start, grip + nut_thickness, tolerance, id)?;
                let mating = model::resolve_metric_thread_mating(
                    external,
                    external_start,
                    internal,
                    internal_start,
                    engagement,
                    tolerance,
                )
                .map_err(|e| error(Some(id), e))?;
                no_phase_correction(&mating, id)?;
                Some(mating.overlap_length_mm)
            }
            Mode::TappedReceiver => {
                let [(start, internal, internal_start)] = tapped.as_slice() else {
                    return Err(error(
                        Some(id),
                        "Tapped mode requires exactly one modeled receiving thread",
                    ));
                };
                if (*start - grip).abs() > tolerance
                    || hardware
                        .iter()
                        .any(|r| matches!(r.role, Role::Nut | Role::NutWasher | Role::Insert))
                {
                    return Err(error(
                        Some(id),
                        "Tapped entry must follow the clamped grip and cannot carry nut-side hardware",
                    ));
                }
                let mating = model::resolve_metric_thread_mating(
                    external,
                    external_start,
                    internal,
                    *internal_start,
                    engagement,
                    tolerance,
                )
                .map_err(|e| error(Some(id), e))?;
                no_phase_correction(&mating, id)?;
                Some(mating.overlap_length_mm)
            }
            Mode::WoodInsert => {
                return Err(error(
                    Some(id),
                    "Purchased insert usable internal thread interval is unknown; supply an explicit independently attributed insert design before claiming engagement",
                ));
            }
            Mode::DirectWood | Mode::HangerBolt => {
                return Err(error(
                    Some(id),
                    "Wood or hanger engagement requires its distinct known usable profile; a nominal stock envelope is not a modeled metric mating thread",
                ));
            }
        };
        let owned = hardware.iter().map(|r| r.instance).collect::<BTreeSet<_>>();
        for (i, a) in hardware.iter().enumerate() {
            for b in &hardware[i + 1..] {
                checkpoint(interrupt)?;
                collision(a.solid, b.solid, model, id, &mut collision_queries)?;
            }
            for path in model.snapshot().leaf_occurrences.keys() {
                if owned.contains(&path.root) {
                    continue;
                }
                checkpoint(interrupt)?;
                let other = model.leaf_solid(path).ok_or_else(|| {
                    error(Some(id), "Physical component has no current native leaf")
                })?;
                collision(a.solid, other, model, id, &mut collision_queries)?;
            }
        }
        checks.push(ScrewConnectionCheck{connection:id,mode:source.mode,hardware_instances:hardware.iter().map(|r|r.instance).collect(),receiver_features:source.receivers.iter().map(|r|r.feature).collect(),grip_mm:grip,minimum_engagement_mm:engagement,blind_tip_clearance_mm:clearance,actual_engagement_mm:actual_engagement,native_component_collision_checked:true,scope:"Current placed complete-profile metric engagement, phase, ordered hardware, receiving stack and strict native component non-interference; no strength or installation certification".into()});
    }
    Ok(checks)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_stack_must_be_contiguous_and_match_authored_grip() {
        let id = EntityId::new();
        assert!(require_contiguous(&mut [(2., 5.), (0., 2.)], 5., 1e-7, id).is_ok());
        for (mut spans, end) in [
            (vec![(0., 2.), (2.1, 5.)], 5.),
            (vec![(0., 3.), (2., 5.)], 5.),
            (vec![(0., 4.)], 5.),
            (vec![(0., f64::NAN)], 5.),
        ] {
            assert!(require_contiguous(&mut spans, end, 1e-7, id).is_err());
        }
        assert!(require_contiguous(&mut [], 0., 1e-7, id).is_ok());
    }
}

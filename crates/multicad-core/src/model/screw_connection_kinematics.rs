//! Source-derived connection kinematics. Component geometry is never a pose
//! supplier: actual typed parameter/frame sources and the captured axis are.
use super::{
    self as model, MechanicalPartDefinition, ResolvedMechanicalPart, RigidPose,
    ScrewConnectionHardwareRole as Role, ScrewConnectionHardwareSource as HardwareSource,
    ScrewConnectionMode as Mode, ScrewConnectionRequest, ScrewHardwareConnectionBinding,
    ScrewHardwarePlacementDefinition, TransferPoseProjection, angle, length, source::invalid,
};
use crate::{
    document::{Document, DocumentError, Entity, EntityData, EntityId},
    parameters::EvaluatedParameters,
    project_file::ContentHash,
    references::TopologySnapshot,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::TAU,
};
const MAX_SOURCES: usize = 8192;
thread_local! {
    // Computation-local recursion protection, not a source cache. Entries are
    // removed on every scope exit, including errors and unwinding.
    static ACTIVE: std::cell::RefCell<BTreeSet<(EntityId, EntityId, u8)>> = const { std::cell::RefCell::new(BTreeSet::new()) };
}
struct Guard((EntityId, EntityId, u8));
impl Guard {
    fn enter(document: &Document, driver: EntityId, purpose: u8) -> Result<Self, DocumentError> {
        let key = (document.id(), driver, purpose);
        ACTIVE.with(|active| {
            let mut active=active.borrow_mut();
            if active.len()>=128 || !active.insert(key) {
                return Err(invalid("Associative connection pose dependencies contain a cycle or exceed 128 nested evaluations"));
            }
            Ok(Self(key))
        })
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.with(|active| {
            active.borrow_mut().remove(&self.0);
        });
    }
}

fn request_for_driver(
    document: &Document,
    driver: EntityId,
    source: &ScrewHardwarePlacementDefinition,
) -> Result<Option<ScrewConnectionRequest>, DocumentError> {
    let Some(binding) = &source.connection else {
        return Ok(None);
    };
    // Staging may introduce the typed backlink before the owning record. It is
    // retained in the fingerprint; a runtime pose cannot use that incomplete state.
    if document.get(binding.connection).is_none() {
        return Ok(None);
    }
    let request = model::read_screw_connection_request(document, binding.connection)?;
    let row = request
        .hardware
        .iter()
        .find(|r| r.role_id == binding.role)
        .ok_or_else(|| invalid("Associative hardware role is absent from its owning connection"))?;
    if row.driver != driver || row.instance != source.owner || request.axis.id != source.axis {
        return Err(invalid(
            "Associative hardware driver, role, owner and connection axis disagree",
        ));
    }
    if let Some(entity) = document.get(source.owner) {
        let expected = match &row.source {
            HardwareSource::Generated { part } => part.body,
            HardwareSource::Library { placement, .. } => placement.wrapper,
        };
        if !matches!(&entity.data,EntityData::Instance{component,placement_source:Some(actual),..}if *actual==driver&&*component==expected)
        {
            return Err(invalid(
                "Associative hardware owner must use this exact component and retained driver",
            ));
        }
    }

    Ok(Some(request))
}
fn capture<'a>(
    sources: &mut BTreeMap<EntityId, Option<&'a Entity>>,
    document: &'a Document,
    id: EntityId,
) -> Result<(), DocumentError> {
    sources.insert(id, document.get(id));
    if sources.len() > MAX_SOURCES {
        return Err(invalid(
            "Connection pose source closure exceeds its 8192-source budget",
        ));
    }
    Ok(())
}
fn parameter_suppliers(
    document: &Document,
    source: &MechanicalPartDefinition,
    deps: &mut BTreeSet<EntityId>,
) -> Result<(), DocumentError> {
    // The actual provider reader has already compared every authored link with
    // this typed input set. No unknown extension or arbitrary link is projected.
    for link in source.inputs()? {
        if link.geometry.is_some()
            || link.role != format!("parameter/{}", link.target)
            || document
                .get(link.target)
                .is_some_and(|e| !matches!(e.data, EntityData::Parameter { .. }))
        {
            return Err(invalid(
                "Connection pose suppliers must be typed mechanical-part Parameters",
            ));
        }
        deps.insert(link.target);
    }
    Ok(())
}
fn projection(
    document: &Document,
    driver: EntityId,
    source: &ScrewHardwarePlacementDefinition,
) -> Result<TransferPoseProjection, DocumentError> {
    let binding = source
        .connection
        .as_ref()
        .ok_or_else(|| invalid("Choose an associative connection driver"))?;
    let mut dependencies = BTreeSet::from([source.axis]);
    let mut sources = BTreeMap::new();
    capture(&mut sources, document, driver)?;
    capture(&mut sources, document, binding.connection)?;
    let request = request_for_driver(document, driver, source)?;
    let mut pose_fingerprints = vec![];
    if let Some(request) = &request {
        for dimension in [
            &request.parameters.grip,
            &request.parameters.minimum_engagement,
            &request.parameters.blind_tip_clearance,
        ] {
            for link in super::common::parameter_links(vec![dimension]) {
                if document
                    .get(link.target)
                    .is_some_and(|e| !matches!(e.data, EntityData::Parameter { .. }))
                {
                    return Err(invalid("Connection controls must read actual Parameters"));
                }
                dependencies.insert(link.target);
            }
        }
        if let Some(end) = &request.parameters.end_reference {
            dependencies.insert(end.receiver.producer);
            let pose = model::occurrence_pose_projection(document, &end.occurrence, None)?;
            dependencies.extend(pose.geometry_dependencies);
            pose_fingerprints.push((end.face.intent.id, pose.fingerprint));
        }
        for receiver in &request.receivers {
            if let Some(face) = &receiver.source.entry_reference {
                dependencies.insert(receiver.source.input.producer);
                let pose =
                    model::occurrence_pose_projection(document, &receiver.source.occurrence, None)?;
                dependencies.extend(pose.geometry_dependencies);
                pose_fingerprints.push((face.intent.id, pose.fingerprint));
            }
        }
        for row in &request.hardware {
            let (feature, body) = match &row.source {
                HardwareSource::Generated { part } => (part.feature, part.body),
                HardwareSource::Library {
                    definition,
                    placement,
                } => {
                    let active = crate::symbols::read_instance(document, placement.record)?;
                    if active.instance != row.instance
                        || active.definition != placement.definition
                        || active.pin != definition.pin()?
                        || active.ids != placement.ids
                    {
                        return Err(invalid(
                            "Connection Library pose supplier lost its exact active pin or private map",
                        ));
                    }
                    for id in [
                        placement.record,
                        placement.definition,
                        placement.wrapper,
                        placement.wrapper_metadata,
                        placement.member,
                    ] {
                        capture(&mut sources, document, id)?;
                    }
                    for id in placement.ids.values() {
                        capture(&mut sources, document, *id)?;
                    }
                    let body = *placement
                        .ids
                        .get(&definition.root)
                        .ok_or_else(|| invalid("Library hardware lost its mapped Part"))?;
                    let Some(EntityData::Part { features, .. }) =
                        document.get(body).map(|e| &e.data)
                    else {
                        return Err(invalid(
                            "Connection Library pose requires a single mechanical Part",
                        ));
                    };
                    let feature = features
                        .iter()
                        .rev()
                        .find(|id| {
                            matches!(
                                document.get(**id).map(|e| &e.data),
                                Some(EntityData::Feature {
                                    suppressed: false,
                                    ..
                                })
                            )
                        })
                        .ok_or_else(|| {
                            invalid("Connection Library Part has no active mechanical provider")
                        })?;
                    (*feature, body)
                }
            };
            let actual = model::read_mechanical_part(document, feature)?;
            if actual.output.body != body {
                return Err(invalid("Mechanical pose supplier belongs to another Part"));
            }
            parameter_suppliers(document, &actual, &mut dependencies)?;
            capture(&mut sources, document, feature)?;
            capture(&mut sources, document, body)?;
            // Component pose is derived, but its exact canonical identity and
            // fallback are part of the captured authority and fingerprint.
            capture(&mut sources, document, row.instance)?;
        }
    }
    // Actual retained scalar values are fingerprinted too. Executable order
    // follows the same explicit Parameter dependencies.
    let mut pending = dependencies.iter().copied().collect::<Vec<_>>();
    let mut visited = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        capture(&mut sources, document, id)?;
        if let Some(EntityData::Parameter { inputs, .. }) = document.get(id).map(|e| &e.data) {
            for link in inputs {
                if link.geometry.is_some()
                    || document
                        .get(link.target)
                        .is_some_and(|e| !matches!(e.data, EntityData::Parameter { .. }))
                {
                    return Err(invalid(
                        "Connection scalar supplier contains a non-Parameter dependency",
                    ));
                }
                pending.push(link.target);
            }
        }
    }
    let bytes = serde_json::to_vec(&(binding, &request, sources, pose_fingerprints))
        .map_err(|e| invalid(e.to_string()))?;
    Ok(TransferPoseProjection {
        geometry_dependencies: dependencies,
        fingerprint: ContentHash::of(&bytes),
    })
}
/// Typed metadata projection for graph ordering and reference/cache binding.
/// The raw owning-record backlink is not an executable geometry dependency.
pub fn screw_hardware_connection_pose_projection(
    document: &Document,
    id: EntityId,
) -> Result<Option<TransferPoseProjection>, DocumentError> {
    if !matches!(document.get(id).map(|e|&e.data),Some(EntityData::Extension{definition,..})if definition.type_id==model::SCREW_HARDWARE_PLACEMENT_TYPE)
    {
        return Ok(None);
    }
    let source = model::read_screw_hardware_placement(document, id)?;
    if source.connection.is_none() {
        return Ok(None);
    }
    let _guard = Guard::enter(document, id, 0)?;
    projection(document, id, &source).map(Some)
}
pub(crate) fn screw_hardware_connection_evaluation_dependencies(
    document: &Document,
    id: EntityId,
) -> Result<Option<BTreeSet<EntityId>>, DocumentError> {
    Ok(screw_hardware_connection_pose_projection(document, id)?.map(|p| p.geometry_dependencies))
}
fn signed_turn(hand: model::ThreadHand, station: f64, pitch: f64) -> Result<f64, DocumentError> {
    if !station.is_finite() || !pitch.is_finite() || pitch <= 0. {
        return Err(invalid(
            "Connection phase requires a finite station and positive pitch",
        ));
    }
    let sign = if hand == model::ThreadHand::Right {
        1.
    } else {
        -1.
    };
    Ok(sign * TAU * station.rem_euclid(pitch) / pitch)
}
fn resolved_rows(
    document: &Document,
    p: &EvaluatedParameters,
    request: &ScrewConnectionRequest,
) -> Result<Vec<ResolvedMechanicalPart>, DocumentError> {
    request
        .hardware
        .iter()
        .map(|r| model::resolve_screw_connection_hardware(document, p, r))
        .collect()
}
fn washer(part: &ResolvedMechanicalPart) -> Result<f64, DocumentError> {
    if let ResolvedMechanicalPart::Washer { thickness_mm, .. } = part {
        Ok(*thickness_mm)
    } else {
        Err(invalid(
            "A washer role must resolve to actual canonical washer geometry",
        ))
    }
}
fn phase_reference(
    request: &ScrewConnectionRequest,
    parts: &[ResolvedMechanicalPart],
    screw_station: f64,
    grip: f64,
) -> Result<Option<(f64, f64, model::ThreadHand)>, DocumentError> {
    let rows = request
        .hardware
        .iter()
        .zip(parts)
        .filter(|(r, _)| matches!(r.role, Role::Screw | Role::HangerBolt))
        .collect::<Vec<_>>();
    let [(row, part)] = rows.as_slice() else {
        return Err(invalid(
            "Connection kinematics requires exactly one screw or hanger role",
        ));
    };
    match (row.role, *part) {
        (
            Role::Screw,
            ResolvedMechanicalPart::MachineScrew {
                underhead_length_mm,
                thread,
                ..
            },
        ) => Ok(Some((
            screw_station + underhead_length_mm - thread.length_mm,
            thread.lead_mm,
            thread.hand,
        ))),
        (Role::HangerBolt, ResolvedMechanicalPart::CustomConnection { parameters }) => {
            let model::ResolvedCustomConnectionHardware::HangerBolt {
                plain_length_mm,
                metric_thread,
                ..
            } = parameters.as_ref()
            else {
                return Err(invalid(
                    "Hanger role requires explicit custom hanger geometry",
                ));
            };
            Ok(Some((
                grip - plain_length_mm - metric_thread.length_mm,
                metric_thread.lead_mm,
                metric_thread.hand,
            )))
        }
        (Role::Screw, ResolvedMechanicalPart::CustomConnection { parameters })
            if matches!(
                parameters.as_ref(),
                model::ResolvedCustomConnectionHardware::WoodScrew { .. }
            ) =>
        {
            Ok(None)
        }
        (Role::Screw, ResolvedMechanicalPart::WoodScrew { .. }) => Ok(None),
        _ => Err(invalid(
            "Connection primary role has incompatible mechanical source",
        )),
    }
}
/// Resolve one retained role from the current canonical source, without writing
/// private parameters or consulting final receiving/hardware aggregate geometry.
pub fn resolve_connection_hardware_pose(
    document: &Document,
    p: &EvaluatedParameters,
    binding: &ScrewHardwareConnectionBinding,
    owner: EntityId,
    axis: EntityId,
    revision: u64,
    snapshots: &BTreeMap<EntityId, TopologySnapshot>,
) -> Result<RigidPose, DocumentError> {
    let Some(EntityData::Instance {
        placement_source: Some(driver),
        ..
    }) = document.get(owner).map(|e| &e.data)
    else {
        return Err(invalid(
            "Associative hardware owner is missing or has no retained driver",
        ));
    };
    let _guard = Guard::enter(document, *driver, 1)?;
    let source = model::read_screw_hardware_placement(document, *driver)?;
    if source.owner != owner || source.axis != axis || source.connection.as_ref() != Some(binding) {
        return Err(invalid(
            "Associative connection binding belongs to another owner, driver or axis",
        ));
    }
    let request = request_for_driver(document, *driver, &source)?.ok_or_else(|| {
        invalid("The owning connection record is missing; repair it before resolving hardware")
    })?;
    let parts = resolved_rows(document, p, &request)?;
    let grip = model::resolve_screw_connection_grip(document, p, &request, revision, snapshots)?;
    if !grip.is_finite() || grip < 0. {
        return Err(invalid("Connection grip must be finite and nonnegative"));
    }
    let mut head = 0.;
    let mut nut = 0.;
    for (row, part) in request.hardware.iter().zip(&parts) {
        if row.role == Role::HeadWasher {
            head += washer(part)?;
        } else if row.role == Role::NutWasher {
            nut += washer(part)?;
        }
    }
    let screw_station = if let Some((_, part)) = request
        .hardware
        .iter()
        .zip(&parts)
        .find(|(row, _)| row.role == Role::Screw)
    {
        model::resolve_screw_connection_head_station(&request, p, part, head)?
    } else {
        0.
    };
    let reference = phase_reference(&request, &parts, screw_station, grip)?;
    let mut screw_roll = 0.;
    if matches!(request.mode, Mode::TappedReceiver | Mode::WoodInsert) {
        let entry = if request.mode == Mode::TappedReceiver {
            let row = request
                .receivers
                .iter()
                .find(|r| matches!(r.source.cut, model::ConnectionReceiverCut::Tapped { .. }))
                .ok_or_else(|| invalid("Tapped connection has no actual receiving thread"))?;
            row.source
                .resolve_current(document, p, revision, snapshots)?
                .station_mm
        } else {
            grip
        };
        let Some((start, pitch, hand)) = reference else {
            return Err(invalid(
                "Metric receiving thread requires an actual metric screw phase reference",
            ));
        };
        screw_roll = signed_turn(hand, start - entry, pitch)?;
    }
    let mut head_station = screw_station;
    let mut nut_station = if request.mode == Mode::HangerBolt {
        -nut
    } else {
        grip
    };
    for (row, part) in request.hardware.iter().zip(&parts) {
        let (station, roll) = match row.role {
            Role::HeadWasher => {
                let at = head_station;
                head_station += washer(part)?;
                (at, 0.)
            }
            Role::NutWasher => {
                let at = nut_station;
                nut_station += washer(part)?;
                (at, 0.)
            }
            Role::Nut => {
                let ResolvedMechanicalPart::HexNut { thread, .. } = part else {
                    return Err(invalid("Nut role requires an actual internal metric nut"));
                };
                let at = if request.mode == Mode::HangerBolt {
                    -nut - thread.length_mm
                } else {
                    grip + nut
                };
                let Some((start, pitch, hand)) = reference else {
                    return Err(invalid(
                        "Nut placement requires a known metric phase reference",
                    ));
                };
                (at, screw_roll + signed_turn(hand, at - start, pitch)?)
            }
            Role::Screw => (screw_station, screw_roll),
            Role::HangerBolt | Role::Insert => (grip, 0.),
        };
        if row.role_id == binding.role {
            let mount = super::screw_connections::mount(part);
            let control = model::NumericTransform {
                translation: [length(0.), length(0.), length(station)],
                angle: angle(roll.to_degrees()),
                ..Default::default()
            }
            .resolve(p)?;
            let (axis_source, face) = model::read_screw_connection_axis(document, axis)?;
            let axis_pose = axis_source.resolve(document, p, &face, revision, snapshots)?;
            return mount.inverse()?.then(control)?.then(axis_pose);
        }
    }
    Err(invalid(
        "Associative hardware role is absent from its connection",
    ))
}

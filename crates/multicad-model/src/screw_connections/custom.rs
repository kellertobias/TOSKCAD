//! Explicit engineered wood-thread engagement. A purchased envelope is never
//! substituted for a known usable profile or installation recommendation.
use super::*;
use multicad_core::model::{
    ConnectionReceiverExtent, ResolvedCustomConnectionHardware as Custom, ResolvedThread,
    ScrewConnectionRequest,
};
use multicad_kernel::{CurvedEdge2d, CurvedProfile};
pub(super) struct Inputs<'a> {
    pub document: &'a Document,
    pub revision: u64,
    pub parameters: &'a EvaluatedParameters,
    pub model: &'a Model,
    pub source: &'a ScrewConnectionRequest,
    pub hardware: &'a [Hardware<'a>],
    pub axis: ReferenceFrame,
    pub topologies: &'a BTreeMap<EntityId, TopologySnapshot>,
    pub queries: &'a mut usize,
    pub interrupt: Option<&'a dyn Fn() -> bool>,
    pub head_station_mm: Option<f64>,
}
fn query<'a>(
    a: &Solid,
    b: &Solid,
    op: BooleanOperation,
    input: &mut Inputs<'a>,
) -> Result<Option<Solid>, ModelError> {
    checkpoint(input.interrupt)?;
    if *input.queries >= 4096 {
        return Err(error(
            Some(input.source.connection),
            "Connection validation exceeds the 4096 native candidate-query budget",
        ));
    }
    *input.queries += 1;
    a.boolean(b, op, input.model.policy)
        .map_err(|e| error(Some(input.source.connection), e))
}
fn annulus(
    input: &Inputs<'_>,
    r0: f64,
    r1: f64,
    start: f64,
    end: f64,
    role: &str,
) -> Result<Solid, ModelError> {
    let origin = std::array::from_fn(|i| input.axis.origin_mm[i] + start * input.axis.normal()[i]);
    let profile = CurvedProfile {
        frame: PlaneFrame {
            origin,
            u: input.axis.u,
            v: input.axis.v,
        },
        outer: vec![CurvedEdge2d::Circle {
            center: [0., 0.],
            radius_mm: r1,
        }],
        holes: vec![vec![CurvedEdge2d::Circle {
            center: [0., 0.],
            radius_mm: r0,
        }]],
    };
    Solid::extrude_curved_profile_named(
        &profile,
        end - start,
        input.source.connection,
        &[
            multicad_core::references::Origin {
                source: input.source.connection,
                role: format!("connection.wood/{role}/outer"),
            },
            multicad_core::references::Origin {
                source: input.source.connection,
                role: format!("connection.wood/{role}/inner"),
            },
        ],
        input.model.policy,
    )
    .map_err(|e| error(Some(input.source.connection), e))
}
struct Receiver<'a> {
    request: &'a model::ConnectionReceiverRequest,
    settings: model::ResolvedConnectionReceiver,
    depth: f64,
}
fn receivers<'a>(input: &Inputs<'a>) -> Result<Vec<Receiver<'a>>, ModelError> {
    let mut out = vec![];
    for request in &input.source.receivers {
        checkpoint(input.interrupt)?;
        let settings = request
            .source
            .resolve_current(
                input.document,
                input.parameters,
                input.revision,
                input.topologies,
            )
            .map_err(|e| error(Some(input.source.connection), e))?;
        if settings.thread.is_some() {
            return Err(error(
                Some(input.source.connection),
                "Engineered wood modes require independent round pilot/install bores, not an inferred receiving thread",
            ));
        }
        let depth = if let Some(depth) = settings.depth_mm {
            depth
        } else {
            let predecessor = input
                .model
                .solid(request.source.input.producer)
                .ok_or_else(|| {
                    error(
                        Some(input.source.connection),
                        "Wood receiving predecessor is unavailable",
                    )
                })?;
            let local = request
                .source
                .local_pose(
                    input.document,
                    input.parameters,
                    input.revision,
                    input.topologies,
                )
                .map_err(|e| error(Some(input.source.connection), e))?
                .frame(ReferenceFrame {
                    origin_mm: [0.; 3],
                    u: [1., 0., 0.],
                    v: [0., 1., 0.],
                });
            predecessor
                .transfer_through_depth(
                    PlaneFrame {
                        origin: local.origin_mm,
                        u: local.u,
                        v: local.v,
                    },
                    false,
                    input.model.policy,
                )
                .map_err(|e| error(Some(input.source.connection), e))?
                - input.model.policy.minimum_feature_mm.max(
                    8. * input
                        .model
                        .policy
                        .linear_tolerance_mm
                        .max(predecessor.geometry_policy().linear_tolerance_mm),
                )
        };
        out.push(Receiver {
            request,
            settings,
            depth,
        });
    }
    Ok(out)
}
fn unique<'a>(
    rows: &'a [Hardware<'a>],
    role: Role,
    id: EntityId,
) -> Result<&'a Hardware<'a>, ModelError> {
    let r = rows.iter().filter(|r| r.role == role).collect::<Vec<_>>();
    let [one] = r.as_slice() else {
        return Err(error(
            Some(id),
            "Connection requires exactly one actual hardware component for each primary role",
        ));
    };
    Ok(*one)
}
fn washer_stack(input: &Inputs<'_>, grip: f64) -> Result<(f64, f64), ModelError> {
    let id = input.source.connection;
    let mut head = 0.;
    let mut nut = 0.;
    for row in input.hardware {
        if matches!(row.role, Role::HeadWasher | Role::NutWasher) {
            let ResolvedMechanicalPart::Washer { thickness_mm, .. } = row.part else {
                return Err(error(
                    Some(id),
                    "Washer roles require actual washer geometry",
                ));
            };
            if row.role == Role::HeadWasher {
                head += thickness_mm;
            } else {
                nut += thickness_mm;
            }
        }
    }
    let mut a = input.head_station_mm.unwrap_or(-head);
    let mut b = if input.source.mode == Mode::HangerBolt {
        -nut
    } else {
        grip
    };
    for row in input.hardware {
        let ResolvedMechanicalPart::Washer { thickness_mm, .. } = row.part else {
            continue;
        };
        let expected = match row.role {
            Role::HeadWasher => {
                let v = a;
                a += thickness_mm;
                v
            }
            Role::NutWasher => {
                let v = b;
                b += thickness_mm;
                v
            }
            _ => continue,
        };
        at(
            input.axis,
            placed_frame(&row.part, row.pose),
            expected,
            input.model.policy.linear_tolerance_mm,
            id,
        )?;
    }
    Ok((head, nut))
}
fn world_interval(
    input: &Inputs<'_>,
    thread: &ResolvedThread,
    start_frame: ReferenceFrame,
    complete: [f64; 2],
) -> Result<(f64, f64), ModelError> {
    let id = input.source.connection;
    let tol = input.model.policy.linear_tolerance_mm;
    let (a, radial) = station(input.axis, start_frame.origin_mm);
    if radial > tol
        || dot(start_frame.normal(), input.axis.normal()) < 1. - 1e-9
        || !complete[0].is_finite()
        || !complete[1].is_finite()
        || complete[0] < 0.
        || complete[1] > thread.length_mm
        || complete[1] <= complete[0]
    {
        return Err(error(
            Some(id),
            "Custom thread has no known coaxial complete-profile interval",
        ));
    }
    let end = std::array::from_fn(|i| {
        start_frame.origin_mm[i] + thread.length_mm * start_frame.normal()[i]
    });
    if station(input.axis, end).1 > tol {
        return Err(error(
            Some(id),
            "Custom thread axis diverges across its actual physical span",
        ));
    }
    Ok((a + complete[0], a + complete[1]))
}
struct Wood<'a> {
    hardware: &'a Hardware<'a>,
    thread: &'a ResolvedThread,
    start: ReferenceFrame,
    interval: [f64; 2],
}
pub(super) fn validate(mut input: Inputs<'_>) -> Result<ScrewConnectionCheck, ModelError> {
    let id = input.source.connection;
    let mode = input.source.mode;
    let tol = input.model.policy.linear_tolerance_mm;
    let grip = multicad_core::model::resolve_screw_connection_grip(
        input.document,
        input.parameters,
        input.source,
        input.revision,
        input.topologies,
    )
    .map_err(|e| error(Some(id), e))?;
    let minimum = mm(
        &input.source.parameters.minimum_engagement,
        input.parameters,
        id,
    )?;
    let clearance = mm(
        &input.source.parameters.blind_tip_clearance,
        input.parameters,
        id,
    )?;
    if input.hardware.iter().any(|r| !match mode {
        Mode::DirectWood => matches!(r.role, Role::Screw | Role::HeadWasher),
        Mode::WoodInsert => matches!(r.role, Role::Screw | Role::HeadWasher | Role::Insert),
        Mode::HangerBolt => matches!(r.role, Role::HangerBolt | Role::Nut | Role::NutWasher),
        _ => false,
    }) {
        return Err(error(
            Some(id),
            "Hardware role is incompatible with this engineered wood connection mode",
        ));
    }
    let (head_washers, nut_washers) = washer_stack(&input, grip)?;
    let mut metric = None;
    let mut metric_tip = None;
    let wood = match mode {
        Mode::DirectWood => {
            let h = unique(input.hardware, Role::Screw, id)?;
            let ResolvedMechanicalPart::CustomConnection { parameters } = &h.part else {
                return Err(error(
                    Some(id),
                    "Wood thread and point geometry are unknown; choose an explicit custom modeled wood screw",
                ));
            };
            let Custom::WoodScrew {
                wood_thread,
                wood_thread_start,
                wood_complete_interval_mm,
                ..
            } = parameters.as_ref()
            else {
                return Err(error(
                    Some(id),
                    "Direct wood mode requires a distinct custom wood screw",
                ));
            };
            at(
                input.axis,
                placed_frame(&h.part, h.pose),
                input.head_station_mm.unwrap_or(-head_washers),
                tol,
                id,
            )?;
            Wood {
                hardware: h,
                thread: wood_thread,
                start: h.pose.frame(*wood_thread_start),
                interval: *wood_complete_interval_mm,
            }
        }
        Mode::WoodInsert => {
            let h = unique(input.hardware, Role::Insert, id)?;
            let ResolvedMechanicalPart::CustomConnection { parameters } = &h.part else {
                return Err(error(
                    Some(id),
                    "Purchased insert usable thread endpoints are unknown; choose an explicit custom modeled female insert",
                ));
            };
            let Custom::WoodInsert {
                outer_thread,
                inner_thread,
                outer_thread_start,
                inner_thread_start,
                outer_complete_interval_mm,
                ..
            } = parameters.as_ref()
            else {
                return Err(error(
                    Some(id),
                    "Insert mode requires a distinct custom female wood insert, not a hex nut",
                ));
            };
            at(input.axis, placed_frame(&h.part, h.pose), grip, tol, id)?;
            let screw = unique(input.hardware, Role::Screw, id)?;
            let ResolvedMechanicalPart::MachineScrew {
                underhead_length_mm,
                thread: external,
                ..
            } = &screw.part
            else {
                return Err(error(
                    Some(id),
                    "Female insert mode requires a known actual metric screw",
                ));
            };
            at(
                input.axis,
                placed_frame(&screw.part, screw.pose),
                input.head_station_mm.unwrap_or(-head_washers),
                tol,
                id,
            )?;
            let external_start = model::machine_screw_thread_start(&screw.part, screw.pose)
                .map_err(|e| error(Some(id), e))?;
            let mating = model::resolve_metric_thread_mating(
                external,
                external_start,
                inner_thread,
                h.pose.frame(*inner_thread_start),
                minimum,
                tol,
            )
            .map_err(|e| error(Some(id), e))?;
            no_phase_correction(&mating, id)?;
            metric = Some(mating.overlap_length_mm);
            let tip = input.head_station_mm.unwrap_or(-head_washers) + underhead_length_mm;
            metric_tip = Some(tip);
            if inner_thread.length_mm < outer_thread.length_mm
                && tip > grip + inner_thread.length_mm - clearance + tol
            {
                return Err(error(
                    Some(id),
                    "Metric screw bottoms against the custom insert's actual blind inner floor",
                ));
            }
            Wood {
                hardware: h,
                thread: outer_thread,
                start: h.pose.frame(*outer_thread_start),
                interval: *outer_complete_interval_mm,
            }
        }
        Mode::HangerBolt => {
            let h = unique(input.hardware, Role::HangerBolt, id)?;
            let ResolvedMechanicalPart::CustomConnection { parameters } = &h.part else {
                return Err(error(
                    Some(id),
                    "Purchased hanger wood/metric usable spans are unknown; choose explicit custom modeled hanger geometry",
                ));
            };
            let Custom::HangerBolt {
                wood_thread,
                metric_thread,
                wood_thread_start,
                metric_thread_start,
                wood_complete_interval_mm,
                ..
            } = parameters.as_ref()
            else {
                return Err(error(
                    Some(id),
                    "Hanger mode requires a distinct two-ended hanger bolt",
                ));
            };
            at(input.axis, placed_frame(&h.part, h.pose), grip, tol, id)?;
            let nut = unique(input.hardware, Role::Nut, id)?;
            let ResolvedMechanicalPart::HexNut {
                thread: internal, ..
            } = &nut.part
            else {
                return Err(error(
                    Some(id),
                    "Hanger mode requires an actual modeled metric nut",
                ));
            };
            let internal_start = placed_frame(&nut.part, nut.pose);
            at(
                input.axis,
                internal_start,
                -nut_washers - internal.length_mm,
                tol,
                id,
            )?;
            let mating = model::resolve_metric_thread_mating(
                metric_thread,
                h.pose.frame(*metric_thread_start),
                internal,
                internal_start,
                minimum,
                tol,
            )
            .map_err(|e| error(Some(id), e))?;
            no_phase_correction(&mating, id)?;
            metric = Some(mating.overlap_length_mm);
            Wood {
                hardware: h,
                thread: wood_thread,
                start: h.pose.frame(*wood_thread_start),
                interval: *wood_complete_interval_mm,
            }
        }
        _ => {
            return Err(error(
                Some(id),
                "Expected an engineered wood connection mode",
            ));
        }
    };
    let regions = receivers(&input)?;
    let mut clamps = vec![];
    let mut pilots = vec![];
    for r in &regions {
        let a = r.settings.station_mm;
        let b = a + r.depth;
        if (a - grip).abs() <= tol {
            pilots.push(r);
        } else if a < grip - tol && b <= grip + tol {
            if !matches!(
                r.request.source.extent,
                ConnectionReceiverExtent::Through {}
            ) {
                return Err(error(
                    Some(id),
                    "Clamped layers require actual finite through-material extents, not an assumed internal blind station",
                ));
            }
            clamps.push((a, b));
        } else {
            return Err(error(
                Some(id),
                "Receiving rows must describe the contiguous clamp stack and one wood entry at the back face",
            ));
        }
    }
    require_contiguous(&mut clamps, grip, tol, id)?;
    let [pilot] = pilots.as_slice() else {
        return Err(error(
            Some(id),
            "Select exactly one independent wood pilot/install receiving part at the back of the clamped stack",
        ));
    };
    let (wood_begin, wood_end) = world_interval(&input, wood.thread, wood.start, wood.interval)?;
    let end = (grip + pilot.depth).min(wood_end);
    let begin = grip.max(wood_begin);
    let wood_engagement = end - begin;
    if wood_engagement < minimum || wood_engagement <= 0. {
        return Err(error(
            Some(id),
            "Actual known wood thread/profile overlap is insufficient; edit the authored span, grip or position",
        ));
    }
    let deepest_tip = metric_tip.map_or(wood_end, |tip: f64| tip.max(wood_end));
    if pilot.settings.depth_mm.is_some() && deepest_tip > grip + pilot.depth - clearance + tol {
        return Err(error(
            Some(id),
            "Actual custom wood tip violates the declared pilot blind-bottom clearance",
        ));
    }
    let radius = wood.thread.major_radius_mm;
    let root = radius - wood.thread.profile.depth_mm;
    let pilot_radius = pilot.settings.diameter_mm / 2.;
    if pilot_radius < root || pilot_radius >= radius {
        return Err(error(
            Some(id),
            "Engineered pilot/install radius must clear the actual thread core and leave positive modeled flank engagement; no universal pilot ratio is inferred",
        ));
    }
    let recipient = input
        .model
        .leaf_solid(&pilot.request.source.occurrence)
        .ok_or_else(|| {
            error(
                Some(id),
                "Wood receiving occurrence has no current physical native leaf",
            )
        })?;
    let required = annulus(
        &input,
        pilot_radius,
        radius + pilot.settings.minimum_wall_mm,
        begin,
        end,
        "required.stock",
    )?;
    if query(&required, recipient, BooleanOperation::Subtract, &mut input)?.is_some() {
        return Err(error(
            Some(id),
            "Actual wood stock lacks a complete surrounding wall across the known engagement interval",
        ));
    }
    let allowed = annulus(&input, root, radius, begin, end, "allowed.flanks")?;
    let common = query(
        wood.hardware.solid,
        recipient,
        BooleanOperation::Intersect,
        &mut input,
    )?
    .ok_or_else(|| {
        error(
            Some(id),
            "No positive native wood flank/material engagement exists",
        )
    })?;
    if query(&common, &allowed, BooleanOperation::Subtract, &mut input)?.is_some() {
        return Err(error(
            Some(id),
            "Wood hardware interferes outside its actual bounded flank region",
        ));
    }
    let owned = input
        .hardware
        .iter()
        .map(|r| r.instance)
        .collect::<BTreeSet<_>>();
    for (i, a) in input.hardware.iter().enumerate() {
        for b in &input.hardware[i + 1..] {
            checkpoint(input.interrupt)?;
            collision(a.solid, b.solid, input.model, id, input.queries)?;
        }
        for path in input.model.snapshot().leaf_occurrences.keys() {
            if owned.contains(&path.root) {
                continue;
            }
            if a.instance == wood.hardware.instance && *path == pilot.request.source.occurrence {
                continue;
            }
            checkpoint(input.interrupt)?;
            let other = input
                .model
                .leaf_solid(path)
                .ok_or_else(|| error(Some(id), "Physical component has no exact current solid"))?;
            collision(a.solid, other, input.model, id, input.queries)?;
        }
    }
    Ok(ScrewConnectionCheck {
        connection: id,
        mode,
        hardware_instances: input.hardware.iter().map(|r| r.instance).collect(),
        receiver_features: input.source.receivers.iter().map(|r| r.feature).collect(),
        grip_mm: grip,
        minimum_engagement_mm: minimum,
        blind_tip_clearance_mm: clearance,
        actual_engagement_mm: metric.or(Some(wood_engagement)),
        actual_wood_engagement_mm: Some(wood_engagement),
        native_component_collision_checked: true,
        scope: "Current actual engineered wood flank/material overlap and bounded stock wall; metric insert/hanger mating and phase independently checked; all other native component interference refused. Flat ends and independently authored pilot geometry; no purchased profile, installation, load or strength certification".into(),
    })
}

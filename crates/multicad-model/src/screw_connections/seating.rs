//! Native material witnesses on both sides of an explicitly derived bearing surface.
use super::*;
use multicad_kernel::{CurvedEdge2d, CurvedProfile, RevolutionAxis};
pub(super) struct Inputs<'a> {
    pub document: &'a Document,
    pub revision: u64,
    pub parameters: &'a EvaluatedParameters,
    pub model: &'a Model,
    pub source: &'a model::ScrewConnectionRequest,
    pub hardware: &'a [Hardware<'a>],
    pub axis: ReferenceFrame,
    pub topologies: &'a BTreeMap<EntityId, TopologySnapshot>,
    pub queries: &'a mut usize,
    pub interrupt: Option<&'a dyn Fn() -> bool>,
    pub grip: f64,
}
struct Band<'a> {
    solid: &'a Solid,
    inner: f64,
    outer: f64,
}
struct Receiver<'a> {
    source: &'a model::ConnectionReceiverRequest,
    settings: model::ResolvedConnectionReceiver,
}
fn fail(input: &Inputs<'_>, message: impl ToString) -> ModelError {
    error(Some(input.source.connection), message.to_string())
}
fn covered(
    input: &mut Inputs<'_>,
    required: &Solid,
    solid: &Solid,
    label: &str,
) -> Result<(), ModelError> {
    checkpoint(input.interrupt)?;
    if *input.queries >= 4096 {
        return Err(fail(
            input,
            "Connection bearing validation exceeds the 4096 native query budget",
        ));
    }
    *input.queries += 1;
    if required
        .boolean(solid, BooleanOperation::Subtract, input.model.policy)
        .map_err(|e| fail(input, e))?
        .is_some()
    {
        return Err(fail(
            input,
            format!(
                "Actual {label} has no complete native material adjoining the bearing surface; a floating head/washer/nut is not contact"
            ),
        ));
    }
    Ok(())
}
fn annulus(
    input: &Inputs<'_>,
    r0: f64,
    r1: f64,
    start: f64,
    height: f64,
    label: &str,
) -> Result<Solid, ModelError> {
    let id = input.source.connection;
    let profile = CurvedProfile {
        frame: PlaneFrame {
            origin: std::array::from_fn(|i| {
                input.axis.origin_mm[i] + start * input.axis.normal()[i]
            }),
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
    let names = ["outer", "inner"].map(|role| multicad_core::references::Origin {
        source: id,
        role: format!("connection.bearing/{label}/{role}"),
    });
    Solid::extrude_curved_profile_named(&profile, height, id, &names, input.model.policy)
        .map_err(|e| fail(input, e))
}
fn meridian(input: &Inputs<'_>, points: [[f64; 2]; 4], label: &str) -> Result<Solid, ModelError> {
    let id = input.source.connection;
    let n = input.axis.normal();
    let profile = CurvedProfile {
        frame: PlaneFrame {
            origin: input.axis.origin_mm,
            u: input.axis.u,
            v: n,
        },
        outer: (0..4)
            .map(|i| CurvedEdge2d::Line {
                a: points[i],
                b: points[(i + 1) % 4],
            })
            .collect(),
        holes: vec![],
    };
    let names = (0..4)
        .map(|i| multicad_core::references::Origin {
            source: id,
            role: format!("connection.bearing/{label}/edge/{i}"),
        })
        .collect::<Vec<_>>();
    Solid::revolve_curved_profile_named(
        &profile,
        RevolutionAxis {
            origin_mm: input.axis.origin_mm,
            direction: n,
        },
        std::f64::consts::TAU,
        id,
        &names,
        input.model.policy,
    )
    .map_err(|e| fail(input, e))
}
fn flat(
    input: &mut Inputs<'_>,
    upper: Band<'_>,
    lower: Band<'_>,
    station: f64,
    label: &str,
) -> Result<(), ModelError> {
    let eps = input
        .model
        .policy
        .minimum_feature_mm
        .max(16. * input.model.policy.linear_tolerance_mm);
    let r0 = upper.inner.max(lower.inner) + 2. * eps;
    let r1 = upper.outer.min(lower.outer) - 2. * eps;
    if !r0.is_finite() || !r1.is_finite() || r1 - r0 < eps {
        return Err(fail(
            input,
            "Bearing components have no positive overlapping flat annular land",
        ));
    }
    // Keep the policy-sized height exact rather than recovering it by
    // subtracting nearby absolute stations (which can round below the minimum).
    let above = annulus(input, r0, r1, station - eps, eps, &format!("{label}/above"))?;
    let below = annulus(input, r0, r1, station, eps, &format!("{label}/below"))?;
    covered(
        input,
        &above,
        upper.solid,
        &format!("{label} upper bearing"),
    )?;
    covered(
        input,
        &below,
        lower.solid,
        &format!("{label} lower bearing"),
    )
}
fn washer<'a>(h: &Hardware<'a>) -> Result<Band<'a>, ModelError> {
    let ResolvedMechanicalPart::Washer {
        bore_radius_mm,
        outside_radius_mm,
        ..
    } = &h.part
    else {
        return Err(error(
            Some(h.instance),
            "A bearing washer role needs actual washer geometry",
        ));
    };
    Ok(Band {
        solid: h.solid,
        inner: *bore_radius_mm,
        outer: *outside_radius_mm,
    })
}
fn nut_band<'a>(h: &Hardware<'a>) -> Result<Band<'a>, ModelError> {
    let ResolvedMechanicalPart::HexNut {
        thread,
        across_flats_mm,
    } = &h.part
    else {
        return Err(error(
            Some(h.instance),
            "A nut bearing role needs actual modeled nut geometry",
        ));
    };
    Ok(Band {
        solid: h.solid,
        inner: thread.major_radius_mm,
        outer: across_flats_mm / 2.,
    })
}
fn head<'a>(h: &'a Hardware<'_>) -> Result<(&'a model::ResolvedScrewHead, f64), ModelError> {
    match &h.part {
        ResolvedMechanicalPart::MachineScrew {
            head,
            shank_radius_mm,
            ..
        } => Ok((head, *shank_radius_mm)),
        ResolvedMechanicalPart::CustomConnection { parameters } => match parameters.as_ref() {
            model::ResolvedCustomConnectionHardware::WoodScrew {
                head,
                plain_radius_mm,
                ..
            } => Ok((head, *plain_radius_mm)),
            _ => Err(error(
                Some(h.instance),
                "Head bearing needs a headed custom screw",
            )),
        },
        _ => Err(error(
            Some(h.instance),
            "Head bearing needs actual modeled screw geometry",
        )),
    }
}
fn stock<'a>(input: &Inputs<'a>, receiver: &Receiver<'_>) -> Result<Band<'a>, ModelError> {
    let solid = input
        .model
        .leaf_solid(&receiver.source.source.occurrence)
        .ok_or_else(|| fail(input, "Bearing receiver has no current physical solid"))?;
    Ok(Band {
        solid,
        inner: receiver.settings.diameter_mm / 2.,
        outer: f64::INFINITY,
    })
}
fn unique<'a>(input: &Inputs<'a>, role: Role) -> Result<&'a Hardware<'a>, ModelError> {
    let rows = input
        .hardware
        .iter()
        .filter(|h| h.role == role)
        .collect::<Vec<_>>();
    let [row] = rows.as_slice() else {
        return Err(fail(
            input,
            format!("Bearing validation needs exactly one {role:?}"),
        ));
    };
    Ok(*row)
}
fn conical(
    input: &mut Inputs<'_>,
    screw: &Hardware<'_>,
    receiver: &Receiver<'_>,
) -> Result<(), ModelError> {
    let (head, shank) = head(screw)?;
    let model::ResolvedScrewHead::Countersunk {
        radius_mm,
        height_mm,
        included_angle_deg,
        edge_chamfer_mm,
        neck_radius_mm,
    } = head
    else {
        return Err(fail(
            input,
            "A conical bearing requires an actual countersunk head",
        ));
    };
    let Some((true, diameter, depth)) = receiver.settings.seat else {
        return Err(fail(
            input,
            "A countersunk head requires its compatible receiving cone",
        ));
    };
    let eps = input
        .model
        .policy
        .minimum_feature_mm
        .max(16. * input.model.policy.linear_tolerance_mm);
    let theta = included_angle_deg.to_radians() / 2.;
    let slope = theta.tan();
    let mouth = diameter / 2.;
    let bore = receiver.settings.diameter_mm / 2.;
    let lo = (shank + neck_radius_mm * (1. - theta.cos())).max(bore) + 4. * eps;
    let hi = radius_mm
        .min(shank + (height_mm - edge_chamfer_mm) * slope)
        .min(mouth)
        - 4. * eps;
    if hi - lo < eps {
        return Err(fail(
            input,
            "Actual head and seat have no bounded conical bearing land outside the neck blend/rim/chamfer",
        ));
    }
    let a = (mouth - hi) / slope;
    let b = (mouth - lo) / slope;
    if a < 0. || b > depth || b - a < eps {
        return Err(fail(
            input,
            "Conical bearing witnesses lie outside the actual receiving cone",
        ));
    }
    let inside = meridian(
        input,
        [[hi - eps, a], [hi, a], [lo, b], [lo - eps, b]],
        "cone/head",
    )?;
    let outside = meridian(
        input,
        [[hi, a], [hi + eps, a], [lo + eps, b], [lo, b]],
        "cone/stock",
    )?;
    let recipient = stock(input, receiver)?;
    covered(
        input,
        &inside,
        screw.solid,
        "countersunk head conical bearing",
    )?;
    covered(
        input,
        &outside,
        recipient.solid,
        "receiving conical bearing",
    )
}
/// Validates actual bearing independently from component non-interference.
/// Returns the derived screw neck station, or None for a headless hanger.
pub(super) fn validate(mut input: Inputs<'_>) -> Result<Option<f64>, ModelError> {
    let tol = input.model.policy.linear_tolerance_mm;
    let mut receivers = vec![];
    for source in &input.source.receivers {
        receivers.push(Receiver {
            source,
            settings: source
                .source
                .resolve_current(
                    input.document,
                    input.parameters,
                    input.revision,
                    input.topologies,
                )
                .map_err(|e| fail(&input, e))?,
        });
    }
    let front = receivers
        .iter()
        .find(|r| r.settings.station_mm.abs() <= tol)
        .ok_or_else(|| {
            fail(
                &input,
                "Bearing support needs a receiving entry at the actual connection front",
            )
        })?;
    let seats = receivers
        .iter()
        .filter(|r| r.settings.seat.is_some())
        .collect::<Vec<_>>();
    if seats.iter().any(|r| {
        r.settings.station_mm.abs() > tol
            || r.source.source.occurrence != input.source.axis.source.occurrence
    }) {
        return Err(fail(
            &input,
            "Only a seat in the selected actual front receiving entry is supported",
        ));
    }
    if input.source.mode == Mode::HangerBolt && !seats.is_empty() {
        return Err(fail(
            &input,
            "A headless hanger does not support a screw-head receiving seat",
        ));
    }
    let head_washers = input
        .hardware
        .iter()
        .filter(|h| h.role == Role::HeadWasher)
        .collect::<Vec<_>>();
    let nut_washers = input
        .hardware
        .iter()
        .filter(|h| h.role == Role::NutWasher)
        .collect::<Vec<_>>();
    let sum = |rows: &[&Hardware<'_>]| -> Result<f64, ModelError> {
        rows.iter()
            .map(|h| match h.part {
                ResolvedMechanicalPart::Washer { thickness_mm, .. } => Ok(thickness_mm),
                _ => Err(error(
                    Some(h.instance),
                    "Washer roles need actual washer geometry",
                )),
            })
            .sum()
    };
    let head_sum = sum(&head_washers)?;
    let mut neck = None;
    if input.source.mode != Mode::HangerBolt {
        let screw = unique(&input, Role::Screw)?;
        let b = model::resolve_screw_connection_head_station(
            input.source,
            input.parameters,
            &screw.part,
            head_sum,
        )
        .map_err(|e| fail(&input, e))?;
        at(
            input.axis,
            placed_frame(&screw.part, screw.pose),
            b,
            tol,
            input.source.connection,
        )?;
        neck = Some(b);
        if let Some(receiver) = seats
            .first()
            .filter(|r| r.settings.seat.is_some_and(|s| s.0))
        {
            conical(&mut input, screw, receiver)?;
        } else {
            let (head, shank) = head(screw)?;
            let mut previous = Band {
                solid: screw.solid,
                inner: shank + head.neck_radius_mm(),
                outer: head.minimum_radius_mm(),
            };
            let mut station = b;
            for row in &head_washers {
                at(
                    input.axis,
                    placed_frame(&row.part, row.pose),
                    station,
                    tol,
                    input.source.connection,
                )?;
                flat(
                    &mut input,
                    previous,
                    washer(row)?,
                    station,
                    "head washer stack",
                )?;
                let ResolvedMechanicalPart::Washer { thickness_mm, .. } = row.part else {
                    unreachable!()
                };
                station += thickness_mm;
                previous = washer(row)?;
            }
            let support = if input.source.mode == Mode::WoodInsert && input.grip.abs() <= tol {
                let insert = unique(&input, Role::Insert)?;
                let ResolvedMechanicalPart::CustomConnection { parameters } = &insert.part else {
                    return Err(fail(
                        &input,
                        "An insert supporting a screw head must have known actual geometry",
                    ));
                };
                let model::ResolvedCustomConnectionHardware::WoodInsert {
                    inner_thread,
                    outer_thread,
                    ..
                } = parameters.as_ref()
                else {
                    return Err(fail(
                        &input,
                        "Head support is not a modeled female wood insert",
                    ));
                };
                Band {
                    solid: insert.solid,
                    inner: inner_thread.major_radius_mm,
                    outer: outer_thread.major_radius_mm - outer_thread.profile.depth_mm,
                }
            } else {
                stock(&input, front)?
            };
            flat(
                &mut input,
                previous,
                support,
                station,
                "head to receiving support",
            )?;
        }
    }
    if matches!(input.source.mode, Mode::ThroughNut | Mode::HangerBolt) {
        let nut = unique(&input, Role::Nut)?;
        if input.source.mode == Mode::HangerBolt {
            let mut previous = nut_band(nut)?;
            let mut station = -sum(&nut_washers)?;
            for row in &nut_washers {
                flat(
                    &mut input,
                    previous,
                    washer(row)?,
                    station,
                    "hanger nut washer stack",
                )?;
                let ResolvedMechanicalPart::Washer { thickness_mm, .. } = row.part else {
                    unreachable!()
                };
                station += thickness_mm;
                previous = washer(row)?;
            }
            let support = stock(&input, front)?;
            flat(&mut input, previous, support, 0., "hanger front support")?;
        } else {
            let back = receivers
                .iter()
                .filter(|r| r.settings.station_mm < input.grip + tol)
                .max_by(|a, b| a.settings.station_mm.total_cmp(&b.settings.station_mm))
                .ok_or_else(|| fail(&input, "Nut bearing has no final clamped receiving part"))?;
            let mut previous = stock(&input, back)?;
            let mut station = input.grip;
            for row in &nut_washers {
                flat(
                    &mut input,
                    previous,
                    washer(row)?,
                    station,
                    "back nut washer stack",
                )?;
                let ResolvedMechanicalPart::Washer { thickness_mm, .. } = row.part else {
                    unreachable!()
                };
                station += thickness_mm;
                previous = washer(row)?;
            }
            at(
                input.axis,
                placed_frame(&nut.part, nut.pose),
                station,
                tol,
                input.source.connection,
            )?;
            flat(
                &mut input,
                previous,
                nut_band(nut)?,
                station,
                "nut to receiving support",
            )?;
        }
    }
    Ok(neck)
}

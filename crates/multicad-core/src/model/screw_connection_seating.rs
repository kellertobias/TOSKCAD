//! Engineering seating datums derived from actual authored head and seat geometry.
//! Native callers must separately prove current front-entry position, material
//! bearing contact, receiving coverage and non-interference.
use super::{
    ConnectionReceiverSeat, ResolvedCustomConnectionHardware, ResolvedMechanicalPart,
    ResolvedScrewHead, ScrewConnectionMode, ScrewConnectionRequest, source::invalid,
};
use crate::{
    document::DocumentError,
    parameters::{EvaluatedParameters, Unit},
};

/// Screw neck station along the connection axis. Only seats at the front entry
/// are supported. A captured face's zero manual station is a sentinel, not
/// evidence of its current location; native validation must establish station0.
pub fn resolve_screw_connection_head_station(
    request: &ScrewConnectionRequest,
    parameters: &EvaluatedParameters,
    screw: &ResolvedMechanicalPart,
    head_washer_sum_mm: f64,
) -> Result<f64, DocumentError> {
    if !head_washer_sum_mm.is_finite() || head_washer_sum_mm < 0. {
        return Err(invalid(
            "Head washer thickness must be finite and nonnegative",
        ));
    }
    let (head, shank) = match screw {
        ResolvedMechanicalPart::MachineScrew {
            head,
            shank_radius_mm,
            ..
        } => (head, *shank_radius_mm),
        ResolvedMechanicalPart::CustomConnection { parameters } => match parameters.as_ref() {
            ResolvedCustomConnectionHardware::WoodScrew {
                head,
                plain_radius_mm,
                ..
            } => (head, *plain_radius_mm),
            _ => {
                return Err(invalid(
                    "A head seat requires a known headed screw; a hanger or insert is not a headed screw",
                ));
            }
        },
        _ => {
            return Err(invalid(
                "Head seating requires actual modeled screw head geometry",
            ));
        }
    };
    if request.mode == ScrewConnectionMode::HangerBolt {
        return Err(invalid(
            "Headless hanger connections do not support a screw-head seat",
        ));
    }
    let seats = request
        .receivers
        .iter()
        .filter(|r| r.source.seat.is_some())
        .collect::<Vec<_>>();
    let seat = match seats.as_slice() {
        [] => {
            if matches!(head, ResolvedScrewHead::Countersunk { .. }) {
                return Err(invalid(
                    "A countersunk screw requires a compatible explicit conical receiving seat",
                ));
            }
            return Ok(-head_washer_sum_mm);
        }
        [receiver] => {
            if receiver
                .source
                .station
                .value_in(Unit::Millimeter, Some(parameters))
                .map_err(|e| invalid(&e))?
                .abs()
                > 1e-10
            {
                return Err(invalid(
                    "Only a receiving seat at the connection front entry is supported",
                ));
            }
            receiver.source.seat.as_ref().expect("filtered seat")
        }
        _ => return Err(invalid("Select exactly one front receiving head seat")),
    };
    match seat {
        ConnectionReceiverSeat::Counterbore { diameter, depth } => {
            if matches!(head, ResolvedScrewHead::Countersunk { .. }) {
                return Err(invalid(
                    "A countersunk head cannot bear on a cylindrical counterbore",
                ));
            }
            let radius = diameter
                .value_in(Unit::Millimeter, Some(parameters))
                .map_err(|e| invalid(&e))?
                / 2.;
            let depth = depth
                .value_in(Unit::Millimeter, Some(parameters))
                .map_err(|e| invalid(&e))?;
            if !radius.is_finite()
                || !depth.is_finite()
                || depth <= 0.
                || radius < head.maximum_radius_mm()
            {
                return Err(invalid(
                    "The counterbore must have positive depth and contain the complete head envelope, including hex corners",
                ));
            }
            // Washer outer diameters and actual stack fit remain native checks.
            Ok(depth - head_washer_sum_mm)
        }
        ConnectionReceiverSeat::Countersink { diameter, depth } => {
            if head_washer_sum_mm != 0. {
                return Err(invalid(
                    "Flat head washers on a conical seat require a separate bearing design",
                ));
            }
            let ResolvedScrewHead::Countersunk {
                included_angle_deg,
                radius_mm,
                height_mm,
                edge_chamfer_mm,
                neck_radius_mm,
            } = head
            else {
                return Err(invalid(
                    "A conical receiving seat requires an actual countersunk head",
                ));
            };
            let receiver = seats[0].source.resolve(parameters)?;
            let bore = receiver.diameter_mm / 2.;
            let mouth = diameter
                .value_in(Unit::Millimeter, Some(parameters))
                .map_err(|e| invalid(&e))?
                / 2.;
            let depth = depth
                .value_in(Unit::Millimeter, Some(parameters))
                .map_err(|e| invalid(&e))?;
            let theta = included_angle_deg.to_radians() / 2.;
            let slope = theta.tan();
            if !mouth.is_finite()
                || !depth.is_finite()
                || depth <= 0.
                || mouth <= bore
                || !slope.is_finite()
                || slope <= 0.
                || ((mouth - bore) / depth - slope).abs() > 1e-10 * slope.abs().max(1.)
            {
                return Err(invalid(
                    "Countersunk head and receiving cone must have the same positive supporting slope",
                ));
            }
            let lower = (shank + neck_radius_mm * (1. - theta.cos())).max(bore);
            let upper = radius_mm
                .min(shank + (height_mm - edge_chamfer_mm) * slope)
                .min(mouth);
            if upper <= lower {
                return Err(invalid(
                    "Head and receiving seat have no positive actual conical bearing interval outside the neck blend, rim and chamfer",
                ));
            }
            let station = (mouth - shank) / slope;
            if !station.is_finite() {
                return Err(invalid("The countersunk neck station is nonfinite"));
            }
            Ok(station)
        }
    }
}

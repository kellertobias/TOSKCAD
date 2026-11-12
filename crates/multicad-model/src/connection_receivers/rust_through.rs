//! Exact round through receivers with optional certified flat counterbores.
use super::*;
use multicad_kernel::CircularPocketTool;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FlatSeatProof {
    pub radius_mm: f64,
    pub depth_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ThroughReceiverProof {
    /// Mouth centers in the receiving producer's geometry space, from entry
    /// to exit. Their full surrounding disks are certified through the stock.
    pub mouths_mm: [[f64; 3]; 2],
    pub radius_mm: f64,
    pub protected_radius_mm: f64,
    pub head_seat: Option<FlatSeatProof>,
}

pub(crate) fn build(
    inputs: Inputs<'_>,
    id: EntityId,
    source: &ConnectionReceiverDefinition,
) -> Result<(Solid, ThroughReceiverProof), ModelError> {
    let policy = inputs.policy;
    let fail = |message| error(Some(id), message);
    let resolved = source
        .resolve_current(
            inputs.document,
            inputs.parameters,
            inputs.revision,
            inputs.snapshots,
        )
        .map_err(|cause| error(Some(id), cause))?;
    if resolved.depth_mm.is_some()
        || resolved.thread.is_some()
        || resolved.seat.is_some_and(|seat| seat.0)
    {
        return Err(fail(
            "Rust receiving cuts require a round through bore with an optional flat counterbore",
        ));
    }
    let pose = source
        .local_pose(
            inputs.document,
            inputs.parameters,
            inputs.revision,
            inputs.snapshots,
        )
        .map_err(|cause| error(Some(id), cause))?;
    let local = pose.frame(ReferenceFrame {
        origin_mm: [0.; 3],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
    });
    let frame = PlaneFrame {
        origin: local.origin_mm,
        u: local.u,
        v: local.v,
    };
    let stock = inputs
        .solids
        .get(&source.input.producer)
        .ok_or_else(|| fail("Receiving predecessor has no current Rust solid"))?;
    let tolerance = policy
        .linear_tolerance_mm
        .max(stock.geometry_policy().linear_tolerance_mm);
    let overrun = policy.minimum_feature_mm.max(8. * tolerance);
    let reach = stock
        .transfer_through_depth(frame, false, policy)
        .map_err(|cause| error(Some(id), cause))?;
    let normal = frame.normal();
    let at = |depth: f64| std::array::from_fn(|i| frame.origin[i] + depth * normal[i]);
    let radius_mm = resolved.diameter_mm / 2.;
    let tool = CircularPocketTool {
        frame: PlaneFrame {
            origin: at(-overrun),
            ..frame
        },
        center_uv: [0.; 2],
        radius_mm,
        displacement_mm: normal.map(|value| value * (reach + overrun)),
        origin: origin(id, "bore"),
    };
    let head_seat = resolved.seat.map(|(_, diameter, depth_mm)| FlatSeatProof {
        radius_mm: diameter / 2.,
        depth_mm,
    });
    let (solid, mouths_mm) = if let Some(seat) = head_seat {
        stock.rust_connection_counterbore(
            &tool,
            seat.radius_mm,
            seat.depth_mm,
            resolved.minimum_wall_mm,
            origin(id, "seat"),
            policy,
        )
    } else {
        stock.rust_connection_through_bore(&tool, resolved.minimum_wall_mm, policy)
    }
    .map_err(|cause| error(Some(id), cause))?;
    let expected = [frame.origin, at(reach - overrun)];
    if mouths_mm
        .into_iter()
        .zip(expected)
        .any(|(actual, required)| {
            let [x, y, z] = std::array::from_fn(|i| actual[i] - required[i]);
            x.hypot(y).hypot(z) > tolerance
        })
    {
        return Err(fail(
            "Receiving bore does not enter or exit at the declared stock stations",
        ));
    }
    Ok((
        solid,
        ThroughReceiverProof {
            mouths_mm,
            radius_mm,
            protected_radius_mm: radius_mm + resolved.minimum_wall_mm,
            head_seat,
        },
    ))
}

#[cfg(test)]
mod tests;

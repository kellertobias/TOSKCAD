//! An unseated, exact blind metric tap in previously unmachined orthogonal stock.
use super::*;
use multicad_core::model::{ConnectionReceiverCut, ThreadFamily};
use multicad_kernel::BlindThreadCut;

pub(super) fn build(
    inputs: Inputs<'_>,
    id: EntityId,
    source: &ConnectionReceiverDefinition,
) -> Result<Solid, ModelError> {
    let fail = |message| error(Some(id), message);
    let policy = inputs.policy;
    let resolved = source
        .resolve_current(
            inputs.document,
            inputs.parameters,
            inputs.revision,
            inputs.snapshots,
        )
        .map_err(|cause| error(Some(id), cause))?;
    let (Some(depth), Some(floor), Some(mut thread)) =
        (resolved.depth_mm, resolved.floor_mm, resolved.thread)
    else {
        return Err(fail(
            "Rust tapped receivers require an explicit blind depth and floor",
        ));
    };
    if resolved.seat.is_some() {
        return Err(fail("Rust blind taps require an unseated entry"));
    }
    if !matches!(&source.cut, ConnectionReceiverCut::Tapped { thread } if thread.family == ThreadFamily::IsoMetric)
    {
        return Err(fail(
            "Rust blind taps currently require a modeled ISO metric thread",
        ));
    }
    if (thread.length_mm - depth).abs() > policy.linear_tolerance_mm {
        return Err(fail(
            "Tapped physical thread length must equal the actual receiving depth",
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
    thread.frame = multicad_core::sketch::Frame {
        origin: local.origin_mm,
        u: local.u,
        v: local.v,
        n: local.normal(),
    };
    let (profile, root, entry) = crate::threads::rust_profile(id, &thread)?;
    if root.is_some() {
        return Err(fail(
            "Rust blind taps require the supported flat-root internal profile",
        ));
    }
    let stock = inputs
        .solids
        .get(&source.input.producer)
        .ok_or_else(|| fail("Blind tap predecessor has no current Rust solid"))?;
    stock
        .rust_cut_blind_internal_thread(
            BlindThreadCut {
                profile,
                entry,
                receiving_radius_mm: resolved.diameter_mm / 2.,
                minimum_wall_mm: resolved.minimum_wall_mm.max(thread.minimum_wall_mm),
                minimum_floor_mm: floor,
            },
            id,
            policy,
        )
        .map_err(|cause| error(Some(id), cause))
}

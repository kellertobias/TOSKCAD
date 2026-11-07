//! Native connection receiving cuts. All tools live in receiving-producer space.
use crate::{ModelError, error};
use multicad_core::{
    document::{Document, EntityId},
    model::{ConnectionReceiverDefinition, ThreadMode},
    parameters::EvaluatedParameters,
    references::{Origin, ReferenceFrame, TopologySnapshot},
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, RevolutionAxis,
    Solid,
};
use std::{collections::BTreeMap, f64::consts::TAU};
fn native<T>(
    id: EntityId,
    result: Result<T, multicad_kernel::GeometryError>,
) -> Result<T, ModelError> {
    result.map_err(|e| error(Some(id), e))
}
fn profile(frame: PlaneFrame, r: f64) -> CurvedProfile {
    CurvedProfile {
        frame,
        outer: vec![CurvedEdge2d::Circle {
            center: [0., 0.],
            radius_mm: r,
        }],
        holes: vec![],
    }
}
fn covered(
    id: EntityId,
    stock: &Solid,
    required: &Solid,
    policy: GeometryPolicy,
    label: &str,
) -> Result<(), ModelError> {
    if native(
        id,
        required.boolean(stock, BooleanOperation::Subtract, policy),
    )?
    .is_some()
    {
        return Err(error(
            Some(id),
            format!(
                "Receiving {label} is incomplete: cut breaks out, crosses a void, or lacks the declared wall/floor"
            ),
        ));
    }
    Ok(())
}
fn origin(id: EntityId, role: &str) -> Origin {
    Origin {
        source: id,
        role: format!("connection.receiver/{role}"),
    }
}
/// Exact meridian annulus or cone, named per actual generating edge.
fn meridian(
    id: EntityId,
    frame: PlaneFrame,
    points: &[[f64; 2]],
    label: &str,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let normal = frame.normal();
    let section = CurvedProfile {
        frame: PlaneFrame {
            origin: frame.origin,
            u: frame.u,
            v: normal,
        },
        outer: (0..points.len())
            .map(|i| CurvedEdge2d::Line {
                a: points[i],
                b: points[(i + 1) % points.len()],
            })
            .collect(),
        holes: vec![],
    };
    let names = (0..points.len())
        .map(|i| origin(id, &format!("{label}/edge/{i}")))
        .collect::<Vec<_>>();
    native(
        id,
        Solid::revolve_curved_profile_named(
            &section,
            RevolutionAxis {
                origin_mm: frame.origin,
                direction: normal,
            },
            TAU,
            id,
            &names,
            policy,
        ),
    )
}
fn wall(
    id: EntityId,
    frame: PlaneFrame,
    r0: f64,
    r1: f64,
    depth: f64,
    width: f64,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    meridian(
        id,
        frame,
        &[[r0, 0.], [r0 + width, 0.], [r1 + width, depth], [r1, depth]],
        "required.wall",
        policy,
    )
}
fn subtract(
    id: EntityId,
    stock: &Solid,
    tool: &Solid,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let exact = GeometryPolicy {
        boolean_fuzzy_mm: 0.,
        ..policy
    };
    let common = native(id, stock.boolean(tool, BooleanOperation::Intersect, exact))?
        .ok_or_else(|| error(Some(id), "Receiving tool removes no positive material"))?;
    if native(id, common.volume_mm3())? <= policy.minimum_feature_mm.powi(3) {
        return Err(error(
            Some(id),
            "Receiving tool removes no policy-sized positive material",
        ));
    }
    let before = native(id, stock.volume_mm3())?;
    let result = native(id, stock.boolean(tool, BooleanOperation::Subtract, policy))?
        .ok_or_else(|| error(Some(id), "Receiving cut removes the entire Part"))?;
    if native(id, result.solid_count())? != 1 || native(id, result.volume_mm3())? >= before {
        return Err(error(
            Some(id),
            "Receiving cut splits the Part or removes no verifiable material",
        ));
    }
    Ok(result)
}
pub(crate) struct Inputs<'a> {
    pub document: &'a Document,
    pub parameters: &'a EvaluatedParameters,
    pub revision: u64,
    pub snapshots: &'a BTreeMap<EntityId, TopologySnapshot>,
    pub solids: &'a BTreeMap<EntityId, Solid>,
    pub policy: GeometryPolicy,
}
pub(crate) fn build(
    inputs: Inputs<'_>,
    id: EntityId,
    source: &ConnectionReceiverDefinition,
) -> Result<Solid, ModelError> {
    let policy = inputs.policy;
    let resolved = source
        .resolve_current(
            inputs.document,
            inputs.parameters,
            inputs.revision,
            inputs.snapshots,
        )
        .map_err(|e| error(Some(id), e))?;
    let pose = source
        .local_pose(
            inputs.document,
            inputs.parameters,
            inputs.revision,
            inputs.snapshots,
        )
        .map_err(|e| error(Some(id), e))?;
    let world = pose.frame(ReferenceFrame {
        origin_mm: [0.; 3],
        u: [1., 0., 0.],
        v: [0., 1., 0.],
    });
    let frame = PlaneFrame {
        origin: world.origin_mm,
        u: world.u,
        v: world.v,
    };
    let target = inputs.solids.get(&source.input.producer).ok_or_else(|| {
        error(
            Some(id),
            "Receiving predecessor has no current native solid",
        )
    })?;
    if native(id, target.solid_count())? != 1 {
        return Err(error(
            Some(id),
            "Receiving cuts require one connected predecessor solid",
        ));
    }
    let r = resolved.diameter_mm / 2.;
    let wall_width = resolved.minimum_wall_mm;
    if r < policy.minimum_feature_mm || wall_width < policy.minimum_feature_mm {
        return Err(error(
            Some(id),
            "Receiving bore and wall are below the native minimum feature",
        ));
    }
    // A receiving bore must actually enter from free space at this station,
    // not create an inaccessible enclosed cavity inside the stock.
    let entry_radius = resolved.seat.map_or(r, |(_, diameter, _)| diameter / 2.) + wall_width;
    let entry_frame = PlaneFrame {
        origin: std::array::from_fn(|i| {
            frame.origin[i] - frame.normal()[i] * policy.minimum_feature_mm
        }),
        ..frame
    };
    let before_entry = native(
        id,
        Solid::extrude_curved_profile_named(
            &profile(entry_frame, entry_radius),
            policy.minimum_feature_mm,
            id,
            &[origin(id, "required.entry.free")],
            policy,
        ),
    )?;
    if native(
        id,
        target.boolean(
            &before_entry,
            BooleanOperation::Intersect,
            GeometryPolicy {
                boolean_fuzzy_mm: 0.,
                ..policy
            },
        ),
    )?
    .is_some()
    {
        return Err(error(
            Some(id),
            "Receiving station is inside material instead of an accessible entry surface",
        ));
    }
    let (depth, cut_depth) = if let Some(depth) = resolved.depth_mm {
        (depth, depth)
    } else {
        let reach = native(id, target.transfer_through_depth(frame, false, policy))?;
        let tolerance = policy
            .linear_tolerance_mm
            .max(target.geometry_policy().linear_tolerance_mm);
        let actual = reach - policy.minimum_feature_mm.max(8. * tolerance);
        (actual, reach)
    };
    if depth < policy.minimum_feature_mm {
        return Err(error(
            Some(id),
            "Receiving depth is below the native minimum feature",
        ));
    }
    // Bounds establish reach only. This volumetric witness proves the complete
    // annular stock at every axial station, including concavities and voids.
    let required = wall(id, frame, r, r, depth, wall_width, policy)?;
    covered(id, target, &required, policy, "axial wall")?;
    if let Some(floor) = resolved.floor_mm {
        if floor < policy.minimum_feature_mm {
            return Err(error(
                Some(id),
                "Receiving floor is below the native minimum feature",
            ));
        }
        let floor_frame = PlaneFrame {
            origin: std::array::from_fn(|i| frame.origin[i] + frame.normal()[i] * depth),
            ..frame
        };
        let required = native(
            id,
            Solid::extrude_curved_profile_named(
                &profile(floor_frame, r + wall_width),
                floor,
                id,
                &[origin(id, "required.floor")],
                policy,
            ),
        )?;
        covered(id, target, &required, policy, "blind floor")?;
    }
    if let Some((cone, diameter, seat_depth)) = resolved.seat {
        if seat_depth >= depth {
            return Err(error(
                Some(id),
                "Receiving seat must end before the complete bore exits",
            ));
        }
        let rr = diameter / 2.;
        let end = if cone { r } else { rr };
        let required = wall(id, frame, rr, end, seat_depth, wall_width, policy)?;
        covered(id, target, &required, policy, "seat profile wall")?;
    }
    let mut result = if let Some(mut thread) = resolved.thread {
        if (thread.length_mm - depth).abs() > policy.linear_tolerance_mm {
            return Err(error(
                Some(id),
                "Tapped physical thread length must equal the actual receiving depth; no thread length is inferred or rewritten",
            ));
        }
        // An exact shared-axis pose replaces only the identity source frame.
        // Thread length, diameter, fit and every authored profile remain intact.
        thread.frame = multicad_core::sketch::Frame {
            origin: frame.origin,
            u: frame.u,
            v: frame.v,
            n: frame.normal(),
        };
        crate::threads::build_resolved(id, *thread, ThreadMode::CutInternal, Some(target), policy)?
    } else {
        native(
            id,
            target.cut_transfer_profile_named(
                &profile(frame, r),
                cut_depth,
                id,
                &[origin(id, "bore")],
                policy,
            ),
        )?
    };
    if let Some((cone, diameter, seat_depth)) = resolved.seat {
        let rr = diameter / 2.;
        if cone {
            let tool = meridian(
                id,
                frame,
                &[[0., 0.], [rr, 0.], [r, seat_depth], [0., seat_depth]],
                "countersink",
                policy,
            )?;
            result = subtract(id, &result, &tool, policy)?;
        } else {
            result = native(
                id,
                result.cut_transfer_profile_named(
                    &profile(frame, rr),
                    seat_depth,
                    id,
                    &[origin(id, "counterbore")],
                    policy,
                ),
            )?;
        }
    }
    let topology = native(id, result.topology_snapshot(id, inputs.revision))?;
    if topology.elements.iter().any(|e| {
        e.key.kind == multicad_core::references::TopologyKind::Face && e.origins.is_empty()
    }) {
        return Err(error(
            Some(id),
            "Receiving cut has incomplete native face ancestry",
        ));
    }
    Ok(result)
}

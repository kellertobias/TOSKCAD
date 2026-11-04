use super::*;
use multicad_kernel::RevolutionAxis;

/// Local r/z mask. Apply before placement so endpoints never depend on hand.
pub(super) fn apply(
    feature: EntityId,
    tool: Solid,
    resolved: &ResolvedThread,
    mode: ThreadMode,
    policy: GeometryPolicy,
) -> Result<Solid, ModelError> {
    let Some(t) = &resolved.end_treatments else {
        return Ok(tool);
    };
    let internal = mode == ThreadMode::CutInternal;
    let major = resolved.major_radius_mm;
    let minor = major - resolved.profile.depth_mm;
    let c = t.start_chamfer_radial_depth_mm;
    let w = t.end_relief_length_mm;
    if c == 0. && w == 0. {
        return Ok(tool);
    }
    let end = t.end_relief_radial_depth_mm;
    let l = resolved.length_mm;
    let middle = if internal { minor } else { major };
    let start = if internal { minor + c } else { major - c };
    let tail = if w == 0. {
        middle
    } else if internal {
        major + end
    } else {
        minor - end
    };
    let mut points = vec![[0., 0.], [start, 0.]];
    if c > 0. {
        points.push([middle, c]);
    }
    points.push([middle, l - w]);
    if w > 0. {
        points.push([tail, l - w]);
        points.push([tail, l]);
    }
    points.push([0., l]);
    points.dedup();
    let edges = (0..points.len())
        .map(|i| CurvedEdge2d::Line {
            a: points[i],
            b: points[(i + 1) % points.len()],
        })
        .collect::<Vec<_>>();
    let origins = (0..edges.len())
        .map(|i| Origin {
            source: feature,
            role: format!("thread.custom_end_treatment/edge/{i}"),
        })
        .collect::<Vec<_>>();
    let mask = Solid::revolve_curved_profile_named(
        &CurvedProfile {
            frame: PlaneFrame {
                origin: [0.; 3],
                u: [1., 0., 0.],
                v: [0., 0., 1.],
            },
            outer: edges,
            holes: vec![],
        },
        RevolutionAxis {
            origin_mm: [0.; 3],
            direction: [0., 0., 1.],
        },
        std::f64::consts::TAU,
        feature,
        &origins,
        policy,
    )
    .map_err(|e| error(Some(feature), e))?;
    let treated = boolean(
        feature,
        &tool,
        &mask,
        if internal {
            BooleanOperation::Union
        } else {
            BooleanOperation::Intersect
        },
        policy,
    )?
    .ok_or_else(|| {
        error(
            Some(feature),
            "Thread treatment removed the complete native solid",
        )
    })?;
    if treated.solid_count().map_err(|e| error(Some(feature), e))? != 1 {
        return Err(error(
            Some(feature),
            "Thread end treatment disconnected native geometry",
        ));
    }
    Ok(treated)
}

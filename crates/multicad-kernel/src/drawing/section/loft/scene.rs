use super::*;
use multicad_rust_kernel::{
    curved_projection::Curve as RoundCurve,
    planar_boolean::{
        LoftRoundCutEdge, LoftSceneCut, LoftSceneMember, project_loft_scene,
        project_loft_scene_section,
    },
    planar_projection::Options,
    planar_section,
    round_occlusion::RoundOccluder,
};

pub(in crate::drawing) fn project_scene(
    members: &[(usize, &Solid)],
    options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    project(members, options, None).map(|(vectors, _)| vectors)
}

pub(in crate::drawing) fn project_retained_scene(
    solids: &[DrawingSolid<'_>],
    options: DrawingProjection,
    depth: f64,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    if options.detail.is_some() {
        return Err(err(
            GeometryErrorKind::Unsupported,
            "Rust retained analytic assembly views do not support a combined detail window",
        ));
    }
    let members = projection_members(solids);
    let cut = LoftSceneCut {
        near: depth,
        far: options.section_depth_mm.map(|distance| depth - distance),
        windows: [options.section_u_mm, options.section_v_mm],
    };
    let (mut vectors, clipped) = project(&members, options, Some(cut))?;
    let section_options = DrawingProjection {
        section_only: true,
        section_depth_mm: None,
        maximum_curves: options.maximum_curves - vectors.len() as u32,
        maximum_points: options.maximum_points
            - vectors
                .iter()
                .map(|vector| vector.preview_mm.len() as u32)
                .sum::<u32>(),
        ..options
    };
    let section_inputs = clipped
        .into_iter()
        .enumerate()
        .filter(|&(_, clipped)| clipped)
        .map(|(index, _)| {
            let (source, solid) = members[index];
            (
                source,
                DrawingSolid {
                    solid,
                    hatch: solids[source].hatch,
                },
            )
        });
    vectors.extend(machined::project_members(
        section_inputs,
        section_options,
        depth,
    )?);
    Ok(vectors)
}

fn project(
    members: &[(usize, &Solid)],
    options: DrawingProjection,
    cut: Option<LoftSceneCut>,
) -> Result<(Vec<ProjectedVector>, Vec<bool>), GeometryError> {
    let axes = planar::view_axes(options.frame);
    let rounds: Vec<_> = members
        .iter()
        .map(|(_, solid)| {
            if solid.nonplanar_loft_body().is_some() {
                return Ok(None);
            }
            let Some(body) = curved::round_occluder(solid, options.frame)? else {
                return Ok(None);
            };
            let retained = cut
                .map(|cut| retained_round(solid, &body, axes, options, cut))
                .transpose()?
                .flatten();
            Ok(Some((body, retained)))
        })
        .collect::<Result<_, _>>()?;
    let is_planar =
        |solid: &Solid| solid.nonplanar_loft_body().is_none() && solid.sphere_body().is_none();
    let planar_inputs: Vec<_> = members
        .iter()
        .zip(&rounds)
        .filter(|((_, solid), round)| round.is_none() && is_planar(solid))
        .map(|((_, solid), _)| DrawingSolid { solid, hatch: None })
        .collect();
    let (planar_bodies, tolerance) = planar::prepare(&planar_inputs, options.frame)?;
    let tolerance = members.iter().fold(tolerance, |value, (_, solid)| {
        value.max(solid.geometry_policy().linear_tolerance_mm)
    });
    let mut clipped = vec![false; members.len()];
    let planar_bodies = if let Some(cut) = cut {
        let retained = planar_section::retain(
            planar_bodies,
            planar_section::Options {
                depth: cut.near,
                windows: cut.windows,
                tolerance,
                segment_limit: (options.maximum_curves as usize)
                    .min(options.maximum_points as usize / 2),
            },
            cut.far,
        )
        .map_err(planar::projection_error)?;
        for ((index, _), (body, trimmed)) in members
            .iter()
            .enumerate()
            .zip(&rounds)
            .filter(|((_, (_, solid)), round)| round.is_none() && is_planar(solid))
            .map(|(member, _)| member)
            .zip(retained.bodies.iter().zip(retained.trimmed))
        {
            clipped[index] = trimmed && !body.faces.is_empty();
        }
        retained.bodies
    } else {
        planar_bodies
    };
    let mut planar_bodies = planar_bodies.iter();
    let scene: Vec<_> = members
        .iter()
        .zip(&rounds)
        .map(|((_, solid), round)| {
            if let Some((body, placement)) = solid.nonplanar_loft_body() {
                let Frame { axes, offset } = Frame::new(placement, options.frame.origin, axes);
                LoftSceneMember::Loft {
                    body: body.loft_geometry(),
                    axes,
                    offset,
                }
            } else if let Some((body, placement)) = solid.sphere_body() {
                LoftSceneMember::Sphere {
                    center: sphere::drawing_center(placement.origin, options.frame),
                    radius: body.radius_mm(),
                }
            } else if let Some((body, retained)) = round {
                if let Some(retained) = retained {
                    LoftSceneMember::RetainedRound {
                        body,
                        edges: &retained.edges,
                        near_trimmed: retained.near_trimmed,
                    }
                } else {
                    LoftSceneMember::Round(body)
                }
            } else {
                LoftSceneMember::Planar(planar_bodies.next().expect("prepared every planar member"))
            }
        })
        .collect();
    let projection = Options {
        tolerance,
        hidden_lines: options.hidden_lines,
        tangent_edges: options.tangent_edges,
        detail: options.detail,
        segment_limit: options.maximum_curves as usize,
    };
    let edges = if let Some(cut) = cut {
        project_loft_scene_section(&scene, projection, cut).map(|result| {
            for index in result.clipped_members {
                clipped[index] = true;
            }
            result.edges
        })
    } else {
        project_loft_scene(&scene, projection)
    }
    .map_err(|error| match error {
        Error::Invalid | Error::Unresolved => err(
            GeometryErrorKind::InvalidResult,
            "Invalid loft drawing scene boundary",
        ),
        Error::Limit => limit(),
    })?;
    let mut points = options.maximum_points as usize;
    let vectors = edges
        .into_iter()
        .map(|(source, edge)| {
            let source = members[source].0;
            let role = if edge.hidden {
                VectorRole::Hidden
            } else {
                VectorRole::Visible
            };
            if let Some(ellipse) = edge.ellipse {
                let curve = multicad_rust_kernel::curved_projection::Curve::Ellipse(ellipse);
                let preview_mm = curve.preview(options.chord_tolerance_mm, points).map_err(
                    |cause| match cause {
                        multicad_rust_kernel::curved_projection::Error::Invalid => err(
                            GeometryErrorKind::InvalidResult,
                            format!("Invalid ellipse in loft drawing scene: {ellipse:?}"),
                        ),
                        _ => curved::projection_error(cause),
                    },
                )?;
                charge(&mut points, preview_mm.len())?;
                Ok(ProjectedVector {
                    source_index: source,
                    role,
                    curve: curved::ellipse_vector(ellipse),
                    preview_mm,
                })
            } else {
                publish_one(
                    source,
                    map_curve(edge.curve, |[u, v, _]| [u, v]),
                    role,
                    options,
                    &mut points,
                )
            }
        })
        .collect::<Result<_, _>>()?;
    Ok((vectors, clipped))
}

#[derive(Default)]
struct RoundCut {
    edges: Vec<LoftRoundCutEdge>,
    near_trimmed: bool,
}

fn retained_round(
    solid: &Solid,
    body: &RoundOccluder,
    axes: [[f64; 3]; 3],
    options: DrawingProjection,
    cut: LoftSceneCut,
) -> Result<Option<RoundCut>, GeometryError> {
    let [min, max] = body.bounds();
    let tolerance = solid.geometry_policy().linear_tolerance_mm;
    let limits = [
        cut.windows[0],
        cut.windows[1],
        Some([cut.far.unwrap_or(min[2] - 2. * tolerance), cut.near]),
    ];
    if limits.iter().enumerate().all(|(axis, span)| {
        span.is_none_or(|[lo, hi]| min[axis] >= lo - tolerance && max[axis] <= hi + tolerance)
    }) {
        return Ok(None);
    }
    let mut retained = RoundCut::default();
    if limits.iter().enumerate().any(|(axis, span)| {
        span.is_some_and(|[lo, hi]| max[axis] <= lo + tolerance || min[axis] >= hi - tolerance)
    }) {
        return Ok(Some(retained));
    }
    let mut previous = Vec::<(usize, f64)>::new();
    for axis in [2, 0, 1] {
        let Some(span) = limits[axis] else { continue };
        let [u, v] = [(axis + 1) % 3, (axis + 2) % 3];
        let section = DrawingProjection {
            frame: PlaneFrame {
                u: axes[u],
                v: axes[v],
                ..options.frame
            },
            section_only: true,
            section_u_mm: limits[u],
            section_v_mm: limits[v],
            ..options
        };
        for level in span {
            if level <= min[axis] + tolerance || level >= max[axis] - tolerance {
                continue;
            }
            let edges = super::super::annulus::circular_edges(solid, section, level)?;
            if edges.is_empty() {
                continue;
            }
            retained.near_trimmed |= axis == 2 && level == cut.near;
            retained.edges.extend(edges.into_iter().filter_map(|curve| {
                if let RoundCurve::Line(ends) = curve
                    && previous.iter().any(|&(old_axis, old_level)| {
                        let index = usize::from(old_axis == v);
                        old_axis != axis
                            && ends
                                .iter()
                                .all(|p| (p[index] - old_level).abs() <= tolerance)
                    })
                {
                    return None;
                }
                Some(LoftRoundCutEdge { axis, level, curve })
            }));
            previous.push((axis, level));
        }
    }
    Ok(Some(retained))
}

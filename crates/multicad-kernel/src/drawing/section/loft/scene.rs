use super::*;
use multicad_rust_kernel::{
    planar_boolean::{
        LoftSceneCut, LoftSceneMember, project_loft_scene, project_loft_scene_section,
    },
    planar_projection::Options,
    planar_section,
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
            "Rust retained loft assemblies do not support a combined detail window",
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
    let planar_inputs: Vec<_> = members
        .iter()
        .filter(|(_, solid)| solid.nonplanar_loft_body().is_none())
        .map(|(_, solid)| DrawingSolid { solid, hatch: None })
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
            .filter(|(_, (_, solid))| solid.nonplanar_loft_body().is_none())
            .zip(retained.bodies.iter().zip(retained.trimmed))
        {
            clipped[index] = trimmed && !body.faces.is_empty();
        }
        retained.bodies
    } else {
        planar_bodies
    };
    let axes = planar::view_axes(options.frame);
    let mut planar_bodies = planar_bodies.iter();
    let scene: Vec<_> = members
        .iter()
        .map(|(_, solid)| {
            if let Some((body, placement)) = solid.nonplanar_loft_body() {
                let Frame { axes, offset } = Frame::new(placement, options.frame.origin, axes);
                LoftSceneMember::Loft {
                    body: body.loft_geometry(),
                    axes,
                    offset,
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
            for index in result.clipped_lofts {
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
    let vectors = publish(
        edges.into_iter().map(|(source, edge)| {
            (
                members[source].0,
                map_curve(edge.curve, |[u, v, _]| [u, v]),
                if edge.hidden {
                    VectorRole::Hidden
                } else {
                    VectorRole::Visible
                },
            )
        }),
        options,
    )?;
    Ok((vectors, clipped))
}

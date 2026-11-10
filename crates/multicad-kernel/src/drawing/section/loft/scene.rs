use super::*;
use multicad_rust_kernel::{
    planar_boolean::{LoftSceneMember, project_loft_scene},
    planar_projection::Options,
};

pub(in crate::drawing) fn project_scene(
    members: &[(usize, &Solid)],
    options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let planar_inputs: Vec<_> = members
        .iter()
        .filter(|(_, solid)| solid.nonplanar_loft_body().is_none())
        .map(|(_, solid)| DrawingSolid { solid, hatch: None })
        .collect();
    let (planar_bodies, tolerance) = planar::prepare(&planar_inputs, options.frame)?;
    let tolerance = members.iter().fold(tolerance, |value, (_, solid)| {
        value.max(solid.geometry_policy().linear_tolerance_mm)
    });
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
    let edges = project_loft_scene(
        &scene,
        Options {
            tolerance,
            hidden_lines: options.hidden_lines,
            tangent_edges: options.tangent_edges,
            detail: options.detail,
            segment_limit: options.maximum_curves as usize,
        },
    )
    .map_err(|error| match error {
        Error::Invalid | Error::Unresolved => err(
            GeometryErrorKind::InvalidResult,
            "Invalid loft drawing scene boundary",
        ),
        Error::Limit => limit(),
    })?;
    publish(
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
    )
}

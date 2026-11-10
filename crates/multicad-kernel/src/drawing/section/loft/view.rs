use super::*;
use multicad_rust_kernel::planar_boolean::LoftProjection;

pub(in crate::drawing) fn project_view(
    input: &DrawingSolid<'_>,
    options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let (body, placement) = input
        .solid
        .nonplanar_loft_body()
        .expect("loft drawing source");
    let frame = Frame::new(
        placement,
        options.frame.origin,
        planar::view_axes(options.frame),
    );
    let tolerance = input.solid.geometry_policy().linear_tolerance_mm;
    let mut stations = [0., 1.];
    let mut section = None;
    if let Some(depth) = options.section_offset_mm {
        if options.section_u_mm.is_some()
            || options.section_v_mm.is_some()
            || options.detail.is_some()
            || frame.axes[2][0].hypot(frame.axes[2][1]) > 64. * f64::EPSILON
        {
            return Err(err(
                GeometryErrorKind::Unsupported,
                "Rust retained twisted loft drawings currently require an axial section without lateral clipping",
            ));
        }
        let height = body.drawing_height() * frame.axes[2][2];
        let near = (depth - frame.offset[2]) / height;
        let far = options
            .section_depth_mm
            .map_or(if height > 0. { 0. } else { 1. }, |distance| {
                (depth - distance - frame.offset[2]) / height
            });
        stations = if height > 0. {
            [far, near]
        } else {
            [near, far]
        };
        stations = [stations[0].max(0.), stations[1].min(1.)];
        let t = tolerance / height.abs();
        if stations[1] - stations[0] <= t {
            return Ok(Vec::new());
        }
        if (t..1. - t).contains(&near) {
            section = Some(depth);
        }
    }
    let detail = options
        .detail
        .map(|([u, v], radius)| ([u - frame.offset[0], v - frame.offset[1]], radius));
    let edges = body
        .drawing_edges(LoftProjection {
            axes: frame.axes,
            stations,
            tolerance,
            hidden_lines: options.hidden_lines,
            detail,
            edge_limit: options.maximum_curves as usize,
        })
        .map_err(|error| match error {
            Error::Invalid => err(
                GeometryErrorKind::Input,
                "Invalid twisted loft drawing frame",
            ),
            Error::Unresolved => err(
                GeometryErrorKind::InvalidResult,
                "Invalid twisted loft projection",
            ),
            Error::Limit => limit(),
        })?;
    let mut output = publish(
        edges.into_iter().map(|edge| {
            let curve = map_curve(edge.curve, |[u, v, _]| {
                [u + frame.offset[0], v + frame.offset[1]]
            });
            let role = if edge.hidden {
                VectorRole::Hidden
            } else {
                VectorRole::Visible
            };
            (curve, role)
        }),
        options,
        0,
    )?;
    if let Some(depth) = section {
        let points = output
            .iter()
            .map(|vector| vector.preview_mm.len() as u32)
            .sum::<u32>();
        let section_options = DrawingProjection {
            maximum_curves: options.maximum_curves - output.len() as u32,
            maximum_points: options.maximum_points - points,
            section_only: true,
            section_depth_mm: None,
            ..options
        };
        output.extend(machined::project(
            &[DrawingSolid {
                solid: input.solid,
                hatch: input.hatch,
            }],
            section_options,
            depth,
        )?);
    }
    Ok(output)
}

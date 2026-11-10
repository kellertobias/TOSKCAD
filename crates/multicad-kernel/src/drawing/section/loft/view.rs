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
    let mut depth_clip = None;
    let mut section = None;
    let mut windows = [None; 2];
    if let Some(depth) = options.section_offset_mm {
        if options.detail.is_some() {
            return Err(err(
                GeometryErrorKind::Unsupported,
                "Rust retained twisted loft drawings do not support a combined detail window",
            ));
        }
        windows = [options.section_u_mm, options.section_v_mm];
        for (axis, window) in windows.iter_mut().enumerate() {
            if let Some(range) = window {
                *range = range.map(|value| value - frame.offset[axis]);
                let [min, max] = body.drawing_depth_span(frame.axes[axis]);
                if range[1] <= min + tolerance || range[0] >= max - tolerance {
                    return Ok(Vec::new());
                }
                if range[0] <= min + tolerance && range[1] >= max - tolerance {
                    *window = None;
                }
            }
        }
        let windowed = windows.iter().any(Option::is_some);
        let n = frame.axes[2];
        let near = depth - frame.offset[2];
        let [min, max] = body.drawing_depth_span(n);
        let far = options
            .section_depth_mm
            .map_or(min, |distance| near - distance);
        if near.min(max) - far.max(min) <= tolerance {
            return Ok(Vec::new());
        }
        let truncated = near < max - tolerance || far > min + tolerance;
        if near <= max + tolerance && (truncated || windowed) {
            section = Some(depth);
        }
        if !windowed && n[0].hypot(n[1]) <= 64. * f64::EPSILON {
            let height = body.drawing_height() * n[2];
            stations = if height > 0. {
                [far / height, near / height]
            } else {
                [near / height, far / height]
            };
            stations = [stations[0].max(0.), stations[1].min(1.)];
        } else if truncated {
            depth_clip = Some([far.max(min), near.min(max)]);
        }
    }
    let detail = options
        .detail
        .map(|([u, v], radius)| ([u - frame.offset[0], v - frame.offset[1]], radius));
    let edges = body
        .drawing_edges(LoftProjection {
            axes: frame.axes,
            stations,
            depth: depth_clip,
            windows,
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
    if let Some(depth) = section
        && !output.is_empty()
    {
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

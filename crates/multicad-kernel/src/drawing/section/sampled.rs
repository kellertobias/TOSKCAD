//! Clipping and hatching for tolerance-bounded section loops.
use super::*;
use multicad_rust_kernel::warped_union::{self, SectionClipPlane, SectionEdge};

pub(in crate::drawing) fn publish(
    input: &DrawingSolid<'_>,
    rings: Vec<Vec<[f64; 2]>>,
    options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let mut curves = options.maximum_curves as usize;
    let mut points = options.maximum_points as usize;
    let tolerance = input.solid.geometry_policy().linear_tolerance_mm;
    let loops = rings
        .into_iter()
        .map(|ring| {
            ring.iter()
                .zip(ring.iter().cycle().skip(1))
                .map(|(&a, &b)| SectionEdge {
                    ends_mm: [a, b].map(|[x, y]| [x, y, 0.]),
                    curve: warped_union::Curve::Line,
                })
                .collect()
        })
        .collect();
    let planes: Vec<_> = [options.section_u_mm, options.section_v_mm]
        .into_iter()
        .enumerate()
        .filter_map(|(axis, window)| window.map(|window| (axis, window)))
        .flat_map(|(axis, [lo, hi])| {
            [(lo, 1.), (hi, -1.)].map(|(level, sign)| SectionClipPlane {
                origin_mm: std::array::from_fn(|i| if i == axis { level } else { 0. }),
                inward_normal: std::array::from_fn(|i| if i == axis { sign } else { 0. }),
            })
        })
        .collect();
    let loops = warped_union::clip_section_loops(
        loops,
        [0., 0., 1.],
        &planes,
        tolerance,
        curves.min(points / 2),
    )
    .map_err(|cause| match cause {
        warped_union::Error::Limit => err(
            GeometryErrorKind::Limit,
            "Rust sampled section exceeded its output budget",
        ),
        _ => err(
            GeometryErrorKind::InvalidResult,
            "Invalid sampled section boundary",
        ),
    })?;
    let boundary: Vec<_> = loops
        .into_iter()
        .flatten()
        .map(|edge| edge.ends_mm.map(|[x, y, _]| [x, y]))
        .collect();
    curves -= boundary.len();
    points -= 2 * boundary.len();
    let mut output: Vec<_> = boundary
        .iter()
        .map(|&ends| planar::line(0, VectorRole::Section, ends))
        .collect();
    if let Some(hatch) = input.hatch {
        output.extend(section::publish_hatch(
            0,
            hatch,
            &boundary,
            tolerance,
            &mut curves,
            &mut points,
            &mut section_hatch::Budget::default(),
        )?);
    }
    Ok(output)
}

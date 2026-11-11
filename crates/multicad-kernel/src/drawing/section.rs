//! Publish section boundaries and hatches from exact planar Rust geometry.
use super::*;
use multicad_core::materials::HatchPattern;
use multicad_rust_kernel::{planar_section, section_hatch};

mod annulus;
mod end_on;
mod loft;
mod machined;
mod retained;
mod warped;
pub(super) use annulus::{project_circular, project_concentric};
pub(super) use loft::{project_scene as project_loft_scene, project_view as project_loft_view};

pub(super) fn project(
    solids: &[DrawingSolid<'_>],
    options: DrawingProjection,
    depth: f64,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    if !options.section_only
        && let [input] = solids
        && input.solid.nonplanar_loft_body().is_some()
    {
        return project_loft_view(input, options);
    }
    if !options.section_only
        && solids
            .iter()
            .any(|input| contains_loft(input.solid) || contains_sphere(input.solid))
    {
        return loft::project_retained_scene(solids, options, depth);
    }
    if !options.section_only
        && options.detail.is_none()
        && let Some(result) = warped::project_retained(solids, options, depth)
    {
        return result;
    }
    if options.detail.is_none()
        && solids
            .iter()
            .any(|input| machined::needs_analytic_section(input.solid))
    {
        if options.section_only {
            return machined::project(solids, options, depth);
        }
        if !options.hidden_lines && !options.tangent_edges {
            return match retained::project(solids, options, depth) {
                Err(error) if error.kind == GeometryErrorKind::Unsupported => {
                    end_on::project(solids, options, depth)
                }
                result => result,
            };
        }
    }
    project_planar(solids, options, depth)
}

fn project_planar(
    solids: &[DrawingSolid<'_>],
    options: DrawingProjection,
    depth: f64,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    if options.detail.is_some() {
        return Err(err(
            GeometryErrorKind::Unsupported,
            "Rust planar section projection does not support a combined detail crop",
        ));
    }
    let (bodies, tolerance) = planar::prepare(solids, options.frame)?;
    let mut curves = options.maximum_curves as usize;
    let mut points = options.maximum_points as usize;
    let section = planar_section::Options {
        depth,
        windows: [options.section_u_mm, options.section_v_mm],
        tolerance,
        segment_limit: curves.min(points / 2),
    };
    let mut boundaries =
        planar_section::section(&bodies, section).map_err(planar::projection_error)?;
    let mut output = Vec::new();
    if !options.section_only {
        let far = options.section_depth_mm.map(|distance| depth - distance);
        let retained =
            planar_section::retain(bodies, section, far).map_err(planar::projection_error)?;
        if retained.bodies.iter().all(|body| body.faces.is_empty()) {
            return Ok(output);
        }
        for ((boundary, body), changed) in boundaries
            .iter_mut()
            .zip(&retained.bodies)
            .zip(retained.trimmed)
        {
            if !changed || body.faces.is_empty() {
                boundary.clear();
            }
        }
        output = planar::project_bodies(&retained.bodies, options, tolerance)?;
        curves -= output.len();
        points -= 2 * output.len();
    }
    let section_count: usize = boundaries.iter().map(Vec::len).sum();
    if section_count > curves.min(points / 2) {
        return Err(planar::projection_error(
            multicad_rust_kernel::planar_projection::ProjectionError::Limit,
        ));
    }
    curves -= section_count;
    points -= 2 * section_count;
    let mut hatch_budget = section_hatch::Budget::default();
    for (source, (input, boundary)) in solids.iter().zip(boundaries).enumerate() {
        output.extend(
            boundary
                .iter()
                .map(|&ends| planar::line(source, VectorRole::Section, ends)),
        );
        if boundary.is_empty() {
            continue;
        }
        let Some(hatch) = input.hatch else { continue };
        output.extend(publish_hatch(
            source,
            hatch,
            &boundary,
            tolerance,
            &mut curves,
            &mut points,
            &mut hatch_budget,
        )?);
    }
    Ok(output)
}

pub(super) fn publish_hatch(
    source: usize,
    hatch: DrawingHatch,
    boundary: &[[[f64; 2]; 2]],
    tolerance: f64,
    curves: &mut usize,
    points: &mut usize,
    hatch_budget: &mut section_hatch::Budget,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let pattern = match hatch.pattern {
        HatchPattern::None => return Ok(Vec::new()),
        HatchPattern::Lines => section_hatch::Pattern::Lines,
        HatchPattern::Cross => section_hatch::Pattern::Cross,
        HatchPattern::Alternating => section_hatch::Pattern::Alternating,
        HatchPattern::Solid => section_hatch::Pattern::Solid,
        HatchPattern::Dots => section_hatch::Pattern::Dots,
        HatchPattern::SmallCrosses => section_hatch::Pattern::SmallCrosses,
        HatchPattern::Brick => section_hatch::Pattern::Brick,
        HatchPattern::Concrete => section_hatch::Pattern::Concrete,
        HatchPattern::Grass => section_hatch::Pattern::Grass,
    };
    let point_cost = if pattern == section_hatch::Pattern::Solid {
        3
    } else {
        2
    };
    let strokes = section_hatch::hatch_with_budget(
        boundary,
        pattern,
        section_hatch::Options {
            angle_rad: hatch.angle_rad,
            spacing_mm: hatch.spacing_mm,
            line_width_mm: hatch.line_width_mm,
            tolerance_mm: tolerance,
            stroke_limit: (*curves).min(*points / point_cost),
        },
        hatch_budget,
    )
    .map_err(|cause| match cause {
        section_hatch::HatchError::Invalid => err(
            GeometryErrorKind::InvalidResult,
            "Rust drawing section has an invalid hatch region",
        ),
        section_hatch::HatchError::Limit => err(
            GeometryErrorKind::Limit,
            "Rust drawing section exceeded its hatch budget",
        ),
    })?;
    *curves -= strokes.len();
    *points -= point_cost * strokes.len();
    Ok(strokes
        .into_iter()
        .map(|stroke| match stroke {
            section_hatch::Stroke::Line(ends) => planar::line(source, VectorRole::Hatch, ends),
            section_hatch::Stroke::Triangle(points) => ProjectedVector {
                source_index: source,
                role: VectorRole::Hatch,
                curve: VectorCurve::HatchTriangle { points },
                preview_mm: points.to_vec(),
            },
        })
        .collect())
}

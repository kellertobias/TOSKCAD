//! Drawing adapter for exact circular cylinders and annuli.
use super::*;
use multicad_rust_kernel::{
    curved_projection::{self, Curve, Cylinder, Detail, Error},
    round_occlusion::RoundOccluder,
};

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(x, y)| x * y).sum()
}

fn unit(vector: [f64; 3]) -> [f64; 3] {
    let length = dot(vector, vector).sqrt();
    vector.map(|v| v / length)
}

fn projection_error(error: Error) -> GeometryError {
    match error {
        Error::Invalid => err(
            GeometryErrorKind::InvalidResult,
            "Invalid analytic circular solid projection",
        ),
        Error::Limit => err(
            GeometryErrorKind::Limit,
            "Curved drawing projection exceeded its point budget",
        ),
    }
}

pub(super) fn circular_dimensions(solid: &Solid) -> Option<(PlaneFrame, f64, f64, Option<f64>)> {
    if let Some((body, frame)) = solid.revolution_body() {
        return body.circular_sweep(frame);
    }
    if let Some((body, frame)) = solid.annulus_body() {
        let geometry = &body.geometry;
        return Some((
            frame,
            geometry.outer_radius_mm(),
            geometry.height_mm(),
            Some(geometry.inner_radius_mm()),
        ));
    }
    let (cylinder, frame) = solid.axial_cylinder_frame()?;
    Some((
        frame,
        cylinder.circle().radius().as_f64(),
        cylinder.height().as_f64(),
        None,
    ))
}

pub(super) fn is_round(solid: &Solid) -> bool {
    circular_dimensions(solid).is_some()
}

pub(super) fn contains_round(solid: &Solid) -> bool {
    is_round(solid)
        || solid
            .rust_compound()
            .is_some_and(|compound| compound.members().any(contains_round))
}

fn projection_cylinder(solid: &Solid, frame: PlaneFrame) -> Option<(Cylinder, Option<f64>)> {
    let (placement, radius, height, bore) = circular_dimensions(solid)?;
    let [u, v, n] = planar::view_axes(frame);
    let base = std::array::from_fn(|i| placement.origin[i] - frame.origin[i]);
    let axis = unit(placement.normal());
    Some((
        Cylinder {
            base: [dot(base, u), dot(base, v)],
            axis: [u, v, n].map(|direction| dot(direction, axis)),
            radius,
            height,
        },
        bore,
    ))
}

fn round_occluder(
    solid: &Solid,
    frame: PlaneFrame,
) -> Result<Option<RoundOccluder>, GeometryError> {
    let Some((cylinder, bore)) = projection_cylinder(solid, frame) else {
        return Ok(None);
    };
    let placement = circular_dimensions(solid)
        .expect("classified circular solid")
        .0;
    let offset = std::array::from_fn(|i| placement.origin[i] - frame.origin[i]);
    let depth = dot(offset, planar::view_axes(frame)[2]);
    RoundOccluder::new(
        [cylinder.base[0], cylinder.base[1], depth],
        cylinder.axis,
        cylinder.radius,
        cylinder.height,
        bore,
    )
    .map(Some)
    .map_err(planar::projection_error)
}

fn screen_bounds(solid: &Solid, frame: PlaneFrame) -> Result<[[f64; 3]; 2], GeometryError> {
    let bounds = solid.bounds()?;
    let axes = planar::view_axes(frame);
    Ok([false, true].map(|upper| {
        axes.map(|axis| {
            let corner = std::array::from_fn(|i| {
                let coordinate = if upper == (axis[i] >= 0.) {
                    bounds.max[i]
                } else {
                    bounds.min[i]
                };
                coordinate - frame.origin[i]
            });
            dot(corner, axis)
        })
    }))
}

pub(super) fn project_scene(
    solids: &[DrawingSolid<'_>],
    mut options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    if let [input] = solids
        && is_round(input.solid)
    {
        return project(input, options, 0);
    }
    let mut planar_sources = Vec::new();
    let mut planar_solids = Vec::new();
    let mut curved = Vec::new();
    let mut regions = Vec::new();
    for (source, solid) in projection_members(solids) {
        let input = DrawingSolid { solid, hatch: None };
        if let Some(round) = round_occluder(input.solid, options.frame)? {
            regions.push((round.bounds(), true));
            curved.push((source, input, round));
        } else {
            regions.push((screen_bounds(input.solid, options.frame)?, false));
            planar_sources.push(source);
            planar_solids.push(input);
        }
    }
    regions.sort_unstable_by(|a, b| a.0[0][0].total_cmp(&b.0[0][0]));
    let tolerance = GeometryPolicy::default().linear_tolerance_mm;
    for (i, ([a_min, a_max], a_round)) in regions.iter().copied().enumerate() {
        for ([b_min, b_max], b_round) in regions[i + 1..].iter().copied() {
            if b_min[0] > a_max[0] + tolerance {
                break;
            }
            if (!a_round && !b_round)
                || a_min[1] > b_max[1] + tolerance
                || b_min[1] > a_max[1] + tolerance
            {
                continue;
            }
            // Straight edges can be split against exact curved material.
            // Until round edges also carry depth, prove the round is in front
            // everywhere in the overlap before publishing its own edge trims.
            let separated = if a_round {
                !b_round && a_min[2] > b_max[2] + tolerance
            } else {
                b_min[2] > a_max[2] + tolerance
            };
            if !separated {
                return Err(err(
                    GeometryErrorKind::Unsupported,
                    "Rust projection cannot yet resolve round edges behind overlapping geometry",
                ));
            }
        }
    }
    let rounds: Vec<_> = curved.iter().map(|(_, _, round)| *round).collect();
    let mut output = if planar_solids.is_empty() {
        Vec::new()
    } else {
        planar::project_scene(&planar_solids, &rounds, options)?
    };
    for vector in &mut output {
        vector.source_index = planar_sources[vector.source_index];
    }
    options.maximum_curves -= output.len() as u32;
    options.maximum_points -= output
        .iter()
        .map(|vector| vector.preview_mm.len() as u32)
        .sum::<u32>();
    for (source, input, _) in curved {
        let projected = project(&input, options, source)?;
        options.maximum_curves -= projected.len() as u32;
        options.maximum_points -= projected
            .iter()
            .map(|vector| vector.preview_mm.len() as u32)
            .sum::<u32>();
        output.extend(projected);
    }
    output.sort_by_key(|vector| vector.source_index);
    Ok(output)
}

pub(super) fn project_annulus_depth(
    input: &DrawingSolid<'_>,
    options: DrawingProjection,
    depth: f64,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let (body, frame) = input.solid.annulus_body().expect("isolated annulus");
    let [u, v, toward_viewer] = planar::view_axes(options.frame);
    let axis = unit(frame.normal());
    let facing = dot(axis, toward_viewer);
    if dot(axis, u).hypot(dot(axis, v)) > 32. * f64::EPSILON
        || options.section_u_mm.is_some()
        || options.section_v_mm.is_some()
        || options.detail.is_some()
    {
        return Err(err(
            GeometryErrorKind::Unsupported,
            "A Rust annulus section with geometry behind the cut requires a transverse plane and no lateral crop",
        ));
    }
    let base = dot(
        std::array::from_fn(|i| frame.origin[i] - options.frame.origin[i]),
        toward_viewer,
    );
    let end = base + facing * body.geometry.height_mm();
    let low = base.min(end);
    let high = base.max(end);
    let far = options
        .section_depth_mm
        .map_or(f64::NEG_INFINITY, |d| depth - d);
    let tolerance = GeometryPolicy::default().linear_tolerance_mm;
    if high.min(depth) - low.max(far) <= tolerance {
        return Ok(Vec::new());
    }
    if depth < high - tolerance {
        // With the view along the bore, every retained cross section projects
        // onto the same two circles. The cut rims replace those visible rims.
        return section::project_circular(
            input,
            DrawingProjection {
                section_only: true,
                section_depth_mm: None,
                ..options
            },
            depth,
        );
    }
    project(input, options, 0)
}

fn project(
    input: &DrawingSolid<'_>,
    options: DrawingProjection,
    source_index: usize,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let (cylinder, bore) = projection_cylinder(input.solid, options.frame).ok_or_else(|| {
        err(
            GeometryErrorKind::Unsupported,
            "Rust curved projection requires a circular cylinder or annulus",
        )
    })?;
    let detail = options
        .detail
        .map(|(center, radius)| Detail { center, radius });
    let edges = if let Some(inner) = bore {
        curved_projection::project_annulus(cylinder, inner, options.hidden_lines, detail)
    } else {
        curved_projection::project(cylinder, options.hidden_lines, detail)
    }
    .map_err(projection_error)?;
    publish(edges, options, source_index)
}

pub(super) fn publish(
    edges: Vec<curved_projection::Edge>,
    options: DrawingProjection,
    source_index: usize,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    if edges.len() > options.maximum_curves as usize {
        return Err(err(
            GeometryErrorKind::Limit,
            "Curved drawing projection exceeded its curve budget",
        ));
    }
    let mut points = options.maximum_points as usize;
    edges
        .into_iter()
        .map(|edge| {
            let preview_mm = edge
                .curve
                .preview(options.chord_tolerance_mm, points)
                .map_err(projection_error)?;
            points -= preview_mm.len();
            let curve = match edge.curve {
                Curve::Line([a, b]) => VectorCurve::Line { a, b },
                Curve::Ellipse(ellipse) => VectorCurve::Ellipse {
                    center: ellipse.center,
                    u: ellipse.u,
                    v: ellipse.v,
                    major_mm: ellipse.major,
                    minor_mm: ellipse.minor,
                    start_rad: ellipse.trim[0],
                    end_rad: ellipse.trim[1],
                },
            };
            Ok(ProjectedVector {
                source_index,
                role: if edge.hidden {
                    VectorRole::Hidden
                } else {
                    VectorRole::Visible
                },
                curve,
                preview_mm,
            })
        })
        .collect()
}

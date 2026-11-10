//! Drawing adapter for exact circular cylinders and annuli.
use super::*;
use multicad_rust_kernel::curved_projection::{self, Curve, Cylinder, Detail, Error};

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

fn circular_dimensions(solid: &Solid) -> Option<(PlaneFrame, f64, f64, Option<f64>)> {
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

fn screen_bounds(solid: &Solid, frame: PlaneFrame) -> Result<[[f64; 2]; 2], GeometryError> {
    if let Some((round, _)) = projection_cylinder(solid, frame) {
        return Ok(std::array::from_fn(|i| {
            let end = round.axis[i] * round.height;
            let other_axes = [round.axis[(i + 1) % 3], round.axis[(i + 2) % 3]];
            let radial = round.radius * other_axes[0].hypot(other_axes[1]);
            [
                round.base[i] + end.min(0.) - radial,
                round.base[i] + end.max(0.) + radial,
            ]
        }));
    }
    let bounds = solid.bounds()?;
    let [u, v, _] = planar::view_axes(frame);
    Ok([u, v].map(|axis| {
        let support = |upper: bool| {
            let corner = std::array::from_fn(|i| {
                let coordinate = if upper == (axis[i] >= 0.) {
                    bounds.max[i]
                } else {
                    bounds.min[i]
                };
                coordinate - frame.origin[i]
            });
            dot(corner, axis)
        };
        [support(false), support(true)]
    }))
}

pub(super) fn project_scene(
    solids: &[DrawingSolid<'_>],
    mut options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    if let [input] = solids {
        return project(input, options, 0);
    }
    let mut regions = solids
        .iter()
        .map(|input| {
            Ok((
                screen_bounds(input.solid, options.frame)?,
                is_round(input.solid),
            ))
        })
        .collect::<Result<Vec<_>, GeometryError>>()?;
    regions.sort_unstable_by(|a, b| a.0[0][0].total_cmp(&b.0[0][0]));
    let tolerance = GeometryPolicy::default().linear_tolerance_mm;
    for (i, (a, curved)) in regions.iter().enumerate() {
        for (b, other_curved) in &regions[i + 1..] {
            if b[0][0] > a[0][1] + tolerance {
                break;
            }
            if (*curved || *other_curved)
                && a[1][0] <= b[1][1] + tolerance
                && b[1][0] <= a[1][1] + tolerance
            {
                return Err(err(
                    GeometryErrorKind::Unsupported,
                    "Rust drawing projection cannot yet resolve occlusion between overlapping circular and other solids",
                ));
            }
        }
    }
    let planar_inputs: Vec<_> = solids
        .iter()
        .enumerate()
        .filter(|(_, input)| !is_round(input.solid))
        .map(|(source, input)| {
            (
                source,
                DrawingSolid {
                    solid: input.solid,
                    hatch: input.hatch,
                },
            )
        })
        .collect();
    let (planar_sources, planar_solids): (Vec<_>, Vec<_>) = planar_inputs.into_iter().unzip();
    let mut output = if planar_solids.is_empty() {
        Vec::new()
    } else {
        planar::project(&planar_solids, options)?
    };
    for vector in &mut output {
        vector.source_index = planar_sources[vector.source_index];
    }
    options.maximum_curves -= output.len() as u32;
    options.maximum_points -= output
        .iter()
        .map(|vector| vector.preview_mm.len() as u32)
        .sum::<u32>();
    for (source, input) in solids
        .iter()
        .enumerate()
        .filter(|(_, input)| is_round(input.solid))
    {
        let projected = project(input, options, source)?;
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
        return section::project_annulus(
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

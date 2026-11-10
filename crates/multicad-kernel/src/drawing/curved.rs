//! Drawing adapter for exact circular cylinders and annuli.
use super::*;
use multicad_rust_kernel::{
    curved_projection::{self, Curve, Cylinder, Detail, Error},
    planar_projection,
    round_occlusion::RoundOccluder,
};

mod apertures;
mod machined;
mod opaque;
mod pockets;
mod shadows;
pub(super) mod stadium;
mod stepped;

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

pub(super) fn needs_curved_scene(solid: &Solid) -> bool {
    is_round(solid)
        || pockets::has_machined_stock(solid)
        || apertures::has_through_bores(solid)
        || solid.rounded_slot_body().is_some()
        || solid.revolution_body().is_some()
        || solid.cosmetic_fastener_body().is_some()
        || solid
            .rust_compound()
            .is_some_and(|compound| compound.members().any(needs_curved_scene))
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

pub(super) fn round_occluder(
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

pub(super) fn project_scene(
    solids: &[DrawingSolid<'_>],
    options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    if let [input] = solids
        && is_round(input.solid)
    {
        return project(input, options, 0);
    }

    let members = projection_members(solids);
    match project_members(&members, options) {
        Err(error) if error.kind == GeometryErrorKind::Unsupported && !options.hidden_lines => {
            let visible = opaque::visible_members(&members, options.frame)?;
            if visible.len() == members.len() {
                Err(error)
            } else {
                project_members(&visible, options)
            }
        }
        result => result,
    }
}

fn project_members(
    members: &[(usize, &Solid)],
    options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let mut apertures = None;
    let mut shadows = None;
    let mut planar_members = Vec::new();
    let mut analytic_stocks = Vec::new();
    let mut round_sources = Vec::new();
    let mut rounds = Vec::new();
    let mut rounded = stadium::Shapes::default();
    for (index, &(source, solid)) in members.iter().enumerate() {
        if solid.rounded_slot_body().is_some() {
            rounded.add(solid, source, options)?;
        } else if let Some(round) = round_occluder(solid, options.frame)? {
            round_sources.push(source);
            rounds.push(round);
        } else if let Some(bands) = stepped::rounds(solid, index, options)? {
            round_sources.extend(std::iter::repeat_n(source, bands.len()));
            rounds.extend(bands);
        } else if options.hidden_lines
            && let Some(stock) = machined::hex_stock(solid, source, analytic_stocks.len(), options)?
        {
            analytic_stocks.push((source, stock, solid.geometry_policy().linear_tolerance_mm));
        } else {
            let proxy = if options.hidden_lines {
                machined::stock(solid, source, planar_members.len(), options)?
            } else if apertures::has_through_bores(solid) {
                let scene = apertures::Scene::get(&mut apertures, members, options.frame)?;
                Some(scene.stock(solid, index, source)?)
            } else {
                let cover = |mouth| {
                    shadows::Shadows::get(&mut shadows, members, options.frame)?
                        .covers(index, mouth)
                };
                pockets::stock(
                    solid,
                    source,
                    planar_members.len(),
                    options,
                    cover,
                    |cavity| {
                        let scene = apertures::Scene::get(&mut apertures, members, options.frame)?;
                        scene.oblique(index, source, cavity)
                    },
                )?
            };
            planar_members.push((source, solid, proxy));
        }
    }
    let inputs: Vec<_> = planar_members
        .iter()
        .map(|(_, solid, proxy)| DrawingSolid {
            solid: proxy.as_ref().map_or(*solid, |stock| &stock.solid),
            hatch: None,
        })
        .collect();
    let (mut bodies, mut tolerance) = planar::prepare(&inputs, options.frame)?;
    let analytic_offset = bodies.len();
    let mut rims = Vec::new();
    let mut openings = Vec::new();
    let mut intrinsic_hidden = Vec::new();
    let mut planar_sources: Vec<_> = planar_members
        .iter()
        .map(|(source, _, _)| *source)
        .collect();
    for (_, _, proxy) in planar_members {
        if let Some(proxy) = proxy {
            rims.extend(proxy.rims);
            rounded.rims.extend(proxy.spatial_rims);
            openings.extend(proxy.openings);
            intrinsic_hidden.extend(proxy.intrinsic_hidden);
        }
    }
    for (source, mut stock, linear_tolerance) in analytic_stocks {
        tolerance = tolerance.max(linear_tolerance);
        bodies.push(stock.solid);
        planar_sources.push(source);
        rims.extend(stock.rims);
        intrinsic_hidden.extend(stock.intrinsic_hidden);
        for opening in &mut stock.openings {
            opening.body += analytic_offset;
        }
        openings.extend(stock.openings);
    }
    let projection = planar_projection::project_scene_with_candidates(
        &bodies,
        &rounds,
        planar_projection::SceneExtras {
            rims: &rims,
            openings: &openings,
            blocker_bodies: &rounded.bodies,
            blocker_rounds: &rounded.rounds,
            lines: &rounded.lines,
            spatial_rims: &rounded.rims,
            stadium_openings: &[],
            intrinsic_hidden: &intrinsic_hidden,
        },
        planar_projection::Options {
            tolerance,
            hidden_lines: options.hidden_lines,
            tangent_edges: options.tangent_edges,
            detail: options.detail,
            segment_limit: (options.maximum_curves as usize)
                .min(options.maximum_points as usize / 2),
        },
    )
    .map_err(planar::projection_error)?;

    publish_scene(projection, options, &planar_sources, &round_sources)
}

pub(super) fn publish_scene(
    (lines, round_edges, supplied): planar_projection::SceneProjection,
    mut options: DrawingProjection,
    planar_sources: &[usize],
    round_sources: &[usize],
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let mut output: Vec<_> = lines
        .into_iter()
        .map(|line| {
            planar::line(
                planar_sources[line.source],
                if line.hidden {
                    VectorRole::Hidden
                } else {
                    VectorRole::Visible
                },
                line.ends,
            )
        })
        .collect();
    options.maximum_curves -= output.len() as u32;
    options.maximum_points -= 2 * output.len() as u32;
    let mut add = |source, edges| -> Result<(), GeometryError> {
        let projected = publish(edges, options, source)?;
        options.maximum_curves -= projected.len() as u32;
        options.maximum_points -= projected
            .iter()
            .map(|vector| vector.preview_mm.len() as u32)
            .sum::<u32>();
        output.extend(projected);
        Ok(())
    };
    for segment in round_edges {
        add(round_sources[segment.source], vec![segment.edge])?;
    }
    for segment in supplied {
        add(segment.source, vec![segment.edge])?;
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

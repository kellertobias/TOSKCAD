//! Axial views and perpendicular sections of spherical bevel gears.
//!
//! Visibility, slab clipping and section boundaries come from the exact
//! pure-kernel view; this adapter places the local curves on the sheet.
use super::*;
use multicad_rust_kernel::bevel_gear::{
    Error as BevelError,
    view::{AxialView, Role, ViewCurve},
};

pub(in crate::drawing) fn project_bevel(
    input: &DrawingSolid<'_>,
    options: DrawingProjection,
) -> Option<Result<Vec<ProjectedVector>, GeometryError>> {
    let (body, placement) = input.solid.bevel_body()?;
    let [u, v, toward_viewer] = planar::view_axes(options.frame);
    let axis = placement.normal();
    if dot(axis, u).hypot(dot(axis, v)) > 64. * f64::EPSILON
        || options.detail.is_some()
        || options.section_u_mm.is_some()
        || options.section_v_mm.is_some()
    {
        return Some(Err(err(
            GeometryErrorKind::Unsupported,
            "Rust bevel drawing requires an uncropped view along the gear axis",
        )));
    }
    let alignment = dot(axis, toward_viewer);
    let heel_side = alignment > 0.;
    let offset = dot(
        std::array::from_fn(|i| placement.origin[i] - options.frame.origin[i]),
        toward_viewer,
    );
    // Local axial height of a viewing depth along the viewer direction.
    let height = |depth: f64| (depth - offset) / alignment;
    let (slab, section) =
        options
            .section_offset_mm
            .map_or(([f64::NEG_INFINITY, f64::INFINITY], None), |depth| {
                let near = height(depth);
                let far = options.section_depth_mm.map(|limit| height(depth - limit));
                let slab = if heel_side {
                    [far.unwrap_or(f64::NEG_INFINITY), near]
                } else {
                    [near, far.unwrap_or(f64::INFINITY)]
                };
                (slab, Some(near))
            });
    let screen = Screen {
        origin: options.frame.origin,
        axes: [u, v],
        toward_viewer,
    };
    let policy = input.solid.geometry_policy();
    Some((|| {
        let items = body
            .geometry
            .axial_view(&AxialView {
                heel_side,
                slab,
                section,
                section_only: options.section_only,
                hidden: options.hidden_lines,
                tangent: options.tangent_edges,
                tolerance: options.chord_tolerance_mm / 2.,
                max_points: options.maximum_points as usize,
            })
            .map_err(|error| match error {
                BevelError::Limit => err(
                    GeometryErrorKind::Limit,
                    "Bevel drawing exceeds its spline or point budget",
                ),
                _ => err(
                    GeometryErrorKind::Input,
                    "Invalid bevel drawing tolerance or section",
                ),
            })?;
        let local = |[x, y, _]: [f64; 3]| screen.point(placement.point([x, y]));
        let mut splines = Vec::new();
        let mut rims = [Vec::new(), Vec::new()];
        for item in items {
            let hidden = item.role == Role::Hidden;
            let curve = match item.curve {
                ViewCurve::Radial { phi, radii } => {
                    Curve::Line(radii.map(|r| local([r * phi.cos(), r * phi.sin(), 0.])))
                }
                ViewCurve::Arc {
                    radius,
                    start,
                    sweep,
                } => screen.curve(Some(AnalyticGeometry::CircularArc {
                    frame: multicad_core::references::ReferenceFrame {
                        origin_mm: placement.origin,
                        u: placement.u,
                        v: placement.v,
                    },
                    radius_mm: radius,
                    start_rad: start,
                    sweep_rad: sweep,
                }))?,
                ViewCurve::Spline { spline, samples } => {
                    splines.push(ProjectedVector {
                        source_index: 0,
                        role: match item.role {
                            Role::Visible => VectorRole::Visible,
                            Role::Hidden => VectorRole::Hidden,
                            Role::Section => VectorRole::Section,
                        },
                        curve: VectorCurve::BSpline {
                            degree: 3,
                            periodic: false,
                            weights: vec![1.; spline.poles_mm.len()],
                            poles: spline.poles_mm.into_iter().map(local).collect(),
                            knots: spline.knots,
                            multiplicities: spline
                                .multiplicities
                                .into_iter()
                                .map(u32::from)
                                .collect(),
                            first: 0.,
                            last: 1.,
                        },
                        preview_mm: samples.into_iter().map(local).collect(),
                    });
                    continue;
                }
            };
            rims[usize::from(item.role == Role::Section)].push(Edge { hidden, curve });
        }
        let limit = || {
            err(
                GeometryErrorKind::Limit,
                "Bevel drawing exceeds the curve budget",
            )
        };
        let mut remaining = options;
        remaining.maximum_curves = (options.maximum_curves as usize)
            .checked_sub(splines.len())
            .ok_or_else(limit)? as u32;
        remaining.maximum_points = (options.maximum_points as usize)
            .checked_sub(splines.iter().map(|s| s.preview_mm.len()).sum())
            .ok_or_else(limit)? as u32;
        let [edges, cuts] = rims;
        let mut vectors = curved::publish(edges, remaining, 0)?;
        remaining.maximum_curves -= vectors.len() as u32;
        remaining.maximum_points -=
            vectors.iter().map(|v| v.preview_mm.len()).sum::<usize>() as u32;
        let mut sections = curved::publish(cuts, remaining, 0)?;
        for vector in &mut sections {
            vector.role = VectorRole::Section;
        }
        vectors.extend(sections);
        vectors.extend(splines);
        if let Some(hatch) = input
            .hatch
            .filter(|hatch| hatch.pattern != multicad_core::materials::HatchPattern::None)
            .filter(|_| section.is_some())
        {
            let boundary = hatch_boundary(&vectors, policy.linear_tolerance_mm);
            let mut curves = options.maximum_curves as usize - vectors.len();
            let mut points = options.maximum_points as usize
                - vectors
                    .iter()
                    .map(|vector| vector.preview_mm.len())
                    .sum::<usize>();
            if !boundary.is_empty() {
                vectors.extend(section::publish_hatch(
                    0,
                    hatch,
                    &boundary,
                    policy.linear_tolerance_mm,
                    &mut curves,
                    &mut points,
                    &mut multicad_rust_kernel::section_hatch::Budget::default(),
                )?);
            }
        }
        Ok(vectors)
    })())
}

/// Closed section boundary segments whose shared ends match exactly.
///
/// Independently evaluated piece ends agree to rounding error, so ends
/// closer than the snapping distance use one representative point.
/// Interior samples closer than the snapping distance to their predecessor
/// are dropped so every segment keeps a positive length.
fn hatch_boundary(vectors: &[ProjectedVector], tolerance: f64) -> Vec<[Point; 2]> {
    let snap = tolerance * 64.;
    let mut ends: Vec<Point> = Vec::new();
    let mut representative = |p: Point| {
        if let Some(&q) = ends
            .iter()
            .find(|q| (p[0] - q[0]).hypot(p[1] - q[1]) <= snap)
        {
            return q;
        }
        ends.push(p);
        p
    };
    let mut segments = Vec::new();
    for vector in vectors.iter().filter(|v| v.role == VectorRole::Section) {
        let preview = &vector.preview_mm;
        let first = representative(preview[0]);
        let last = representative(preview[preview.len() - 1]);
        let mut path = vec![first];
        for &p in &preview[1..preview.len() - 1] {
            let previous = path[path.len() - 1];
            let far = |q: Point| (p[0] - q[0]).hypot(p[1] - q[1]) > snap;
            if far(previous) && far(last) {
                path.push(p);
            }
        }
        path.push(last);
        segments.extend(path.windows(2).map(|pair| [pair[0], pair[1]]));
    }
    segments
}

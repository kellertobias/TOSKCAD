//! Exact axial drawings of Rust ZA worms and section cuts of generated
//! worm wheels.
//!
//! Every boundary point of a worm is visible from an end exactly when no
//! material lies between it and the viewer along the axis. A column at
//! radius `r` and angle `θ` meets the nearest cap or cut face when
//! `r ≤ R(θ, z_near)`. Otherwise `R(θ, z_near) < r < R_major` and the thread
//! occupies a set of positive length at that radius in every pitch, so a
//! retained length of at least one pitch puts material in front of every
//! deeper point. Hence only the near boundary `R(θ, z_near)`, the crest
//! circle (the silhouette of the crest land) and the near bore circle are
//! visible. The far boundary, the root circle outside the near root land
//! and the far bore circle are hidden; parts coincident with a visible
//! circle are drawn once as visible. Flanks have a nonzero axial normal
//! component everywhere, so there are no further silhouettes.
//!
//! A cut perpendicular to either axis is the exact planar section, drawn
//! within the chord tolerance. Generated wheels viewed along their axis use
//! the proven visibility structure of `worm_gear/visibility.rs`: a boundary
//! point is visible exactly when its projection lies in the open tooth space
//! of the near face. Non-axial views, details and lateral crops of views
//! remain unavailable.
use super::*;
use multicad_core::references::ReferenceFrame;
use multicad_rust_kernel::worm_gear::{ViewCurve, ViewError};

fn unsupported(message: &str) -> GeometryError {
    err(GeometryErrorKind::Unsupported, message)
}

fn view_error(cause: ViewError) -> GeometryError {
    match cause {
        ViewError::Unproven => unsupported(
            "Rust worm-wheel views need the proven axial visibility structure, which this wheel does not satisfy",
        ),
        ViewError::Limit => err(
            GeometryErrorKind::Limit,
            "Rust worm-wheel view exceeded its curve budget",
        ),
        ViewError::Invalid => err(GeometryErrorKind::Input, "Invalid Rust worm-wheel view"),
    }
}

fn budget(cause: multicad_rust_kernel::display::MeshError) -> GeometryError {
    err(
        GeometryErrorKind::Limit,
        format!("Rust worm drawing exceeded its point budget: {cause:?}"),
    )
}

pub(in crate::drawing) fn project_worm_gearing(
    input: &DrawingSolid<'_>,
    options: DrawingProjection,
) -> Option<Result<Vec<ProjectedVector>, GeometryError>> {
    let frame = input
        .solid
        .rust_worm()
        .map(|(_, frame)| frame)
        .or_else(|| input.solid.rust_worm_wheel().map(|(_, frame)| frame))?;
    let [u, v, toward_viewer] = planar::view_axes(options.frame);
    let axis = frame.normal();
    if dot(axis, u).hypot(dot(axis, v)) > 64. * f64::EPSILON || options.detail.is_some() {
        return Some(Err(unsupported(
            "Rust worm and worm-wheel drawings require an uncropped view along the gear axis",
        )));
    }
    let alignment = dot(axis, toward_viewer);
    let offset = dot(
        std::array::from_fn(|i| frame.origin[i] - options.frame.origin[i]),
        toward_viewer,
    );
    let screen = Screen {
        origin: options.frame.origin,
        axes: [u, v],
        toward_viewer,
    };
    let local_height = |depth: f64| (depth - offset) / alignment;
    let place = move |[x, y]: Point, z: f64| -> [f64; 3] {
        std::array::from_fn(|i| frame.origin[i] + frame.u[i] * x + frame.v[i] * y + axis[i] * z)
    };
    let chord = options.chord_tolerance_mm / 2.;
    let limit = options.maximum_points as usize;
    let project_rings = |rings: Vec<Vec<Point>>, z: f64| -> Vec<Vec<Point>> {
        rings
            .into_iter()
            .map(|ring| {
                let mut ring: Vec<_> = ring
                    .into_iter()
                    .map(|p| screen.point(place(p, z)))
                    .collect();
                if alignment < 0. {
                    ring.reverse();
                }
                ring
            })
            .collect()
    };
    if let (Some(depth), true) = (options.section_offset_mm, options.section_only) {
        let z = local_height(depth);
        return Some((|| {
            let rings = match (input.solid.rust_worm(), input.solid.rust_worm_wheel()) {
                (Some((worm, _)), _) if z.abs() <= worm.length_mm() / 2. => {
                    let mut rings = vec![worm.section_ring(z, chord, limit).map_err(budget)?];
                    rings.extend(
                        worm.bore_radius_mm()
                            .map(|radius| circle(radius, chord, true)),
                    );
                    rings
                }
                (_, Some((wheel, _))) if z.abs() <= wheel.half_width_mm() => {
                    wheel.section_rings(z, chord, limit).map_err(budget)?
                }
                _ => return Ok(Vec::new()),
            };
            section::sampled::publish(input, project_rings(rings, z), options)
        })());
    }
    if options.section_u_mm.is_some() || options.section_v_mm.is_some() {
        return Some(Err(unsupported(
            "Rust worm and worm-wheel views do not support lateral section crops",
        )));
    }
    let policy = input.solid.geometry_policy();
    let near = options.section_offset_mm.unwrap_or(f64::INFINITY);
    let far = options
        .section_depth_mm
        .map_or(f64::NEG_INFINITY, |distance| near - distance);
    let Some((worm, _)) = input.solid.rust_worm() else {
        let (wheel, _) = input.solid.rust_worm_wheel()?;
        let half = wheel.half_width_mm();
        let ends = [-half, half].map(|z| offset + alignment * z);
        let top = near.min(ends[0].max(ends[1]));
        let bottom = far.max(ends[0].min(ends[1]));
        if top - bottom <= policy.linear_tolerance_mm {
            return Some(Ok(Vec::new()));
        }
        let [near_z, far_z] = [top, bottom].map(|depth| local_height(depth).clamp(-half, half));
        let cut = top < ends[0].max(ends[1]) - policy.linear_tolerance_mm;
        return Some((|| {
            let mut vectors = Vec::new();
            if cut {
                let rings = wheel.section_rings(near_z, chord, limit).map_err(budget)?;
                vectors = section::sampled::publish(input, project_rings(rings, near_z), options)?;
            }
            let curves = wheel
                .axial_view([near_z, far_z], !cut, options.hidden_lines, chord, limit)
                .map_err(view_error)?;
            let mut edges = Vec::new();
            for curve in curves {
                match curve {
                    ViewCurve::Arc {
                        z_mm,
                        radius_mm,
                        start_rad,
                        sweep_rad,
                        hidden,
                    } => edges.push(arc(
                        &screen,
                        place,
                        z_mm,
                        radius_mm,
                        [start_rad, sweep_rad],
                        hidden,
                    )?),
                    ViewCurve::Polyline { points_mm, hidden } => {
                        for pair in points_mm.windows(2) {
                            let [a, b] =
                                [pair[0], pair[1]].map(|p| screen.point(place([p[0], p[1]], p[2])));
                            edges.push(Edge {
                                hidden,
                                curve: Curve::Line([a, b]),
                            });
                        }
                    }
                }
            }
            publish_after(vectors, edges, options)
        })());
    };
    let half = worm.length_mm() / 2.;
    let ends = [-half, half].map(|z| offset + alignment * z);
    let top = near.min(ends[0].max(ends[1]));
    let bottom = far.max(ends[0].min(ends[1]));
    if top - bottom <= policy.linear_tolerance_mm {
        return Some(Ok(Vec::new()));
    }
    if top - bottom < worm.pitch_mm() {
        return Some(Err(unsupported(
            "Rust worm views need at least one thread pitch behind the near face",
        )));
    }
    let [near_z, far_z] = [top, bottom].map(|depth| local_height(depth).clamp(-half, half));
    let cut = top < ends[0].max(ends[1]) - policy.linear_tolerance_mm;
    let far_end = bottom <= ends[0].min(ends[1]) + policy.linear_tolerance_mm;
    Some((|| {
        let major = worm.major_radius_mm();
        let root = worm.root_radius_mm();
        let ring = worm.section_ring(near_z, chord, limit).map_err(budget)?;
        let bore = worm.bore_radius_mm();
        let mut vectors = Vec::new();
        let mut edges = Vec::new();
        let on = |a: Point, b: Point, radius: f64| {
            [a, b]
                .iter()
                .all(|p| (p[0].hypot(p[1]) - radius).abs() <= 1e-9 * radius)
        };
        let segments = |ring: &[Point], skip: &[f64]| -> Vec<[Point; 2]> {
            (0..ring.len())
                .map(|i| [ring[i], ring[(i + 1) % ring.len()]])
                .filter(|&[a, b]| !skip.iter().any(|&radius| on(a, b, radius)))
                .collect()
        };
        if cut {
            let mut rings = vec![ring];
            rings.extend(bore.map(|radius| circle(radius, chord, true)));
            vectors = section::sampled::publish(input, project_rings(rings, near_z), options)?;
        } else {
            for [a, b] in segments(&ring, &[major]) {
                edges.push(Edge {
                    hidden: false,
                    curve: Curve::Line([a, b].map(|p| screen.point(place(p, near_z)))),
                });
            }
            if let Some(radius) = bore {
                edges.push(arc(&screen, place, near_z, radius, [0., TAU], false)?);
            }
        }
        edges.push(arc(&screen, place, near_z, major, [0., TAU], false)?);
        if options.hidden_lines {
            let [start, sweep] = worm.root_span(near_z);
            edges.push(arc(
                &screen,
                place,
                near_z,
                root,
                [start + sweep, TAU - sweep],
                true,
            )?);
            if far_end {
                let back = worm.section_ring(far_z, chord, limit).map_err(budget)?;
                for [a, b] in segments(&back, &[major, root]) {
                    edges.push(Edge {
                        hidden: true,
                        curve: Curve::Line([a, b].map(|p| screen.point(place(p, far_z)))),
                    });
                }
            }
        }
        publish_after(vectors, edges, options)
    })())
}

/// A closed circle polyline within `tolerance`, clockwise for a hole.
fn circle(radius: f64, tolerance: f64, hole: bool) -> Vec<Point> {
    let step = 2. * (1. - tolerance / radius).max(-1.).acos();
    let count = (TAU / step).ceil().max(3.) as usize;
    let sign = if hole { -1. } else { 1. };
    (0..count)
        .map(|i| {
            let (s, c) = (sign * TAU * i as f64 / count as f64).sin_cos();
            [radius * c, radius * s]
        })
        .collect()
}

/// A circular arc about the axis at local height `z`.
fn arc(
    screen: &Screen,
    place: impl Fn(Point, f64) -> [f64; 3],
    z: f64,
    radius: f64,
    [start, sweep]: [f64; 2],
    hidden: bool,
) -> Result<Edge, GeometryError> {
    let origin = place([0., 0.], z);
    let [u, v] = [[1., 0.], [0., 1.]].map(|p| {
        let q = place(p, z);
        std::array::from_fn(|i| q[i] - origin[i])
    });
    Ok(Edge {
        hidden,
        curve: screen.curve(Some(AnalyticGeometry::CircularArc {
            frame: ReferenceFrame {
                origin_mm: origin,
                u,
                v,
            },
            radius_mm: radius,
            start_rad: start,
            sweep_rad: sweep,
        }))?,
    })
}

/// Publish projected edges after already published section vectors.
fn publish_after(
    mut vectors: Vec<ProjectedVector>,
    edges: Vec<Edge>,
    options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let mut remaining = options;
    remaining.maximum_curves = remaining
        .maximum_curves
        .checked_sub(vectors.len() as u32)
        .ok_or_else(|| budget(multicad_rust_kernel::display::MeshError::BudgetExceeded))?;
    remaining.maximum_points -= vectors
        .iter()
        .map(|v| v.preview_mm.len() as u32)
        .sum::<u32>();
    vectors.extend(curved::publish(edges, remaining, 0)?);
    Ok(vectors)
}

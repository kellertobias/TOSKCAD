//! Exact axial views of a rectangular panel with certified parallel blind cuts.
use super::*;
use multicad_core::references::{AnalyticGeometry, TopologyKind};
use multicad_rust_kernel::curved_projection::{Curve, Edge, Ellipse};
use std::{collections::BTreeMap, f64::consts::TAU};

mod chamfered;
mod normal_sweep;
mod spline;
pub(super) use chamfered::project_chamfered_bores;
pub(super) use normal_sweep::{project_normal_sweep, transverse_rails};
pub(super) use spline::{contains_spline_sweep, project_spline_sweep};

type Point = [f64; 2];

pub(super) fn project_spur_gear(
    input: &DrawingSolid<'_>,
    options: DrawingProjection,
) -> Option<Result<Vec<ProjectedVector>, GeometryError>> {
    let (body, frame) = input.solid.spur_gear_body()?;
    let [u, v, toward_viewer] = planar::view_axes(options.frame);
    let axis = frame.normal();
    if dot(axis, u).hypot(dot(axis, v)) > 32. * f64::EPSILON || options.detail.is_some() {
        return Some(Err(err(
            GeometryErrorKind::Unsupported,
            "Rust spur gear drawing currently requires an uncropped axial view",
        )));
    }
    let screen = Screen {
        origin: options.frame.origin,
        axes: [u, v],
        toward_viewer,
    };
    Some((|| {
        let mut planar = Vec::new();
        if let Some((rim, hidden)) = body.hub_end_on_rim(frame, dot(axis, toward_viewer) > 0.)
            && (!hidden || options.hidden_lines)
        {
            planar.push(Edge {
                hidden,
                curve: screen.curve(Some(rim))?,
            });
        }
        publish_rotary_rims(
            &screen,
            body.end_on_rims(frame),
            |p| screen.point(frame.point(p)),
            planar,
            options,
        )
    })())
}

pub(super) fn section_herringbone(
    input: &DrawingSolid<'_>,
    options: DrawingProjection,
    depth: f64,
) -> Option<Result<Vec<ProjectedVector>, GeometryError>> {
    let (body, placement) = input.solid.herringbone_body()?;
    let [u, v, toward_viewer] = planar::view_axes(options.frame);
    let normal = placement.normal();
    if dot(normal, u).hypot(dot(normal, v)) > 64. * f64::EPSILON || options.detail.is_some() {
        return Some(Err(err(
            GeometryErrorKind::Unsupported,
            "Rust herringbone sections require a plane perpendicular to the gear axis",
        )));
    }
    let alignment = dot(normal, toward_viewer);
    let offset = std::array::from_fn(|i| placement.origin[i] - options.frame.origin[i]);
    let z = (depth - dot(offset, toward_viewer)) / alignment;
    let height = body.profile().height_mm();
    let policy = input.solid.geometry_policy();
    if z < -policy.linear_tolerance_mm || z > height + policy.linear_tolerance_mm {
        return Some(Ok(Vec::new()));
    }
    let frame = body.section(placement, z.clamp(0., height), 1.);
    let screen = Screen {
        origin: options.frame.origin,
        axes: [u, v],
        toward_viewer,
    };
    let project = |[x, y]: Point| {
        screen.point(std::array::from_fn(|i| {
            frame.u[i].mul_add(x, frame.v[i].mul_add(y, frame.origin_mm[i]))
        }))
    };
    Some((|| {
        let clipped = options.section_u_mm.is_some() || options.section_v_mm.is_some();
        let hatch = input
            .hatch
            .filter(|hatch| hatch.pattern != multicad_core::materials::HatchPattern::None);
        let sampled = if clipped || hatch.is_some() {
            Some(sample_gear_section(
                body.profile(),
                project,
                alignment < 0.,
                options,
                hatch.is_some(),
                policy,
            )?)
        } else {
            None
        };
        if clipped {
            return section::sampled::publish(
                input,
                sampled.expect("section window needs samples"),
                options,
            );
        }
        let mut vectors = publish_rotary_rims(
            &screen,
            body.section_rims(frame),
            project,
            Vec::new(),
            options,
        )?;
        for vector in &mut vectors {
            vector.role = VectorRole::Section;
        }
        if let Some((hatch, rings)) = hatch.zip(sampled) {
            let boundary: Vec<_> = rings
                .into_iter()
                .flat_map(|ring| {
                    ring.iter()
                        .zip(ring.iter().cycle().skip(1))
                        .map(|(&a, &b)| [a, b])
                        .collect::<Vec<_>>()
                })
                .collect();
            let mut curves = options.maximum_curves as usize - vectors.len();
            let mut points = options.maximum_points as usize
                - vectors
                    .iter()
                    .map(|vector| vector.preview_mm.len())
                    .sum::<usize>();
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
        Ok(vectors)
    })())
}

fn sample_gear_section(
    profile: &multicad_rust_kernel::spur_gear::Profile,
    project: impl Fn(Point) -> Point,
    reverse: bool,
    options: DrawingProjection,
    hatched: bool,
    policy: GeometryPolicy,
) -> Result<Vec<Vec<Point>>, GeometryError> {
    use multicad_rust_kernel::spur_gear::Edge;
    let mut tolerance = options.chord_tolerance_mm / 2.;
    if hatched {
        tolerance = tolerance.min(policy.minimum_feature_mm / 4.);
    }
    let mut remaining = options.maximum_points as usize;
    let mut append = |ring: &mut Vec<Point>, edge: Edge| {
        let failed = || {
            err(
                GeometryErrorKind::Limit,
                "Gear section exceeds its chord or point budget",
            )
        };
        let mut samples = edge.sample(tolerance, 0.3).map_err(|_| failed())?;
        samples.pop(); // The following edge supplies the shared endpoint.
        remaining = remaining.checked_sub(samples.len()).ok_or_else(failed)?;
        ring.extend(samples.into_iter().map(&project));
        Ok::<(), GeometryError>(())
    };
    let mut outline = Vec::new();
    for edge in profile.outline() {
        append(&mut outline, edge.edge)?;
    }
    let mut rings = vec![outline];
    if let Some(bore) = profile.bore() {
        let mut hole = Vec::new();
        append(&mut hole, Edge::Planar(bore))?;
        rings.push(hole);
    }
    if reverse {
        rings.iter_mut().for_each(|ring| ring.reverse());
    }
    Ok(rings)
}

fn publish_rotary_rims(
    screen: &Screen,
    rims: impl Iterator<Item = (multicad_rust_kernel::spur_gear::Edge, AnalyticGeometry)>,
    project: impl Fn(Point) -> Point,
    mut planar: Vec<Edge>,
    options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    use multicad_rust_kernel::spur_gear;
    let mut vectors = Vec::new();
    let mut points = 0usize;
    for (edge, geometry) in rims {
        match edge {
            spur_gear::Edge::Planar(_) => {
                planar.push(Edge {
                    hidden: false,
                    curve: screen.curve(Some(geometry))?,
                });
            }
            spur_gear::Edge::Involute(flank) => {
                let failed = || {
                    err(
                        GeometryErrorKind::Limit,
                        "Gear drawing exceeds its spline or point budget",
                    )
                };
                let tolerance = options.chord_tolerance_mm / 2.;
                let spline = flank.cubic_spline(tolerance).map_err(|_| failed())?;
                let preview_mm: Vec<_> = edge
                    .sample(tolerance, 0.3)
                    .map_err(|_| failed())?
                    .into_iter()
                    .map(&project)
                    .collect();
                points = points.saturating_add(preview_mm.len());
                if points > options.maximum_points as usize {
                    return Err(failed());
                }
                vectors.push(ProjectedVector {
                    source_index: 0,
                    role: VectorRole::Visible,
                    curve: VectorCurve::BSpline {
                        degree: 3,
                        periodic: false,
                        weights: vec![1.; spline.poles_mm.len()],
                        poles: spline.poles_mm.into_iter().map(&project).collect(),
                        knots: spline.knots,
                        multiplicities: spline.multiplicities.into_iter().map(u32::from).collect(),
                        first: 0.,
                        last: 1.,
                    },
                    preview_mm,
                });
            }
        }
    }
    let mut remaining = options;
    remaining.maximum_curves = (options.maximum_curves as usize)
        .checked_sub(vectors.len())
        .filter(|count| *count >= planar.len())
        .ok_or_else(|| {
            err(
                GeometryErrorKind::Limit,
                "Gear drawing exceeds the curve budget",
            )
        })? as u32;
    remaining.maximum_points -= points as u32;
    vectors.extend(curved::publish(planar, remaining, 0)?);
    Ok(vectors)
}

pub(super) fn project(
    input: &DrawingSolid<'_>,
    options: DrawingProjection,
) -> Option<Result<Vec<ProjectedVector>, GeometryError>> {
    let (stock, frame) = input.solid.orthogonal_body()?;
    let [u, v, toward_viewer] = planar::view_axes(options.frame);
    let [near, far] = stock.parallel_panel_faces(frame, toward_viewer)?;
    if options.detail.is_some() {
        return Some(Err(err(
            GeometryErrorKind::Unsupported,
            "Rust face-on panel projection does not support a detail crop",
        )));
    }
    let screen = Screen {
        origin: options.frame.origin,
        axes: [u, v],
        toward_viewer,
    };
    Some((|| {
        let topology = input.solid.rust_topology().expect("Rust panel topology")?;
        let geometry: BTreeMap<_, _> = topology
            .elements
            .into_iter()
            .filter(|element| element.key.kind == TopologyKind::Edge)
            .map(|element| (element.key.ordinal, element.geometry))
            .collect();
        let curves = |ring: &[(u32, bool)]| {
            ring.iter()
                .map(|&(edge, _)| screen.curve(geometry.get(&edge).copied().flatten()))
                .collect::<Result<Vec<_>, _>>()
        };

        // Looking down the cut axis collapses each floor wire onto its mouth
        // and the rear stock silhouette onto the front outline. The certified
        // blind floors occlude any opposing cavity behind those openings.
        let near_wires = stock.loops(near);
        let visible = near_wires
            .iter()
            .map(|ring| curves(ring))
            .collect::<Result<Vec<_>, _>>()?;
        let mut hidden = Vec::new();
        if options.hidden_lines {
            let mouths: Vec<_> = visible
                .iter()
                .skip(1)
                .map(|wire| (wire, bounds(wire)))
                .collect();
            for ring in stock.loops(far).iter().skip(1) {
                let wire = curves(ring)?;
                let extents = bounds(&wire);
                let mut overlaps = mouths
                    .iter()
                    .filter(|(_, bounds)| overlap(*bounds, extents));
                if overlaps.clone().any(|(front, _)| same_wire(front, &wire)) {
                    continue;
                }
                if overlaps.next().is_some() {
                    return Err(err(
                        GeometryErrorKind::Unsupported,
                        "Rust face-on panel projection cannot separate overlapping opposite openings",
                    ));
                }
                hidden.extend(wire);
            }
        }
        let edges = visible
            .into_iter()
            .flatten()
            .map(|curve| Edge {
                hidden: false,
                curve,
            })
            .chain(hidden.into_iter().map(|curve| Edge {
                hidden: true,
                curve,
            }))
            .collect();
        curved::publish(edges, options, 0)
    })())
}

struct Screen {
    origin: [f64; 3],
    axes: [[f64; 3]; 2],
    toward_viewer: [f64; 3],
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(x, y)| x * y).sum()
}

impl Screen {
    fn point(&self, point: [f64; 3]) -> Point {
        let offset = std::array::from_fn(|i| point[i] - self.origin[i]);
        self.axes.map(|axis| dot(axis, offset))
    }

    fn curve(&self, geometry: Option<AnalyticGeometry>) -> Result<Curve, GeometryError> {
        match geometry {
            Some(AnalyticGeometry::Line { a_mm, b_mm }) => {
                Ok(Curve::Line([self.point(a_mm), self.point(b_mm)]))
            }
            Some(AnalyticGeometry::CircularArc {
                frame,
                radius_mm,
                start_rad,
                sweep_rad,
            }) => {
                // Store an increasing trim starting at zero and a
                // counterclockwise screen basis, including rear-facing arcs.
                let reversed = dot(frame.normal(), self.toward_viewer) * sweep_rad < 0.;
                let start = if reversed {
                    start_rad + sweep_rad
                } else {
                    start_rad
                };
                let (sin, cos) = start.sin_cos();
                let direction = std::array::from_fn(|i| cos * frame.u[i] + sin * frame.v[i]);
                let [x, y] = self.axes.map(|axis| dot(axis, direction));
                let length = x.hypot(y);
                let u = [x / length, y / length];
                Ok(Curve::Ellipse(Ellipse {
                    center: self.point(frame.origin_mm),
                    u,
                    v: [-u[1], u[0]],
                    major: radius_mm,
                    minor: radius_mm,
                    trim: [0., sweep_rad.abs()],
                }))
            }
            _ => Err(err(
                GeometryErrorKind::InvalidResult,
                "Rust face-on panel has an invalid analytic rim",
            )),
        }
    }
}

fn bounds(wire: &[Curve]) -> [[f64; 2]; 2] {
    wire.iter().fold(
        [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]],
        |[lo, hi], curve| {
            let [low, high] = match *curve {
                Curve::Line([a, b]) => [
                    std::array::from_fn(|i| a[i].min(b[i])),
                    std::array::from_fn(|i| a[i].max(b[i])),
                ],
                Curve::Ellipse(arc) => [
                    arc.center.map(|x| x - arc.major),
                    arc.center.map(|x| x + arc.major),
                ],
            };
            [
                std::array::from_fn(|i| lo[i].min(low[i])),
                std::array::from_fn(|i| hi[i].max(high[i])),
            ]
        },
    )
}

fn overlap([a, b]: [Point; 2], [c, d]: [Point; 2]) -> bool {
    (0..2).all(|i| a[i] <= d[i] || same_coordinate(a[i], d[i]))
        && (0..2).all(|i| c[i] <= b[i] || same_coordinate(c[i], b[i]))
}

fn same_coordinate(a: f64, b: f64) -> bool {
    (a - b).abs() <= 32. * f64::EPSILON * a.abs().max(b.abs()).max(1.)
}

fn same_point(a: Point, b: Point) -> bool {
    a.into_iter().zip(b).all(|(a, b)| same_coordinate(a, b))
}

fn same_curve(a: Curve, b: Curve) -> bool {
    match (a, b) {
        (Curve::Line([a, b]), Curve::Line([c, d])) => {
            (same_point(a, c) && same_point(b, d)) || (same_point(a, d) && same_point(b, c))
        }
        (Curve::Ellipse(a), Curve::Ellipse(b)) => {
            same_point(a.center, b.center)
                && same_coordinate(a.major, b.major)
                && same_coordinate(a.trim[1], b.trim[1])
                && (a.trim[1] == TAU || same_point(a.u, b.u))
        }
        _ => false,
    }
}

fn same_wire(a: &[Curve], b: &[Curve]) -> bool {
    a.len() == b.len() && a.iter().all(|&a| b.iter().any(|&b| same_curve(a, b)))
}

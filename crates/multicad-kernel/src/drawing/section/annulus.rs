//! Plane cuts of a finite circular annulus. The section curves stay analytic.
//! Hatching uses a conservative chord approximation of the cut material.
use super::*;
use multicad_rust_kernel::curved_projection::{Curve, Ellipse, Error as CurveError};
use std::f64::consts::TAU;

type Point = [f64; 2];
type Span = [f64; 2];
type Segment = [Point; 2];
const ANGULAR_ROUNDOFF: f64 = 32. * f64::EPSILON;
const MAX_HATCH_RIM_SEGMENTS: usize = 100_000;

fn dot(a: Point, b: Point) -> f64 {
    a[0].mul_add(b[0], a[1] * b[1])
}

fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn at(origin: Point, direction: Point, distance: f64) -> Point {
    std::array::from_fn(|i| direction[i].mul_add(distance, origin[i]))
}

fn length(point: Point) -> f64 {
    point[0].hypot(point[1])
}

fn perpendicular([x, y]: Point) -> Point {
    [-y, x]
}

fn half_width(radius: f64, distance: f64) -> Option<f64> {
    (distance < radius).then(|| ((radius - distance) * (radius + distance)).sqrt())
}

fn limit() -> GeometryError {
    err(
        GeometryErrorKind::Limit,
        "Rust annulus section exceeded its geometry or output budget",
    )
}

#[derive(Clone, Copy)]
struct Frame2 {
    origin: Point,
    u: Point,
    v: Point,
}

impl Frame2 {
    fn point(self, [u, v]: Point) -> Point {
        at(at(self.origin, self.u, u), self.v, v)
    }

    fn window(self, windows: [Option<Span>; 2]) -> Vec<Halfspace> {
        windows
            .into_iter()
            .enumerate()
            .filter_map(|(i, window)| window.map(|window| (i, window)))
            .flat_map(|(i, [lo, hi])| {
                let normal = [self.u[i], self.v[i]];
                [
                    Halfspace {
                        normal,
                        max: hi - self.origin[i],
                    },
                    Halfspace {
                        normal: normal.map(|v| -v),
                        max: self.origin[i] - lo,
                    },
                ]
            })
            .collect()
    }
}

#[derive(Clone, Copy)]
struct Halfspace {
    normal: Point,
    max: f64,
}

impl Halfspace {
    fn signed_distance(self, point: Point) -> f64 {
        dot(self.normal, point) - self.max
    }
}

pub(in crate::drawing) fn project_circular(
    input: &DrawingSolid<'_>,
    options: DrawingProjection,
    depth: f64,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    if !options.section_only || options.detail.is_some() {
        return Err(err(
            GeometryErrorKind::Unsupported,
            "Rust circular sections require a section-only view without a detail crop",
        ));
    }
    let (placement, outer, h, bore) =
        curved::circular_dimensions(input.solid).ok_or_else(|| {
            err(
                GeometryErrorKind::Unsupported,
                "Rust circular section requires a circular cylinder or annulus",
            )
        })?;
    let axes = planar::view_axes(options.frame);
    let local = |point: [f64; 3]| {
        axes.map(|axis| axis.into_iter().zip(point).map(|(a, b)| a * b).sum::<f64>())
    };
    let base = local(std::array::from_fn(|i| {
        placement.origin[i] - options.frame.origin[i]
    }));
    let axis = local(placement.normal());
    let scale = axis[0].hypot(axis[1]).hypot(axis[2]);
    let [x, y, c] = axis.map(|coordinate| coordinate / scale);
    let s = x.hypot(y);
    let u = if s > ANGULAR_ROUNDOFF {
        [x / s, y / s]
    } else {
        [1., 0.]
    };
    let v = perpendicular(u);
    let origin = [base[0], base[1]];
    let delta_depth = depth - base[2];
    let inner = bore.unwrap_or(0.);
    let tolerance = input.solid.geometry_policy().linear_tolerance_mm;
    let radial_depth = s * outer;
    if delta_depth < (c * h).min(0.) - radial_depth - tolerance
        || delta_depth > (c * h).max(0.) + radial_depth + tolerance
    {
        return Ok(Vec::new());
    }
    let windows = [options.section_u_mm, options.section_v_mm];
    let hatch = input
        .hatch
        .filter(|hatch| hatch.pattern != HatchPattern::None);

    let (frame, output, boundary) = if c.abs() <= ANGULAR_ROUNDOFF {
        // The plane meets the cylinders along generators, or misses them
        // entirely. A tangent generator has no material area.
        let distance = delta_depth.abs();
        let Some(outer) = half_width(outer, distance) else {
            return Ok(Vec::new());
        };
        let inner = half_width(inner, distance).unwrap_or(0.);
        let frame = Frame2 { origin, u, v };
        let planes = frame.window(windows);
        let intervals = if inner > 0. {
            vec![[-outer, -inner], [inner, outer]]
        } else {
            vec![[-outer, outer]]
        };
        let mut boundary = Vec::new();
        for [lo, hi] in intervals {
            let corners = [[0., lo], [h, lo], [h, hi], [0., hi]];
            let ring = clip_polygon(corners, &planes, tolerance);
            boundary.extend(polygon_edges(&ring).map(|(ends, _)| ends));
        }
        let output = boundary
            .iter()
            .map(|ends| planar::line(0, VectorRole::Section, ends.map(|p| frame.point(p))))
            .collect();
        (frame, output, boundary)
    } else {
        // At axial coordinate z0 the section plane crosses the stock axis.
        // In section coordinates along and across the projected axis, the
        // cylinder cuts are homothetic ellipses centered at that intersection.
        let z0 = delta_depth / c;
        let frame = Frame2 {
            origin: at(origin, u, s * z0),
            u,
            v,
        };
        let mut planes = frame.window(windows);
        if s <= ANGULAR_ROUNDOFF {
            if z0 < -tolerance || z0 > h + tolerance {
                return Ok(Vec::new());
            }
        } else {
            planes.extend([
                Halfspace {
                    normal: [-1., 0.],
                    max: z0 / s,
                },
                Halfspace {
                    normal: [1., 0.],
                    max: (h - z0) / s,
                },
            ]);
        }
        if planes.iter().enumerate().any(|(i, plane)| {
            planes[..i].iter().any(|previous| {
                length(sub(plane.normal, previous.normal.map(|value| -value))) <= ANGULAR_ROUNDOFF
                    && plane.max + previous.max <= tolerance
            })
        }) {
            return Ok(Vec::new());
        }
        let outer = [outer / c.abs(), outer];
        let inner = [inner / c.abs(), inner];
        if frame.origin.iter().any(|coordinate| coordinate.abs() > 1e7) || outer[0] > 1e7 {
            return Err(limit());
        }
        let output = elliptical_section(frame, outer, inner, &planes, options, tolerance)?;
        let boundary = if hatch.is_some() && !output.is_empty() {
            hatch_boundary(outer, inner, &planes, options.chord_tolerance_mm, tolerance)?
        } else {
            Vec::new()
        };
        (frame, output, boundary)
    };

    finish_section(frame, output, boundary, hatch, options, tolerance)
}

pub(in crate::drawing) fn project_concentric(
    origin: Point,
    [outer, inner]: [f64; 2],
    input: &DrawingSolid<'_>,
    options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let frame = Frame2 {
        origin,
        u: [1., 0.],
        v: [0., 1.],
    };
    let outer = [outer; 2];
    let inner = [inner; 2];
    let planes = frame.window([options.section_u_mm, options.section_v_mm]);
    let tolerance = input.solid.geometry_policy().linear_tolerance_mm;
    let output = elliptical_section(frame, outer, inner, &planes, options, tolerance)?;
    let hatch = input
        .hatch
        .filter(|hatch| hatch.pattern != HatchPattern::None);
    let boundary = if hatch.is_some() && !output.is_empty() {
        hatch_boundary(outer, inner, &planes, options.chord_tolerance_mm, tolerance)?
    } else {
        Vec::new()
    };
    finish_section(frame, output, boundary, hatch, options, tolerance)
}

fn finish_section(
    frame: Frame2,
    mut output: Vec<ProjectedVector>,
    boundary: Vec<Segment>,
    hatch: Option<DrawingHatch>,
    options: DrawingProjection,
    tolerance: f64,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let mut curves = (options.maximum_curves as usize)
        .checked_sub(output.len())
        .ok_or_else(limit)?;
    let mut points = (options.maximum_points as usize)
        .checked_sub(output.iter().map(|vector| vector.preview_mm.len()).sum())
        .ok_or_else(limit)?;
    if let Some(hatch) = hatch {
        let boundary = boundary
            .into_iter()
            .map(|ends| ends.map(|p| frame.point(p)))
            .collect::<Vec<_>>();
        output.extend(publish_hatch(
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

fn elliptical_section(
    frame: Frame2,
    outer: Point,
    inner: Point,
    planes: &[Halfspace],
    options: DrawingProjection,
    tolerance: f64,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let mut points = options.maximum_points as usize;
    let mut output = Vec::new();
    for [major, minor] in [outer, inner] {
        if minor == 0. {
            continue;
        }
        for trim in ellipse_trims([major, minor], planes) {
            let ellipse = Ellipse {
                center: frame.origin,
                u: frame.u,
                v: frame.v,
                major,
                minor,
                trim,
            };
            let preview_mm = Curve::Ellipse(ellipse)
                .preview(options.chord_tolerance_mm, points)
                .map_err(|error| match error {
                    CurveError::Limit => limit(),
                    CurveError::Invalid => err(
                        GeometryErrorKind::InvalidResult,
                        "Invalid Rust annulus section ellipse",
                    ),
                })?;
            points -= preview_mm.len();
            output.push(ProjectedVector {
                source_index: 0,
                role: VectorRole::Section,
                curve: VectorCurve::Ellipse {
                    center: ellipse.center,
                    u: ellipse.u,
                    v: ellipse.v,
                    major_mm: major,
                    minor_mm: minor,
                    start_rad: trim[0],
                    end_rad: trim[1],
                },
                preview_mm,
            });
        }
    }
    for (i, plane) in planes.iter().enumerate() {
        let origin = at([0.; 2], plane.normal, plane.max);
        let direction = perpendicular(plane.normal);
        let Some(span) = ellipse_interval(outer, origin, direction)
            .and_then(|span| clip_span(origin, direction, span, planes, i, tolerance))
        else {
            continue;
        };
        let hole = (inner[1] > 0.)
            .then(|| ellipse_interval(inner, origin, direction))
            .flatten();
        for span in subtract_span(span, hole, tolerance) {
            output.push(planar::line(
                0,
                VectorRole::Section,
                span.map(|distance| frame.point(at(origin, direction, distance))),
            ));
        }
    }
    Ok(output)
}

fn ellipse_trims(radii: Point, planes: &[Halfspace]) -> Vec<Span> {
    let mut cuts = vec![0., TAU];
    for plane in planes {
        let x = radii[0] * plane.normal[0];
        let y = radii[1] * plane.normal[1];
        let relative = plane.max / x.hypot(y);
        if relative.abs() < 1. {
            let middle = y.atan2(x);
            let angle = relative.acos();
            cuts.extend([middle - angle, middle + angle].map(|t| t.rem_euclid(TAU)));
        }
    }
    cuts.sort_unstable_by(f64::total_cmp);
    cuts.dedup();
    let mut spans: Vec<Span> = Vec::new();
    for &[a, b] in cuts.array_windows() {
        let (sin, cos) = a.midpoint(b).sin_cos();
        let point = [radii[0] * cos, radii[1] * sin];
        if planes
            .iter()
            .all(|plane| plane.signed_distance(point) <= 0.)
        {
            if let Some(span) = spans.last_mut().filter(|span| span[1] == a) {
                span[1] = b;
            } else {
                spans.push([a, b]);
            }
        }
    }
    spans
}

fn ellipse_interval(radii: Point, origin: Point, direction: Point) -> Option<Span> {
    let divide = |p: Point| std::array::from_fn(|i| p[i] / radii[i]);
    let direction = divide(direction);
    let speed = length(direction);
    let axis = direction.map(|x| x / speed);
    let origin = divide(origin);
    let distance = dot(origin, perpendicular(axis)).abs();
    if distance >= 1. {
        return None;
    }
    let along = -dot(origin, axis);
    let half = ((1. - distance) * (1. + distance)).sqrt();
    Some([(along - half) / speed, (along + half) / speed])
}

fn clip_span(
    origin: Point,
    direction: Point,
    mut span: Span,
    planes: &[Halfspace],
    own: usize,
    tolerance: f64,
) -> Option<Span> {
    for (i, plane) in planes.iter().enumerate() {
        if i == own {
            continue;
        }
        let slope = dot(plane.normal, direction);
        let offset = plane.signed_distance(origin);
        if slope.abs() <= ANGULAR_ROUNDOFF {
            if offset > tolerance || (offset.abs() <= tolerance && i < own) {
                return None;
            }
        } else if slope > 0. {
            span[1] = span[1].min(-offset / slope);
        } else {
            span[0] = span[0].max(-offset / slope);
        }
    }
    (span[1] - span[0] > tolerance).then_some(span)
}

fn subtract_span(span: Span, hole: Option<Span>, tolerance: f64) -> Vec<Span> {
    let [lo, hi] = span;
    let Some([a, b]) = hole else {
        return vec![span];
    };
    [[lo, hi.min(a)], [lo.max(b), hi]]
        .into_iter()
        .filter(|[a, b]| b - a > tolerance)
        .collect()
}

#[derive(Clone, Copy)]
struct Corner {
    point: Point,
    incoming: Option<usize>,
}

fn polygon_edges(ring: &[Corner]) -> impl Iterator<Item = (Segment, Option<usize>)> + '_ {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(a, b)| ([a.point, b.point], b.incoming))
}

fn clip_polygon(
    points: impl IntoIterator<Item = Point>,
    planes: &[Halfspace],
    tolerance: f64,
) -> Vec<Corner> {
    let mut ring = points
        .into_iter()
        .map(|point| Corner {
            point,
            incoming: None,
        })
        .collect::<Vec<_>>();
    for (plane_index, plane) in planes.iter().enumerate() {
        let mut result: Vec<Corner> = Vec::new();
        let mut add = |point: Point, incoming| {
            if result
                .last()
                .is_none_or(|last| length(sub(last.point, point)) > tolerance)
            {
                result.push(Corner { point, incoming });
            }
        };
        for ([a, b], tag) in polygon_edges(&ring) {
            let da = plane.signed_distance(a);
            let db = plane.signed_distance(b);
            if (da <= 0.) != (db <= 0.) {
                let point = at(a, sub(b, a), da / (da - db));
                add(point, if da <= 0. { tag } else { Some(plane_index) });
            }
            if db <= 0. {
                let incoming = if da.abs().max(db.abs()) <= tolerance && tag.is_none() {
                    Some(plane_index)
                } else {
                    tag
                };
                add(b, incoming);
            }
        }
        if result.len() > 1
            && length(sub(result[0].point, result.last().unwrap().point)) <= tolerance
        {
            result[0].incoming = result.pop().unwrap().incoming;
        }
        if result.len() < 3 {
            return Vec::new();
        }
        ring = result;
    }
    ring
}

fn hatch_boundary(
    outer: Point,
    inner: Point,
    planes: &[Halfspace],
    deflection: f64,
    tolerance: f64,
) -> Result<Vec<Segment>, GeometryError> {
    // Use an inscribed outer polygon and a circumscribed hole so no
    // annotation enters the bore or leaves the actual section material.
    // The midpoint error of the circumscribed ring is bounded by twice
    // the same-angle sagitta. Also keep the two polygon rings nested.
    let target = deflection.min((outer[1] - inner[1]) / 4.);
    let step = 4. * (target / (4. * outer[0])).sqrt().min(1.).asin();
    let count = (TAU / step).ceil().max(8.);
    if count > MAX_HATCH_RIM_SEGMENTS as f64 {
        return Err(limit());
    }
    let count = count as usize;
    let sample = |radii: Point, scale| {
        clip_polygon(
            (0..count).map(|i| {
                let (sin, cos) = (TAU * i as f64 / count as f64).sin_cos();
                [scale * radii[0] * cos, scale * radii[1] * sin]
            }),
            planes,
            tolerance,
        )
    };
    let outer = sample(outer, 1.);
    let inner = if inner[1] > 0. {
        sample(inner, 1. / (std::f64::consts::PI / count as f64).cos())
    } else {
        Vec::new()
    };
    let mut boundary = polygon_edges(&outer)
        .chain(polygon_edges(&inner))
        .filter_map(|(ends, tag)| tag.is_none().then_some(ends))
        .collect::<Vec<_>>();
    for (i, plane) in planes.iter().enumerate() {
        let direction = perpendicular(plane.normal);
        let span = |ring: &[Corner]| {
            let points = polygon_edges(ring)
                .filter(|(_, tag)| *tag == Some(i))
                .flat_map(|(ends, _)| ends);
            let [a, b] = points.fold([None, None], |[a, b]: [Option<Point>; 2], p| {
                [
                    Some(
                        a.filter(|&a| dot(a, direction) <= dot(p, direction))
                            .unwrap_or(p),
                    ),
                    Some(
                        b.filter(|&b| dot(b, direction) >= dot(p, direction))
                            .unwrap_or(p),
                    ),
                ]
            });
            Some([a?, b?])
        };
        let Some([a, b]) = span(&outer) else {
            continue;
        };
        let mut add = |start: Point, end: Point| {
            if dot(sub(end, start), direction) > tolerance {
                boundary.push([start, end]);
            }
        };
        if let Some([c, d]) = span(&inner) {
            add(a, c);
            add(d, b);
        } else {
            add(a, b);
        }
    }
    Ok(boundary)
}

//! Publish exact loft cuts, tessellating only their two-dimensional previews.
use super::*;
use multicad_rust_kernel::planar_boolean::{Error, LoftSectionCurve};

mod clip;

type Point = [f64; 2];

#[derive(Clone, Copy)]
enum Curve {
    Line([Point; 2]),
    Quadratic([Control; 3]),
}

impl Curve {
    fn ends(self) -> [Point; 2] {
        match self {
            Self::Line(ends) => ends,
            Self::Quadratic([a, _, b]) => [a.point, b.point],
        }
    }
}

pub(super) fn project(
    solid: &Solid,
    options: DrawingProjection,
    source: usize,
    depth: f64,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let (body, placement) = solid.nonplanar_loft_body().expect("loft drawing source");
    let frame = options.frame;
    let axes = planar::view_axes(frame);
    let origin = std::array::from_fn(|i| axes[2][i].mul_add(depth, frame.origin[i]));
    let tolerance = solid.geometry_policy().linear_tolerance_mm;
    let curves = body
        .section(placement, origin, axes[2], tolerance)
        .map_err(|cause| {
            err(
                match cause {
                    Error::Invalid => GeometryErrorKind::Input,
                    Error::Unresolved => GeometryErrorKind::InvalidResult,
                    Error::Limit => GeometryErrorKind::Limit,
                },
                format!("Rust loft section failed: {cause:?}"),
            )
        })?;
    let origin_delta = std::array::from_fn(|i| placement.origin[i] - frame.origin[i]);
    let dot = |a: [f64; 3], b: [f64; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
    let [u, v] = [axes[0], axes[1]];
    let origin = [dot(u, origin_delta), dot(v, origin_delta)];
    let directions = [placement.u, placement.v, placement.normal()];
    let [u, v] = [u, v].map(|axis| directions.map(|direction| dot(axis, direction)));
    let project = |point| [dot(u, point) + origin[0], dot(v, point) + origin[1]];
    let mut curves: Vec<_> = curves
        .into_iter()
        .map(|curve| match curve {
            LoftSectionCurve::Line(ends) => Curve::Line(ends.map(project)),
            LoftSectionCurve::Quadratic { poles, weights } => {
                Curve::Quadratic(std::array::from_fn(|i| Control {
                    point: project(poles[i]),
                    weight: weights[i],
                }))
            }
        })
        .collect();
    for (axis, window) in [options.section_u_mm, options.section_v_mm]
        .into_iter()
        .enumerate()
    {
        if let Some([lower, upper]) = window {
            curves = clip::half_plane(curves, axis, lower, 1., tolerance)?;
            curves = clip::half_plane(curves, axis, upper, -1., tolerance)?;
        }
    }
    if curves.len() > options.maximum_curves as usize {
        return Err(limit());
    }
    let mut points = options.maximum_points as usize;
    curves
        .into_iter()
        .map(|curve| {
            let vector = match curve {
                Curve::Line(ends) => {
                    charge(&mut points, 2)?;
                    planar::line(source, VectorRole::Section, ends)
                }
                Curve::Quadratic(controls) => {
                    let mut preview = vec![controls[0].point];
                    charge(&mut points, 1)?;
                    sample(
                        controls,
                        options.chord_tolerance_mm,
                        0,
                        &mut points,
                        &mut preview,
                    )?;
                    ProjectedVector {
                        source_index: source,
                        role: VectorRole::Section,
                        curve: VectorCurve::BSpline {
                            degree: 2,
                            periodic: false,
                            poles: controls.map(|control| control.point).to_vec(),
                            weights: controls.map(|control| control.weight).to_vec(),
                            knots: vec![0., 1.],
                            multiplicities: vec![3, 3],
                            first: 0.,
                            last: 1.,
                        },
                        preview_mm: preview,
                    }
                }
            };
            Ok(vector)
        })
        .collect()
}

fn limit() -> GeometryError {
    err(
        GeometryErrorKind::Limit,
        "Rust loft section exceeded its curve or point budget",
    )
}

fn charge(remaining: &mut usize, count: usize) -> Result<(), GeometryError> {
    *remaining = remaining.checked_sub(count).ok_or_else(limit)?;
    Ok(())
}

#[derive(Clone, Copy)]
struct Control {
    point: Point,
    weight: f64,
}

impl Control {
    fn interpolate(self, other: Self, t: f64) -> Self {
        let second = other.weight * t;
        let weight = (1. - t).mul_add(self.weight, second);
        Self {
            point: interpolate(self.point, other.point, second / weight),
            weight,
        }
    }
}

fn interpolate(a: Point, b: Point, t: f64) -> Point {
    std::array::from_fn(|i| t.mul_add(b[i] - a[i], a[i]))
}

fn subdivide([a, b, c]: [Control; 3], t: f64) -> [[Control; 3]; 2] {
    let left = a.interpolate(b, t);
    let right = b.interpolate(c, t);
    let mid = left.interpolate(right, t);
    [[a, left, mid], [mid, right, c]]
}

fn sample(
    [a, b, c]: [Control; 3],
    deflection: f64,
    depth: u32,
    remaining: &mut usize,
    output: &mut Vec<Point>,
) -> Result<(), GeometryError> {
    let edge = std::array::from_fn::<_, 2, _>(|i| c.point[i] - a.point[i]);
    let offset = std::array::from_fn::<_, 2, _>(|i| b.point[i] - a.point[i]);
    let length_squared = edge[0].mul_add(edge[0], edge[1] * edge[1]);
    let t = if length_squared > 0. {
        (offset[0].mul_add(edge[0], offset[1] * edge[1]) / length_squared).clamp(0., 1.)
    } else {
        0.
    };
    // With positive weights, every subarc lies in its control triangle.
    // The middle control's distance to the chord bounds the entire subarc.
    if offset[0]
        .mul_add(1., -t * edge[0])
        .hypot(offset[1] - t * edge[1])
        <= deflection
    {
        charge(remaining, 1)?;
        output.push(c.point);
        return Ok(());
    }
    if depth == 48 {
        return Err(limit());
    }
    let [left, right] = subdivide([a, b, c], 0.5);
    sample(left, deflection, depth + 1, remaining, output)?;
    sample(right, deflection, depth + 1, remaining, output)
}

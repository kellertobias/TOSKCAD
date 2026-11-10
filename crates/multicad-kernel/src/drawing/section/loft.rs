//! Publish exact loft cuts, tessellating only their two-dimensional previews.
use super::*;
use multicad_rust_kernel::planar_boolean::{Error, LoftSectionCurve};

mod clip;
mod scene;
pub(in crate::drawing) use scene::project_scene;
mod view;
pub(in crate::drawing) use view::project_view;

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
    let projection = Frame::new(placement, frame.origin, axes);
    let mut curves: Vec<_> = curves
        .into_iter()
        .map(|curve| projection.curve(curve))
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
    publish(
        curves
            .into_iter()
            .map(|curve| (source, curve, VectorRole::Section)),
        options,
    )
}

struct Frame {
    axes: [[f64; 3]; 3],
    offset: [f64; 3],
}

impl Frame {
    fn new(placement: PlaneFrame, origin: [f64; 3], screen: [[f64; 3]; 3]) -> Self {
        let delta = std::array::from_fn(|i| placement.origin[i] - origin[i]);
        let directions = [placement.u, placement.v, placement.normal()];
        Self {
            axes: screen.map(|axis| directions.map(|direction| dot(axis, direction))),
            offset: screen.map(|axis| dot(axis, delta)),
        }
    }

    fn curve(&self, curve: LoftSectionCurve) -> Curve {
        map_curve(curve, |point| {
            [
                dot(self.axes[0], point) + self.offset[0],
                dot(self.axes[1], point) + self.offset[1],
            ]
        })
    }
}

fn map_curve(curve: LoftSectionCurve, project: impl Fn([f64; 3]) -> Point) -> Curve {
    match curve {
        LoftSectionCurve::Line(ends) => Curve::Line(ends.map(project)),
        LoftSectionCurve::Quadratic { poles, weights } => {
            Curve::Quadratic(std::array::from_fn(|i| Control {
                point: project(poles[i]),
                weight: weights[i],
            }))
        }
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn publish(
    curves: impl ExactSizeIterator<Item = (usize, Curve, VectorRole)>,
    options: DrawingProjection,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    if curves.len() > options.maximum_curves as usize {
        return Err(limit());
    }
    let mut points = options.maximum_points as usize;
    curves
        .map(|(source, curve, role)| {
            let vector = match curve {
                Curve::Line(ends) => {
                    charge(&mut points, 2)?;
                    planar::line(source, role, ends)
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
                        role,
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

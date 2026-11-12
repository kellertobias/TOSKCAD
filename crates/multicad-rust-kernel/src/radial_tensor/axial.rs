//! Common physical axis and retained near-affine axial parameter receipts.
use super::*;
use crate::tapered_thread::{RadialChart, certify_radial_charts};

#[derive(Clone)]
pub struct AxialBoundary {
    pub(super) boundary: TrimmedBoundary,
    pub(super) chart: Chart,
    pub(super) tensors: Vec<ChartTensor>,
    pub(super) caps: [usize; 2],
    pub(super) cones: Vec<usize>,
    pub(super) axial_error: f64,
    pub(super) tolerance: f64,
}
#[derive(Clone)]
pub(super) struct Chart {
    pub axis_origin: Point,
    pub axis_normal: Point,
    pub bottom_point: Point,
    pub axes: [[I; 3]; 3],
    pub height: I,
    pub nominal_height: f64,
    pub radius: I,
    pub radius_range: I,
    pub slope: I,
}
#[derive(Clone)]
pub(super) struct ChartTensor {
    pub face: usize,
    pub surface: RetainedTensor,
    pub rims: [(usize, bool); 2],
    pub spans: [usize; 2],
    pub controls: Vec<[I; 3]>,
    pub reversed_v: bool,
}
impl Chart {
    pub fn project(&self, p: Point) -> [I; 3] {
        self.project_interval(p.map(I::point))
    }
    pub fn project_interval(&self, p: [I; 3]) -> [I; 3] {
        let d = std::array::from_fn(|i| p[i] - I::point(self.axis_origin[i]));
        let z = std::array::from_fn(|i| p[i] - I::point(self.bottom_point[i]));
        [
            dot(d, self.axes[0]),
            dot(d, self.axes[1]),
            dot(z, self.axes[2]),
        ]
    }
}
impl AxialBoundary {
    pub fn input(&self) -> &BoundaryInput {
        self.boundary.input()
    }
    pub fn height_bounds_mm(&self) -> [f64; 2] {
        [self.chart.height.lo, self.chart.height.hi]
    }
    pub fn start_radius_bounds_mm(&self) -> [f64; 2] {
        [self.chart.radius.lo, self.chart.radius.hi]
    }
    pub fn axial_parameter_error_mm(&self) -> f64 {
        self.axial_error
    }
}
impl TrimmedBoundary {
    /// Certify an axis/chart without changing the original finite nets.
    /// Ordered material and embedding remain separate obligations.
    pub fn certify_axial(
        self,
        tolerance: f64,
        minimum: f64,
        interrupt: Option<&dyn Fn() -> bool>,
    ) -> Result<AxialBoundary, Error> {
        if !tolerance.is_finite() || tolerance <= 0. || !minimum.is_finite() || minimum <= 0. {
            return Err(Error::Invalid);
        }
        if self.error_mm > tolerance / 16. {
            return Err(Error::Unresolved);
        }
        let mut work = Work {
            remaining: 2_000_000,
            interrupt,
        };
        work.step()?;
        let input = self.input();
        let mut planes = Vec::new();
        let mut cones = Vec::new();
        for (i, face) in input.faces.iter().enumerate() {
            match &input.surfaces[face.surface] {
                Surface::Plane(axis) => planes.push((i, *axis)),
                Surface::Cone {
                    axis,
                    radius_mm,
                    semi_angle_rad,
                } => cones.push((i, *axis, *radius_mm, *semi_angle_rad)),
                Surface::Tensor(_) => {}
            }
        }
        let n = cones.len();
        if !(1..=8).contains(&n)
            || planes.len() != 2
            || self.tensors.len() != 3 * n
            || input.faces.len() != 4 * n + 2
        {
            return Err(Error::Unsupported);
        }
        let (_, axis, radius, angle) = cones[0];
        let basis = checked_axis(axis)?;
        let dominant = (0..3)
            .max_by(|&a, &b| axis.normal[a].abs().total_cmp(&axis.normal[b].abs()))
            .ok_or(Error::Invalid)?;
        let sign = if axis.normal[dominant] < 0. { -1. } else { 1. };
        let axes = [
            basis[0],
            basis[1].map(|v| v * I::point(sign)),
            basis[2].map(|v| v * I::point(sign)),
        ];
        let mut caps = [None; 2];
        for &(face, plane) in &planes {
            let f = &input.faces[face];
            let direction = if plane.normal == axis.normal {
                1.
            } else if plane.normal == axis.normal.map(|v| -v) {
                -1.
            } else {
                return Err(Error::Unsupported);
            };
            let top = direction * sign * if f.forward { 1. } else { -1. } > 0.;
            let end = usize::from(top);
            if caps[end].replace((face, plane.origin)).is_some() {
                return Err(Error::Invalid);
            }
        }
        let [bottom, top] = caps;
        let (bottom, bottom_point) = bottom.ok_or(Error::Invalid)?;
        let (top, top_point) = top.ok_or(Error::Invalid)?;
        let delta = std::array::from_fn(|i| I::point(top_point[i]) - I::point(bottom_point[i]));
        let height = dot(delta, axes[2]).positive()?;
        if height.lo < minimum || height.lo <= 32. * tolerance || height.hi > 2e7 {
            return Err(Error::Unresolved);
        }
        for &(face, a, r, alpha) in &cones {
            let f = &input.faces[face];
            if !f.forward
                || a.origin != axis.origin
                || a.normal != axis.normal
                || r != radius
                || alpha != angle
            {
                return Err(Error::Unsupported);
            }
        }
        let base = std::array::from_fn(|i| I::point(bottom_point[i]) - I::point(axis.origin[i]));
        let unsigned = if angle == 0. {
            I::point(0.)
        } else {
            I::point(angle).sin() / I::point(angle).cos()
        };
        unsigned.checked()?;
        let r0 = (I::point(radius) + unsigned * dot(base, basis[2])).positive()?;
        let slope = unsigned * I::point(sign);
        let r1 = (r0 + slope * height).positive()?;
        if r0.lo.min(r1.lo) < minimum || r0.lo.min(r1.lo) <= 32. * tolerance {
            return Err(Error::Unresolved);
        }
        let nominal_height = height.midpoint();
        let chart = Chart {
            axis_origin: axis.origin,
            axis_normal: axis.normal,
            bottom_point,
            axes,
            height,
            nominal_height,
            radius: r0,
            radius_range: I::new(r0.lo.min(r1.lo), r0.hi.max(r1.hi)),
            slope,
        };
        let mut charts = Vec::with_capacity(self.tensors.len());
        let mut axial_error = 0f64;
        for bound in &self.tensors {
            let face = bound.face;
            let t = &bound.surface;
            work.step()?;
            let spans = t.spans();
            let [width, height] = t.dimensions();
            let mut controls = Vec::with_capacity(t.poles().len());
            for chunk in t.poles().chunks(128) {
                work.step()?;
                for &p in chunk {
                    let q = chart.project(p);
                    for coordinate in q {
                        coordinate.checked()?;
                    }
                    controls.push(q);
                }
            }
            let mut increasing = true;
            let mut decreasing = true;
            for row in controls.chunks_exact(height) {
                for pair in row.windows(2) {
                    let dz = pair[1][2] - pair[0][2];
                    increasing &= dz.lo > 0.;
                    decreasing &= dz.hi < 0.;
                }
            }
            if !increasing && !decreasing {
                return Err(Error::Unresolved);
            }
            let reversed_v = decreasing;
            if reversed_v {
                for row in controls.chunks_exact_mut(height) {
                    row.reverse();
                }
            }
            let reversed_u = !(input.faces[face].forward ^ reversed_v);
            if reversed_u {
                controls = controls
                    .chunks_exact(height)
                    .rev()
                    .flat_map(|row| row.iter().copied())
                    .collect();
            }
            for row in controls.chunks_exact(height) {
                for (j, p) in row.iter().enumerate() {
                    let expected = I::point(nominal_height) * I::point(j as f64)
                        / I::point((height - 1) as f64);
                    axial_error = axial_error.max((p[2] - expected).magnitude());
                }
            }
            if controls.len() != width * height || axial_error > tolerance / 32. {
                return Err(Error::Unresolved);
            }
            let rims = std::array::from_fn(|end| {
                let raw_end = end ^ usize::from(reversed_v);
                let (edge, forward) = bound.edges[2 * raw_end];
                (edge, forward ^ (reversed_u ^ (raw_end == 1)))
            });
            charts.push(ChartTensor {
                face,
                surface: t.clone(),
                rims,
                spans,
                controls,
                reversed_v,
            });
        }
        let checks = charts
            .iter()
            .map(|t| RadialChart {
                spans: t.spans,
                controls: &t.controls,
                outward: true,
            })
            .collect::<Vec<_>>();
        certify_radial_charts(&checks, interrupt)?;
        Ok(AxialBoundary {
            boundary: self,
            chart,
            tensors: charts,
            caps: [bottom, top],
            cones: cones.into_iter().map(|(face, ..)| face).collect(),
            axial_error,
            tolerance,
        })
    }
}

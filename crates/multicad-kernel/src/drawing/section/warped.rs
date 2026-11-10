//! Publish the exact line and circle intersection of a trimmed analytic shell.
use super::*;
use multicad_rust_kernel::{
    curved_projection::{Curve, Edge, Ellipse},
    warped_union::{Curve as Trim, Error, SectionEdge, Shell},
};
use std::f64::consts::FRAC_PI_2;

type Point = [f64; 3];
type Segment = [[f64; 2]; 2];

pub(super) fn project(
    shell: &Shell,
    placement: PlaneFrame,
    options: DrawingProjection,
    policy: GeometryPolicy,
    source: usize,
    depth: f64,
    hatch: bool,
) -> Result<(Vec<ProjectedVector>, Vec<Segment>), GeometryError> {
    let frame = Frame::new(placement, options.frame);
    let normal = frame.directions.map(|axis| axis[2]);
    let offset = (depth - frame.origin[2]) / dot(normal, normal);
    let origin = normal.map(|component| offset * component);
    let tolerance = policy.linear_tolerance_mm;
    let loops = shell
        .section(origin, normal, tolerance, options.maximum_curves as usize)
        .map_err(|error| match error {
            Error::Invalid => err(
                GeometryErrorKind::InvalidResult,
                "Invalid trimmed shell section geometry",
            ),
            Error::Limit => limit(),
            Error::Unsupported => err(
                GeometryErrorKind::Unsupported,
                "Rust trimmed shell section requires a transverse cut through rectangular cylinder patches",
            ),
        })?;
    let windows = [options.section_u_mm, options.section_v_mm];
    if windows.iter().any(Option::is_some) {
        let bounds = frame.bounds(&loops);
        if windows
            .into_iter()
            .zip(bounds)
            .any(|(window, [lo, hi])| window.is_some_and(|[a, b]| hi <= a || lo >= b))
        {
            return Ok((Vec::new(), Vec::new()));
        }
        if windows
            .into_iter()
            .zip(bounds)
            .any(|(window, [lo, hi])| window.is_some_and(|[a, b]| lo < a || hi > b))
        {
            return Err(err(
                GeometryErrorKind::Unsupported,
                "Rust trimmed shell section cannot yet clip a curved section window",
            ));
        }
    }
    let edges = loops
        .iter()
        .flatten()
        .map(|&edge| Edge {
            curve: frame.curve(edge),
            hidden: false,
        })
        .collect();
    let mut output = curved::publish(edges, options, source)?;
    for vector in &mut output {
        vector.role = VectorRole::Section;
    }
    let boundary = if hatch {
        frame.hatch(
            &loops,
            options
                .chord_tolerance_mm
                .min(policy.minimum_feature_mm / 4.),
        )?
    } else {
        Vec::new()
    };
    Ok((output, boundary))
}

struct Frame {
    origin: Point,
    directions: [Point; 3],
}

impl Frame {
    fn new(placement: PlaneFrame, view: PlaneFrame) -> Self {
        let screen = planar::view_axes(view);
        let directions = planar::view_axes(placement).map(|axis| screen.map(|d| dot(axis, d)));
        let origin = screen.map(|d| dot(sub(placement.origin, view.origin), d));
        Self { origin, directions }
    }

    fn point(&self, p: Point) -> [f64; 2] {
        let offset = self.vector(p);
        [self.origin[0] + offset[0], self.origin[1] + offset[1]]
    }

    fn vector(&self, p: Point) -> [f64; 2] {
        std::array::from_fn(|i| dot(p, self.directions.map(|axis| axis[i])))
    }

    fn curve(&self, edge: SectionEdge) -> Curve {
        match edge.curve {
            Trim::Line => Curve::Line(edge.ends_mm.map(|p| self.point(p))),
            Trim::Ellipse(arc) => {
                let ellipse = arc.ellipse();
                let [a, b] = [ellipse.major_axis(), ellipse.minor_axis()];
                let [major, minor] = ellipse.semiaxes_mm();
                let (sin, cos) = arc.start_rad().sin_cos();
                let sign = arc.sweep_rad().signum();
                let radial = std::array::from_fn(|i| a[i] * cos + b[i] * sin);
                let tangent = std::array::from_fn(|i| (-a[i] * sin + b[i] * cos) * sign);
                Curve::Ellipse(Ellipse {
                    center: self.point(ellipse.center_mm()),
                    u: self.vector(radial),
                    v: self.vector(tangent),
                    major,
                    minor,
                    trim: [0., arc.sweep_rad().abs()],
                })
            }
        }
    }

    fn bounds(&self, loops: &[Vec<SectionEdge>]) -> [[f64; 2]; 2] {
        std::array::from_fn(|axis| {
            let direction = self.directions.map(|v| v[axis]);
            loops
                .iter()
                .flatten()
                .fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], edge| {
                    let [a, b] = match edge.curve {
                        Trim::Line => {
                            let [a, b] = edge.ends_mm.map(|p| dot(direction, p));
                            [a.min(b), a.max(b)]
                        }
                        Trim::Ellipse(arc) => arc.range_along(direction),
                    };
                    [lo.min(a + self.origin[axis]), hi.max(b + self.origin[axis])]
                })
        })
    }

    fn hatch(
        &self,
        loops: &[Vec<SectionEdge>],
        deflection: f64,
    ) -> Result<Vec<[[f64; 2]; 2]>, GeometryError> {
        const LIMIT: usize = 100_000;
        let mut boundary = Vec::new();
        for &edge in loops.iter().flatten() {
            let [start, end] = edge.ends_mm.map(|p| self.point(p));
            match self.curve(edge) {
                Curve::Line(_) => boundary.push([start, end]),
                Curve::Ellipse(arc) => {
                    let clockwise = arc.u[0] * arc.v[1] - arc.u[1] * arc.v[0] < 0.;
                    let radius = arc.major;
                    let target = 4. * (deflection / (4. * (radius + deflection))).sqrt().asin();
                    let count = (arc.trim[1] / target.min(FRAC_PI_2)).ceil() as usize;
                    if count > (LIMIT - boundary.len()) / (1 + usize::from(clockwise)) {
                        return Err(limit());
                    }
                    let step = arc.trim[1] / count as f64;
                    let at = |angle: f64, radius: f64| {
                        let (sin, cos) = angle.sin_cos();
                        std::array::from_fn(|i| {
                            arc.center[i] + radius * (arc.u[i] * cos + arc.v[i] * sin)
                        })
                    };
                    let mut previous = start;
                    for i in 1..=count {
                        let next = if i == count {
                            end
                        } else {
                            at(i as f64 * step, radius)
                        };
                        if clockwise {
                            // Use tangent segments on concave trims to keep
                            // the published fill on the material side.
                            let tip = at((i as f64 - 0.5) * step, radius / (step / 2.).cos());
                            boundary.push([previous, tip]);
                            previous = tip;
                        }
                        boundary.push([previous, next]);
                        previous = next;
                    }
                }
            }
            if boundary.len() >= LIMIT {
                return Err(limit());
            }
        }
        Ok(boundary)
    }
}

fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn dot(a: Point, b: Point) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn limit() -> GeometryError {
    err(
        GeometryErrorKind::Limit,
        "Rust trimmed shell section exceeded its geometry budget",
    )
}

//! Publish the exact line and conic intersections of a trimmed analytic shell.
use super::*;
use multicad_rust_kernel::{
    curved_projection::{Curve, Edge, Ellipse},
    warped_union::{
        Curve as Trim, Error, SectionClipPlane, SectionEdge, Shell, clip_section_loops,
    },
};
use std::f64::consts::{FRAC_PI_2, TAU};

mod retained;
pub(super) use retained::project as project_retained;

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
    let loops = frame.intersection(shell, depth, options.maximum_curves, policy)?;
    let loops = frame.clip(loops, options, policy)?;
    let output = frame.vectors(&loops, options, source)?;
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

    fn intersection(
        &self,
        shell: &Shell,
        depth: f64,
        maximum_curves: u32,
        policy: GeometryPolicy,
    ) -> Result<Vec<Vec<SectionEdge>>, GeometryError> {
        self.section_at(shell, 2, depth, maximum_curves, policy)
    }

    fn local_plane(&self, axis: usize, coordinate: f64) -> (Point, Point) {
        let normal = self.directions.map(|direction| direction[axis]);
        let offset = (coordinate - self.origin[axis]) / dot(normal, normal);
        (normal.map(|component| offset * component), normal)
    }

    fn section_at(
        &self,
        shell: &Shell,
        axis: usize,
        coordinate: f64,
        maximum_curves: u32,
        policy: GeometryPolicy,
    ) -> Result<Vec<Vec<SectionEdge>>, GeometryError> {
        let (origin, normal) = self.local_plane(axis, coordinate);
        shell
            .section(
                origin,
                normal,
                policy.linear_tolerance_mm,
                maximum_curves as usize,
            )
            .map_err(section_error)
    }

    fn clip(
        &self,
        loops: Vec<Vec<SectionEdge>>,
        options: DrawingProjection,
        policy: GeometryPolicy,
    ) -> Result<Vec<Vec<SectionEdge>>, GeometryError> {
        let windows = [options.section_u_mm, options.section_v_mm];
        if loops.is_empty() || windows.iter().all(Option::is_none) {
            return Ok(loops);
        }
        let mut planes = Vec::new();
        for (axis, (window, [lo, hi])) in windows.into_iter().zip(self.bounds(&loops)).enumerate() {
            let Some([a, b]) = window else { continue };
            if hi <= a || lo >= b {
                return Ok(Vec::new());
            }
            for (active, bound, sign) in [(lo < a, a, 1.), (hi > b, b, -1.)] {
                if !active {
                    continue;
                }
                let (origin_mm, normal) = self.local_plane(axis, bound);
                planes.push(SectionClipPlane {
                    origin_mm,
                    inward_normal: normal.map(|value| value * sign),
                });
            }
        }
        if planes.is_empty() {
            return Ok(loops);
        }
        clip_section_loops(
            loops,
            self.directions.map(|direction| direction[2]),
            &planes,
            policy.linear_tolerance_mm,
            options.maximum_curves as usize,
        )
        .map_err(section_error)
    }

    fn vectors(
        &self,
        loops: &[Vec<SectionEdge>],
        options: DrawingProjection,
        source: usize,
    ) -> Result<Vec<ProjectedVector>, GeometryError> {
        let edges = loops
            .iter()
            .flatten()
            .flat_map(|&edge| self.curves(edge).into_iter().flatten())
            .map(|curve| Edge {
                curve,
                hidden: false,
            })
            .collect();
        let mut output = curved::publish(edges, options, source)?;
        for vector in &mut output {
            vector.role = VectorRole::Section;
        }
        Ok(output)
    }

    fn point(&self, p: Point) -> [f64; 2] {
        let offset = self.vector(p);
        [self.origin[0] + offset[0], self.origin[1] + offset[1]]
    }

    fn vector(&self, p: Point) -> [f64; 2] {
        std::array::from_fn(|i| dot(p, self.directions.map(|axis| axis[i])))
    }

    fn ellipse(&self, arc: multicad_rust_kernel::trim_curve::EllipseArc) -> Ellipse {
        let ellipse = arc.ellipse();
        let [major, minor] = ellipse.semiaxes_mm();
        let sign = arc.sweep_rad().signum();
        let [u, v] = [ellipse.major_axis(), ellipse.minor_axis()].map(|axis| self.vector(axis));
        let sweep = arc.sweep_rad().abs();
        if major == minor {
            let (sin, cos) = arc.start_rad().sin_cos();
            return Ellipse {
                center: self.point(ellipse.center_mm()),
                u: std::array::from_fn(|i| u[i] * cos + v[i] * sin),
                v: std::array::from_fn(|i| (-u[i] * sin + v[i] * cos) * sign),
                major,
                minor,
                trim: [0., sweep],
            };
        }
        let start = (sign * arc.start_rad()).rem_euclid(TAU);
        Ellipse {
            center: self.point(ellipse.center_mm()),
            u,
            v: v.map(|value| value * sign),
            major,
            minor,
            trim: [start, start + sweep],
        }
    }

    fn curves(&self, edge: SectionEdge) -> [Option<Curve>; 2] {
        match edge.curve {
            Trim::Line => [Some(Curve::Line(edge.ends_mm.map(|p| self.point(p)))), None],
            Trim::Ellipse(arc) => {
                let ellipse = self.ellipse(arc);
                let [start, end] = ellipse.trim;
                [[start, end.min(TAU)], [0., end - TAU]].map(|trim| {
                    (trim[1] > trim[0]).then_some(Curve::Ellipse(Ellipse { trim, ..ellipse }))
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
            match edge.curve {
                Trim::Line => boundary.push([start, end]),
                Trim::Ellipse(trim) => {
                    let arc = self.ellipse(trim);
                    let clockwise = arc.u[0] * arc.v[1] - arc.u[1] * arc.v[0] < 0.;
                    let radius = arc.major;
                    let target = 4. * (deflection / (4. * (radius + deflection))).sqrt().asin();
                    let sweep = trim.sweep_rad().abs();
                    let count = (sweep / target.min(FRAC_PI_2)).ceil() as usize;
                    if count > (LIMIT - boundary.len()) / (1 + usize::from(clockwise)) {
                        return Err(limit());
                    }
                    let step = sweep / count as f64;
                    let at = |travel: f64| arc.point(arc.trim[0] + travel);
                    let mut previous = start;
                    for i in 1..=count {
                        let next = if i == count { end } else { at(i as f64 * step) };
                        if clockwise {
                            // Use tangent segments on concave trims to keep
                            // the published fill on the material side.
                            let midpoint = at((i as f64 - 0.5) * step);
                            let tip = std::array::from_fn(|i| {
                                arc.center[i] + (midpoint[i] - arc.center[i]) / (step / 2.).cos()
                            });
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

fn section_error(error: Error) -> GeometryError {
    match error {
        Error::Invalid => err(
            GeometryErrorKind::InvalidResult,
            "Invalid trimmed shell section geometry",
        ),
        Error::Limit => limit(),
        Error::Unsupported => err(
            GeometryErrorKind::Unsupported,
            "Rust could not resolve the section against the trimmed shell boundary",
        ),
    }
}

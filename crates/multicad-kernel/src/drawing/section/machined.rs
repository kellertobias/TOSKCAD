//! Exact plane intersections of independent machined stock and insert bodies.
use super::*;
use multicad_rust_kernel::{
    curved_projection::{Curve, Edge as ProjectedEdge, Ellipse},
    orthogonal_section::{self, Edge, Options, SectionError},
};
use std::f64::consts::FRAC_PI_2;

type Point = [f64; 2];
type Segment = [Point; 2];

pub(super) fn needs_analytic_section(solid: &Solid) -> bool {
    solid
        .orthogonal_body()
        .is_some_and(|(body, _)| body.has_curved_cuts())
        || solid.rounded_slot_body().is_some()
        || curved::is_round(solid)
        || solid
            .rust_compound()
            .is_some_and(|body| body.members().any(needs_analytic_section))
}

pub(super) fn project(
    solids: &[DrawingSolid<'_>],
    options: DrawingProjection,
    depth: f64,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let mut output = SectionOutput {
        options,
        vectors: Vec::new(),
        hatch_budget: section_hatch::Budget::default(),
    };
    for (source, input) in solids.iter().enumerate() {
        output.body(input.solid, input.hatch, source, depth)?;
    }
    Ok(output.vectors)
}

struct SectionOutput {
    options: DrawingProjection,
    vectors: Vec<ProjectedVector>,
    hatch_budget: section_hatch::Budget,
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn difference(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}

fn limit() -> GeometryError {
    err(
        GeometryErrorKind::Limit,
        "Rust section exceeded its curve or point budget",
    )
}

fn unsupported(message: &str) -> GeometryError {
    err(GeometryErrorKind::Unsupported, message)
}

fn cut_error(cause: SectionError) -> GeometryError {
    match cause {
        SectionError::Limit => limit(),
        SectionError::Invalid => err(
            GeometryErrorKind::InvalidResult,
            "Invalid exact machined section boundary",
        ),
        SectionError::Unsupported => {
            unsupported("Rust section cannot yet trim this exact curved boundary")
        }
    }
}

impl SectionOutput {
    fn extend(&mut self, vectors: Vec<ProjectedVector>) -> Result<(), GeometryError> {
        self.options.maximum_curves = self
            .options
            .maximum_curves
            .checked_sub(vectors.len() as u32)
            .ok_or_else(limit)?;
        let points = vectors
            .iter()
            .map(|v| v.preview_mm.len() as u32)
            .sum::<u32>();
        self.options.maximum_points = self
            .options
            .maximum_points
            .checked_sub(points)
            .ok_or_else(limit)?;
        self.vectors.extend(vectors);
        Ok(())
    }

    fn body(
        &mut self,
        solid: &Solid,
        hatch: Option<DrawingHatch>,
        source: usize,
        depth: f64,
    ) -> Result<(), GeometryError> {
        if let Some(compound) = solid.rust_compound() {
            for member in compound.members() {
                self.body(member, hatch, source, depth)?;
            }
            return Ok(());
        }
        let policy = solid.geometry_policy();
        let tolerance = policy.linear_tolerance_mm;
        let screen = planar::view_axes(self.options.frame);
        let bounds = solid.bounds()?;
        let ends = [false, true].map(|upper| {
            let corner = std::array::from_fn(|i| {
                if upper == (screen[2][i] >= 0.) {
                    bounds.max[i]
                } else {
                    bounds.min[i]
                }
            });
            dot(difference(corner, self.options.frame.origin), screen[2])
        });
        if depth < ends[0] - tolerance || depth > ends[1] + tolerance {
            return Ok(());
        }
        if let Some((body, placement)) = solid.orthogonal_body()
            && body.has_curved_cuts()
        {
            let frame = SliceFrame::new(placement, self.options, depth)?;
            let loops = body
                .section(frame.options(self.options, tolerance)?)
                .map_err(cut_error)?;
            return self.publish(source, hatch, policy, &loops, frame, true);
        }
        if let Some((body, placement)) = solid.rounded_slot_body() {
            let frame = SliceFrame::new(placement, self.options, depth)?;
            let (profile, axial) = body.section_profile();
            let windows = frame.windows(self.options)?;
            let loops =
                stadium_section(profile, axial, frame.axis, frame.depth, windows, tolerance)?;
            return self.publish(source, hatch, policy, &loops, frame, false);
        }

        let input = DrawingSolid { solid, hatch };
        let mut vectors = if curved::is_round(solid) {
            project_circular(&input, self.options, depth)?
        } else {
            project_planar(&[input], self.options, depth)?
        };
        for vector in &mut vectors {
            vector.source_index = source;
        }
        self.extend(vectors)
    }

    fn publish(
        &mut self,
        source: usize,
        hatch: Option<DrawingHatch>,
        policy: GeometryPolicy,
        loops: &[Vec<Edge>],
        frame: SliceFrame,
        enlarge_voids: bool,
    ) -> Result<(), GeometryError> {
        let edges = loops
            .iter()
            .flatten()
            .map(|&edge| ProjectedEdge {
                hidden: false,
                curve: frame.curve(edge),
            })
            .collect();
        let mut vectors = curved::publish(edges, self.options, source)?;
        for vector in &mut vectors {
            vector.role = VectorRole::Section;
        }
        self.extend(vectors)?;

        if let Some(hatch) = hatch.filter(|h| h.pattern != HatchPattern::None) {
            let mut deflection = self.options.chord_tolerance_mm;
            if enlarge_voids {
                deflection = deflection.min(policy.minimum_feature_mm / 4.);
            }
            let boundary = hatch_segments(loops, frame, enlarge_voids, deflection)?;
            let mut curves = self.options.maximum_curves as usize;
            let mut points = self.options.maximum_points as usize;
            let strokes = publish_hatch(
                source,
                hatch,
                &boundary,
                policy.linear_tolerance_mm,
                &mut curves,
                &mut points,
                &mut self.hatch_budget,
            )?;
            self.extend(strokes)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct SliceFrame {
    axis: usize,
    depth: f64,
    origin: Point,
    directions: [Point; 2],
}

impl SliceFrame {
    fn new(
        placement: PlaneFrame,
        options: DrawingProjection,
        depth: f64,
    ) -> Result<Self, GeometryError> {
        let screen = planar::view_axes(options.frame);
        let local = planar::view_axes(placement);
        let components = local.map(|axis| dot(axis, screen[2]));
        let Some(axis) = components.iter().position(|value| value.abs() > 0.5) else {
            return Err(unsupported(
                "Rust machined sections require a stock-aligned cutting plane",
            ));
        };
        if components
            .iter()
            .enumerate()
            .any(|(i, value)| i != axis && value.abs() > 32. * f64::EPSILON)
        {
            return Err(unsupported(
                "Rust machined sections require a stock-aligned cutting plane",
            ));
        }
        let offset = difference(placement.origin, options.frame.origin);
        let depth = (depth - dot(offset, screen[2])) / components[axis];
        let slice = std::array::from_fn(|i| local[axis][i].mul_add(depth, offset[i]));
        let project = |vector| [dot(vector, screen[0]), dot(vector, screen[1])];
        Ok(Self {
            axis,
            depth,
            origin: project(slice),
            directions: [local[(axis + 1) % 3], local[(axis + 2) % 3]].map(project),
        })
    }

    fn point(self, [a, b]: Point) -> Point {
        let vector = self.vector([a, b]);
        std::array::from_fn(|i| self.origin[i] + vector[i])
    }

    fn vector(self, [a, b]: Point) -> Point {
        std::array::from_fn(|i| a.mul_add(self.directions[0][i], b * self.directions[1][i]))
    }

    fn curve(self, edge: Edge) -> Curve {
        match edge {
            Edge::Line(ends) => Curve::Line(ends.map(|point| self.point(point))),
            Edge::Arc {
                center,
                radius,
                start_rad,
                sweep_rad,
            } => {
                let (sin, cos) = start_rad.sin_cos();
                let sign = sweep_rad.signum();
                Curve::Ellipse(Ellipse {
                    center: self.point(center),
                    u: self.vector([cos, sin]),
                    v: self.vector([-sign * sin, sign * cos]),
                    major: radius,
                    minor: radius,
                    trim: [0., sweep_rad.abs()],
                })
            }
        }
    }

    fn options(self, view: DrawingProjection, tolerance: f64) -> Result<Options, GeometryError> {
        Ok(Options {
            axis: self.axis,
            depth_mm: self.depth,
            tolerance_mm: tolerance,
            segment_limit: view.maximum_curves as usize,
            windows_mm: self.windows(view)?,
        })
    }

    fn windows(self, options: DrawingProjection) -> Result<[Option<Point>; 2], GeometryError> {
        let mut local = [None; 2];
        for (screen, window) in [options.section_u_mm, options.section_v_mm]
            .into_iter()
            .enumerate()
        {
            let Some([a, b]) = window else { continue };
            let components = self.directions.map(|direction| direction[screen]);
            let axis = usize::from(components[1].abs() > components[0].abs());
            if components[1 - axis].abs() > 32. * f64::EPSILON {
                return Err(unsupported(
                    "Rust machined section windows must align with the cut's stock axes",
                ));
            }
            let [a, b] = [a, b].map(|x| (x - self.origin[screen]) / components[axis]);
            local[axis] = Some([a.min(b), a.max(b)]);
        }
        Ok(local)
    }
}

fn stadium_section(
    profile: multicad_rust_kernel::stadium::Stadium,
    axial: Point,
    axis: usize,
    depth: f64,
    windows: [Option<Point>; 2],
    tolerance: f64,
) -> Result<Vec<Vec<Edge>>, GeometryError> {
    if axis == 2 {
        if depth < axial[0] - tolerance || depth > axial[1] + tolerance {
            return Ok(Vec::new());
        }
        for (axis, window) in windows.into_iter().enumerate() {
            let Some([lo, hi]) = window else { continue };
            let direction = |sign| std::array::from_fn(|i| if i == axis { sign } else { 0. });
            let min = -profile.support(direction(-1.));
            let max = profile.support(direction(1.));
            if max <= lo || min >= hi {
                return Ok(Vec::new());
            }
            if min < lo || max > hi {
                return Err(unsupported(
                    "Rust section cannot yet clip a rounded insert perimeter",
                ));
            }
        }
        return Ok(vec![orthogonal_section::stadium_edges(profile).to_vec()]);
    }
    let Some(side) =
        orthogonal_section::stadium_interval(profile, axis, depth).map_err(cut_error)?
    else {
        return Ok(Vec::new());
    };
    let mut intervals = if axis == 0 {
        [side, axial]
    } else {
        [axial, side]
    };
    for (interval, window) in intervals.iter_mut().zip(windows) {
        if let Some([lo, hi]) = window {
            interval[0] = interval[0].max(lo);
            interval[1] = interval[1].min(hi);
        }
    }
    let [[a, b], [c, d]] = intervals;
    if b - a <= tolerance || d - c <= tolerance {
        return Ok(Vec::new());
    }
    let corners = [[a, c], [b, c], [b, d], [a, d]];
    Ok(vec![
        (0..4)
            .map(|i| Edge::Line([corners[i], corners[(i + 1) % 4]]))
            .collect(),
    ])
}

fn hatch_segments(
    loops: &[Vec<Edge>],
    frame: SliceFrame,
    enlarge_voids: bool,
    deflection: f64,
) -> Result<Vec<Segment>, GeometryError> {
    const LIMIT: usize = 100_000;
    let mut segments = Vec::new();
    for edge in loops.iter().flatten() {
        match *edge {
            Edge::Line(ends) => segments.push(ends.map(|point| frame.point(point))),
            Edge::Arc {
                center,
                radius,
                start_rad,
                sweep_rad,
            } => {
                // Keep chord approximations within material. A void uses the
                // outer tangent polygon, while an insert uses inner chords.
                let target = 4. * (deflection / (4. * (radius + deflection))).sqrt().asin();
                let count = (sweep_rad.abs() / target.min(FRAC_PI_2)).ceil() as usize;
                let points = if enlarge_voids { 2 } else { 1 };
                if count > (LIMIT - segments.len()) / points {
                    return Err(limit());
                }
                let step = sweep_rad / count as f64;
                let at = |angle: f64, radius: f64| {
                    let (sin, cos) = angle.sin_cos();
                    frame.point([
                        radius.mul_add(cos, center[0]),
                        radius.mul_add(sin, center[1]),
                    ])
                };
                let mut last = at(start_rad, radius);
                for i in 0..count {
                    let start = step.mul_add(i as f64, start_rad);
                    let next = at(step.mul_add((i + 1) as f64, start_rad), radius);
                    if enlarge_voids {
                        let tip = at(start + step / 2., radius / (step / 2.).cos());
                        segments.push([last, tip]);
                        last = tip;
                    }
                    segments.push([last, next]);
                    last = next;
                }
            }
        }
        if segments.len() >= LIMIT {
            return Err(limit());
        }
    }
    Ok(segments)
}

//! Exact transverse visibility for a single normal sweep. Section boundaries
//! become monotone depth branches, and certified piercings either face the
//! view or lie wholly behind intact material.
use super::*;
use std::f64::consts::PI;

const ANGULAR: f64 = 32. * f64::EPSILON;
const WORK_LIMIT: usize = 2_000_000;
type Span = [f64; 2];

fn unsupported(message: &str) -> GeometryError {
    err(GeometryErrorKind::Unsupported, message)
}

pub(in crate::drawing) fn transverse_rails(
    solid: &Solid,
    frame: PlaneFrame,
    options: DrawingProjection,
) -> Result<Vec<[Point3; 2]>, GeometryError> {
    let (body, _) = solid.normal_sweep_body().expect("classified normal sweep");
    let (rings, height) = body.drawing_section();
    let [u, v, toward_viewer] = planar::view_axes(options.frame);
    let chart = Chart::new(
        frame,
        &Screen {
            origin: options.frame.origin,
            axes: [u, v],
            toward_viewer,
        },
    );
    let tolerance = solid.geometry_policy().linear_tolerance_mm;
    let profile = Profile::new(rings, chart, tolerance, options.tangent_edges)?;
    Ok(profile
        .rails
        .into_iter()
        .map(|[s, depth]| {
            [0., height].map(|at| {
                let point = chart.point(s, at);
                std::array::from_fn(|i| point[i] + chart.depth[i] * depth)
            })
        })
        .collect())
}

pub(super) fn project(
    input: &DrawingSolid<'_>,
    options: DrawingProjection,
    frame: PlaneFrame,
    screen: &Screen,
) -> Result<Vec<ProjectedVector>, GeometryError> {
    let (body, _) = input
        .solid
        .normal_sweep_body()
        .expect("classified normal sweep");
    let (rings, height) = body.drawing_section();
    let tolerance = input.solid.geometry_policy().linear_tolerance_mm;
    let chart = Chart::new(frame, screen);
    let profile = Profile::new(rings, chart, tolerance, options.tangent_edges)?;
    let openings = body
        .drawing_piercings(frame)
        .map(|(faces, outline)| Opening::new(faces, outline, chart))
        .collect::<Result<Vec<_>, _>>()?;
    let mut work = profile.remaining;
    let mut opening_roles = Vec::with_capacity(openings.len());
    for (i, opening) in openings.iter().enumerate() {
        charge(&mut work, i + profile.bands.len() + profile.rails.len())?;
        for other in &openings[..i] {
            opening.check_overlap(other, tolerance)?;
        }
        opening_roles.push(opening.roles(&profile, tolerance)?);
    }

    let mut edges = Vec::new();
    let mut line = |hidden: bool, [a, b]: [Point; 2]| {
        if (!hidden || options.hidden_lines) && (a[0] - b[0]).hypot(a[1] - b[1]) > tolerance {
            edges.push(Edge {
                hidden,
                curve: Curve::Line([a, b]),
            });
        }
    };
    for [low, high] in &profile.visible {
        for at in [0., height] {
            line(
                false,
                [*low, *high].map(|s| screen.point(chart.point(s, at))),
            );
            // Both sides of a closed section project onto the same cap span.
            line(
                true,
                [*low, *high].map(|s| screen.point(chart.point(s, at))),
            );
        }
    }
    for &[s, depth] in &profile.rails {
        let front = profile.front_at(s, tolerance);
        line(
            depth + tolerance < front,
            [0., height].map(|a| screen.point(chart.point(s, a))),
        );
    }
    for (opening, roles) in openings.into_iter().zip(opening_roles) {
        if let Some(roles) = roles {
            for (face, hidden) in opening.faces.into_iter().zip(roles) {
                if !hidden || options.hidden_lines {
                    for &rim in opening.outline {
                        edges.push(Edge {
                            hidden,
                            curve: rim_curve(rim, face, screen)?,
                        });
                    }
                }
            }
        } else if options.hidden_lines {
            for [a, b] in piercing_edges(opening.faces, opening.outline, screen.toward_viewer) {
                let [a, b] = [screen.point(a), screen.point(b)];
                if (a[0] - b[0]).hypot(a[1] - b[1]) > tolerance {
                    edges.push(Edge {
                        hidden: true,
                        curve: Curve::Line([a, b]),
                    });
                }
            }
        }
    }
    curved::publish(edges, options, 0)
}

fn rim_curve(
    edge: ProfileEdge,
    frame: ReferenceFrame,
    screen: &Screen,
) -> Result<Curve, GeometryError> {
    match edge {
        ProfileEdge::Line { a, b } => Ok(Curve::Line(
            [a, b].map(|p| screen.point(face_point(frame, p))),
        )),
        ProfileEdge::Circle {
            center, radius_mm, ..
        } => Ok(Curve::Ellipse(Ellipse {
            center: screen.point(face_point(frame, center)),
            u: [1., 0.],
            v: [0., 1.],
            major: radius_mm,
            minor: radius_mm,
            trim: [0., TAU],
        })),
        _ => Err(unsupported(
            "Rust transverse sweep requires circular or rectangular piercings",
        )),
    }
}

#[derive(Clone, Copy)]
struct Chart {
    origin: Point3,
    axis: Point3,
    side: Point3,
    depth: Point3,
    section_side: Point,
    section_depth: Point,
}

impl Chart {
    fn new(frame: PlaneFrame, screen: &Screen) -> Self {
        let axis = normalized(frame.normal());
        let depth = screen.toward_viewer;
        let side = normalized(cross3(depth, axis));
        Self {
            origin: frame.origin,
            axis,
            side,
            depth,
            section_side: [dot(frame.u, side), dot(frame.v, side)],
            section_depth: [dot(frame.u, depth), dot(frame.v, depth)],
        }
    }

    fn section(self, p: Point) -> Point {
        [dot2(self.section_side, p), dot2(self.section_depth, p)]
    }

    fn coordinates(self, p: Point3) -> Point3 {
        let p = std::array::from_fn(|i| p[i] - self.origin[i]);
        [dot(self.side, p), dot(self.axis, p), dot(self.depth, p)]
    }

    fn point(self, side: f64, axial: f64) -> Point3 {
        std::array::from_fn(|i| self.origin[i] + self.axis[i] * axial + self.side[i] * side)
    }
}

#[derive(Clone, Copy)]
enum Depth {
    Line([Point; 2]),
    Circle {
        center: Point,
        radius: f64,
        sign: f64,
    },
}

#[derive(Clone, Copy)]
struct Branch {
    span: Span,
    depth: Depth,
}

impl Branch {
    fn at(self, s: f64) -> f64 {
        match self.depth {
            Depth::Line([a, b]) => ((s - a[0]) / (b[0] - a[0])).mul_add(b[1] - a[1], a[1]),
            Depth::Circle {
                center,
                radius,
                sign,
            } => {
                let x = (s - center[0]).abs().min(radius);
                sign.mul_add(((radius - x) * (radius + x)).sqrt(), center[1])
            }
        }
    }

    fn depth_range(self, [lo, hi]: Span) -> Span {
        let [a, b] = [self.at(lo), self.at(hi)];
        let mut range = [a.min(b), a.max(b)];
        if let Depth::Circle { center, .. } = self.depth
            && (lo..=hi).contains(&center[0])
        {
            let at = self.at(center[0]);
            range = [range[0].min(at), range[1].max(at)];
        }
        range
    }
}

struct Profile {
    bands: Vec<Branch>,
    rails: Vec<Point>,
    walls: Vec<Point>,
    visible: Vec<Span>,
    remaining: usize,
}

impl Profile {
    fn new(
        rings: &[Vec<ProfileEdge>],
        chart: Chart,
        tolerance: f64,
        tangent_edges: bool,
    ) -> Result<Self, GeometryError> {
        let mut branches = Vec::new();
        let mut rails = Vec::new();
        let mut walls = Vec::new();
        for ring in rings {
            for (&edge, &previous) in ring.iter().zip(ring.iter().cycle().skip(ring.len() - 1)) {
                if !matches!(edge, ProfileEdge::Circle { .. })
                    && (tangent_edges || is_rail(previous, edge, chart))
                {
                    rails.push(chart.section(edge.start()));
                }
                if let ProfileEdge::Line { a, b } = edge {
                    let [a, b] = [chart.section(a), chart.section(b)];
                    let span = [a[0].min(b[0]), a[0].max(b[0])];
                    if span[1] - span[0] <= tolerance {
                        walls.extend([a, b]);
                    } else {
                        branches.push(Branch {
                            span,
                            depth: Depth::Line([a, b]),
                        });
                    }
                    continue;
                }
                arc_branches(edge, chart, &mut branches, &mut rails);
            }
        }
        let mut levels: Vec<_> = branches.iter().flat_map(|branch| branch.span).collect();
        levels.sort_unstable_by(f64::total_cmp);
        levels.dedup_by(|a, b| (*a - *b).abs() <= tolerance);
        let mut remaining = WORK_LIMIT;
        charge(&mut remaining, levels.len().saturating_mul(branches.len()))?;
        charge(
            &mut remaining,
            rails.len().saturating_mul(levels.len() + walls.len()),
        )?;
        let mut bands = Vec::new();
        let mut visible: Vec<Span> = Vec::new();
        for pair in levels.windows(2) {
            let span = [pair[0], pair[1]];
            let s = span[0].midpoint(span[1]);
            let mut crossings = 0;
            let front = branches
                .iter()
                .filter(|branch| (branch.span[0]..branch.span[1]).contains(&s))
                .inspect(|_| crossings += 1)
                .max_by(|a, b| a.at(s).total_cmp(&b.at(s)));
            if crossings % 2 != 0 {
                return Err(unsupported(
                    "Rust transverse sweep cannot certify a closed section band",
                ));
            }
            let Some(front) = front else { continue };
            // A checked section has noncrossing exact boundaries. Within
            // each monotone band their depth order cannot change.
            bands.push(Branch { span, ..*front });
            if let Some(last) = visible.last_mut()
                && last[1] == span[0]
            {
                last[1] = span[1];
            } else {
                visible.push(span);
            }
        }
        rails.sort_unstable_by(|a, b| a[0].total_cmp(&b[0]).then_with(|| a[1].total_cmp(&b[1])));
        rails.dedup_by(|a, b| (a[0] - b[0]).abs() <= tolerance && (a[1] - b[1]).abs() <= tolerance);
        Ok(Self {
            bands,
            rails,
            walls,
            visible,
            remaining,
        })
    }

    fn front_at(&self, s: f64, tolerance: f64) -> f64 {
        self.bands
            .iter()
            .filter(|band| s >= band.span[0] - tolerance && s <= band.span[1] + tolerance)
            .map(|band| band.at(s.clamp(band.span[0], band.span[1])))
            .chain(
                self.walls
                    .iter()
                    .filter(|wall| (wall[0] - s).abs() <= tolerance)
                    .map(|wall| wall[1]),
            )
            .fold(f64::NEG_INFINITY, f64::max)
    }

    fn front_range(&self, span: Span, tolerance: f64) -> Result<Span, GeometryError> {
        if !self
            .visible
            .iter()
            .any(|&extent| extent[0] <= span[0] + tolerance && extent[1] >= span[1] - tolerance)
        {
            return Err(unsupported(
                "Rust transverse sweep cannot certify the stock across a piercing",
            ));
        }
        let mut range = [f64::INFINITY, f64::NEG_INFINITY];
        for band in &self.bands {
            let intersection = [span[0].max(band.span[0]), span[1].min(band.span[1])];
            if intersection[0] <= intersection[1] {
                let [lo, hi] = band.depth_range(intersection);
                range = [range[0].min(lo), range[1].max(hi)];
            }
        }
        Ok(range)
    }
}

fn is_rail(previous: ProfileEdge, edge: ProfileEdge, chart: Chart) -> bool {
    let tangent = |edge: ProfileEdge, at: Point| {
        let direction = if let ProfileEdge::Arc {
            center, clockwise, ..
        } = edge
        {
            let sign = if clockwise { -1. } else { 1. };
            [sign * (center[1] - at[1]), sign * (at[0] - center[0])]
        } else {
            let [a, b] = [edge.start(), edge.end()];
            [b[0] - a[0], b[1] - a[1]]
        };
        let length = direction[0].hypot(direction[1]);
        direction.map(|v| v / length)
    };
    let [a, b] = [
        tangent(previous, previous.end()),
        tangent(edge, edge.start()),
    ];
    (a[0] * b[1] - a[1] * b[0]).abs() > ANGULAR
        || dot2(a, b) < 0.
        || ((!matches!(previous, ProfileEdge::Line { .. })
            || !matches!(edge, ProfileEdge::Line { .. }))
            && dot2(a, chart.section_side).abs() <= ANGULAR)
}

fn arc_branches(
    edge: ProfileEdge,
    chart: Chart,
    branches: &mut Vec<Branch>,
    rails: &mut Vec<Point>,
) {
    let center = match edge {
        ProfileEdge::Arc { center, .. } | ProfileEdge::Circle { center, .. } => center,
        _ => unreachable!("classified profile arc"),
    };
    let radius = edge.radius_mm().expect("circular edge");
    let sweep = edge.signed_sweep_rad().expect("circular edge");
    let start = edge.start();
    let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
    let side_angle = chart.section_side[1].atan2(chart.section_side[0]);
    let center = chart.section(center);
    let mut events = vec![(0., chart.section(start)), (1., chart.section(edge.end()))];
    for (angle, side) in [(side_angle, 1.), (side_angle + PI, -1.)] {
        let part = ((angle - start_angle) * sweep.signum()).rem_euclid(TAU) / sweep.abs();
        if part > ANGULAR && part < 1. - ANGULAR {
            let point = [center[0] + side * radius, center[1]];
            events.push((part, point));
            rails.push(point);
        } else if matches!(edge, ProfileEdge::Circle { .. })
            && (part <= ANGULAR || part >= 1. - ANGULAR)
        {
            rails.push([center[0] + side * radius, center[1]]);
        }
    }
    events.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    for pair in events.windows(2) {
        let [(a, p), (b, q)] = [pair[0], pair[1]];
        let span = [p[0].min(q[0]), p[0].max(q[0])];
        let sign = (chart.section(edge.point(a.midpoint(b)))[1] - center[1]).signum();
        branches.push(Branch {
            span,
            depth: Depth::Circle {
                center,
                radius,
                sign,
            },
        });
    }
}

struct Opening<'a> {
    faces: [ReferenceFrame; 2],
    outline: &'a [ProfileEdge],
    bounds: [Span; 3],
    depths: Option<[f64; 2]>,
    signature: Vec<f64>,
}

impl<'a> Opening<'a> {
    fn new(
        faces: [ReferenceFrame; 2],
        outline: &'a [ProfileEdge],
        chart: Chart,
    ) -> Result<Self, GeometryError> {
        let normal = normalized(faces[0].normal());
        let angular_distance = cross3(normal, chart.depth);
        let depths = if dot(normal, chart.depth).abs() <= ANGULAR {
            None
        } else if dot(angular_distance, angular_distance).sqrt() <= ANGULAR {
            Some(faces.map(|face| chart.coordinates(face.origin_mm)[2]))
        } else {
            return Err(unsupported(
                "Rust transverse sweep requires face-on or edge-on piercings",
            ));
        };
        let mut bounds = [[f64::INFINITY, f64::NEG_INFINITY]; 3];
        let mut signature = Vec::new();
        match outline {
            [
                ProfileEdge::Circle {
                    center, radius_mm, ..
                },
            ] => {
                let radial = [chart.side, chart.axis, chart.depth]
                    .map(|axis| radius_mm * dot(axis, faces[0].u).hypot(dot(axis, faces[0].v)));
                for face in faces {
                    let [s, a, d] = chart.coordinates(face_point(face, *center));
                    extend_bounds(&mut bounds, [s, a, d], radial);
                }
                let [s, a, _] = chart.coordinates(face_point(faces[0], *center));
                signature.extend([s, a, *radius_mm]);
            }
            _ => {
                let mut vertices = Vec::new();
                for edge in outline {
                    for (i, face) in faces.into_iter().enumerate() {
                        let [s, a, d] = chart.coordinates(face_point(face, edge.start()));
                        extend_bounds(&mut bounds, [s, a, d], [0.; 3]);
                        if i == 0 {
                            vertices.push([s, a]);
                        }
                    }
                }
                vertices.sort_unstable_by(|a, b| {
                    a[0].total_cmp(&b[0]).then_with(|| a[1].total_cmp(&b[1]))
                });
                signature.extend(vertices.into_iter().flatten());
            }
        }
        Ok(Self {
            faces,
            outline,
            bounds,
            depths,
            signature,
        })
    }

    fn check_overlap(&self, other: &Self, tolerance: f64) -> Result<(), GeometryError> {
        if self.depths.is_none() && other.depths.is_none() {
            return Ok(());
        }
        let overlaps = (0..2).all(|i| {
            self.bounds[i][0] <= other.bounds[i][1] + tolerance
                && other.bounds[i][0] <= self.bounds[i][1] + tolerance
        });
        let same =
            self.depths.is_some()
                && other.depths.is_some()
                && self.signature.len() == other.signature.len()
                && self.signature.iter().zip(&other.signature).all(|(&a, &b)| {
                    (a - b).abs() <= 32. * f64::EPSILON * a.abs().max(b.abs()).max(1.)
                });
        if overlaps && !same {
            return Err(unsupported(
                "Rust transverse sweep cannot certify overlapping projected piercings",
            ));
        }
        Ok(())
    }

    fn roles(&self, profile: &Profile, tolerance: f64) -> Result<Option<[bool; 2]>, GeometryError> {
        let [low, high] = profile.front_range(self.bounds[0], tolerance)?;
        if let Some(depths) = self.depths {
            if profile.rails.iter().any(|rail| {
                (self.bounds[0][0] - tolerance..=self.bounds[0][1] + tolerance).contains(&rail[0])
            }) {
                return Err(unsupported(
                    "Rust transverse sweep cannot certify a piercing crossing a stock edge",
                ));
            }
            let near = depths[0].max(depths[1]);
            if low > near + tolerance {
                return Ok(Some([true; 2]));
            }
            if low >= near - tolerance && high <= near + tolerance {
                return Ok(Some(depths.map(|depth| depth + tolerance < near)));
            }
        } else if low > self.bounds[2][1] + tolerance {
            return Ok(None);
        }
        Err(unsupported(
            "Rust transverse sweep cannot prove complete stock cover of a piercing",
        ))
    }
}

fn extend_bounds(bounds: &mut [Span; 3], center: Point3, radius: Point3) {
    for i in 0..3 {
        bounds[i][0] = bounds[i][0].min(center[i] - radius[i]);
        bounds[i][1] = bounds[i][1].max(center[i] + radius[i]);
    }
}

fn face_point(frame: ReferenceFrame, [u, v]: Point) -> Point3 {
    std::array::from_fn(|i| frame.origin_mm[i] + u * frame.u[i] + v * frame.v[i])
}

fn normalized(p: Point3) -> Point3 {
    let length = dot(p, p).sqrt();
    p.map(|v| v / length)
}

fn dot2(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn cross3(a: Point3, b: Point3) -> Point3 {
    std::array::from_fn(|i| a[(i + 1) % 3] * b[(i + 2) % 3] - a[(i + 2) % 3] * b[(i + 1) % 3])
}

fn charge(budget: &mut usize, work: usize) -> Result<(), GeometryError> {
    *budget = budget.checked_sub(work).ok_or_else(|| {
        err(
            GeometryErrorKind::Limit,
            "Rust transverse sweep projection exceeded its work budget",
        )
    })?;
    Ok(())
}

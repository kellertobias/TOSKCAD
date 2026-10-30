//! Transient sketch placement aids: geometry snapping, dwell-acquired
//! references, horizontal/vertical assistive lines, and perpendicular and
//! tangent targets for a line being drawn from a known start point.
//!
//! Everything here is pure and display-only. It never reads or writes the
//! canonical document; the sketch editor commits the resolved position through
//! the shared sketch transaction API. Time is injected by the caller (seconds
//! from any monotonic origin) so acquisition is testable without a wall clock.
//! See `docs/architecture/sketch-snapping.md` for the interaction policy.
use multicad_core::{
    constraint_sketch::{CurveKind, Sketch},
    document::EntityId,
};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::TAU;

/// A candidate must stay hovered this long before it becomes a reference.
pub const DWELL_SECONDS: f64 = 0.5;
/// The pointer may drift this far (screen pixels) from where the dwell
/// started without restarting the dwell timer.
pub const DWELL_STABILITY_PX: f32 = 4.0;
/// Acquisition distance for point-like targets (vertices, midpoints, centers,
/// references and guide intersections).
pub const POINT_SNAP_PX: f32 = 8.0;
/// Acquisition distance for points on existing lines, circles and arcs.
pub const CURVE_SNAP_PX: f32 = 6.0;
/// Acquisition distance for a horizontal/vertical assistive line.
pub const GUIDE_SNAP_PX: f32 = 6.0;
/// A target that was shown in the previous frame stays eligible until the
/// pointer leaves this multiple of its acquisition distance (hysteresis).
pub const RELEASE_FACTOR: f32 = 1.5;
/// At most this many references are held; acquiring another drops the oldest.
pub const MAX_REFERENCES: usize = 2;
/// Relative tolerance for recognizing rectangles and regular polygons.
const SHAPE_TOLERANCE: f64 = 1e-6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// Stable identity of a snap target across frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetKey {
    Point(EntityId),
    LineMidpoint(EntityId),
    /// Sorted ids of the lines that form the recognized shape.
    ShapeCenter(Vec<EntityId>),
    Reference(u64),
    ReferenceMidpoint(u64, u64),
    GuideIntersection {
        horizontal: u64,
        vertical: u64,
    },
    GuideOnLine {
        reference: u64,
        axis: Axis,
        line: EntityId,
    },
    OnCurve(EntityId),
    Guide {
        reference: u64,
        axis: Axis,
    },
    /// Foot of the perpendicular from the line start onto a curve. Circles
    /// and arcs have a near (0) and a far (1) foot.
    Perpendicular(EntityId, u8),
    /// Tangent point on a circle or arc as seen from the line start; the two
    /// tangents are numbered counter-clockwise (0) and clockwise (1).
    Tangent(EntityId, u8),
}
impl TargetKey {
    /// Competing-target order: lower classes win regardless of distance;
    /// within a class the nearest target (in screen pixels) wins.
    pub fn class(&self) -> u8 {
        match self {
            Self::Point(_) => 0,
            Self::LineMidpoint(_)
            | Self::ShapeCenter(_)
            | Self::Reference(_)
            | Self::ReferenceMidpoint(..) => 1,
            Self::GuideIntersection { .. }
            | Self::GuideOnLine { .. }
            | Self::Perpendicular(..)
            | Self::Tangent(..) => 2,
            Self::OnCurve(_) => 3,
            Self::Guide { .. } => 4,
        }
    }
    /// Targets that dwelling can acquire (or release, for references).
    pub fn acquirable(&self) -> bool {
        matches!(
            self,
            Self::Point(_)
                | Self::LineMidpoint(_)
                | Self::ShapeCenter(_)
                | Self::OnCurve(_)
                | Self::Reference(_)
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    Rectangle,
    RegularPolygon(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    Endpoint,
    /// A circle or arc center point.
    Center,
    LineMidpoint,
    ShapeCenter(ShapeKind),
    Reference,
    ReferenceMidpoint,
    GuideIntersection,
    GuideOnLine,
    OnCurve,
    Guide(Axis),
    Perpendicular,
    Tangent,
}
impl TargetKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Endpoint => "Endpoint",
            Self::Center => "Center",
            Self::LineMidpoint => "Midpoint",
            Self::ShapeCenter(ShapeKind::Rectangle) => "Rectangle center",
            Self::ShapeCenter(ShapeKind::RegularPolygon(_)) => "Polygon center",
            Self::Reference => "Reference",
            Self::ReferenceMidpoint => "Midpoint of references",
            Self::GuideIntersection => "Alignment intersection",
            Self::GuideOnLine => "Alignment on line",
            Self::OnCurve => "On curve",
            Self::Guide(Axis::Horizontal) => "Horizontal alignment",
            Self::Guide(Axis::Vertical) => "Vertical alignment",
            Self::Perpendicular => "Perpendicular",
            Self::Tangent => "Tangent",
        }
    }
}

/// A dotted assistive line from an acquired reference through the target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideLine {
    pub from: [f64; 2],
    pub axis: Axis,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub key: TargetKey,
    pub kind: TargetKind,
    /// Exact sketch-plane position in millimeters; this is what gets committed.
    pub position: [f64; 2],
    /// Existing sketch point to reuse instead of creating a new point.
    pub anchor: Option<EntityId>,
    pub guides: Vec<GuideLine>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reference {
    pub serial: u64,
    pub source: TargetKey,
    pub kind: TargetKind,
    pub position: [f64; 2],
}

#[derive(Clone, Copy, Debug)]
struct Arc {
    id: EntityId,
    center: [f64; 2],
    radius: f64,
    /// Counter-clockwise start angle and sweep.
    start: f64,
    sweep: f64,
}
impl Arc {
    fn contains(&self, angle: f64) -> bool {
        (angle - self.start).rem_euclid(TAU) <= self.sweep + 1e-12
    }
}

/// Snap-relevant view of a solved sketch, in sketch-plane millimeters.
#[derive(Clone, Debug, Default)]
pub struct SnapGeometry {
    points: Vec<(EntityId, [f64; 2], bool)>,
    lines: Vec<(EntityId, [f64; 2], [f64; 2])>,
    circles: Vec<(EntityId, [f64; 2], f64)>,
    arcs: Vec<Arc>,
    centers: Vec<(Vec<EntityId>, [f64; 2], ShapeKind)>,
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn len(a: [f64; 2]) -> f64 {
    a[0].hypot(a[1])
}
fn mid(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.]
}

impl SnapGeometry {
    /// Builds the snap view from a solved sketch. B-spline curves contribute
    /// their poles as points but no on-curve or center targets.
    pub fn from_sketch(sketch: &Sketch) -> Self {
        let position: BTreeMap<EntityId, [f64; 2]> = sketch
            .points
            .iter()
            .map(|p| (p.id, p.position_mm))
            .collect();
        let centers: BTreeSet<EntityId> = sketch
            .curves
            .iter()
            .filter_map(|c| match c.kind {
                CurveKind::Circle { center, .. } | CurveKind::Arc { center, .. } => Some(center),
                _ => None,
            })
            .collect();
        let mut geometry = Self {
            points: sketch
                .points
                .iter()
                .map(|p| (p.id, p.position_mm, centers.contains(&p.id)))
                .collect(),
            ..Default::default()
        };
        let mut edges = vec![];
        for curve in &sketch.curves {
            match &curve.kind {
                CurveKind::Line { start, end } => {
                    if let (Some(&a), Some(&b)) = (position.get(start), position.get(end))
                        && start != end
                        && len(sub(b, a)) > 1e-12
                    {
                        geometry.lines.push((curve.id, a, b));
                        edges.push((curve.id, *start, *end));
                    }
                }
                CurveKind::Circle { center, radius_mm } => {
                    if let Some(&c) = position.get(center)
                        && *radius_mm > 1e-12
                    {
                        geometry.circles.push((curve.id, c, *radius_mm));
                    }
                }
                CurveKind::Arc {
                    center,
                    start,
                    end,
                    clockwise,
                } => {
                    if let (Some(&c), Some(&a), Some(&b)) =
                        (position.get(center), position.get(start), position.get(end))
                    {
                        let radius = len(sub(a, c));
                        if radius > 1e-12 {
                            let angle = |p: [f64; 2]| (p[1] - c[1]).atan2(p[0] - c[0]);
                            let (from, to) = if *clockwise {
                                (angle(b), angle(a))
                            } else {
                                (angle(a), angle(b))
                            };
                            let mut sweep = (to - from).rem_euclid(TAU);
                            if sweep <= 1e-12 {
                                sweep = TAU;
                            }
                            geometry.arcs.push(Arc {
                                id: curve.id,
                                center: c,
                                radius,
                                start: from,
                                sweep,
                            });
                        }
                    }
                }
                CurveKind::BSpline { .. } => {}
            }
        }
        geometry.centers = shape_centers(&position, &edges);
        geometry
    }
    fn point(&self, id: EntityId) -> Option<[f64; 2]> {
        self.points.iter().find(|p| p.0 == id).map(|p| p.1)
    }
    fn has_curve(&self, id: EntityId) -> bool {
        self.lines.iter().any(|l| l.0 == id)
            || self.circles.iter().any(|c| c.0 == id)
            || self.arcs.iter().any(|a| a.id == id)
    }
    /// Current position of an acquirable source, or `None` if it disappeared.
    fn locate(&self, key: &TargetKey) -> Option<[f64; 2]> {
        match key {
            TargetKey::Point(id) => self.point(*id),
            TargetKey::LineMidpoint(id) => self
                .lines
                .iter()
                .find(|l| l.0 == *id)
                .map(|l| mid(l.1, l.2)),
            TargetKey::ShapeCenter(ids) => self.centers.iter().find(|c| &c.0 == ids).map(|c| c.1),
            _ => None,
        }
    }
    /// Recognized shape centers, for tests and diagnostics.
    #[cfg(test)]
    pub fn shape_centers(&self) -> impl Iterator<Item = ([f64; 2], ShapeKind)> + '_ {
        self.centers.iter().map(|c| (c.1, c.2))
    }
}

/// Centers are only defined for shapes with an unambiguous symmetry center:
/// rectangles (closed four-line loops with four right angles; center is the
/// diagonal intersection) and regular polygons (closed loops of three or more
/// lines whose points have no other line connections, with equal side lengths
/// and all vertices equidistant from their mean). Circles and arcs already
/// carry an explicit center point. Every other loop has no center target.
fn shape_centers(
    position: &BTreeMap<EntityId, [f64; 2]>,
    edges: &[(EntityId, EntityId, EntityId)],
) -> Vec<(Vec<EntityId>, [f64; 2], ShapeKind)> {
    let mut adjacency: BTreeMap<EntityId, Vec<(EntityId, EntityId)>> = BTreeMap::new();
    for &(line, a, b) in edges {
        adjacency.entry(a).or_default().push((b, line));
        adjacency.entry(b).or_default().push((a, line));
    }
    let mut found: BTreeMap<Vec<EntityId>, ([f64; 2], ShapeKind)> = BTreeMap::new();
    let p = |id: &EntityId| position[id];
    // Rectangles: every 4-cycle a-b-c-d with right angles at all corners.
    for (&a, neighbors) in &adjacency {
        for (i, &(b, ab)) in neighbors.iter().enumerate() {
            for &(d, ad) in &neighbors[i + 1..] {
                if b == d || ab == ad {
                    continue;
                }
                for &(c, bc) in &adjacency[&b] {
                    if c == a || c == d || bc == ab {
                        continue;
                    }
                    for &(c2, dc) in &adjacency[&d] {
                        if c2 != c || dc == ad || dc == bc {
                            continue;
                        }
                        let corners = [p(&a), p(&b), p(&c), p(&d)];
                        let right = (0..4).all(|k| {
                            let o = corners[k];
                            let u = sub(corners[(k + 1) % 4], o);
                            let v = sub(corners[(k + 3) % 4], o);
                            let scale = len(u) * len(v);
                            scale > 1e-18 && dot(u, v).abs() <= SHAPE_TOLERANCE * scale
                        });
                        if right {
                            let mut key = vec![ab, bc, dc, ad];
                            key.sort();
                            found
                                .entry(key)
                                .or_insert((mid(corners[0], corners[2]), ShapeKind::Rectangle));
                        }
                    }
                }
            }
        }
    }
    // Regular polygons: isolated simple loops with equal sides and radii.
    let mut visited = BTreeSet::new();
    for &start in adjacency.keys() {
        if visited.contains(&start) {
            continue;
        }
        let mut loop_points = vec![start];
        let mut loop_lines = vec![];
        let mut previous_line = None;
        let mut current = start;
        let closed = loop {
            visited.insert(current);
            let links = &adjacency[&current];
            if links.len() != 2 {
                break false;
            }
            let Some(&(next, line)) = links.iter().find(|l| Some(l.1) != previous_line) else {
                break false;
            };
            loop_lines.push(line);
            previous_line = Some(line);
            if next == start {
                break true;
            }
            if visited.contains(&next) {
                break false;
            }
            loop_points.push(next);
            current = next;
        };
        let n = loop_points.len();
        if !closed || n < 3 {
            continue;
        }
        let vertices: Vec<[f64; 2]> = loop_points.iter().map(p).collect();
        let sides: Vec<f64> = (0..n)
            .map(|k| len(sub(vertices[(k + 1) % n], vertices[k])))
            .collect();
        let center = [
            vertices.iter().map(|v| v[0]).sum::<f64>() / n as f64,
            vertices.iter().map(|v| v[1]).sum::<f64>() / n as f64,
        ];
        let radii: Vec<f64> = vertices.iter().map(|v| len(sub(*v, center))).collect();
        let equal = |values: &[f64]| {
            let reference = values[0];
            reference > 1e-12
                && values
                    .iter()
                    .all(|v| (v - reference).abs() <= SHAPE_TOLERANCE * reference)
        };
        if equal(&sides) && equal(&radii) {
            loop_lines.sort();
            found
                .entry(loop_lines)
                .or_insert((center, ShapeKind::RegularPolygon(n)));
        }
    }
    found
        .into_iter()
        .map(|(key, (center, kind))| (key, center, kind))
        .collect()
}

/// One pointer sample. `to_screen` maps a sketch-plane point to screen
/// pixels with the current camera, so tolerances stay in pixels through
/// pan and zoom.
pub struct Query<'a> {
    pub pointer_mm: [f64; 2],
    pub pointer_px: [f32; 2],
    pub to_screen: &'a dyn Fn([f64; 2]) -> Option<[f32; 2]>,
    /// Start of the line being drawn, if any. It enables perpendicular and
    /// tangent targets relative to that point.
    pub from: Option<[f64; 2]>,
}

struct Candidate {
    target: Target,
    tolerance: f32,
}

fn candidates(geometry: &SnapGeometry, references: &[Reference], q: &Query) -> Vec<Candidate> {
    let mut out = vec![];
    let mut push = |key, kind, position, anchor, guides, tolerance| {
        out.push(Candidate {
            target: Target {
                key,
                kind,
                position,
                anchor,
                guides,
            },
            tolerance,
        })
    };
    for &(id, position, center) in &geometry.points {
        let kind = if center {
            TargetKind::Center
        } else {
            TargetKind::Endpoint
        };
        push(
            TargetKey::Point(id),
            kind,
            position,
            Some(id),
            vec![],
            POINT_SNAP_PX,
        );
    }
    for &(id, a, b) in &geometry.lines {
        push(
            TargetKey::LineMidpoint(id),
            TargetKind::LineMidpoint,
            mid(a, b),
            None,
            vec![],
            POINT_SNAP_PX,
        );
    }
    for (key, center, shape) in &geometry.centers {
        push(
            TargetKey::ShapeCenter(key.clone()),
            TargetKind::ShapeCenter(*shape),
            *center,
            None,
            vec![],
            POINT_SNAP_PX,
        );
    }
    for r in references {
        push(
            TargetKey::Reference(r.serial),
            TargetKind::Reference,
            r.position,
            match r.source {
                TargetKey::Point(id) => Some(id),
                _ => None,
            },
            vec![],
            POINT_SNAP_PX,
        );
    }
    if let [a, b] = references
        && len(sub(a.position, b.position)) > 1e-12
    {
        let (first, second) = if a.serial < b.serial { (a, b) } else { (b, a) };
        push(
            TargetKey::ReferenceMidpoint(first.serial, second.serial),
            TargetKind::ReferenceMidpoint,
            mid(a.position, b.position),
            None,
            vec![],
            POINT_SNAP_PX,
        );
    }
    let pointer = q.pointer_mm;
    for h in references {
        for v in references {
            if h.serial != v.serial
                && (h.position[0] - v.position[0]).abs() > 1e-12
                && (h.position[1] - v.position[1]).abs() > 1e-12
            {
                push(
                    TargetKey::GuideIntersection {
                        horizontal: h.serial,
                        vertical: v.serial,
                    },
                    TargetKind::GuideIntersection,
                    [v.position[0], h.position[1]],
                    None,
                    vec![
                        GuideLine {
                            from: h.position,
                            axis: Axis::Horizontal,
                        },
                        GuideLine {
                            from: v.position,
                            axis: Axis::Vertical,
                        },
                    ],
                    POINT_SNAP_PX,
                );
            }
        }
    }
    for r in references {
        for axis in [Axis::Horizontal, Axis::Vertical] {
            let i = match axis {
                Axis::Horizontal => 1,
                Axis::Vertical => 0,
            };
            for &(line, a, b) in &geometry.lines {
                let d = b[i] - a[i];
                if d.abs() <= 1e-12 {
                    continue;
                }
                let t = (r.position[i] - a[i]) / d;
                if !(-1e-12..=1. + 1e-12).contains(&t) {
                    continue;
                }
                let mut position = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
                position[i] = r.position[i];
                push(
                    TargetKey::GuideOnLine {
                        reference: r.serial,
                        axis,
                        line,
                    },
                    TargetKind::GuideOnLine,
                    position,
                    None,
                    vec![GuideLine {
                        from: r.position,
                        axis,
                    }],
                    POINT_SNAP_PX,
                );
            }
            let mut position = pointer;
            position[i] = r.position[i];
            push(
                TargetKey::Guide {
                    reference: r.serial,
                    axis,
                },
                TargetKind::Guide(axis),
                position,
                None,
                vec![GuideLine {
                    from: r.position,
                    axis,
                }],
                GUIDE_SNAP_PX,
            );
        }
    }
    for &(id, a, b) in &geometry.lines {
        let ab = sub(b, a);
        let t = (dot(sub(pointer, a), ab) / dot(ab, ab)).clamp(0., 1.);
        push(
            TargetKey::OnCurve(id),
            TargetKind::OnCurve,
            [a[0] + t * ab[0], a[1] + t * ab[1]],
            None,
            vec![],
            CURVE_SNAP_PX,
        );
    }
    let radial = |center: [f64; 2], radius: f64| {
        let d = sub(pointer, center);
        let l = len(d);
        (l > 1e-12).then(|| {
            (
                [center[0] + d[0] * radius / l, center[1] + d[1] * radius / l],
                d[1].atan2(d[0]),
            )
        })
    };
    for &(id, center, radius) in &geometry.circles {
        if let Some((position, _)) = radial(center, radius) {
            push(
                TargetKey::OnCurve(id),
                TargetKind::OnCurve,
                position,
                None,
                vec![],
                CURVE_SNAP_PX,
            );
        }
    }
    for arc in &geometry.arcs {
        if let Some((position, angle)) = radial(arc.center, arc.radius)
            && arc.contains(angle)
        {
            push(
                TargetKey::OnCurve(arc.id),
                TargetKind::OnCurve,
                position,
                None,
                vec![],
                CURVE_SNAP_PX,
            );
        }
    }
    if let Some(from) = q.from {
        for (key, kind, position) in relative_targets(geometry, from) {
            push(key, kind, position, None, vec![], POINT_SNAP_PX);
        }
    }
    out
}

/// Perpendicular feet and tangent points as seen from `from`, the start of
/// the line being drawn. A line's foot must lie on the segment; circle and
/// arc targets lie on the circle (arcs: inside their sweep). A target at
/// `from` itself would give a zero-length line and is omitted.
fn relative_targets(
    geometry: &SnapGeometry,
    from: [f64; 2],
) -> Vec<(TargetKey, TargetKind, [f64; 2])> {
    let mut out = vec![];
    let far_enough = |p: [f64; 2]| len(sub(p, from)) > 1e-9 * (1. + len(from));
    for &(id, a, b) in &geometry.lines {
        let ab = sub(b, a);
        let t = dot(sub(from, a), ab) / dot(ab, ab);
        let foot = [a[0] + t * ab[0], a[1] + t * ab[1]];
        if (0. ..=1.).contains(&t) && far_enough(foot) {
            out.push((
                TargetKey::Perpendicular(id, 0),
                TargetKind::Perpendicular,
                foot,
            ));
        }
    }
    let round = geometry
        .circles
        .iter()
        .map(|&(id, center, radius)| (id, center, radius, None))
        .chain(
            geometry
                .arcs
                .iter()
                .map(|arc| (arc.id, arc.center, arc.radius, Some(*arc))),
        );
    for (id, center, radius, arc) in round {
        let d = sub(from, center);
        let distance = len(d);
        if distance <= 1e-12 {
            continue;
        }
        let base = d[1].atan2(d[0]);
        let on = |angle: f64| {
            [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
            ]
        };
        let inside = |angle: f64| arc.is_none_or(|arc| arc.contains(angle));
        let mut candidates = vec![
            (
                TargetKey::Perpendicular(id, 0),
                TargetKind::Perpendicular,
                base,
            ),
            (
                TargetKey::Perpendicular(id, 1),
                TargetKind::Perpendicular,
                base + std::f64::consts::PI,
            ),
        ];
        if distance > radius * (1. + 1e-9) {
            let spread = (radius / distance).acos();
            candidates.push((
                TargetKey::Tangent(id, 0),
                TargetKind::Tangent,
                base + spread,
            ));
            candidates.push((
                TargetKey::Tangent(id, 1),
                TargetKind::Tangent,
                base - spread,
            ));
        }
        for (key, kind, angle) in candidates {
            let position = on(angle);
            if inside(angle) && far_enough(position) {
                out.push((key, kind, position));
            }
        }
    }
    out
}

/// Chooses the snap target for one pointer sample. A target is eligible
/// within its acquisition distance, or within `RELEASE_FACTOR` times that
/// distance when it is `previous`. The lowest class wins, then the nearest.
pub fn resolve(
    geometry: &SnapGeometry,
    references: &[Reference],
    q: &Query,
    previous: Option<&TargetKey>,
) -> Option<Target> {
    let pointer = q.pointer_px;
    let mut best: Option<(u8, f32, Target)> = None;
    for candidate in candidates(geometry, references, q) {
        let Some(screen) = (q.to_screen)(candidate.target.position) else {
            continue;
        };
        let distance = (screen[0] - pointer[0]).hypot(screen[1] - pointer[1]);
        if !distance.is_finite() {
            continue;
        }
        let limit = if previous == Some(&candidate.target.key) {
            candidate.tolerance * RELEASE_FACTOR
        } else {
            candidate.tolerance
        };
        if distance > limit {
            continue;
        }
        let class = candidate.target.key.class();
        if best
            .as_ref()
            .is_none_or(|(c, d, _)| (class, distance) < (*c, *d))
        {
            best = Some((class, distance, candidate.target));
        }
    }
    best.map(|(_, _, mut target)| {
        // Also show guides for references the target is exactly aligned with.
        for r in references {
            for (axis, i) in [(Axis::Horizontal, 1), (Axis::Vertical, 0)] {
                let scale = 1e-9 * (1. + r.position[i].abs());
                if (target.position[i] - r.position[i]).abs() <= scale
                    && len(sub(target.position, r.position)) > 1e-9
                    && !target
                        .guides
                        .iter()
                        .any(|g| g.axis == axis && g.from == r.position)
                {
                    target.guides.push(GuideLine {
                        from: r.position,
                        axis,
                    });
                }
            }
        }
        target
    })
}

/// The position to show and commit. A geometry target always wins over the
/// grid; the grid only applies when no geometry target is indicated.
pub fn placement(
    target: Option<&Target>,
    pointer_mm: [f64; 2],
    grid_spacing: Option<f64>,
) -> ([f64; 2], Option<EntityId>) {
    match (target, grid_spacing) {
        (Some(target), _) => (target.position, target.anchor),
        (None, Some(spacing)) if spacing > 0. => {
            (pointer_mm.map(|v| (v / spacing).round() * spacing), None)
        }
        (None, _) => (pointer_mm, None),
    }
}

#[derive(Clone, Debug)]
struct Dwell {
    key: TargetKey,
    since: f64,
    origin_px: [f32; 2],
    done: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DwellEvent {
    Acquired(u64),
    Released(u64),
}

#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub target: Option<Target>,
    pub event: Option<DwellEvent>,
    /// Seconds until a pending dwell completes; callers request a repaint so
    /// acquisition happens without further pointer motion.
    pub repaint_after: Option<f64>,
}

/// Per-sketch transient snap state: acquired references, the dwell timer and
/// the target shown in the previous frame.
#[derive(Clone, Debug, Default)]
pub struct SnapAssist {
    references: Vec<Reference>,
    dwell: Option<Dwell>,
    last: Option<TargetKey>,
    shown: Option<Target>,
    sketch: Option<EntityId>,
    next_serial: u64,
}

impl SnapAssist {
    pub fn references(&self) -> &[Reference] {
        &self.references
    }
    /// The target shown in the current frame, if any.
    pub fn shown(&self) -> Option<&Target> {
        self.shown.as_ref()
    }
    /// Removes every reference and pending dwell.
    pub fn clear(&mut self) {
        self.references.clear();
        self.dwell = None;
        self.last = None;
        self.shown = None;
    }
    /// Starts a frame: nothing is shown until `update` runs again.
    pub fn begin_frame(&mut self) {
        self.shown = None;
    }
    /// The caller may adjust the placement after snapping (for example the
    /// arc tool projects onto its radius). The target stays shown only if it
    /// still describes the final placement.
    pub fn settle(&mut self, final_position: [f64; 2]) {
        if self.shown.as_ref().is_some_and(|t| {
            (t.position[0] - final_position[0]).hypot(t.position[1] - final_position[1]) > 1e-9
        }) {
            self.shown = None;
        }
    }
    /// References belong to one sketch; switching sketches clears them.
    pub fn bind(&mut self, sketch: EntityId) {
        if self.sketch != Some(sketch) {
            self.clear();
            self.sketch = Some(sketch);
        }
    }
    /// The pointer left the canvas: keep references, drop dwell/hysteresis.
    pub fn pointer_left(&mut self) {
        self.dwell = None;
        self.last = None;
        self.shown = None;
    }
    /// Re-anchors references to current solved geometry. Point, midpoint and
    /// center references follow their source; on-curve references keep their
    /// acquired position while the curve exists. Missing sources are dropped.
    pub fn revalidate(&mut self, geometry: &SnapGeometry) {
        self.references.retain_mut(|r| match &r.source {
            TargetKey::OnCurve(id) => geometry.has_curve(*id),
            source => match geometry.locate(source) {
                Some(position) => {
                    r.position = position;
                    true
                }
                None => false,
            },
        });
    }
    pub fn update(&mut self, geometry: &SnapGeometry, q: &Query, now: f64) -> Outcome {
        let target = resolve(geometry, &self.references, q, self.last.as_ref());
        let mut outcome = Outcome::default();
        match target.as_ref().filter(|t| t.key.acquirable()) {
            None => self.dwell = None,
            Some(candidate) => {
                let stable = self.dwell.as_ref().is_some_and(|d| {
                    d.key == candidate.key
                        && (d.origin_px[0] - q.pointer_px[0])
                            .hypot(d.origin_px[1] - q.pointer_px[1])
                            <= DWELL_STABILITY_PX
                });
                if !stable {
                    self.dwell = Some(Dwell {
                        key: candidate.key.clone(),
                        since: now,
                        origin_px: q.pointer_px,
                        done: false,
                    });
                }
                let dwell = self.dwell.as_mut().expect("dwell set");
                if !dwell.done {
                    let elapsed = now - dwell.since;
                    if elapsed >= DWELL_SECONDS {
                        dwell.done = true;
                        outcome.event = Some(self.toggle(candidate));
                    } else {
                        outcome.repaint_after = Some(DWELL_SECONDS - elapsed);
                    }
                }
            }
        }
        // Re-resolve after a toggle so the shown target reflects references.
        let target = if outcome.event.is_some() {
            let target = resolve(
                geometry,
                &self.references,
                q,
                target.as_ref().map(|t| &t.key),
            );
            // An acquired on-curve point now resolves as a reference; keep the
            // completed dwell on it so it is not immediately released again.
            if let (Some(dwell), Some(t)) = (self.dwell.as_mut(), target.as_ref())
                && t.key.acquirable()
            {
                dwell.key = t.key.clone();
            }
            target
        } else {
            target
        };
        self.last = target.as_ref().map(|t| t.key.clone());
        self.shown = target.clone();
        outcome.target = target;
        outcome
    }
    fn toggle(&mut self, target: &Target) -> DwellEvent {
        let existing = match &target.key {
            TargetKey::Reference(serial) => {
                self.references.iter().position(|r| r.serial == *serial)
            }
            key
            @ (TargetKey::Point(_) | TargetKey::LineMidpoint(_) | TargetKey::ShapeCenter(_)) => {
                self.references.iter().position(|r| &r.source == key)
            }
            _ => None,
        };
        if let Some(index) = existing {
            return DwellEvent::Released(self.references.remove(index).serial);
        }
        let serial = self.next_serial;
        self.next_serial += 1;
        self.references.push(Reference {
            serial,
            source: target.key.clone(),
            kind: target.kind,
            position: target.position,
        });
        if self.references.len() > MAX_REFERENCES {
            self.references.remove(0);
        }
        DwellEvent::Acquired(serial)
    }
}

#[cfg(test)]
#[path = "sketch_snap_tests.rs"]
mod tests;

//! Plan outline editor shared by the Wall and Floor slab forms.
//!
//! A wall path or slab outline is a list of corners on a horizontal plane.
//! Each corner keeps exact Inspector fields (literal millimeters or
//! `=$parameter` expressions). In the viewport the user draws corners with
//! snapping, closes the outline on its first corner, picks a corner or
//! segment and moves it with live segment dimensions. Painting never edits
//! the document; the form commits only on Apply, so Cancel keeps the source.
use super::{TerminationViewport, dimension::DimensionField, value_entry::format_number};
use crate::egui;
use multicad_core::{
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    model::{self, length},
    parameters::{EvaluatedParameters, Unit},
    sketch::Frame,
};

/// What the outline becomes. A wall path may stay open; a slab outline is
/// always a closed polygon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OutlineKind {
    WallPath,
    SlabOutline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Selection {
    Corner(usize),
    /// Segment `i` runs from corner `i` to corner `i + 1` (wrapping when closed).
    Segment(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Idle,
    Drawing,
    Moving(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SnapKind {
    Free,
    Grid,
    Orthogonal,
    Corner,
    Close,
}
impl SnapKind {
    fn label(self) -> &'static str {
        match self {
            Self::Free => "",
            Self::Grid => "grid",
            Self::Orthogonal => "orthogonal",
            Self::Corner => "corner",
            Self::Close => "close outline",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Snapped {
    pub point: [f64; 2],
    pub kind: SnapKind,
}

/// A geometric problem with an actionable message and where to show it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct OutlineIssue {
    pub message: String,
    pub at: Option<[f64; 2]>,
    pub segments: Vec<usize>,
}

#[derive(Clone)]
struct Snapshot {
    corners: Vec<[DimensionField; 2]>,
    closed: bool,
}

/// Grid steps offered for snapping, in millimeters (0 = off).
const GRID_STEPS: [f64; 5] = [0., 10., 50., 100., 500.];
/// Pick tolerance for handles and snapping, in screen pixels.
const PICK_PX: f32 = 9.;
/// Angular window for orthogonal snapping.
const ORTHO_DEG: f64 = 8.;

fn mm(value: f64) -> DimensionField {
    DimensionField::from_dimension(&length(value), "mm").expect("millimeters are a length unit")
}
fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
/// Distance from `p` to segment `a`-`b`.
fn segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let d = sub(b, a);
    let l2 = d[0] * d[0] + d[1] * d[1];
    if l2 < 1e-12 {
        return dist(p, a);
    }
    let t = (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2).clamp(0., 1.);
    dist(p, [a[0] + d[0] * t, a[1] + d[1] * t])
}
/// The proper crossing point of two segments, excluding shared endpoints.
fn crossing(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> Option<[f64; 2]> {
    let (r, s) = (sub(b, a), sub(d, c));
    let den = cross(r, s);
    if den.abs() < 1e-12 {
        return None;
    }
    let t = cross(sub(c, a), s) / den;
    let u = cross(sub(c, a), r) / den;
    (t > 1e-9 && t < 1. - 1e-9 && u > 1e-9 && u < 1. - 1e-9)
        .then(|| [a[0] + r[0] * t, a[1] + r[1] * t])
}
fn round_to(v: f64, step: f64) -> f64 {
    if step > 0. {
        (v / step).round() * step
    } else {
        v
    }
}

/// Segment index pairs of an outline with `n` corners.
fn segment_count(n: usize, closed: bool) -> usize {
    match (n, closed) {
        (0 | 1, _) => 0,
        (2, true) => 1,
        (n, true) => n,
        (n, false) => n - 1,
    }
}

/// Geometric checks shown in the viewport and the Inspector before Preview.
/// The native planner stays authoritative; these are early, located hints.
pub(super) fn diagnose(points: &[[f64; 2]], closed: bool, kind: OutlineKind) -> Vec<OutlineIssue> {
    let mut issues = vec![];
    let n = points.len();
    let needed = match kind {
        OutlineKind::SlabOutline => 3,
        OutlineKind::WallPath if closed => 3,
        OutlineKind::WallPath => 2,
    };
    if n < needed {
        issues.push(OutlineIssue {
            message: format!(
                "{} needs at least {needed} corners; draw or add {} more",
                match kind {
                    OutlineKind::SlabOutline => "A slab outline",
                    OutlineKind::WallPath if closed => "A closed room",
                    OutlineKind::WallPath => "A wall path",
                },
                needed - n
            ),
            at: points.last().copied(),
            segments: vec![],
        });
        return issues;
    }
    let count = segment_count(n, closed || kind == OutlineKind::SlabOutline);
    let seg = |i: usize| (points[i], points[(i + 1) % n]);
    for i in 0..count {
        let (a, b) = seg(i);
        if dist(a, b) < 1e-3 {
            issues.push(OutlineIssue {
                message: format!(
                    "Segment {} has no length: corners {} and {} coincide; remove one",
                    i + 1,
                    i + 1,
                    (i + 1) % n + 1
                ),
                at: Some(a),
                segments: vec![i],
            });
        }
    }
    for i in 0..count {
        for j in i + 1..count {
            let adjacent = j == i + 1 || (i == 0 && j == count - 1 && count == n);
            let (a, b) = seg(i);
            let (c, d) = seg(j);
            if adjacent {
                // Adjacent segments that fold back onto each other overlap.
                let (u, v) = (sub(b, a), sub(d, c));
                let parallel = cross(u, v).abs() < 1e-9 * dist(a, b).max(1.) * dist(c, d).max(1.);
                if parallel && u[0] * v[0] + u[1] * v[1] < 0. {
                    let corner = if j == i + 1 { j } else { 0 };
                    issues.push(OutlineIssue {
                        message: format!(
                            "The outline doubles back at corner {}; move or remove it",
                            corner + 1
                        ),
                        at: Some(points[corner]),
                        segments: vec![i, j],
                    });
                }
                continue;
            }
            if let Some(at) = crossing(a, b, c, d) {
                issues.push(OutlineIssue {
                    message: format!(
                        "Segments {} and {} cross at ({}, {}) mm; move corner {} or {} so the outline does not cross itself",
                        i + 1,
                        j + 1,
                        format_number(at[0].round()),
                        format_number(at[1].round()),
                        (i + 1) % n + 1,
                        j + 1,
                    ),
                    at: Some(at),
                    segments: vec![i, j],
                });
            }
        }
    }
    if (closed || kind == OutlineKind::SlabOutline) && issues.is_empty() {
        let area: f64 = (0..n).map(|i| cross(points[i], points[(i + 1) % n])).sum();
        if area.abs() / 2. < 1. {
            issues.push(OutlineIssue {
                message: "The closed outline has no area; spread the corners apart".into(),
                at: points.first().copied(),
                segments: vec![],
            });
        }
    }
    issues
}

/// Corners of existing walls and slabs used as snap targets, excluding the
/// feature being edited. Wall face corners are included so a floor can snap
/// to the inside of a room.
pub(super) fn plan_targets(document: &Document, exclude: Option<EntityId>) -> Vec<[f64; 2]> {
    let Ok(parameters) = multicad_core::parameters::evaluate(document) else {
        return vec![];
    };
    let mut targets = vec![];
    for entity in model::walls(document) {
        if Some(entity.id) == exclude {
            continue;
        }
        if let Some(resolved) = model::read_wall(document, entity.id)
            .ok()
            .and_then(|w| model::resolve_wall(document, &parameters, &w).ok())
        {
            targets.extend(resolved.points.iter().copied());
            if let Ok(plan) = resolved.plan() {
                targets.extend(plan.outer.points.iter().copied());
                for hole in &plan.holes {
                    targets.extend(hole.points.iter().copied());
                }
            }
        }
    }
    for entity in model::slabs(document) {
        if Some(entity.id) == exclude {
            continue;
        }
        if let Some(resolved) = model::read_slab(document, entity.id)
            .ok()
            .and_then(|s| model::resolve_slab(document, &parameters, &s).ok())
        {
            targets.extend(resolved.outline.iter().copied());
        }
    }
    targets
}

pub(super) struct OutlineEditor {
    kind: OutlineKind,
    corners: Vec<[DimensionField; 2]>,
    closed: bool,
    mode: Mode,
    /// The corners before the current draw or move, restored by Cancel.
    before: Option<Snapshot>,
    selected: Option<Selection>,
    pub grid_mm: f64,
    pub ortho: bool,
    pub snap_corners: bool,
    rect_origin: [DimensionField; 2],
    rect_size: [DimensionField; 2],
    next_length: DimensionField,
    next_angle_deg: f64,
    segment_length: DimensionField,
    paste: String,
    hover: Option<Snapped>,
    message: Option<String>,
}

impl OutlineEditor {
    pub fn new(kind: OutlineKind, points: &[[Dimension; 2]], closed: bool) -> Result<Self, String> {
        let corners = points
            .iter()
            .map(|[x, y]| {
                Ok([
                    DimensionField::from_dimension(x, "mm")?,
                    DimensionField::from_dimension(y, "mm")?,
                ])
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            kind,
            corners,
            closed: closed || kind == OutlineKind::SlabOutline,
            mode: Mode::Idle,
            before: None,
            selected: None,
            grid_mm: 10.,
            ortho: true,
            snap_corners: true,
            rect_origin: [mm(0.), mm(0.)],
            rect_size: [mm(4000.), mm(3000.)],
            next_length: mm(1000.),
            next_angle_deg: 0.,
            segment_length: mm(1000.),
            paste: String::new(),
            hover: None,
            message: None,
        })
    }
    pub fn from_mm(kind: OutlineKind, points: &[[f64; 2]], closed: bool) -> Self {
        let points: Vec<_> = points
            .iter()
            .map(|p| [length(p[0]), length(p[1])])
            .collect();
        Self::new(kind, &points, closed).expect("literal millimeters")
    }
    pub fn closed(&self) -> bool {
        self.closed
    }
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.corners.len()
    }
    #[cfg(test)]
    pub fn selected(&self) -> Option<Selection> {
        self.selected
    }
    /// True while viewport clicks belong to this editor (drawing or moving).
    pub fn capturing(&self) -> bool {
        self.mode != Mode::Idle
    }
    pub fn drawing(&self) -> bool {
        self.mode == Mode::Drawing
    }
    /// The authored corners, keeping untouched expressions and units.
    pub fn dimensions(&self, document: &Document) -> Result<Vec<[Dimension; 2]>, String> {
        self.corners
            .iter()
            .enumerate()
            .map(|(i, [x, y])| {
                let f = |field: &DimensionField, axis: &str| {
                    field
                        .resolve(document)
                        .map_err(|e| format!("Corner {} {axis}: {e}", i + 1))
                };
                Ok([f(x, "x")?, f(y, "y")?])
            })
            .collect()
    }
    /// Corner positions in millimeters, with expressions evaluated.
    pub fn positions(&self, document: &Document) -> Result<Vec<[f64; 2]>, String> {
        let parameters =
            multicad_core::parameters::evaluate(document).map_err(|e| e.to_string())?;
        self.positions_with(document, &parameters)
    }
    fn positions_with(
        &self,
        document: &Document,
        parameters: &EvaluatedParameters,
    ) -> Result<Vec<[f64; 2]>, String> {
        self.dimensions(document)?
            .iter()
            .map(|[x, y]| {
                Ok([
                    x.value_in(Unit::Millimeter, Some(parameters))?,
                    y.value_in(Unit::Millimeter, Some(parameters))?,
                ])
            })
            .collect()
    }
    pub fn issues(&self, document: &Document) -> Vec<OutlineIssue> {
        match self.positions(document) {
            Ok(points) => diagnose(&points, self.closed, self.kind),
            Err(message) => vec![OutlineIssue {
                message,
                at: None,
                segments: vec![],
            }],
        }
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            corners: self.corners.clone(),
            closed: self.closed,
        }
    }
    pub(super) fn set_corner(&mut self, index: usize, point: [f64; 2]) {
        self.corners[index][0].set_literal(point[0], "mm");
        self.corners[index][1].set_literal(point[1], "mm");
    }
    /// Replace every corner with literal millimeters.
    pub fn set_points(&mut self, points: &[[f64; 2]], closed: bool) {
        self.corners = points.iter().map(|p| [mm(p[0]), mm(p[1])]).collect();
        self.closed = closed || self.kind == OutlineKind::SlabOutline;
        self.selected = None;
    }
    /// Replace the corners with authored dimensions (for example a wall path
    /// with parameter expressions reused as a floor outline).
    pub fn set_dimensions(
        &mut self,
        points: &[[Dimension; 2]],
        closed: bool,
    ) -> Result<(), String> {
        let replaced = Self::new(self.kind, points, closed)?;
        self.corners = replaced.corners;
        self.closed = replaced.closed;
        self.selected = None;
        Ok(())
    }
    /// Type into one corner field, as the Inspector does.
    #[cfg(test)]
    pub fn set_text(&mut self, corner: usize, axis: usize, text: &str) {
        self.corners[corner][axis].text = text.into();
    }
    /// Start drawing a new outline; Cancel restores the current one.
    pub fn start_drawing(&mut self) {
        self.before = Some(self.snapshot());
        self.corners.clear();
        self.closed = false;
        self.selected = None;
        self.mode = Mode::Drawing;
        self.message = None;
    }
    /// Finish drawing (slabs always close) and keep the drawn corners.
    pub fn finish(&mut self) {
        if self.mode == Mode::Drawing && self.kind == OutlineKind::SlabOutline {
            self.closed = true;
        }
        self.mode = Mode::Idle;
        self.before = None;
        self.hover = None;
    }
    /// Abandon the current draw or move and restore the previous corners.
    /// Returns whether the corners changed back.
    pub fn cancel(&mut self) -> bool {
        self.mode = Mode::Idle;
        self.hover = None;
        match self.before.take() {
            Some(snapshot) => {
                self.corners = snapshot.corners;
                self.closed = snapshot.closed;
                true
            }
            None => false,
        }
    }
    /// Snap a raw plan point. `tolerance` is the pick radius in millimeters.
    pub fn snap(
        &self,
        raw: [f64; 2],
        points: &[[f64; 2]],
        targets: &[[f64; 2]],
        tolerance: f64,
    ) -> Snapped {
        let moving = match self.mode {
            Mode::Moving(i) => Some(i),
            _ => None,
        };
        if self.mode == Mode::Drawing && points.len() >= 3 && dist(raw, points[0]) <= tolerance {
            return Snapped {
                point: points[0],
                kind: SnapKind::Close,
            };
        }
        if self.snap_corners {
            let own = points
                .iter()
                .enumerate()
                .filter(|(i, _)| Some(*i) != moving)
                .map(|(_, p)| *p);
            if let Some(best) = own
                .chain(targets.iter().copied())
                .filter(|p| dist(raw, *p) <= tolerance)
                .min_by(|a, b| dist(raw, *a).total_cmp(&dist(raw, *b)))
            {
                return Snapped {
                    point: best,
                    kind: SnapKind::Corner,
                };
            }
        }
        // The reference for orthogonal snapping: the previous corner while
        // drawing, or the moved corner's previous neighbour.
        let reference = match self.mode {
            Mode::Drawing => points.last().copied(),
            Mode::Moving(i) if points.len() > 1 => {
                let n = points.len();
                if i > 0 {
                    Some(points[i - 1])
                } else if self.closed {
                    Some(points[n - 1])
                } else {
                    Some(points[1])
                }
            }
            _ => None,
        };
        if self.ortho
            && let Some(r) = reference
        {
            let d = sub(raw, r);
            let angle = d[1].atan2(d[0]).to_degrees().rem_euclid(90.);
            if angle.min(90. - angle) <= ORTHO_DEG && dist(raw, r) > 1e-6 {
                let horizontal = d[0].abs() >= d[1].abs();
                let point = if horizontal {
                    [r[0] + round_to(d[0], self.grid_mm), r[1]]
                } else {
                    [r[0], r[1] + round_to(d[1], self.grid_mm)]
                };
                return Snapped {
                    point,
                    kind: SnapKind::Orthogonal,
                };
            }
        }
        if self.grid_mm > 0. {
            return Snapped {
                point: [
                    round_to(raw[0], self.grid_mm),
                    round_to(raw[1], self.grid_mm),
                ],
                kind: SnapKind::Grid,
            };
        }
        Snapped {
            point: raw,
            kind: SnapKind::Free,
        }
    }
    /// A viewport click at `raw` (plan millimeters). Returns whether the
    /// corners changed.
    pub fn click(
        &mut self,
        raw: [f64; 2],
        points: &[[f64; 2]],
        targets: &[[f64; 2]],
        tolerance: f64,
    ) -> bool {
        let snapped = self.snap(raw, points, targets, tolerance);
        match self.mode {
            Mode::Drawing => {
                if snapped.kind == SnapKind::Close {
                    self.closed = true;
                    self.finish();
                    self.message = Some(format!(
                        "Outline closed with {} corners",
                        self.corners.len()
                    ));
                    return true;
                }
                if points
                    .last()
                    .is_some_and(|last| dist(*last, snapped.point) < 1e-6)
                {
                    return false;
                }
                self.corners.push([mm(0.), mm(0.)]);
                let last = self.corners.len() - 1;
                self.set_corner(last, snapped.point);
                true
            }
            Mode::Moving(i) => {
                self.set_corner(i, snapped.point);
                self.mode = Mode::Idle;
                self.before = None;
                self.selected = Some(Selection::Corner(i));
                true
            }
            Mode::Idle => {
                if let Some((i, _)) = points
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| dist(raw, **p) <= tolerance)
                    .min_by(|a, b| dist(raw, *a.1).total_cmp(&dist(raw, *b.1)))
                {
                    self.select(Selection::Corner(i), points);
                    self.before = Some(self.snapshot());
                    self.mode = Mode::Moving(i);
                    return false;
                }
                let n = points.len();
                let count = segment_count(n, self.closed);
                let hit = (0..count)
                    .map(|i| (i, segment_distance(raw, points[i], points[(i + 1) % n])))
                    .filter(|(_, d)| *d <= tolerance)
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                match hit {
                    Some((i, _)) => self.select(Selection::Segment(i), points),
                    None => self.selected = None,
                }
                false
            }
        }
    }
    pub fn select(&mut self, selection: Selection, points: &[[f64; 2]]) {
        if let Selection::Segment(i) = selection
            && let Some(l) = self.segment_lengths(points).get(i)
        {
            self.segment_length.set_literal(l.round(), "mm");
        }
        self.selected = Some(selection);
    }
    pub fn segment_lengths(&self, points: &[[f64; 2]]) -> Vec<f64> {
        let n = points.len();
        (0..segment_count(n, self.closed))
            .map(|i| dist(points[i], points[(i + 1) % n]))
            .collect()
    }
    /// Replace the corners with a `width` x `depth` rectangle at `origin`.
    pub fn rectangle(&mut self, origin: [f64; 2], width: f64, depth: f64) -> Result<(), String> {
        if !(width > 0. && depth > 0. && width.is_finite() && depth.is_finite()) {
            return Err("Rectangle width and depth must be positive".into());
        }
        let [x, y] = origin;
        self.set_points(
            &[
                [x, y],
                [x + width, y],
                [x + width, y + depth],
                [x, y + depth],
            ],
            true,
        );
        Ok(())
    }
    /// Set segment `i` to `value` mm by moving its end corner along it.
    pub fn set_segment_length(
        &mut self,
        i: usize,
        value: f64,
        points: &[[f64; 2]],
    ) -> Result<(), String> {
        let n = points.len();
        if i >= segment_count(n, self.closed) {
            return Err(format!("Segment {} does not exist", i + 1));
        }
        if !(value > 0. && value.is_finite()) {
            return Err("Segment length must be positive".into());
        }
        let (a, b) = (points[i], points[(i + 1) % n]);
        let l = dist(a, b);
        if l < 1e-6 {
            return Err(format!(
                "Segment {} has no direction; move a corner first",
                i + 1
            ));
        }
        let end = [
            a[0] + (b[0] - a[0]) / l * value,
            a[1] + (b[1] - a[1]) / l * value,
        ];
        self.set_corner((i + 1) % n, end);
        Ok(())
    }
    /// Append a corner at `distance` mm and `angle_deg` from the last one.
    pub fn add_by_length(
        &mut self,
        distance: f64,
        angle_deg: f64,
        points: &[[f64; 2]],
    ) -> Result<(), String> {
        let last = *points
            .last()
            .ok_or("Click the first corner in the viewport first")?;
        if !(distance > 0. && distance.is_finite()) {
            return Err("Segment length must be positive".into());
        }
        let (s, c) = angle_deg.to_radians().sin_cos();
        self.corners.push([mm(0.), mm(0.)]);
        let i = self.corners.len() - 1;
        self.set_corner(i, [last[0] + c * distance, last[1] + s * distance]);
        Ok(())
    }
    pub fn insert_midpoint(&mut self, segment: usize, points: &[[f64; 2]]) {
        let n = points.len();
        if segment >= segment_count(n, self.closed) {
            return;
        }
        let (a, b) = (points[segment], points[(segment + 1) % n]);
        self.corners.insert(segment + 1, [mm(0.), mm(0.)]);
        self.set_corner(segment + 1, [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.]);
        self.selected = Some(Selection::Corner(segment + 1));
    }
    pub fn remove_corner(&mut self, index: usize) {
        if index < self.corners.len() {
            self.corners.remove(index);
            self.selected = None;
        }
    }
    /// Replace the corners from "x, y" lines in millimeters.
    pub fn paste_points(&mut self, text: &str) -> Result<(), String> {
        let points = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .enumerate()
            .map(|(i, line)| {
                let values: Vec<f64> = line
                    .split(',')
                    .map(|v| v.trim().parse::<f64>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| format!("Line {} must be \"x, y\" in millimeters", i + 1))?;
                match values.as_slice() {
                    [x, y] if x.is_finite() && y.is_finite() => Ok([*x, *y]),
                    _ => Err(format!("Line {} must be \"x, y\" in millimeters", i + 1)),
                }
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.set_points(&points, self.closed);
        Ok(())
    }

    /// Inspector section. Returns whether the corners changed.
    pub fn show(&mut self, ui: &mut egui::Ui, document: &Document, plane_label: &str) -> bool {
        let mut changed = false;
        let points = self.positions(document).unwrap_or_default();
        ui.label(match self.kind {
            OutlineKind::WallPath => "Wall path",
            OutlineKind::SlabOutline => "Slab outline",
        });
        ui.horizontal_wrapped(|ui| match self.mode {
            Mode::Idle => {
                if ui
                    .button("Draw in viewport")
                    .on_hover_text("Click corners on the plan; click the first corner to close")
                    .clicked()
                {
                    self.start_drawing();
                    changed = true;
                }
            }
            Mode::Drawing => {
                if ui.button("Finish").on_hover_text("Enter").clicked() {
                    self.finish();
                    changed = true;
                }
                if ui.button("Cancel drawing").on_hover_text("Esc").clicked() {
                    changed |= self.cancel();
                }
            }
            Mode::Moving(i) => {
                ui.label(format!("Moving corner {}", i + 1));
                if ui.button("Cancel move").on_hover_text("Esc").clicked() {
                    changed |= self.cancel();
                }
            }
        });
        match self.mode {
            Mode::Drawing => {
                ui.small(format!(
                    "Click corners on {plane_label}. Click the first corner to close, Enter to finish an open path, Esc to cancel and keep the previous outline."
                ));
                ui.horizontal(|ui| {
                    ui.label("Next segment");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.next_length.text)
                            .desired_width(70.)
                            .hint_text("mm"),
                    );
                    ui.label("mm at");
                    ui.add(
                        egui::DragValue::new(&mut self.next_angle_deg)
                            .range(-360.0..=360.0)
                            .suffix("°"),
                    );
                    if ui
                        .button("Add")
                        .on_hover_text(
                            "Append a corner at this length and angle from the last corner",
                        )
                        .clicked()
                    {
                        let result = self
                            .next_length
                            .resolve(document)
                            .and_then(|d| d.value_in(Unit::Millimeter, None))
                            .and_then(|l| self.add_by_length(l, self.next_angle_deg, &points));
                        match result {
                            Ok(()) => changed = true,
                            Err(e) => self.message = Some(e),
                        }
                    }
                });
            }
            Mode::Moving(_) => {
                ui.small("Click the new corner position in the viewport; the neighbouring segment lengths update live.");
            }
            Mode::Idle => {
                ui.small("Click a corner handle in the viewport to move it, or a segment to edit its length.");
            }
        }
        if self.mode == Mode::Idle {
            egui::CollapsingHeader::new("Rectangle by dimensions")
                .id_salt("outline-rectangle")
                .show(ui, |ui| {
                    for (field, label) in self.rect_origin.iter_mut().zip(["Corner x", "Corner y"])
                    {
                        field.show(ui, label, "First corner of the rectangle");
                    }
                    for (field, label) in self.rect_size.iter_mut().zip(["Width (x)", "Depth (y)"])
                    {
                        field.show(ui, label, "Rectangle size along X and Y");
                    }
                    if ui.button("Use rectangle").clicked() {
                        let value = |f: &DimensionField| {
                            f.resolve(document).and_then(|d| {
                                let parameters = multicad_core::parameters::evaluate(document)
                                    .map_err(|e| e.to_string())?;
                                d.value_in(Unit::Millimeter, Some(&parameters))
                            })
                        };
                        let result = (|| {
                            let origin =
                                [value(&self.rect_origin[0])?, value(&self.rect_origin[1])?];
                            let (w, d) = (value(&self.rect_size[0])?, value(&self.rect_size[1])?);
                            self.rectangle(origin, w, d)
                        })();
                        match result {
                            Ok(()) => changed = true,
                            Err(e) => self.message = Some(e),
                        }
                    }
                });
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Snap");
            egui::ComboBox::from_id_salt("outline-grid")
                .selected_text(if self.grid_mm > 0. {
                    format!("{} mm grid", format_number(self.grid_mm))
                } else {
                    "No grid".into()
                })
                .show_ui(ui, |ui| {
                    for step in GRID_STEPS {
                        let label = if step > 0. {
                            format!("{} mm grid", format_number(step))
                        } else {
                            "No grid".into()
                        };
                        ui.selectable_value(&mut self.grid_mm, step, label);
                    }
                });
            ui.checkbox(&mut self.ortho, "Orthogonal");
            ui.checkbox(&mut self.snap_corners, "Corners");
        });
        let lengths = self.segment_lengths(&points);
        let mut remove = None;
        let mut select = None;
        for i in 0..self.corners.len() {
            ui.push_id(("outline-corner", i), |ui| {
                ui.horizontal(|ui| {
                    let selected = self.selected == Some(Selection::Corner(i));
                    if ui
                        .selectable_label(selected, format!("{}", i + 1))
                        .on_hover_text("Select this corner")
                        .clicked()
                    {
                        select = Some(Selection::Corner(i));
                    }
                    for field in self.corners[i].iter_mut() {
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut field.text)
                                    .desired_width(72.)
                                    .hint_text("mm or =$p"),
                            )
                            .changed();
                    }
                    if ui
                        .small_button("×")
                        .on_hover_text("Remove this corner")
                        .clicked()
                    {
                        remove = Some(i);
                    }
                });
                if let Some(l) = lengths.get(i) {
                    let selected = self.selected == Some(Selection::Segment(i));
                    if ui
                        .selectable_label(
                            selected,
                            format!(
                                "   ↳ segment {}: {} mm",
                                i + 1,
                                format_number((l * 10.).round() / 10.)
                            ),
                        )
                        .clicked()
                    {
                        select = Some(Selection::Segment(i));
                    }
                }
            });
        }
        if let Some(selection) = select {
            self.select(selection, &points);
        }
        if let Some(i) = remove {
            self.remove_corner(i);
            changed = true;
        }
        if let Some(Selection::Segment(i)) = self.selected
            && i < lengths.len()
        {
            ui.horizontal(|ui| {
                ui.label(format!("Segment {} length", i + 1));
                ui.add(
                    egui::TextEdit::singleline(&mut self.segment_length.text).desired_width(70.),
                );
                if ui
                    .button("Set")
                    .on_hover_text("Moves the segment's end corner along it")
                    .clicked()
                {
                    let result = self
                        .segment_length
                        .resolve(document)
                        .and_then(|d| d.value_in(Unit::Millimeter, None))
                        .and_then(|l| self.set_segment_length(i, l, &points));
                    match result {
                        Ok(()) => changed = true,
                        Err(e) => self.message = Some(e),
                    }
                }
                if ui
                    .button("Insert corner")
                    .on_hover_text("Split the segment at its midpoint")
                    .clicked()
                {
                    self.insert_midpoint(i, &points);
                    changed = true;
                }
            });
        }
        if self.kind == OutlineKind::WallPath {
            changed |= ui.checkbox(&mut self.closed, "Close into a room").changed();
        }
        for issue in self.issues(document) {
            ui.colored_label(egui::Color32::LIGHT_RED, format!("⚠ {}", issue.message));
        }
        if let Some(message) = &self.message {
            ui.small(message);
        }
        egui::CollapsingHeader::new("Paste coordinates")
            .id_salt("outline-paste")
            .show(ui, |ui| {
                ui.small("x, y in mm, one corner per line. Replaces every corner.");
                ui.add(
                    egui::TextEdit::multiline(&mut self.paste)
                        .desired_rows(4)
                        .desired_width(ui.available_width()),
                );
                if ui.button("Replace corners").clicked() {
                    let text = self.paste.clone();
                    match self.paste_points(&text) {
                        Ok(()) => changed = true,
                        Err(e) => self.message = Some(e),
                    }
                }
            });
        changed
    }

    /// Draw the outline overlay and route pointer input on the plane at
    /// height `z`. Returns whether the corners changed.
    pub fn viewport(
        &mut self,
        document: &Document,
        view: &TerminationViewport<'_>,
        z: f64,
        targets: &[[f64; 2]],
    ) -> bool {
        let Ok(points) = self.positions(document) else {
            return false;
        };
        let frame = Frame {
            origin: [0., 0., z],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
            n: [0., 0., 1.],
        };
        let tolerance = (PICK_PX / view.camera.scale.max(1e-6)) as f64;
        let mut changed = false;
        let (escape, enter) = view.painter.ctx().input(|i| {
            (
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::Enter),
            )
        });
        if escape && self.capturing() {
            changed |= self.cancel();
        } else if enter && self.drawing() {
            self.finish();
            changed = true;
        }
        let raw = view
            .pointer
            .filter(|p| view.rect.contains(*p))
            .and_then(|p| view.camera.on_plane(p, view.rect, frame));
        self.hover = raw.map(|r| self.snap(r, &points, targets, tolerance));
        if view.clicked
            && let Some(r) = raw
        {
            changed |= self.click(r, &points, targets, tolerance);
        }
        let points = if changed {
            self.positions(document).unwrap_or(points)
        } else {
            points
        };
        self.paint(view, z, &points);
        changed
    }

    fn paint(&self, view: &TerminationViewport<'_>, z: f64, points: &[[f64; 2]]) {
        let to_screen = |p: [f64; 2]| {
            let s = view
                .camera
                .project([p[0] as f32, p[1] as f32, z as f32], view.rect);
            egui::pos2(s[0], s[1])
        };
        let line = egui::Color32::from_rgb(64, 196, 255);
        let accent = egui::Color32::from_rgb(255, 176, 64);
        let bad = egui::Color32::from_rgb(255, 80, 80);
        let font = egui::FontId::proportional(11.);
        let issues = diagnose(points, self.closed, self.kind);
        let flagged: Vec<usize> = issues.iter().flat_map(|i| i.segments.clone()).collect();
        let n = points.len();
        let label = |a: [f64; 2], b: [f64; 2], color: egui::Color32| {
            let l = dist(a, b);
            if l < 1e-3 {
                return;
            }
            let (sa, sb) = (to_screen(a), to_screen(b));
            let mid = sa + (sb - sa) / 2.;
            let dir = (sb - sa).normalized();
            let offset = egui::vec2(-dir.y, dir.x) * 12.;
            view.painter.text(
                mid + offset,
                egui::Align2::CENTER_CENTER,
                format!("{} mm", format_number(l.round())),
                font.clone(),
                color,
            );
        };
        for i in 0..segment_count(n, self.closed) {
            let (a, b) = (points[i], points[(i + 1) % n]);
            let selected = self.selected == Some(Selection::Segment(i));
            let color = if flagged.contains(&i) {
                bad
            } else if selected {
                accent
            } else {
                line
            };
            view.painter.line_segment(
                [to_screen(a), to_screen(b)],
                egui::Stroke::new(if selected { 3. } else { 2. }, color),
            );
            label(a, b, color);
        }
        // Rubber band and live dimensions for the pending corner.
        if let Some(hover) = self.hover {
            let p = hover.point;
            let dashed = egui::Stroke::new(1.5, accent);
            match self.mode {
                Mode::Drawing => {
                    if let Some(last) = points.last() {
                        view.painter.extend(egui::Shape::dashed_line(
                            &[to_screen(*last), to_screen(p)],
                            dashed,
                            6.,
                            4.,
                        ));
                        label(*last, p, accent);
                        let d = sub(p, *last);
                        view.painter.text(
                            to_screen(p) + egui::vec2(10., 10.),
                            egui::Align2::LEFT_TOP,
                            format!("{:.1}°", d[1].atan2(d[0]).to_degrees()),
                            font.clone(),
                            accent,
                        );
                    }
                }
                Mode::Moving(i) => {
                    let neighbours = [
                        (i > 0 || self.closed).then(|| points[(i + n - 1) % n]),
                        (i + 1 < n || self.closed).then(|| points[(i + 1) % n]),
                    ];
                    for q in neighbours.into_iter().flatten() {
                        view.painter.extend(egui::Shape::dashed_line(
                            &[to_screen(q), to_screen(p)],
                            dashed,
                            6.,
                            4.,
                        ));
                        label(q, p, accent);
                    }
                }
                Mode::Idle => {}
            }
            if self.mode != Mode::Idle {
                let s = to_screen(p);
                view.painter
                    .circle_stroke(s, 5., egui::Stroke::new(1.5, accent));
                let kind = hover.kind.label();
                view.painter.text(
                    s + egui::vec2(10., -10.),
                    egui::Align2::LEFT_BOTTOM,
                    if kind.is_empty() {
                        format!(
                            "{}, {}",
                            format_number(p[0].round()),
                            format_number(p[1].round())
                        )
                    } else {
                        format!(
                            "{}, {} · {kind}",
                            format_number(p[0].round()),
                            format_number(p[1].round())
                        )
                    },
                    font.clone(),
                    accent,
                );
            }
        }
        for (i, p) in points.iter().enumerate() {
            let s = to_screen(*p);
            let selected = matches!(self.selected, Some(Selection::Corner(k)) if k == i)
                || self.mode == Mode::Moving(i);
            let rect = egui::Rect::from_center_size(s, egui::vec2(8., 8.));
            if selected {
                view.painter.rect_filled(rect, 1., accent);
            } else {
                view.painter
                    .rect_filled(rect, 1., egui::Color32::from_gray(30));
                view.painter.rect_stroke(
                    rect,
                    1.,
                    egui::Stroke::new(1.5, line),
                    egui::StrokeKind::Middle,
                );
            }
            if self.mode == Mode::Drawing && i == 0 && n >= 3 {
                view.painter
                    .circle_stroke(s, 9., egui::Stroke::new(1.5, accent));
            }
            view.painter.text(
                s + egui::vec2(-8., -8.),
                egui::Align2::RIGHT_BOTTOM,
                format!("{}", i + 1),
                font.clone(),
                if selected { accent } else { line },
            );
        }
        for issue in &issues {
            if let Some(at) = issue.at {
                let s = to_screen(at);
                let stroke = egui::Stroke::new(2., bad);
                view.painter.circle_stroke(s, 8., stroke);
                view.painter
                    .line_segment([s + egui::vec2(-5., -5.), s + egui::vec2(5., 5.)], stroke);
                view.painter
                    .line_segment([s + egui::vec2(-5., 5.), s + egui::vec2(5., -5.)], stroke);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;

    const ROOM: [[f64; 2]; 4] = [[0., 0.], [4000., 0.], [4000., 3000.], [0., 3000.]];

    fn doc() -> Document {
        Document::new("Outline").unwrap()
    }

    #[test]
    fn clicks_with_grid_and_orthogonal_snapping_draw_and_close_the_4_by_3_room() {
        let document = doc();
        let mut editor =
            OutlineEditor::from_mm(OutlineKind::WallPath, &[[0., 0.], [4000., 0.]], false);
        editor.start_drawing();
        assert!(editor.capturing() && editor.len() == 0);
        // Slightly imprecise clicks snap to the grid and to orthogonal lines.
        for raw in [[3., -4.], [3996., 12.], [4008., 2997.], [-4., 3003.]] {
            let points = editor.positions(&document).unwrap();
            assert!(editor.click(raw, &points, &[], 20.));
        }
        // A click near the first corner closes the room.
        let points = editor.positions(&document).unwrap();
        assert_eq!(
            editor.snap([8., 9.], &points, &[], 20.).kind,
            SnapKind::Close
        );
        assert!(editor.click([8., 9.], &points, &[], 20.));
        assert!(!editor.capturing() && editor.closed());
        assert_eq!(editor.positions(&document).unwrap(), ROOM.to_vec());
        assert!(editor.issues(&document).is_empty());
        let lengths = editor.segment_lengths(&editor.positions(&document).unwrap());
        assert_eq!(lengths, vec![4000., 3000., 4000., 3000.]);
    }

    #[test]
    fn rectangle_dimensions_and_typed_segments_build_exact_outlines() {
        let document = doc();
        let mut editor = OutlineEditor::from_mm(OutlineKind::SlabOutline, &[], true);
        assert!(editor.rectangle([0., 0.], 0., 3000.).is_err());
        editor.rectangle([0., 0.], 4000., 3000.).unwrap();
        assert_eq!(editor.positions(&document).unwrap(), ROOM.to_vec());
        assert!(editor.closed());

        let mut path = OutlineEditor::from_mm(OutlineKind::WallPath, &[], false);
        path.start_drawing();
        assert!(
            path.add_by_length(4000., 0., &[]).is_err(),
            "needs a first corner"
        );
        assert!(path.click([0., 0.], &[], &[], 10.));
        for angle in [0., 90., 180.] {
            let points = path.positions(&document).unwrap();
            let l = if angle == 90. { 3000. } else { 4000. };
            path.add_by_length(l, angle, &points).unwrap();
        }
        path.finish();
        let points = path.positions(&document).unwrap();
        for (p, q) in points.iter().zip(ROOM) {
            assert!(dist(*p, q) < 1e-6, "{p:?} vs {q:?}");
        }
        assert!(!path.closed(), "a wall path stays open until closed");
    }

    #[test]
    fn cancel_restores_the_outline_before_drawing_or_moving() {
        let document = doc();
        let mut editor = OutlineEditor::from_mm(OutlineKind::WallPath, &ROOM, true);
        editor.start_drawing();
        assert!(editor.click([100., 100.], &[], &[], 10.));
        assert!(editor.cancel());
        assert_eq!(editor.positions(&document).unwrap(), ROOM.to_vec());
        assert!(editor.closed());

        // Pick corner 3, preview a move, then cancel.
        let points = editor.positions(&document).unwrap();
        assert!(!editor.click([3995., 3004.], &points, &[], 10.));
        assert_eq!(editor.selected(), Some(Selection::Corner(2)));
        assert!(editor.capturing());
        assert!(editor.cancel(), "the snapshot is restored");
        assert!(!editor.capturing());
        assert_eq!(editor.positions(&document).unwrap(), ROOM.to_vec());
    }

    #[test]
    fn corners_and_segments_are_selected_and_adjusted() {
        let document = doc();
        let mut editor = OutlineEditor::from_mm(OutlineKind::WallPath, &ROOM, true);
        let points = editor.positions(&document).unwrap();
        // Click corner 3, then its new place: 5000, 3000 (orthogonal to corner 2? no:
        // the reference is corner 2 at 4000, 0, so the free click snaps to the grid).
        assert!(!editor.click([4001., 2999.], &points, &[], 10.));
        assert!(editor.click([5003., 3502.], &points, &[], 10.));
        let moved = editor.positions(&document).unwrap();
        assert_eq!(moved[2], [5000., 3500.]);
        assert_eq!(editor.selected(), Some(Selection::Corner(2)));
        // The inspector fields show the synchronized literal.
        assert_eq!(editor.corners[2][0].text, "5000");
        assert_eq!(editor.corners[2][1].text, "3500");

        // A click on segment 1 selects it; its length can then be typed.
        let mut editor = OutlineEditor::from_mm(OutlineKind::WallPath, &ROOM, true);
        let points = editor.positions(&document).unwrap();
        assert!(!editor.click([2000., 4.], &points, &[], 10.));
        assert_eq!(editor.selected(), Some(Selection::Segment(0)));
        assert_eq!(editor.segment_length.text, "4000");
        editor.set_segment_length(0, 4500., &points).unwrap();
        let points = editor.positions(&document).unwrap();
        assert_eq!(points[1], [4500., 0.]);
        editor.insert_midpoint(0, &points);
        assert_eq!(editor.positions(&document).unwrap()[1], [2250., 0.]);
        editor.remove_corner(1);
        assert_eq!(editor.len(), 4);
        // A click far from everything clears the selection.
        let points = editor.positions(&document).unwrap();
        editor.click([2000., 1500.], &points, &[], 10.);
        assert_eq!(editor.selected(), None);
    }

    #[test]
    fn self_crossing_zero_length_and_short_outlines_have_located_messages() {
        let bowtie = [[0., 0.], [4000., 3000.], [4000., 0.], [0., 3000.]];
        let issues = diagnose(&bowtie, true, OutlineKind::SlabOutline);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(
            issues[0]
                .message
                .contains("Segments 1 and 3 cross at (2000, 1500)")
        );
        assert_eq!(issues[0].at, Some([2000., 1500.]));
        let mut editor = OutlineEditor::from_mm(OutlineKind::SlabOutline, &bowtie, true);
        assert!(!editor.issues(&doc()).is_empty());
        editor
            .paste_points("0, 0\n4000, 0\n4000, 3000\n0, 3000")
            .unwrap();
        assert!(editor.issues(&doc()).is_empty());
        assert!(
            editor
                .paste_points("0, 0\nfour, 0")
                .unwrap_err()
                .contains("Line 2")
        );

        let doubled = [[0., 0.], [4000., 0.], [4000., 0.], [4000., 3000.]];
        let issues = diagnose(&doubled, false, OutlineKind::WallPath);
        assert!(issues[0].message.contains("Segment 2 has no length"));
        let back = [[0., 0.], [4000., 0.], [2000., 0.]];
        assert!(
            diagnose(&back, false, OutlineKind::WallPath)[0]
                .message
                .contains("doubles back")
        );
        let short = diagnose(&[[0., 0.], [1., 0.]], true, OutlineKind::SlabOutline);
        assert!(short[0].message.contains("at least 3 corners"));
        assert!(diagnose(&ROOM, true, OutlineKind::SlabOutline).is_empty());
    }

    #[test]
    fn corner_targets_win_over_grid_and_expressions_survive_untouched() {
        let mut document = doc();
        document
            .insert(multicad_core::document::Entity::new(
                "room_w",
                multicad_core::document::EntityData::Parameter {
                    expression: "4000".into(),
                    unit: "mm".into(),
                    exposed: true,
                    inputs: vec![],
                },
            ))
            .unwrap();
        let mut editor = OutlineEditor::from_mm(OutlineKind::WallPath, &[], false);
        editor.start_drawing();
        let s = editor.snap([3893., 97.], &[], &[[3900., 100.]], 20.);
        assert_eq!(
            s,
            Snapped {
                point: [3900., 100.],
                kind: SnapKind::Corner
            }
        );
        editor.finish();

        let mut editor =
            OutlineEditor::from_mm(OutlineKind::WallPath, &[[0., 0.], [1., 0.]], false);
        editor.corners[1][0].text = "=$room_w".into();
        let dims = editor.dimensions(&document).unwrap();
        assert!(matches!(dims[1][0], Dimension::Expression { .. }));
        assert_eq!(editor.positions(&document).unwrap()[1], [4000., 0.]);
        let mut again = OutlineEditor::new(OutlineKind::WallPath, &dims, false).unwrap();
        assert_eq!(
            again.dimensions(&document).unwrap(),
            dims,
            "expressions round-trip"
        );
        // Moving the other corner keeps the expression on corner 2.
        let points = again.positions(&document).unwrap();
        again.click([0., 0.], &points, &[], 10.);
        again.click([0., 2000.], &points, &[], 10.);
        assert!(matches!(
            again.dimensions(&document).unwrap()[1][0],
            Dimension::Expression { .. }
        ));
    }

    #[test]
    fn viewport_clicks_from_a_plan_camera_draw_through_the_real_hit_path() {
        let document = doc();
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.));
        let camera = Camera {
            yaw: 0.,
            pitch: 1.5,
            scale: 0.1,
            target: [2000., 1500., 0.],
            perspective: false,
        };
        let mut editor = OutlineEditor::from_mm(OutlineKind::SlabOutline, &[], true);
        editor.start_drawing();
        let mut clicks: Vec<[f64; 2]> = ROOM.to_vec();
        clicks.push([0., 0.]);
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter().clone();
            for p in &clicks {
                let s = camera.project([p[0] as f32, p[1] as f32, 300.], rect);
                let view = TerminationViewport {
                    scene: None,
                    report: None,
                    camera,
                    rect,
                    pointer: Some(egui::pos2(s[0], s[1])),
                    clicked: true,
                    painter: &painter,
                    section: None,
                };
                assert!(editor.viewport(&document, &view, 300., &[]));
            }
            // The Inspector section renders the drawn corners without edits.
            assert!(!editor.show(ui, &document, "the floor"));
        })
        .drop_without_applying_deltas();
        assert!(!editor.capturing() && editor.closed());
        assert_eq!(editor.positions(&document).unwrap(), ROOM.to_vec());
    }
}

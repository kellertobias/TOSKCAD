//! Choosing flange edges visually: the numbered base outline in the 3D
//! viewer and in the Inspector, flange guides with their bend and leg
//! dimensions, and diagnostics on the edges they concern. Every click edits
//! the same Inspector rows, so the viewer and the Inspector produce the same
//! checked sheet transaction.
use super::SheetPanel;
use crate::{egui, modeling::TerminationViewport};
use multicad_core::{
    document::Document,
    model::{BendDirection, FlangeGuide, FlangeReview, SheetOutline, ValueSource},
};

/// Screen distance within which the pointer picks an edge or handle.
const PICK_PX: f32 = 9.;

pub(super) const EDGE_TEXT: [&str; 4] = [
    "Edge 1 · along length at v = 0",
    "Edge 2 · along width at u = length",
    "Edge 3 · along length at v = width",
    "Edge 4 · along width at u = 0",
];

const PLAIN: egui::Color32 = egui::Color32::from_rgb(140, 148, 160);
const FLANGED: egui::Color32 = egui::Color32::from_rgb(90, 170, 255);
const HOVER: egui::Color32 = egui::Color32::from_rgb(255, 176, 64);
const SELECTED: egui::Color32 = egui::Color32::from_rgb(255, 230, 120);
const PROBLEM: egui::Color32 = egui::Color32::from_rgb(255, 96, 96);

/// What the pointer is over in the viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Edge(usize),
    /// The direction handle at the leg end of the selected flange.
    Direction(usize),
}

pub(super) fn source(s: ValueSource) -> &'static str {
    match s {
        ValueSource::Stock => "stock",
        ValueSource::Part => "part",
    }
}
fn direction(d: BendDirection) -> &'static str {
    match d {
        BendDirection::Up => "up",
        BendDirection::Down => "down",
    }
}
/// The bend label of a guide, from the resolved values the evaluator uses.
pub(super) fn bend_text(g: &FlangeGuide) -> String {
    let angle = g.bends.first().map_or(0., |b| b.1.abs());
    match g.kind {
        "hem" => format!(
            "Hem {} · gap {} mm",
            direction(g.direction),
            fmt(2. * g.inside_radius_mm)
        ),
        "jog" => format!(
            "Jog {}° {} · R {} mm",
            fmt(angle),
            direction(g.direction),
            fmt(g.inside_radius_mm)
        ),
        _ => format!(
            "{}° {} · R {} mm",
            fmt(angle),
            direction(g.direction),
            fmt(g.inside_radius_mm)
        ),
    }
}
pub(super) fn leg_text(g: &FlangeGuide) -> String {
    format!("Leg {} mm", fmt(g.leg_mm))
}
fn fmt(v: f64) -> String {
    let r = (v * 1000.).round() / 1000.;
    if r == r.trunc() {
        format!("{r:.0}")
    } else {
        format!("{r}")
    }
}

fn distance_to_segment(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0., 1.);
    (a + ab * t).distance(p)
}

impl SheetPanel {
    fn edge_color(
        &self,
        edge: usize,
        review: &FlangeReview,
        hover: Option<Target>,
    ) -> egui::Color32 {
        let on = self
            .rows
            .as_ref()
            .is_some_and(|r| r.get(edge).is_some_and(|r| r.on));
        if review.issues.iter().any(|i| i.edges.contains(&edge)) && on {
            PROBLEM
        } else if self.selected == Some(edge) {
            SELECTED
        } else if hover == Some(Target::Edge(edge)) {
            HOVER
        } else if on {
            FLANGED
        } else {
            PLAIN
        }
    }

    /// The pointer target among projected edges and the selected flange's
    /// direction handle; the handle wins over an edge.
    pub(super) fn target(
        &self,
        review: &FlangeReview,
        project: &dyn Fn([f64; 3]) -> Option<egui::Pos2>,
        pointer: egui::Pos2,
    ) -> Option<Target> {
        let outline = review.outline.as_ref()?;
        if let Some(edge) = self.selected
            && let Some(g) = review.guides.iter().find(|g| g.edge == edge)
            && let Some(p) = project(g.world(outline, g.leg[1], g.width_mm() / 2.))
            && p.distance(pointer) <= PICK_PX * 1.5
        {
            return Some(Target::Direction(edge));
        }
        outline
            .edges
            .iter()
            .filter(|e| e.straight && e.index < 4)
            .filter_map(|e| {
                let a = project(e.start_mm)?;
                let b = project(e.end_mm)?;
                let d = distance_to_segment(pointer, a, b);
                (d <= PICK_PX).then_some((d, e.index))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, e)| Target::Edge(e))
    }

    /// Choose an edge: select its row and turn its flange on. Returns
    /// whether the sheet definition changed.
    pub(super) fn pick_edge(&mut self, edge: usize) -> bool {
        let Some(row) = self.rows.as_mut().and_then(|r| r.get_mut(edge)) else {
            return false;
        };
        self.selected = Some(edge);
        if row.on {
            return false;
        }
        row.on = true;
        self.edited();
        true
    }
    /// Flip the selected flange between up and down.
    pub(super) fn flip_direction(&mut self, edge: usize) -> bool {
        let Some(row) = self
            .rows
            .as_mut()
            .and_then(|r| r.get_mut(edge))
            .filter(|r| r.on)
        else {
            return false;
        };
        row.down = !row.down;
        self.edited();
        true
    }

    /// Viewer interaction while the form is open: hover and click edges,
    /// flip the selected flange with its handle, and draw the outline,
    /// guides, dimensions and diagnostics. Returns whether the sheet
    /// definition changed.
    pub fn viewport(&mut self, document: &Document, view: TerminationViewport<'_>) -> bool {
        if self.rows.is_none() {
            return false;
        }
        let Some(review) = self.review(document) else {
            self.hover = None;
            return false;
        };
        let project = |p: [f64; 3]| {
            let s = view.camera.project(p.map(|v| v as f32), view.rect);
            (s[2] >= 0.1).then(|| egui::pos2(s[0], s[1]))
        };
        let hover = view
            .pointer
            .filter(|p| view.rect.contains(*p))
            .and_then(|p| self.target(&review, &project, p));
        self.hover = hover;
        let changed = match hover {
            Some(Target::Edge(edge)) if view.clicked => self.pick_edge(edge),
            Some(Target::Direction(edge)) if view.clicked => self.flip_direction(edge),
            _ => false,
        };
        // Paint the state after the click so the overlay never lags.
        let review = if changed {
            self.review(document).unwrap_or(review)
        } else {
            review
        };
        self.paint(&review, &view, &project);
        changed
    }

    fn paint(
        &self,
        review: &FlangeReview,
        view: &TerminationViewport<'_>,
        project: &dyn Fn([f64; 3]) -> Option<egui::Pos2>,
    ) {
        let Some(outline) = &review.outline else {
            return;
        };
        let painter = view.painter;
        let font = egui::FontId::proportional(12.);
        let text = |at: egui::Pos2, s: String, color: egui::Color32| {
            let galley = painter.layout_no_wrap(s, font.clone(), color);
            let r = egui::Align2::CENTER_CENTER.anchor_size(at, galley.size());
            painter.rect_filled(r.expand(3.), 3., egui::Color32::from_black_alpha(170));
            painter.galley(r.min, galley, color);
        };
        for g in &review.guides {
            let color = self.edge_color(g.edge, review, self.hover);
            self.paint_guide(g, outline, color, project, painter);
        }
        for e in outline.edges.iter().filter(|e| e.index < 4) {
            let (Some(a), Some(b)) = (project(e.start_mm), project(e.end_mm)) else {
                continue;
            };
            let color = self.edge_color(e.index, review, self.hover);
            let selected =
                self.selected == Some(e.index) || self.hover == Some(Target::Edge(e.index));
            painter.line_segment(
                [a, b],
                egui::Stroke::new(if selected { 4. } else { 2.5 }, color),
            );
            // The number sits just outside the edge, away from the base.
            let mid = e.midpoint_mm();
            let out = std::array::from_fn(|i| mid[i] + e.outward[i] * e.length_mm.max(1.) * 0.02);
            if let (Some(m), Some(o)) = (project(mid), project(out)) {
                let d = (o - m).normalized();
                let d = if d.is_finite() {
                    d
                } else {
                    egui::vec2(0., -1.)
                };
                text(m - d * 16., format!("{}", e.index + 1), color);
            }
        }
        for g in &review.guides {
            if self.selected != Some(g.edge) && self.hover != Some(Target::Edge(g.edge)) {
                continue;
            }
            let half = g.width_mm() / 2.;
            let leg_mid = [
                (g.leg[0][0] + g.leg[1][0]) / 2.,
                (g.leg[0][1] + g.leg[1][1]) / 2.,
            ];
            if let Some(p) = project(g.world(outline, leg_mid, half)) {
                text(p, leg_text(g), SELECTED);
            }
            if let Some((b, _)) = g.bends.first()
                && let Some(p) = project(g.world(outline, *b, half))
            {
                text(p + egui::vec2(0., -18.), bend_text(g), SELECTED);
            }
            if self.selected == Some(g.edge)
                && let Some(p) = project(g.world(outline, g.leg[1], half))
            {
                let hot = self.hover == Some(Target::Direction(g.edge));
                painter.circle(
                    p,
                    7.,
                    if hot {
                        HOVER
                    } else {
                        egui::Color32::from_black_alpha(200)
                    },
                    egui::Stroke::new(2., SELECTED),
                );
                text(p + egui::vec2(0., -20.), "⇅ flip up/down".into(), SELECTED);
            }
        }
        let mut caption = format!(
            "Sheet edges · t {} mm (stock) · R {} mm ({}) · K {} ({}) · click an edge to add or select its flange",
            fmt(review.thickness_mm),
            fmt(review.inside_radius_mm),
            source(review.radius_source),
            fmt(review.k_factor),
            source(review.k_factor_source),
        );
        if !review.issues.is_empty() {
            caption.push_str(&format!(" · {} flange problem(s)", review.issues.len()));
        }
        let at = view.rect.left_bottom() + egui::vec2(10., -12.);
        let galley = painter.layout_no_wrap(caption, egui::FontId::proportional(11.), SELECTED);
        let r = egui::Align2::LEFT_BOTTOM.anchor_size(at, galley.size());
        painter.rect_filled(r.expand(4.), 3., egui::Color32::from_black_alpha(170));
        painter.galley(r.min, galley, SELECTED);
    }

    fn paint_guide(
        &self,
        g: &FlangeGuide,
        outline: &SheetOutline,
        color: egui::Color32,
        project: &dyn Fn([f64; 3]) -> Option<egui::Pos2>,
        painter: &egui::Painter,
    ) {
        let width = g.width_mm();
        let stroke = egui::Stroke::new(1.5, color);
        for along in [0., width] {
            let points: Vec<_> = g
                .section
                .iter()
                .filter_map(|p| project(g.world(outline, *p, along)))
                .collect();
            if points.len() == g.section.len() {
                painter.add(egui::Shape::line(points, stroke));
            }
        }
        for p in [g.leg[0], g.leg[1]] {
            if let (Some(a), Some(b)) = (
                project(g.world(outline, p, 0.)),
                project(g.world(outline, p, width)),
            ) {
                painter.line_segment([a, b], stroke);
            }
        }
    }

    /// The numbered rectangle diagram in the Inspector. Returns the edge
    /// clicked, if any.
    pub(super) fn diagram(
        &mut self,
        ui: &mut egui::Ui,
        review: Option<&FlangeReview>,
    ) -> Option<usize> {
        let size = egui::vec2(ui.available_width().max(120.), 112.);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4., egui::Color32::from_gray(28));
        let (l, w) = review
            .and_then(|r| r.outline.as_ref())
            .filter(|o| o.edges.len() == 4)
            .map_or((2., 1.), |o| (o.edges[0].length_mm, o.edges[1].length_mm));
        let room = rect.shrink(26.);
        let s = (room.width() / l as f32).min(room.height() / w as f32);
        let box_ =
            egui::Rect::from_center_size(room.center(), egui::vec2(l as f32 * s, w as f32 * s));
        // Screen y grows downward, so v = 0 is the bottom edge.
        let corners = [
            box_.left_bottom(),
            box_.right_bottom(),
            box_.right_top(),
            box_.left_top(),
        ];
        let outward = [
            egui::vec2(0., 1.),
            egui::vec2(1., 0.),
            egui::vec2(0., -1.),
            egui::vec2(-1., 0.),
        ];
        let pointer = response.hover_pos();
        let mut hovered = None;
        for e in 0..4 {
            let (a, b) = (corners[e], corners[(e + 1) % 4]);
            if pointer.is_some_and(|p| distance_to_segment(p, a, b) <= PICK_PX) {
                hovered = Some(e);
            }
        }
        let empty = FlangeReview {
            thickness_mm: 0.,
            inside_radius_mm: 0.,
            radius_source: ValueSource::Stock,
            k_factor: 0.,
            k_factor_source: ValueSource::Stock,
            outline: None,
            guides: vec![],
            issues: vec![],
        };
        let review = review.unwrap_or(&empty);
        for e in 0..4 {
            let (a, b) = (corners[e], corners[(e + 1) % 4]);
            let color = self.edge_color(e, review, hovered.map(Target::Edge));
            let on = self.rows.as_ref().is_some_and(|r| r[e].on);
            painter.line_segment(
                [a, b],
                egui::Stroke::new(
                    if self.selected == Some(e) || hovered == Some(e) {
                        4.
                    } else {
                        2.5
                    },
                    color,
                ),
            );
            let mid = a + (b - a) / 2.;
            if on {
                // A short flange stub outside the edge.
                let o = outward[e] * 9.;
                painter.line_segment([a + o, b + o], egui::Stroke::new(1.5, color));
            }
            painter.text(
                mid + outward[e] * 18.,
                egui::Align2::CENTER_CENTER,
                format!("{}", e + 1),
                egui::FontId::proportional(13.),
                color,
            );
        }
        painter.text(
            box_.center(),
            egui::Align2::CENTER_CENTER,
            "base face · u →",
            egui::FontId::proportional(10.),
            egui::Color32::from_gray(150),
        );
        response
            .on_hover_text("Click an edge to add or select its flange")
            .clicked()
            .then_some(hovered)
            .flatten()
    }
}

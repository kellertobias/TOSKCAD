use crate::egui;
use multicad_core::{
    document::EntityId,
    drawings::{self, DrawingEvaluation, Page, TextCell, VectorRole, ViewEvaluation},
};
/// What a click on the paper did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct CanvasEvent {
    pub view: Option<EntityId>,
    /// While picking a section line: the clicked paper point, in mm.
    pub pick_mm: Option<[f64; 2]>,
}
/// `picking` is `Some(first point)` while a section line is being picked.
pub(super) fn show(
    ui: &mut egui::Ui,
    page: &Page,
    index: usize,
    total: usize,
    evaluation: Option<&DrawingEvaluation>,
    selected: Option<EntityId>,
    picking: Option<Option<[f64; 2]>>,
) -> CanvasEvent {
    let layout = match drawings::layout_page(page, index, total) {
        Ok(layout) => layout,
        Err(error) => {
            ui.colored_label(egui::Color32::LIGHT_RED, error.to_string());
            return CanvasEvent::default();
        }
    };
    let available = ui.available_rect_before_wrap();
    let scale = ((available.width() - 32.) / layout.paper_mm[0] as f32)
        .min((available.height() - 32.) / layout.paper_mm[1] as f32)
        .max(0.05);
    let size = egui::vec2(layout.paper_mm[0] as f32, layout.paper_mm[1] as f32) * scale;
    let paper = egui::Rect::from_center_size(available.center(), size);
    let painter = ui.painter().with_clip_rect(available);
    painter.rect_filled(paper, 0, egui::Color32::from_gray(250));
    let rect = |a: [f64; 4]| {
        egui::Rect::from_min_max(
            paper.min + egui::vec2(a[0] as f32, a[1] as f32) * scale,
            paper.min + egui::vec2(a[2] as f32, a[3] as f32) * scale,
        )
    };
    painter.rect_stroke(
        rect(layout.printable_mm),
        0,
        egui::Stroke::new(0.8, egui::Color32::from_gray(170)),
        egui::StrokeKind::Inside,
    );
    let text = |cell: &TextCell, title: bool| {
        let r = rect(cell.rect_mm);
        if title {
            painter.rect_stroke(
                r,
                0,
                egui::Stroke::new(0.7, egui::Color32::BLACK),
                egui::StrokeKind::Inside,
            );
        }
        let mut y = r.top() + scale;
        if !cell.label.is_empty() {
            painter.text(
                egui::pos2(r.left() + scale, y),
                egui::Align2::LEFT_TOP,
                &cell.label,
                egui::FontId::monospace(1.7 * scale),
                egui::Color32::from_gray(90),
            );
            y += 3. * scale;
        }
        for line in &cell.lines {
            painter.text(
                egui::pos2(r.left() + scale, y),
                egui::Align2::LEFT_TOP,
                line,
                egui::FontId::monospace(drawings::FONT_MM as f32 * scale),
                egui::Color32::BLACK,
            );
            y += drawings::LINE_MM as f32 * scale;
        }
    };
    for cell in &layout.title_cells {
        text(cell, true);
    }
    for cell in &layout.notes {
        text(cell, false);
    }
    let mut clicked = None;
    let view_painter = painter.with_clip_rect(paper);
    for view in &page.views {
        let to_screen = |point| {
            let p = view.paper_point(point);
            paper.min + egui::vec2(p[0] as f32, p[1] as f32) * scale
        };
        let anchor =
            paper.min + egui::vec2(view.position_mm[0] as f32, view.position_mm[1] as f32) * scale;
        match evaluation.and_then(|e| e.views.get(&view.id)) {
            Some(ViewEvaluation::Ready { vectors, .. }) => {
                let mut bounds = egui::Rect::NOTHING;
                for vector in vectors {
                    let points: Vec<_> = vector.preview_mm.iter().map(|p| to_screen(*p)).collect();
                    for point in &points {
                        bounds.extend_with(*point);
                    }
                    let color = if vector.role == VectorRole::Hidden {
                        egui::Color32::from_gray(115)
                    } else {
                        egui::Color32::BLACK
                    };
                    let stroke =
                        egui::Stroke::new((vector.line_width_mm as f32 * scale).max(0.6), color);
                    if vector.role == VectorRole::Hidden {
                        view_painter.extend(egui::Shape::dashed_line(
                            &points,
                            stroke,
                            3. * scale,
                            2. * scale,
                        ));
                    } else {
                        view_painter.add(egui::Shape::line(points, stroke));
                    }
                }
                if bounds.is_finite() && bounds.intersects(paper) {
                    let response = ui
                        .interact(
                            bounds.expand(5.).intersect(paper),
                            ui.id().with(view.id),
                            egui::Sense::click(),
                        )
                        .on_hover_text(format!(
                            "{} · {}:{} · click to edit",
                            view.name, view.scale.numerator, view.scale.denominator
                        ));
                    if response.clicked() {
                        clicked = Some(view.id);
                    }
                    if selected == Some(view.id) {
                        view_painter.rect_stroke(
                            bounds.expand(4.),
                            0,
                            egui::Stroke::new(1., egui::Color32::from_rgb(30, 110, 180)),
                            egui::StrokeKind::Outside,
                        );
                    }
                }
            }
            Some(ViewEvaluation::Unresolved { message }) => {
                view_painter.text(
                    anchor,
                    egui::Align2::CENTER_CENTER,
                    format!("{}\nUnresolved: {}", view.name, message),
                    egui::FontId::proportional(11.),
                    egui::Color32::DARK_RED,
                );
            }
            None => {
                view_painter.text(
                    anchor,
                    egui::Align2::CENTER_CENTER,
                    format!("{}\nNative preview pending", view.name),
                    egui::FontId::proportional(11.),
                    egui::Color32::from_gray(100),
                );
            }
        }
    }
    section_guides(&view_painter, page, evaluation, paper.min, scale);
    let mut event = CanvasEvent {
        view: clicked,
        pick_mm: None,
    };
    if let Some(first) = picking {
        let response = ui
            .interact(paper, ui.id().with("section-pick"), egui::Sense::click())
            .on_hover_text("Click the start, then the end of the section line");
        let to_paper = |p: egui::Pos2| {
            let mm = (p - paper.min) / scale;
            [mm.x as f64, mm.y as f64]
        };
        let color = egui::Color32::from_rgb(30, 110, 180);
        if let (Some(first), Some(pointer)) = (first, response.hover_pos()) {
            let start = paper.min + egui::vec2(first[0] as f32, first[1] as f32) * scale;
            view_painter.extend(egui::Shape::dashed_line(
                &[start, pointer],
                egui::Stroke::new(1.4, color),
                5.,
                3.,
            ));
        }
        if response.clicked()
            && let Some(position) = response.interact_pointer_pos()
        {
            event = CanvasEvent {
                view: None,
                pick_mm: Some(to_paper(position)),
            };
        }
    } else {
        ui.allocate_rect(available, egui::Sense::hover())
            .on_hover_text("Drawing paper with source-linked native vector views");
    }
    event
}

/// Draw each section's cut line, and its far limit dashed, in the other views
/// of the page that see the cut plane edge-on.
fn section_guides(
    painter: &egui::Painter,
    page: &Page,
    evaluation: Option<&DrawingEvaluation>,
    origin: egui::Pos2,
    scale: f32,
) {
    let color = egui::Color32::from_rgb(30, 110, 180);
    for parent in &page.views {
        let Some(ViewEvaluation::Ready { vectors, .. }) =
            evaluation.and_then(|e| e.views.get(&parent.id))
        else {
            continue;
        };
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for p in vectors.iter().flat_map(|v| &v.preview_mm) {
            for i in 0..2 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        if !min[0].is_finite() {
            continue;
        }
        let screen = |p: [f64; 2]| {
            let p = parent.paper_point(p);
            origin + egui::vec2(p[0] as f32, p[1] as f32) * scale
        };
        for section in page.views.iter().filter(|v| v.id != parent.id) {
            let Some(guide) = drawings::section_guide(parent, section, (min, max)) else {
                continue;
            };
            let cut = guide.cut.map(screen);
            painter.line_segment(cut, egui::Stroke::new(1.4, color));
            painter.text(
                cut[1],
                egui::Align2::LEFT_BOTTOM,
                &section.name,
                egui::FontId::proportional(10.),
                color,
            );
            if let Some(far) = guide.far {
                painter.extend(egui::Shape::dashed_line(
                    &far.map(screen),
                    egui::Stroke::new(1., color),
                    4.,
                    3.,
                ));
            }
        }
    }
}

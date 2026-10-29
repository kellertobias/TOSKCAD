use crate::egui;
use multicad_core::{
    document::EntityId,
    drawings::{self, DrawingEvaluation, Page, TextCell, VectorRole, ViewEvaluation},
};
/// What a click on the paper did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct CanvasEvent {
    pub view: Option<EntityId>,
    pub annotation: Option<EntityId>,
    /// While picking a section line: the clicked paper point, in mm.
    pub pick_mm: Option<[f64; 2]>,
}
/// `picking` is `Some(first point)` while a section line is being picked.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    page: &Page,
    index: usize,
    total: usize,
    evaluation: Option<&DrawingEvaluation>,
    selected: Option<EntityId>,
    picking: Option<Option<[f64; 2]>>,
    selected_annotation: Option<EntityId>,
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
    let annotation = annotations(
        ui,
        &view_painter,
        page,
        evaluation,
        paper,
        scale,
        selected_annotation,
    );
    let mut event = CanvasEvent {
        view: clicked.filter(|_| annotation.is_none()),
        annotation,
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
                annotation: None,
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

/// Draw page annotations. Model dimensions use the current evaluation; a
/// detached or pending dimension shows a marker instead of an old value.
/// Returns the annotation clicked, if any.
fn annotations(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    page: &Page,
    evaluation: Option<&DrawingEvaluation>,
    paper: egui::Rect,
    scale: f32,
    selected: Option<EntityId>,
) -> Option<EntityId> {
    use drawings::{AnnotationEvaluation, AnnotationKind};
    let to = |p: [f64; 2]| paper.min + egui::vec2(p[0] as f32, p[1] as f32) * scale;
    let mut clicked = None;
    for annotation in &page.annotations {
        let (graphic, width, color) = match &annotation.kind {
            AnnotationKind::Dimension { view, .. } => {
                match evaluation.and_then(|e| e.annotations.get(&annotation.id)) {
                    Some(AnnotationEvaluation::Ready { graphic }) => {
                        (graphic.clone(), 0.18, egui::Color32::BLACK)
                    }
                    other => {
                        let anchor = page
                            .views
                            .iter()
                            .find(|v| v.id == *view)
                            .map_or([20., 20.], |v| v.position_mm);
                        let text = match other {
                            Some(AnnotationEvaluation::Detached { message }) => {
                                format!("Detached dimension: {message}")
                            }
                            _ => "Dimension pending".into(),
                        };
                        painter.text(
                            to([anchor[0], anchor[1] + 12.]),
                            egui::Align2::CENTER_TOP,
                            text,
                            egui::FontId::proportional(10.),
                            egui::Color32::DARK_RED,
                        );
                        continue;
                    }
                }
            }
            AnnotationKind::DraftDimension {
                a,
                b,
                offset_mm,
                precision,
            } => (
                drawings::draft_dimension(page, *a, *b, *offset_mm, *precision),
                0.18,
                egui::Color32::BLACK,
            ),
            AnnotationKind::Draft { line_width_mm, .. } => (
                drawings::sheet_graphic(annotation),
                *line_width_mm,
                egui::Color32::BLACK,
            ),
            _ => (
                drawings::sheet_graphic(annotation),
                0.18,
                egui::Color32::BLACK,
            ),
        };
        let color = if selected == Some(annotation.id) {
            egui::Color32::from_rgb(30, 110, 180)
        } else {
            color
        };
        let stroke = egui::Stroke::new((width as f32 * scale).max(0.6), color);
        for [a, b] in &graphic.lines {
            painter.line_segment([to(*a), to(*b)], stroke);
        }
        for (center, r, start, end) in &graphic.arcs {
            let steps = 48;
            let points: Vec<_> = (0..=steps)
                .map(|i| {
                    let t = start + (end - start) * i as f64 / steps as f64;
                    to([center[0] + r * t.cos(), center[1] + r * t.sin()])
                })
                .collect();
            painter.add(egui::Shape::line(points, stroke));
        }
        for (tip, dir) in &graphic.arrows {
            let length = drawings::ARROW_MM;
            let back = [tip[0] - dir[0] * length, tip[1] - dir[1] * length];
            let n = [-dir[1] * 0.45, dir[0] * 0.45];
            painter.add(egui::Shape::convex_polygon(
                vec![
                    to(*tip),
                    to([back[0] + n[0], back[1] + n[1]]),
                    to([back[0] - n[0], back[1] - n[1]]),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
        if !graphic.text.is_empty() {
            let galley = painter.layout_no_wrap(
                graphic.text.clone(),
                egui::FontId::monospace(drawings::ANNOTATION_TEXT_MM as f32 * scale),
                color,
            );
            let mut shape = egui::epaint::TextShape::new(
                to(graphic.text_mm) - egui::vec2(0., galley.size().y),
                galley,
                color,
            );
            shape.angle = (graphic.text_angle_deg as f32).to_radians();
            painter.add(shape);
        }
        let bounds =
            drawings::table_bounds(annotation).or_else(|| drawings::graphic_bounds(&graphic));
        if let Some([x0, y0, x1, y1]) = bounds {
            let rect = egui::Rect::from_two_pos(to([x0, y0]), to([x1, y1])).expand(3.);
            let response = ui
                .interact(
                    rect.intersect(paper),
                    ui.id().with(("annotation", annotation.id)),
                    egui::Sense::click(),
                )
                .on_hover_text("Click to edit this annotation");
            if response.clicked() {
                clicked = Some(annotation.id);
            }
        }
    }
    clicked
}

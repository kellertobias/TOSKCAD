use crate::egui;
use multicad_core::drawings::{self, Page, TextCell};
pub(super) fn show(ui: &mut egui::Ui, page: &Page, index: usize, total: usize) {
    let layout = match drawings::layout_page(page, index, total) {
        Ok(layout) => layout,
        Err(error) => {
            ui.colored_label(egui::Color32::LIGHT_RED, error.to_string());
            return;
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
    ui.allocate_rect(available, egui::Sense::hover())
        .on_hover_text("Paper-space drawing source. No model views have been generated.");
}

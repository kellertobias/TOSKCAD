//! Paper-space loading feedback. Placeholder frames describe view slots, never
//! guessed model extents. The moving bar is indeterminate, not a percentage.
use crate::egui;
use multicad_core::drawings::{DrawingView, Orientation, Page, PaperSize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PreviewState<'a> {
    Idle,
    Generating,
    Failed(&'a str),
}

pub(super) fn paper_label(page: &Page) -> String {
    let name = match page.settings.paper {
        PaperSize::A4 => "A4",
        PaperSize::A3 => "A3",
        PaperSize::A2 => "A2",
        PaperSize::A1 => "A1",
        PaperSize::A0 => "A0",
        PaperSize::Custom { .. } => "Custom",
    };
    let orientation = match page.settings.orientation {
        Orientation::Portrait => "Portrait",
        Orientation::Landscape => "Landscape",
    };
    let [width, height] = page.settings.dimensions();
    format!("{name} · {width} × {height} mm · {orientation}")
}

pub(super) fn marquee_rect(track: egui::Rect, time: f64) -> egui::Rect {
    let width = track.width() * 0.25;
    let phase = (time / 1.7).rem_euclid(2.) as f32;
    let position = if phase <= 1. { phase } else { 2. - phase };
    egui::Rect::from_min_size(
        egui::pos2(
            track.left() + (track.width() - width) * position,
            track.top(),
        ),
        egui::vec2(width, track.height()),
    )
}

/// Paint progress above the paper so it never covers authored annotations.
pub(super) fn status(
    ui: &egui::Ui,
    painter: &egui::Painter,
    area: egui::Rect,
    page: &Page,
    state: PreviewState<'_>,
) {
    let (title, detail, color) = match state {
        PreviewState::Idle => return,
        PreviewState::Generating => (
            "Generating drawing…",
            format!(
                "Preparing {} drawing view{}",
                page.views.len(),
                if page.views.len() == 1 { "" } else { "s" }
            ),
            ui.visuals().text_color(),
        ),
        PreviewState::Failed(error) => (
            "Drawing generation failed",
            error.chars().take(160).collect(),
            egui::Color32::LIGHT_RED,
        ),
    };
    ui.interact(
        area,
        ui.id().with("drawing-generation-status"),
        egui::Sense::hover(),
    )
    .widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("{title} {detail}"))
    });
    let width = (area.width() - 24.).max(1.);
    let title = painter.layout(title.into(), egui::FontId::proportional(17.), color, width);
    let at = egui::pos2(area.center().x - title.size().x * 0.5, area.top() + 4.);
    let bottom = at.y + title.size().y + 3.;
    painter.galley(at, title, color);
    let detail = painter.layout(detail, egui::FontId::proportional(12.), color, width);
    let at = egui::pos2(area.center().x - detail.size().x * 0.5, bottom);
    let bottom = bottom + detail.size().y + 4.;
    painter.galley(at, detail, color);
    if state == PreviewState::Generating {
        let track = egui::Rect::from_center_size(
            egui::pos2(area.center().x, bottom + 3.),
            egui::vec2(width.min(280.), 5.),
        );
        painter.rect_filled(track, 2., egui::Color32::from_gray(220));
        painter.rect_filled(
            marquee_rect(track, ui.input(|i| i.time)),
            2.,
            egui::Color32::from_rgb(30, 110, 180),
        );
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
    }
}

pub(super) fn placeholder(
    ui: &egui::Ui,
    painter: &egui::Painter,
    area: egui::Rect,
    anchor: egui::Pos2,
    view: &DrawingView,
    state: PreviewState<'_>,
) -> bool {
    if area.width() < 24. || area.height() < 24. {
        return false;
    }
    let size = egui::vec2(area.width().min(160.), area.height().min(72.));
    let center = egui::pos2(
        anchor
            .x
            .clamp(area.left() + size.x * 0.5, area.right() - size.x * 0.5),
        anchor
            .y
            .clamp(area.top() + size.y * 0.5, area.bottom() - size.y * 0.5),
    );
    let frame = egui::Rect::from_center_size(center, size);
    painter.rect_filled(frame, 0, egui::Color32::from_gray(246));
    let corners = [
        frame.left_top(),
        frame.right_top(),
        frame.right_bottom(),
        frame.left_bottom(),
        frame.left_top(),
    ];
    painter.extend(egui::Shape::dashed_line(
        &corners,
        egui::Stroke::new(1., egui::Color32::from_gray(185)),
        5.,
        4.,
    ));
    let text = format!(
        "{}\nScale {}:{}\nView placeholder",
        view.name, view.scale.numerator, view.scale.denominator
    );
    let galley = painter.layout(
        text,
        egui::FontId::proportional(12.),
        egui::Color32::from_gray(90),
        (size.x - 16.).max(1.),
    );
    painter.with_clip_rect(frame.shrink(6.)).galley(
        center - galley.size() * 0.5,
        galley,
        egui::Color32::from_gray(90),
    );
    ui.interact(
        frame,
        ui.id().with(("drawing-placeholder", view.id)),
        egui::Sense::click(),
    )
    .on_hover_text(format!(
        "{} · Scale {}:{}\n{}",
        view.name,
        view.scale.numerator,
        view.scale.denominator,
        match state {
            PreviewState::Generating => {
                "This view's geometry is being generated. The frame is a layout placeholder."
            }
            PreviewState::Failed(_) | PreviewState::Idle => {
                "Click to inspect this view. The frame is a layout placeholder."
            }
        }
    ))
    .clicked()
}

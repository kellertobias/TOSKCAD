//! Fixed-geometry tabs; interaction state changes paint, never layout.
use eframe::egui::{self, Color32, FontId, Pos2, Stroke, Vec2};
#[derive(Clone, Copy)]
pub struct TabColors {
    pub text: Color32,
    pub background: Color32,
    pub active_text: Color32,
    pub active_background: Color32,
}
pub fn tab(ui: &mut egui::Ui, label: &str, selected: bool, colors: TabColors) -> egui::Response {
    let color = if selected {
        colors.active_text
    } else {
        colors.text
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.into(), FontId::proportional(12.0), color);
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(galley.size().x.ceil() + 24.0, 30.0),
        egui::Sense::click(),
    );
    ui.painter().rect_filled(
        rect,
        4,
        if selected {
            colors.active_background
        } else {
            colors.background
        },
    );
    if response.hovered() && !selected {
        ui.painter()
            .rect_filled(rect, 4, Color32::from_white_alpha(18));
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            4,
            Stroke::new(1.0, color),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().galley(
        Pos2::new(
            rect.center().x - galley.size().x * 0.5,
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        color,
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    response.on_hover_text(label)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hover_focus_and_selection_do_not_move_tabs_or_their_neighbors() {
        let ctx = egui::Context::default();
        let mut baseline = None;
        for (hover, focus, selected) in [
            (false, false, false),
            (true, false, false),
            (true, true, false),
            (false, true, true),
        ] {
            let mut rectangles = Vec::new();
            let events = if hover {
                vec![egui::Event::PointerMoved(Pos2::new(35.0, 20.0))]
            } else {
                vec![egui::Event::PointerGone]
            };
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    Vec2::new(600.0, 100.0),
                )),
                events,
                ..Default::default()
            };
            ctx.run_ui(input, |ui| {
                ui.horizontal(|ui| {
                    for label in ["Common", "Sheet metal", "Wood"] {
                        let r = tab(
                            ui,
                            label,
                            selected,
                            TabColors {
                                text: Color32::WHITE,
                                background: Color32::BLUE,
                                active_text: Color32::BLUE,
                                active_background: Color32::WHITE,
                            },
                        );
                        if focus {
                            r.request_focus();
                        }
                        assert!(r.rect.height() >= 30.0);
                        rectangles.push(r.rect);
                    }
                });
            })
            .drop_without_applying_deltas();
            if let Some(expected) = &baseline {
                assert_eq!(&rectangles, expected);
            } else {
                baseline = Some(rectangles);
            }
        }
    }
}

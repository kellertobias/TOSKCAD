//! Fixed-geometry tabs; interaction state changes paint, never layout.
use eframe::egui::{self, Color32, FontId, Pos2, Stroke, Vec2};
/// Inspector content fits the user's resizable panel: text wraps, and the
/// default selector minimum never exceeds the current content width.
pub(crate) fn inspector_layout(ui: &mut egui::Ui) {
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
    let combo_width = ui.spacing().combo_width.min(ui.available_width().max(0.));
    ui.spacing_mut().combo_width = combo_width;
}
/// A one-line Inspector selector. egui's width is a minimum; explicit
/// truncation prevents a long selected value from growing the parent panel.
pub(crate) fn inspector_combo(ui: &egui::Ui, id: impl egui::AsIdSalt) -> egui::ComboBox {
    egui::ComboBox::from_id_salt(id)
        .width(ui.available_width().max(0.))
        .truncate()
}
/// egui's default collapsing header extends to its full unwrapped title.
/// Bound the title before handing it to the header so a disclosure cannot
/// increase the width of a user's resizable Inspector.
pub(crate) fn inspector_collapsing<R>(
    ui: &mut egui::Ui,
    id: impl egui::AsIdSalt,
    title: &str,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::collapsing_header::CollapsingResponse<R> {
    let width =
        (ui.available_width() - ui.spacing().indent - 2. * ui.spacing().button_padding.x).max(0.);
    let text = egui::WidgetText::from(title).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        width,
        egui::TextStyle::Button,
    );
    let mut response = egui::CollapsingHeader::new(text).id_salt(id).show(ui, body);
    response.header_response = response.header_response.on_hover_text(title);
    response
}
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
/// An unframed disclosure glyph sharing the command row's black surface.
pub fn history_prompt(ui: &mut egui::Ui, expanded: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(24., 28.), egui::Sense::click());
    let center = rect.center();
    let points = if expanded {
        vec![
            center + Vec2::new(-4., 2.),
            center + Vec2::new(0., -2.),
            center + Vec2::new(4., 2.),
        ]
    } else {
        vec![
            center + Vec2::new(-2., -4.),
            center + Vec2::new(2., 0.),
            center + Vec2::new(-2., 4.),
        ]
    };
    let color = if response.hovered() || response.has_focus() {
        Color32::WHITE
    } else {
        Color32::from_gray(170)
    };
    ui.painter()
        .add(egui::Shape::line(points, Stroke::new(1.5, color)));
    let label = if expanded {
        "Collapse command history"
    } else {
        "Expand command history"
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
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

//! Fixed-geometry tabs; interaction state changes paint, never layout.
use eframe::egui::{self, Color32, FontId, Pos2, Stroke, Vec2};
/// Inspector content fits the user's resizable panel: text wraps, and the
/// default selector minimum never exceeds the current content width.
pub(crate) fn inspector_layout(ui: &mut egui::Ui) {
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
    let combo_width = ui.spacing().combo_width.min(ui.available_width().max(0.));
    ui.spacing_mut().combo_width = combo_width;
}
/// Finite remaining width on this row. A wrapping horizontal layout reports
/// its whole row through available_width(), even after another control used
/// part of that row; using it as a selector minimum creates sizing feedback.
pub(crate) fn inspector_width(ui: &egui::Ui) -> f32 {
    let width = [
        ui.available_size_before_wrap().x,
        ui.available_width(),
        ui.clip_rect().width(),
    ]
    .into_iter()
    .filter(|width| width.is_finite())
    .reduce(f32::min)
    .unwrap_or(ui.spacing().text_edit_width);
    width.max(0.)
}
/// Keep a persisted resizable Inspector from consuming the entire viewport.
/// Widget contents must still fit their requested width without this limit.
pub(crate) fn inspector_panel(ui: &egui::Ui, id: impl Into<egui::Id>) -> egui::Panel {
    let available = ui.available_width();
    let maximum = if available.is_finite() {
        (available * 0.5).max(0.)
    } else {
        600.
    };
    egui::Panel::right(id)
        .default_size(250.)
        .size_range(160_f32.min(maximum)..=maximum)
        .resizable(true)
}
/// A one-line Inspector selector. egui's width is a minimum; explicit
/// truncation prevents a long selected value from growing the parent panel.
pub(crate) fn inspector_combo(ui: &egui::Ui, id: impl egui::AsIdSalt) -> egui::ComboBox {
    egui::ComboBox::from_id_salt(id)
        .width(inspector_width(ui))
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

/// Consistent centered confirmations; floating tool palettes remain independent.
pub fn modal<R>(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::ModalResponse<R> {
    egui::Modal::new(egui::Id::new(id))
        .area(egui::Modal::default_area(egui::Id::new(id)).fade_in(false))
        .backdrop_color(egui::Color32::from_black_alpha(80))
        .show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.).clamp(240., 520.));
            ui.heading(title);
            ui.add_space(8.);
            contents(ui)
        })
}

#[cfg(test)]
mod modal_tests {
    use super::*;
    #[test]
    fn centered_modal_dims_the_full_view_and_owns_background_clicks_and_escape() {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(1200., 800.));
        let point = Pos2::new(50., 50.);
        let mut background_clicks = 0;
        // Areas determine their content size before the first visible frame.
        for _ in 0..2 {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |_| {
                    modal(&ctx, "test-modal", "Unsaved project", |ui| {
                        ui.label("Keep changes?");
                        ui.button("Cancel")
                    });
                },
            )
            .drop_without_applying_deltas();
        }
        for events in [
            vec![],
            vec![],
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        ] {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let background = ui.put(
                        egui::Rect::from_min_size(Pos2::new(20., 20.), Vec2::new(100., 80.)),
                        egui::Button::new("Underlying action"),
                    );
                    background_clicks += usize::from(background.clicked());
                    let response = modal(&ctx, "test-modal", "Unsaved project", |ui| {
                        ui.label("Keep changes?");
                        ui.button("Cancel")
                    });
                    assert_eq!(response.backdrop_response.rect, screen);
                    if response.response.rect.width() > 400. {
                        assert!(
                            (response.response.rect.center() - screen.center()).length() < 1.,
                            "dialog should be centered: {:?}",
                            response.response.rect
                        );
                    }
                },
            );
            assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Rect(r) if r.rect==screen && r.fill==Color32::from_black_alpha(80))));
            output.drop_without_applying_deltas();
        }
        assert_eq!(background_clicks, 0);
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                events: vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
                ..Default::default()
            },
            |_| {
                let response = modal(&ctx, "test-modal", "Unsaved project", |ui| {
                    ui.label("Keep changes?")
                });
                assert!(response.should_close());
                assert!(
                    !ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)),
                    "Escape must not reach the active model tool"
                );
            },
        )
        .drop_without_applying_deltas();
    }
}

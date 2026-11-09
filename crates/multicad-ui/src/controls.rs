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
/// Dialog actions keep a padded left label and right shortcut column. Use the
/// native Button widget so keyboard focus and accessibility stay intact.
pub(crate) fn dialog_action(
    ui: &mut egui::Ui,
    label: &str,
    shortcut: &str,
    min_width: f32,
    enabled: bool,
    fill: Option<Color32>,
) -> egui::Response {
    let style = ui.style().clone();
    ui.spacing_mut().button_padding = Vec2::new(15., 8.);
    ui.style_mut().override_text_valign = Some(egui::Align::Center);
    let mut label = egui::RichText::new(label);
    let mut shortcut = egui::RichText::new(shortcut);
    if fill.is_some() {
        label = label.color(Color32::WHITE);
        shortcut = shortcut.color(Color32::from_white_alpha(210));
    }
    let mut button = egui::Button::new(label)
        .shortcut_text(shortcut)
        .min_size(Vec2::new(min_width, 36.));
    if let Some(fill) = fill {
        button = button.fill(fill);
    }
    let response = ui.add_enabled(enabled, |ui: &mut egui::Ui| {
        ui.add_sized(Vec2::new(min_width, 36.), button)
    });
    ui.set_style(style);
    response
}
#[cfg(test)]
mod dialog_action_tests {
    use super::*;
    use egui::Rect;
    #[test]
    fn dialog_labels_and_shortcuts_keep_symmetric_padding_and_centering_when_wrapped_or_disabled() {
        for width in [560., 320.] {
            for enabled in [true, false] {
                let ctx = egui::Context::default();
                crate::theme::configure(&ctx);
                let mut rectangles = vec![];
                for _ in 0..3 {
                    rectangles.clear();
                    let output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(
                                Pos2::ZERO,
                                Vec2::new(width, 160.),
                            )),
                            ..Default::default()
                        },
                        |ui| {
                            ui.allocate_ui_with_layout(
                                Vec2::new(ui.available_width(), 36.),
                                egui::Layout::right_to_left(egui::Align::Center)
                                    .with_main_wrap(true),
                                |ui| {
                                    for (label, hint, min_width) in [
                                        ("Save and Quit", "Enter", 195.),
                                        ("Quit", "⌘Q", 135.),
                                        ("Cancel", "Escape", 150.),
                                    ] {
                                        let response = dialog_action(
                                            ui,
                                            label,
                                            hint,
                                            min_width,
                                            enabled,
                                            (label == "Save and Quit").then_some(Color32::BLUE),
                                        );
                                        rectangles.push((label, hint, response.rect));
                                    }
                                },
                            );
                        },
                    );
                    let text_rectangles = output
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::Text(t) => Some((
                                t.galley.text().to_owned(),
                                Rect::from_min_size(t.pos, t.galley.size()),
                            )),
                            _ => None,
                        })
                        .collect::<std::collections::BTreeMap<_, _>>();
                    output.drop_without_applying_deltas();
                    for (label, hint, rect) in &rectangles {
                        let text = |value: &str| {
                            text_rectangles
                                .get(value)
                                .copied()
                                .unwrap_or_else(|| panic!("Missing {value} at width {width}: {rectangles:?}; {text_rectangles:?}"))
                        };
                        let label_rect = text(label);
                        let hint_rect = text(hint);
                        let left = label_rect.left() - rect.left();
                        let right = rect.right() - hint_rect.right();
                        assert!((12. ..=16.).contains(&left), "{label} left padding {left}");
                        assert!(
                            (12. ..=16.).contains(&right),
                            "{label} right padding {right}"
                        );
                        assert!(
                            (left - right).abs() < 1.,
                            "{label}: asymmetric {left}/{right}"
                        );
                        assert!((label_rect.center().y - rect.center().y).abs() < 1.);
                        assert!((hint_rect.center().y - rect.center().y).abs() < 1.);
                        assert!(label_rect.right() + 8. <= hint_rect.left());
                        assert!(
                            rect.left() >= 0. && rect.right() <= width,
                            "{label}: clipped at width {width}"
                        );
                    }
                }
            }
        }
    }
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
    tab_with_icon(ui, None, label, selected, colors)
}
pub fn tab_with_icon(
    ui: &mut egui::Ui,
    icon: Option<crate::icons::Icon>,
    label: &str,
    selected: bool,
    colors: TabColors,
) -> egui::Response {
    let color = if selected {
        colors.active_text
    } else {
        colors.text
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.into(), FontId::proportional(12.0), color);
    let (padding, icon_width) = if icon.is_some() { (6., 19.) } else { (12., 0.) };
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(galley.size().x.ceil() + 2. * padding + icon_width, 30.0),
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
    if let Some(icon) = icon {
        crate::icons::paint_sized(
            ui,
            icon,
            Pos2::new(rect.left() + padding + 7., rect.center().y),
            color,
            14.,
        );
    }
    ui.painter().galley(
        Pos2::new(
            rect.left() + padding + icon_width,
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

/// Reserve input before drawing background controls. A modal area's input
/// layer is effective on its following frame, so pending confirmations must
/// also disable the background on their very first frame. Modal areas create
/// their own enabled UI and remain usable.
pub fn block_modal_background(ui: &mut egui::Ui, pending: bool) {
    if pending {
        // The modal backdrop supplies the dimming. Keep the background's
        // existing opacity so disabled input does not darken it a second time.
        let opacity = ui.opacity();
        ui.disable();
        ui.set_opacity(opacity);
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

#[cfg(test)]
mod modal_first_frame_tests {
    use super::*;
    #[test]
    fn pending_modal_blocks_a_click_already_pressed_before_its_first_frame() {
        let ctx = egui::Context::default();
        let point = Pos2::new(50., 50.);
        let mut clicks = 0;
        for frame in 0..5 {
            let events = match frame {
                2 => vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
                3 => vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
                _ => vec![],
            };
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        Pos2::ZERO,
                        Vec2::new(1200., 800.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let opacity = ui.opacity();
                    block_modal_background(ui, frame >= 3);
                    assert_eq!(ui.opacity(), opacity);
                    let response = ui.put(
                        egui::Rect::from_min_size(Pos2::new(20., 20.), Vec2::new(100., 80.)),
                        egui::Button::new("Underlying action"),
                    );
                    clicks += usize::from(response.clicked());
                    if frame >= 3 {
                        modal(&ctx, "first-modal", "Confirm", |ui| {
                            assert!(
                                ui.is_sizing_pass() || ui.is_enabled(),
                                "The modal must remain usable while its background is disabled"
                            );
                            ui.button("Cancel")
                        });
                    }
                },
            )
            .drop_without_applying_deltas();
        }
        assert_eq!(
            clicks, 0,
            "The pending confirmation must own a released background click immediately"
        );
    }
}

pub(crate) fn centered_action(
    ui: &mut egui::Ui,
    label: &str,
    enabled: bool,
    fill: Color32,
) -> egui::Response {
    let label = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::TextStyle::Button.resolve(ui.style()),
        Color32::WHITE,
    );
    let size = Vec2::new(ui.available_width(), (label.size().y + 16.).max(36.));
    let response = ui.add_enabled(enabled, egui::Button::new("").fill(fill).min_size(size));
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label.text()));
    ui.painter().galley(
        response.rect.center() - label.size() * 0.5,
        label,
        if enabled {
            Color32::WHITE
        } else {
            Color32::GRAY
        },
    );
    response
}

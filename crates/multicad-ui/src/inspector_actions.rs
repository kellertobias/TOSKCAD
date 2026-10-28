//! Fixed Inspector action geometry. Tool owners still decide readiness and
//! perform preview, commit and cancellation through their existing handlers.
use crate::egui::{self, Color32, Rect, Vec2};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    Model,
    Assembly,
    Transfer,
    Targets,
    Plane,
    Tree,
    Repair,
    External,
    Constraints,
    Operation,
    Move,
    Drawing,
}
#[derive(Clone)]
struct Footer {
    owner: Owner,
    rect: Rect,
    clip: Rect,
    accent: Color32,
    root_id: egui::Id,
    root: egui::UiBuilder,
    opacity: f32,
}
fn key() -> egui::Id {
    egui::Id::new("active-inspector-footer")
}
#[derive(Clone, Copy)]
pub(crate) struct Button<'a> {
    pub label: &'a str,
    pub enabled: bool,
    pub help: &'a str,
}
impl<'a> Button<'a> {
    pub fn new(label: &'a str, enabled: bool, help: &'a str) -> Self {
        Self {
            label,
            enabled,
            help,
        }
    }
}
#[derive(Default)]
pub(crate) struct Clicks {
    pub preview: bool,
    pub apply: bool,
    pub cancel: bool,
}
/// The body scrolls independently of the reserved action area. No footer is
/// reserved for empty or read-only Inspectors. Nested callers restore context.
pub(crate) fn scope<R>(
    ui: &mut egui::Ui,
    owner: Option<Owner>,
    accent: Color32,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let available = ui.available_rect_before_wrap();
    // Capture the non-scrolling parent's UI context. Action rows must never
    // inherit a scroll body's culling, opacity, interaction ancestry or IDs.
    let mut root = egui::UiBuilder::new()
        .layer_id(ui.layer_id())
        .style(ui.style().clone())
        .accessibility_parent(ui.id())
        .layout(egui::Layout::top_down(egui::Align::Min));
    if !ui.is_enabled() {
        root = root.disabled();
    }
    if !ui.is_visible() {
        root = root.invisible();
    }
    if ui.is_sizing_pass() {
        root = root.sizing_pass();
    }
    let fixed = owner.map(|owner| Footer {
        owner,
        rect: Rect::from_min_max(
            egui::pos2(
                available.left(),
                (available.bottom() - 88.).max(available.top()),
            ),
            available.max,
        ),
        clip: ui.clip_rect(),
        accent,
        root_id: ui.id().with("inspector-fixed-actions"),
        root,
        opacity: ui.opacity(),
    });
    if let Some(footer) = &fixed {
        ui.painter()
            .rect_filled(footer.rect, 0., Color32::from_rgb(25, 33, 44));
        ui.painter().hline(
            footer.rect.x_range(),
            footer.rect.top(),
            egui::Stroke::new(1., Color32::from_rgb(64, 77, 94)),
        );
    }
    let previous = ui.ctx().data_mut(|data| {
        let old = data.get_temp::<Footer>(key());
        if let Some(footer) = &fixed {
            data.insert_temp(key(), footer.clone());
        } else {
            data.remove::<Footer>(key())
        }
        old
    });
    let rect = Rect::from_min_max(
        available.min,
        egui::pos2(
            available.right(),
            fixed
                .as_ref()
                .map_or(available.bottom(), |footer| footer.rect.top() - 4.)
                .max(available.top()),
        ),
    );
    let mut content = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("inspector-scroll-body")
            .max_rect(rect),
    );
    content.set_clip_rect(rect.intersect(ui.clip_rect()));
    let result = egui::ScrollArea::vertical()
        .id_salt("inspector-content")
        .max_height(rect.height())
        .auto_shrink([false, false])
        .show(&mut content, body)
        .inner;
    ui.ctx().data_mut(|data| {
        if let Some(previous) = previous {
            data.insert_temp(key(), previous);
        } else {
            data.remove::<Footer>(key())
        }
    });
    ui.allocate_rect(available, egui::Sense::hover());
    result
}
/// Render the current owner's action set at the footer. Other tool controls
/// cannot cover or consume that owner's buttons. Standalone forms stay inline.
pub(crate) fn row(
    ui: &mut egui::Ui,
    owner: Owner,
    preview: Option<Button<'_>>,
    apply: Button<'_>,
    cancel: Button<'_>,
) -> Clicks {
    let footer = ui.ctx().data(|data| data.get_temp::<Footer>(key()));
    if footer.as_ref().is_some_and(|f| f.owner != owner) {
        return Clicks::default();
    }
    let mut clicks = Clicks::default();
    if let Some(footer) = footer {
        let rect = footer.rect.shrink2(Vec2::new(8., 6.));
        let mut actions = egui::Ui::new(
            ui.ctx().clone(),
            footer.root_id.with(owner as u8),
            footer.root.clone().max_rect(rect),
        );
        actions.set_opacity(footer.opacity);
        actions.set_clip_rect(footer.clip.intersect(footer.rect));
        let mut slot = |id: &str, rect: Rect, spec: Button<'_>, fill, label| {
            let mut child = actions.new_child(egui::UiBuilder::new().id_salt(id).max_rect(rect));
            child.set_clip_rect(footer.clip.intersect(footer.rect));
            button(&mut child, spec, rect.size(), fill, label).clicked()
        };
        if let Some(preview) = preview {
            clicks.preview = slot(
                "preview",
                Rect::from_min_size(rect.min, egui::vec2(rect.width(), 26.)),
                preview,
                None,
                None,
            );
        }
        let width = ((rect.width() - 6.) * 0.5).max(0.);
        let top = rect.bottom() - 32.;
        clicks.apply = slot(
            "apply",
            Rect::from_min_size(egui::pos2(rect.left(), top), egui::vec2(width, 32.)),
            apply,
            Some(footer.accent),
            Some("Apply"),
        );
        clicks.cancel = slot(
            "cancel",
            Rect::from_min_size(
                egui::pos2(rect.left() + width + 6., top),
                egui::vec2(width, 32.),
            ),
            cancel,
            Some(Color32::WHITE),
            Some("Cancel"),
        );
    } else {
        ui.horizontal_wrapped(|ui| {
            if let Some(preview) = preview {
                clicks.preview = button(ui, preview, Vec2::ZERO, None, None).clicked();
            }
            clicks.apply = button(ui, apply, Vec2::ZERO, None, None).clicked();
            clicks.cancel = button(ui, cancel, Vec2::ZERO, Some(Color32::WHITE), None).clicked();
        });
    }
    clicks
}
fn button(
    ui: &mut egui::Ui,
    spec: Button<'_>,
    size: Vec2,
    fill: Option<Color32>,
    label: Option<&str>,
) -> egui::Response {
    let mut text = egui::RichText::new(label.unwrap_or(spec.label));
    if let Some(fill) = fill {
        text = text.color(if fill == Color32::WHITE {
            Color32::from_rgb(24, 33, 46)
        } else {
            Color32::WHITE
        });
    }
    let mut button = egui::Button::new(text);
    if let Some(fill) = fill {
        button = button.fill(fill);
    }
    if size != Vec2::ZERO {
        button = button.min_size(size).truncate();
    }
    let response = if size == Vec2::ZERO {
        ui.add_enabled(spec.enabled, button)
    } else {
        ui.add_enabled_ui(spec.enabled, |ui| ui.add_sized(size, button))
            .inner
    };
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_description(format!("{}: {}", spec.label, spec.help));
    });
    response
        .on_hover_text(format!("{}\n{}", spec.label, spec.help))
        .on_disabled_hover_text(spec.help)
}

#[cfg(test)]
mod tests;

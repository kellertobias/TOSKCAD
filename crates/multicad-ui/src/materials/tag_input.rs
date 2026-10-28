//! Local tag tokens stay editable until their explicit inline confirmation.
use crate::egui::{self, Color32, Stroke, Vec2};
use std::collections::BTreeSet;
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Tags {
    pub tokens: Vec<String>,
    pub text: String,
}
impl Tags {
    pub fn from_values(values: BTreeSet<String>) -> Self {
        Self {
            tokens: values.into_iter().collect(),
            text: String::new(),
        }
    }
    pub fn values(&self) -> BTreeSet<String> {
        self.tokens
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(self.text.as_str()))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    }
    fn tokenize(&mut self) -> bool {
        let Some(last) = self.text.rfind([',', '\n', '\r']) else {
            return false;
        };
        let prefix = self.text[..=last].to_owned();
        self.text = self.text[last + 1..].to_owned();
        for value in prefix
            .split([',', '\n', '\r'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if !self.tokens.iter().any(|t| t == value) {
                self.tokens.push(value.into());
            }
        }
        true
    }
}
#[derive(Default)]
pub(super) struct Actions {
    pub apply: bool,
    pub discard: bool,
}
pub(super) fn show(
    ui: &mut egui::Ui,
    tags: &mut Tags,
    editable: bool,
    can_apply: bool,
    can_discard: bool,
) -> Actions {
    let mut actions = Actions::default();
    egui::Frame::new().fill(ui.visuals().text_edit_bg_color()).stroke(Stroke::new(1.,Color32::from_rgb(78,98,122))).corner_radius(5).inner_margin(6).show(ui,|ui|{
        let width=(ui.available_width()-52.-ui.spacing().item_spacing.x).max(32.);
        ui.horizontal_top(|ui|{
            ui.add_enabled_ui(editable,|ui|{ui.allocate_ui_with_layout(Vec2::new(width,0.),egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),|ui|{
                ui.spacing_mut().item_spacing=Vec2::new(4.,4.);let mut remove=None;
                for (index,tag) in tags.tokens.iter().enumerate(){let galley=ui.painter().layout_no_wrap(tag.clone(),egui::FontId::proportional(12.),Color32::WHITE);let desired=galley.size().x+28.;let (rect,badge)=ui.allocate_exact_size(Vec2::new(desired.min(width),23.),egui::Sense::hover());badge.on_hover_text(tag);ui.painter().rect_filled(rect,10,Color32::from_rgb(45,70,96));
                    let text_rect=egui::Rect::from_min_max(rect.min+egui::vec2(7.,2.),rect.max-egui::vec2(20.,2.));ui.painter().with_clip_rect(text_rect.intersect(ui.clip_rect())).galley(egui::pos2(text_rect.left(),rect.center().y-galley.size().y/2.),galley,Color32::WHITE);
                    let close=egui::Rect::from_center_size(egui::pos2(rect.right()-10.,rect.center().y),Vec2::splat(18.));let response=ui.interact(close,ui.id().with(("remove-tag",index)),egui::Sense::click());cross(ui,close.shrink(5.),ui.visuals().text_color());response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Button,ui.is_enabled(),format!("Remove tag {tag}")));if response.on_hover_text(format!("Remove tag {tag}")).clicked(){remove=Some(index);}
                }
                if let Some(index)=remove{tags.tokens.remove(index);}
                let entry_width=ui.available_width().max(72.).min(width);let response=ui.add(egui::TextEdit::singleline(&mut tags.text).id(ui.id().with("tag-entry")).hint_text("Add a tag…").frame(egui::Frame::NONE).desired_width(entry_width).min_size(Vec2::new(32.,23.)));
                response.on_hover_text("Type a tag followed by a comma; continue typing the next tag. Confirm with the checkmark.");
                if tags.tokenize(){ui.ctx().request_repaint();}
            });});
            ui.vertical(|ui|{ui.horizontal(|ui|{actions.apply=action(ui,"Apply tags",can_apply,true);actions.discard=action(ui,"Discard tag edits",can_discard,false);});});
        });
    });
    actions
}
fn action(ui: &mut egui::Ui, label: &str, enabled: bool, check: bool) -> bool {
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(24.), egui::Sense::click());
            if response.hovered() {
                ui.painter()
                    .rect_filled(rect, 4, Color32::from_rgb(46, 62, 80));
            }
            let color = if !ui.is_enabled() {
                ui.visuals().weak_text_color()
            } else if check {
                Color32::from_rgb(129, 197, 162)
            } else {
                ui.visuals().text_color()
            };
            let r = rect.shrink(7.);
            if check {
                ui.painter().line_segment(
                    [
                        egui::pos2(r.left(), r.center().y),
                        egui::pos2(r.center().x - 1., r.bottom()),
                    ],
                    Stroke::new(1.7, color),
                );
                ui.painter().line_segment(
                    [
                        egui::pos2(r.center().x - 1., r.bottom()),
                        egui::pos2(r.right(), r.top()),
                    ],
                    Stroke::new(1.7, color),
                );
            } else {
                cross(ui, r, color);
            }
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
            });
            response.on_hover_text(label)
        })
        .inner;
    response.clicked()
}
fn cross(ui: &egui::Ui, rect: egui::Rect, color: Color32) {
    ui.painter().line_segment(
        [rect.left_top(), rect.right_bottom()],
        Stroke::new(1.5, color),
    );
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_top()],
        Stroke::new(1.5, color),
    );
}

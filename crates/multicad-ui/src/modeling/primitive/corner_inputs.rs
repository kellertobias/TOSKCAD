use crate::{camera::Camera, egui, modeling::dimension::DimensionField};
use multicad_core::{commands::Session, document::EntityId, parameters::Unit};

#[derive(Default)]
pub(super) struct CornerInputs {
    corner: Option<usize>,
    focus_first: bool,
    bounds: Option<egui::Rect>,
}

pub(super) struct CornerField<'a> {
    pub label: &'static str,
    pub value: &'a mut DimensionField,
}

#[derive(Default)]
pub(super) struct CornerEdit {
    pub changed: bool,
    pub confirm: bool,
}

impl CornerInputs {
    pub(super) fn open(&mut self, corner: usize) {
        self.corner = Some(corner);
        self.focus_first = true;
    }

    pub(super) fn active(&self) -> bool {
        self.corner.is_some()
    }

    pub(super) fn close(&mut self) {
        *self = Self::default();
    }

    pub(super) fn contains_pointer(&self, position: egui::Pos2) -> bool {
        self.bounds.is_some_and(|bounds| bounds.contains(position))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        body: EntityId,
        viewport: egui::Rect,
        camera: Camera,
        corners: [[f64; 3]; 8],
        session: &Session,
        fields: &mut [CornerField<'_>],
    ) -> CornerEdit {
        let Some(corner) = self.corner.and_then(|index| corners.get(index)) else {
            return CornerEdit::default();
        };
        let bounds = viewport.shrink(6.);
        if bounds.width() < 28. || bounds.height() < 32. || fields.is_empty() {
            self.bounds = None;
            return CornerEdit::default();
        }
        let projected = camera.project(corner.map(|value| value as f32), viewport);
        let anchor = if projected.iter().all(|value| value.is_finite())
            && (!camera.perspective || projected[2] > 0.)
        {
            egui::pos2(projected[0], projected[1])
        } else {
            viewport.center()
        };
        let id = egui::Id::new(("primitive-corner-editor", body));
        let width = (bounds.width() - 18.).min(166.);
        let max_fields_height = (bounds.height() - 44.).clamp(1., 190.);
        let mut edit = CornerEdit::default();
        let mut done = false;
        let popup = egui::Area::new(id)
            .order(egui::Order::Foreground)
            .fixed_pos(anchor + egui::vec2(12., 12.))
            .constrain_to(bounds)
            .default_size([width + 18., (max_fields_height + 44.).min(bounds.height())])
            .enabled(ui.is_enabled())
            .fade_in(false)
            .show(ui.ctx(), |ui| {
                let (selected, confirm) = if ui.is_enabled() && ui.is_visible() {
                    corner_keys(ui, id, fields, std::mem::take(&mut self.focus_first))
                } else {
                    (None, false)
                };
                egui::Frame::popup(ui.style())
                    .inner_margin(8)
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.horizontal(|ui| {
                            ui.strong("Dimensions");
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| done = ui.small_button("Done").clicked(),
                            );
                        });
                        let mut valid = true;
                        egui::ScrollArea::vertical()
                            .id_salt(id.with("fields"))
                            .auto_shrink([false, true])
                            .max_height(max_fields_height)
                            .show(ui, |ui| {
                                for (index, field) in fields.iter_mut().enumerate() {
                                    let field_id = id.with(field.label);
                                    let label = ui.label(field.label);
                                    let input = ui
                                        .add(
                                            egui::TextEdit::singleline(&mut field.value.text)
                                                .id(field_id)
                                                .desired_width(ui.available_width())
                                                .hint_text("Length or =$parameter")
                                                .lock_focus(true)
                                                .return_key(None),
                                        )
                                        .labelled_by(label.id);
                                    if selected == Some(index) {
                                        input.scroll_to_me(None);
                                    }
                                    edit.changed |= input.changed();
                                    if let Err(error) = validate(field.value, session) {
                                        valid = false;
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(error)
                                                    .small()
                                                    .color(ui.visuals().error_fg_color),
                                            )
                                            .wrap(),
                                        );
                                    }
                                }
                            });
                        if let Some(index) = selected {
                            focus_field(ui, id, &fields[index]);
                        }
                        edit.confirm = confirm && valid;
                    });
            });
        self.bounds = Some(popup.response.rect.intersect(bounds));
        if done {
            self.close();
        }
        edit
    }
}

fn corner_keys(
    ui: &mut egui::Ui,
    id: egui::Id,
    fields: &[CornerField<'_>],
    first: bool,
) -> (Option<usize>, bool) {
    let focused = ui.memory(|memory| {
        fields
            .iter()
            .position(|field| memory.has_focus(id.with(field.label)))
    });
    let Some(mut index) = (if first { Some(0) } else { focused }) else {
        return (None, false);
    };
    if first {
        focus_field(ui, id, &fields[index]);
    }
    let mut change_focus = false;
    let mut confirm = false;
    ui.input_mut(|input| {
        input.events.retain(|event| {
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            else {
                return true;
            };
            match key {
                egui::Key::Tab if !modifiers.any() || modifiers.shift_only() => {
                    let step = if modifiers.shift { fields.len() - 1 } else { 1 };
                    index = (index + step) % fields.len();
                    change_focus = true;
                    false
                }
                egui::Key::Enter if !modifiers.any() => {
                    confirm = true;
                    false
                }
                _ => true,
            }
        });
    });
    if change_focus {
        // Keep any text in this input frame on the field that owned it before
        // Tab. egui's default focus direction must not run during that edit.
        ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
    }
    (change_focus.then_some(index), confirm)
}

fn focus_field(ui: &egui::Ui, id: egui::Id, field: &CornerField<'_>) {
    let field_id = id.with(field.label);
    ui.memory_mut(|memory| memory.request_focus(field_id));
    let mut state = egui::TextEdit::load_state(ui.ctx(), field_id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(field.value.text.chars().count()),
        )));
    egui::TextEdit::store_state(ui.ctx(), field_id, state);
    ui.ctx().request_repaint();
}

fn validate(field: &DimensionField, session: &Session) -> Result<(), String> {
    let dimension = field.resolve(session.document())?;
    let value = dimension.value_in(Unit::Millimeter, Some(session.parameters()))?;
    if value > 0. {
        Ok(())
    } else {
        Err("Enter a length greater than zero".into())
    }
}

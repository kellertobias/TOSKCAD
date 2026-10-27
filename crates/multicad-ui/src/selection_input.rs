//! Click intent uses the modifiers captured on mouse-up, before a later key-up
//! in the same input frame. Frame-final keyboard state is not click state.
use crate::egui;

pub(crate) fn click_modifiers(ui: &egui::Ui, response: &egui::Response) -> egui::Modifiers {
    ui.input(|input| {
        if !input.pointer.button_clicked(egui::PointerButton::Primary) {
            return input.modifiers;
        }
        input
            .events
            .iter()
            .rev()
            .find_map(|event| match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers,
                } if response.rect.contains(*pos) => Some(*modifiers),
                _ => None,
            })
            .unwrap_or(input.modifiers)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn click_captures_release_modifiers_before_same_frame_keyboard_release() {
        let ctx = egui::Context::default();
        let pos = egui::pos2(30., 30.);
        for modifiers in [egui::Modifiers::SHIFT, egui::Modifiers::COMMAND] {
            let mut clicked = false;
            for events in [
                vec![],
                vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers,
                    },
                ],
                vec![
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers,
                    },
                    egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                ],
            ] {
                ctx.run_ui(
                    egui::RawInput {
                        focused: true,
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(200., 150.),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let response = ui.add_sized([100., 100.], egui::Button::new("Select"));
                        if response.clicked() {
                            clicked = true;
                            assert_eq!(ui.input(|input| input.modifiers), egui::Modifiers::NONE);
                            assert_eq!(click_modifiers(ui, &response), modifiers);
                        }
                    },
                )
                .drop_without_applying_deltas();
            }
            assert!(clicked);
        }
    }
    #[test]
    fn another_widgets_click_cannot_supply_modifiers_for_keyboard_activation() {
        let ctx = egui::Context::default();
        let pos = egui::pos2(160., 120.);
        ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::SHIFT,
                    },
                    egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                ],
                ..Default::default()
            },
            |ui| {
                let response = ui.add_sized([50., 20.], egui::Button::new("Keyboard"));
                assert_eq!(click_modifiers(ui, &response), egui::Modifiers::NONE);
            },
        )
        .drop_without_applying_deltas();
    }
}

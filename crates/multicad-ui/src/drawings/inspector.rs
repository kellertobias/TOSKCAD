use super::*;
use crate::controls;
use drawings::{Orientation, PaperSize, Projection};
impl DrawingEditor {
    pub(super) fn inspector(&mut self, ui: &mut egui::Ui, session: &mut Session) {
        controls::inspector_layout(ui);
        ui.heading("Drawing Inspector");
        ui.label(
            self.book
                .and_then(|id| session.document().get(id))
                .map_or("No drawing", |entity| entity.name.as_str()),
        );
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if let Some(error) = &self.feed.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if self.feed.pending() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Building this drawing page…");
            });
        }
        self.note_inspector(ui);
        self.draft_placement_inspector(ui);
        if self.selected_view.is_some() {
            self.views_inspector(ui, session);
            return;
        }
        if let Some(page) = &self.draft {
            ui.horizontal_wrapped(|ui| {
                for view in &page.views {
                    if ui
                        .button(&view.name)
                        .on_hover_text("Edit this model view")
                        .clicked()
                    {
                        self.selected_view = Some(view.id);
                    }
                }
            });
        }
        let Some(page) = &mut self.draft else {
            ui.label("Select a page or add a new page.");
            return;
        };
        ui.small("Paper dimensions and note positions use millimeters. Changes remain a draft until Apply.");
        ui.label("Page number");
        ui.text_edit_singleline(&mut page.number).on_hover_text(
            "Authored drawing number; stable page identity does not depend on this text",
        );
        controls::inspector_combo(ui, "paper-size")
            .selected_text(match page.settings.paper {
                PaperSize::A4 => "A4",
                PaperSize::A3 => "A3",
                PaperSize::A2 => "A2",
                PaperSize::A1 => "A1",
                PaperSize::A0 => "A0",
                PaperSize::Custom { .. } => "Custom",
            })
            .show_ui(ui, |ui| {
                for (size, label) in [
                    (PaperSize::A4, "A4"),
                    (PaperSize::A3, "A3"),
                    (PaperSize::A2, "A2"),
                    (PaperSize::A1, "A1"),
                    (PaperSize::A0, "A0"),
                    (
                        PaperSize::Custom {
                            width_mm: 210.,
                            height_mm: 297.,
                        },
                        "Custom",
                    ),
                ] {
                    ui.selectable_value(&mut page.settings.paper, size, label);
                }
            });
        if let PaperSize::Custom {
            width_mm,
            height_mm,
        } = &mut page.settings.paper
        {
            ui.label("Custom width × height (mm)");
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::DragValue::new(width_mm).range(50.0..=2000.));
                ui.add(egui::DragValue::new(height_mm).range(50.0..=2000.));
            });
        }
        controls::inspector_combo(ui, "orientation")
            .selected_text(match page.settings.orientation {
                Orientation::Portrait => "Portrait",
                Orientation::Landscape => "Landscape",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut page.settings.orientation,
                    Orientation::Portrait,
                    "Portrait",
                );
                ui.selectable_value(
                    &mut page.settings.orientation,
                    Orientation::Landscape,
                    "Landscape",
                );
            });
        controls::inspector_combo(ui, "projection")
            .selected_text(match page.settings.projection {
                Projection::FirstAngle => "First angle",
                Projection::ThirdAngle => "Third angle",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut page.settings.projection,
                    Projection::FirstAngle,
                    "First angle",
                );
                ui.selectable_value(
                    &mut page.settings.projection,
                    Projection::ThirdAngle,
                    "Third angle",
                );
            });
        ui.label("Default scale (paper : model)");
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::DragValue::new(&mut page.settings.scale.numerator).range(1..=100000));
            ui.label(":");
            ui.add(egui::DragValue::new(&mut page.settings.scale.denominator).range(1..=100000));
        });
        ui.label("Printable margin (mm)");
        ui.add(
            egui::DragValue::new(&mut page.settings.margin_mm)
                .speed(0.5)
                .range(0.0..=500.),
        );
        ui.checkbox(&mut page.settings.title_block, "Show title block").on_hover_text("Keep the saved drawing information and print its field layout inside the page margins");
        ui.add_enabled(
            page.settings.title_block,
            egui::Checkbox::new(&mut page.settings.app_logo, "Show app logo in title block"),
        )
        .on_hover_text("Print the ToskCAD logo beside the project name; when off, the project field uses the full width. Saved with this page");
        ui.separator();
        ui.strong("Drawing information");
        for (label, value) in [
            ("Title", &mut page.info.title),
            ("Author", &mut page.info.author),
            ("Project", &mut page.info.project),
            ("Revision", &mut page.info.revision),
            ("Checker", &mut page.info.checker),
        ] {
            ui.label(label);
            ui.add(
                egui::TextEdit::multiline(value)
                    .desired_rows(1)
                    .desired_width(f32::INFINITY),
            ).on_hover_text("The complete saved value wraps in its title-block field. Unprintable text produces an explicit fit error");
        }
        ui.small("Page / total is calculated from the saved page order. Missing title values print as —.");
        let mut remove = None;
        for (index, note) in page.notes.iter_mut().enumerate() {
            ui.separator();
            ui.label(format!("Text note {}", index + 1));
            ui.add(
                egui::TextEdit::multiline(&mut note.text)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal_wrapped(|ui| {
                ui.label("X / Y mm");
                for value in &mut note.position_mm {
                    ui.add(egui::DragValue::new(value).speed(1.));
                }
                if crate::icons::compact_button(ui, Icon::Trash, "Remove note").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            page.notes.remove(index);
        }
        self.schedule_inspector(ui, session);
        self.annotations_inspector(ui, session);
        self.pin_dimension_controls(ui, session);
        self.page_actions(ui, session);
    }
}

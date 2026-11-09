//! General project details and saved-file Library inclusion.
use crate::{library::PreviewTextures, parameter_editor::ParameterEditor};
use eframe::egui::{self, Color32, Rect, Sense, Vec2};
use multicad_core::{commands::Session, document::EntityId, project_file::info};

pub const WIDTH: f32 = 336.;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarAction {
    ChooseImage,
    AddToLibrary,
    RemoveEntry(EntityId),
}

pub struct SidebarContext<'a> {
    pub register: Result<(), String>,
    pub library_busy: bool,
    pub choose_image: Result<(), String>,
    pub status: Option<&'a (String, bool)>,
    pub listing: Option<&'a (String, Option<EntityId>)>,
}

fn switch(ui: &mut egui::Ui, label: &str, on: &mut bool) -> egui::Response {
    let text = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::TextStyle::Body.resolve(ui.style()),
        ui.visuals().text_color(),
    );
    let (rect, mut response) =
        ui.allocate_exact_size(Vec2::new(34. + 9. + text.size().x, 25.), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, label)
    });
    if ui.is_rect_visible(rect) {
        let track =
            Rect::from_center_size(rect.left_center() + Vec2::new(17., 0.), Vec2::new(34., 19.));
        let style = ui.style().interact(&response);
        let fill = if *on {
            ui.visuals().selection.bg_fill
        } else {
            style.bg_fill
        };
        ui.painter()
            .rect(track, 10, fill, style.bg_stroke, egui::StrokeKind::Inside);
        let x = if *on {
            track.right() - 10.
        } else {
            track.left() + 10.
        };
        ui.painter()
            .circle_filled(egui::pos2(x, track.center().y), 6.5, style.fg_stroke.color);
        ui.painter().galley(
            egui::pos2(track.right() + 9., rect.center().y - text.size().y * 0.5),
            text,
            ui.visuals().text_color(),
        );
    }
    response
}

pub fn show(
    ui: &mut egui::Ui,
    editor: &mut ParameterEditor,
    session: &mut Session,
    textures: &mut PreviewTextures,
    cx: SidebarContext<'_>,
) -> Option<SidebarAction> {
    let mut action = None;
    editor.sync_clean_session(session);
    ui.label(egui::RichText::new("Project Details").heading().strong());
    ui.add_space(12.);
    let locked = editor.info_locked.clone();
    if let Some(reason) = &locked {
        ui.colored_label(Color32::LIGHT_RED, reason);
    }
    let mut info_focused = false;
    ui.add_enabled_ui(locked.is_none(), |ui| {
        ui.label(
            egui::RichText::new("Project Name")
                .strong()
                .color(Color32::WHITE),
        );
        let name = ui.add(
            egui::TextEdit::singleline(&mut editor.info.name)
                .hint_text("For example, Workshop cabinet")
                .text_color(Color32::WHITE)
                .char_limit(info::MAX_NAME_CHARS)
                .desired_width(f32::INFINITY),
        );
        info_focused |= name.has_focus();
        if info::validate_name(&editor.info.name).is_err() {
            ui.colored_label(Color32::LIGHT_RED, "Enter a project name.");
        }
        ui.add_space(7.);
        let listed = cx.listing.and_then(|(_, id)| *id);
        let mut included = listed.is_some();
        let inclusion = ui
            .add_enabled_ui(
                !cx.library_busy && (included || cx.register.is_ok()),
                |ui| switch(ui, "Include in Library", &mut included),
            )
            .inner;
        let help = if included {
            "Remove this listing without removing its project file"
        } else {
            cx.register
                .as_ref()
                .err()
                .map(String::as_str)
                .unwrap_or("List this saved project in Library")
        };
        if inclusion
            .on_hover_text(help)
            .on_disabled_hover_text(help)
            .changed()
        {
            action = if included {
                Some(SidebarAction::AddToLibrary)
            } else {
                listed.map(SidebarAction::RemoveEntry)
            };
        }
        if let Some((message, error)) = cx.status {
            ui.colored_label(
                if *error {
                    Color32::LIGHT_RED
                } else {
                    ui.visuals().text_color()
                },
                message,
            );
        }
        ui.add_space(13.);
        ui.label(egui::RichText::new("Project Description").strong());
        let description = ui.add(
            egui::TextEdit::multiline(&mut editor.info.description)
                .char_limit(info::MAX_DESCRIPTION_CHARS)
                .desired_rows(6)
                .desired_width(f32::INFINITY)
                .hint_text("What is this part or project for?"),
        );
        info_focused |= description.has_focus();
        ui.add_space(13.);
        ui.label(egui::RichText::new("Preview Image").strong());
        let width = ui.available_width();
        let aspect = editor.info.preview.as_ref().map_or(9. / 16., |image| {
            image.height() as f32 / image.width() as f32
        });
        let size = Vec2::new(width, width * aspect);
        let frame = Rect::from_min_size(ui.cursor().min, size);
        textures.show_framed(
            ui,
            editor.info.preview.as_ref(),
            size,
            "Project preview image",
        );
        let overlay_rect = Rect::from_min_max(
            egui::pos2(frame.left() + 8., frame.bottom() - 34.),
            egui::pos2(frame.right() - 8., frame.bottom() - 5.),
        );
        ui.new_child(
            egui::UiBuilder::new()
                .max_rect(overlay_rect)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        )
        .horizontal(|ui| {
            let fill = crate::theme::PANEL;
            if ui
                .add_enabled(
                    editor.info.preview.is_some(),
                    egui::Button::new("Remove Image").fill(fill),
                )
                .clicked()
            {
                editor.info.preview = None;
            }
            let choose_help = cx
                .choose_image
                .as_ref()
                .err()
                .map(String::as_str)
                .unwrap_or("Use a PNG instead of capturing the 3D view on save");
            if ui
                .add_enabled(
                    cx.choose_image.is_ok(),
                    egui::Button::new("Choose Image…").fill(fill),
                )
                .on_hover_text(choose_help)
                .on_disabled_hover_text(choose_help)
                .clicked()
            {
                action = Some(SidebarAction::ChooseImage);
            }
        });
        ui.add_space(7.);
        switch(
            ui,
            "Capture 3D View on Save",
            &mut editor.info.capture_on_save,
        )
        .on_hover_text("Replace the preview when saving from a visible Model view.");
    });
    if !info_focused
        && locked.is_none()
        && info::validate_name(&editor.info.name).is_ok()
        && let Err(error) = editor.commit_file_info(session)
    {
        ui.colored_label(Color32::LIGHT_RED, error);
    }
    action
}

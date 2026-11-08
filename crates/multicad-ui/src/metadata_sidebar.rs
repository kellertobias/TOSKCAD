//! Project metadata edits share the parameter draft. Library inclusion indexes the saved file.
use crate::{
    library::{LibraryEntry, LibraryOrigin, PreviewTextures},
    parameter_editor::ParameterEditor,
};
use eframe::egui::{self, Color32};
use multicad_core::{commands::Session, document::EntityId, project_file::info};

pub const WIDTH: f32 = 300.;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarAction {
    ChooseImage,
    AddToLibrary,
    ReloadLibrary,
    /// Remove a listing (the file itself is untouched).
    RemoveEntry(EntityId),
}
pub struct SidebarContext<'a> {
    /// `Err` explains why the saved project cannot be included in Library.
    pub register: Result<(), String>,
    pub library_busy: bool,
    pub choose_image: Result<(), String>,
    /// Latest registration result: message and whether it is an error.
    pub status: Option<&'a (String, bool)>,
    /// Listing description and the entry ID when the current path is listed.
    pub listing: Option<&'a (String, Option<EntityId>)>,
    pub library: &'a [LibraryEntry],
    pub library_error: Option<&'a str>,
}
fn enabled_button(ui: &mut egui::Ui, label: &str, state: &Result<(), String>, help: &str) -> bool {
    let response = ui.add_enabled(state.is_ok(), egui::Button::new(label));
    match state {
        Ok(()) => response.on_hover_text(help).clicked(),
        Err(reason) => {
            response.on_disabled_hover_text(reason.as_str());
            false
        }
    }
}
pub fn show(
    ui: &mut egui::Ui,
    editor: &mut ParameterEditor,
    session: &Session,
    textures: &mut PreviewTextures,
    cx: SidebarContext<'_>,
) -> Option<SidebarAction> {
    let mut action = None;
    let mut changed = false;
    egui::ScrollArea::vertical()
        .id_salt("parameters-metadata")
        .show(ui, |ui| {
            ui.heading("Project details");
            ui.add_space(6.);
            let locked = editor.info_locked.clone();
            if let Some(reason) = &locked {
                ui.colored_label(Color32::LIGHT_RED, reason);
            }
            ui.add_enabled_ui(locked.is_none(), |ui| {
                ui.label("Project name");
                ui.label("Shown in Project and Library. Choose the filename when you save.");
                changed |= ui
                    .add(egui::TextEdit::singleline(&mut editor.info.name)
                        .hint_text("For example, Workshop cabinet")
                        .char_limit(info::MAX_NAME_CHARS)
                        .desired_width(f32::INFINITY))
                    .on_hover_text("Shown on the home screen and in the library")
                    .changed();
                if let Err(error) = info::validate_name(&editor.info.name) {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
                ui.label("Project description");
                changed |= ui.scope(|ui| {
                    ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                    egui::ScrollArea::vertical()
                        .id_salt("project-file-description")
                        .max_height(104.)
                        .show(ui, |ui| ui.add(
                            egui::TextEdit::multiline(&mut editor.info.description)
                                .char_limit(info::MAX_DESCRIPTION_CHARS)
                                .desired_rows(4)
                                .desired_width(f32::INFINITY)
                                .hint_text("What is this part or project for?")
                        ).changed()).inner
                }).inner;
                ui.label("Preview image");
                let preview_height = if editor.info.preview.is_some() { 128. } else { 48. };
                textures.show(
                    ui,
                    editor.info.preview.as_ref(),
                    egui::vec2(ui.available_width().min(WIDTH - 24.), preview_height),
                    "File preview image",
                );
                ui.horizontal_wrapped(|ui| {
                    if enabled_button(
                        ui,
                        "Choose image…",
                        &cx.choose_image,
                        "Use a PNG instead of the captured view (at most 1 MiB, 2048×2048 px); turns capture on save off",
                    ) {
                        action = Some(SidebarAction::ChooseImage);
                    }
                    if ui
                        .add_enabled(
                            editor.info.preview.is_some(),
                            egui::Button::new("Remove image"),
                        )
                        .clicked()
                    {
                        editor.info.preview = None;
                        changed = true;
                    }
                });
                changed |= ui
                    .checkbox(
                        &mut editor.info.capture_on_save,
                        "Capture the 3D view on save",
                    )
                    .on_hover_text("Each Save stores the visible Sketch/Model 3D view as the preview. Saving from other modes keeps the current preview.")
                    .changed();
            });
            if editor.info_dirty() {
                ui.colored_label(
                    Color32::from_rgb(232, 190, 92),
                    "Use Apply and Rebuild above to keep these edits, or Discard Edit to revert them.",
                );
            }
            ui.separator();
            ui.heading("Library");
            let listed_id = cx.listing.and_then(|(_, id)| *id);
            let mut included = listed_id.is_some();
            if ui
                .add_enabled(
                    !cx.library_busy && (included || cx.register.is_ok()),
                    egui::Checkbox::new(&mut included, "Include in Library"),
                )
                .on_hover_text("Lists this saved project at its existing location")
                .changed()
            {
                action = if included {
                    Some(SidebarAction::AddToLibrary)
                } else {
                    listed_id.map(SidebarAction::RemoveEntry)
                };
            }
            if let Some((message, _)) = cx.listing {
                ui.label(message);
            }
            if listed_id.is_none() && let Err(reason) = &cx.register {
                ui.label(reason);
            }
            if let Some((message, error)) = cx.status {
                ui.colored_label(
                    if *error {
                        Color32::LIGHT_RED
                    } else {
                        Color32::from_rgb(132, 200, 140)
                    },
                    message,
                );
            }
            ui.label("Open Model, then Library, to browse and place definitions.");
            if let Some(error) = cx.library_error {
                ui.colored_label(Color32::LIGHT_RED, error);
            }
            let listed = cx.library.iter().filter(|entry| entry.origin == LibraryOrigin::ListedOriginal);
            let count = listed.clone().count();
            egui::CollapsingHeader::new(format!("Projects listed in Library ({count})"))
                .id_salt("project-manage-library-listings")
                .show(ui, |ui| {
                    ui.label("These projects are listed at their saved locations. Removing a listing leaves the project file and placed parts intact.");
                    if ui.add_enabled(!cx.library_busy, egui::Button::new("Reload")).clicked() {
                        action = Some(SidebarAction::ReloadLibrary);
                    }
                    if count == 0 && cx.library_error.is_none() {
                        ui.label("No original files listed.");
                    }
                    for entry in listed {
                        ui.push_id(entry.id, |ui| {
                            ui.strong(&entry.name);
                            let path = entry.path.display().to_string();
                            ui.add(egui::Label::new(&path).truncate())
                                .on_hover_text(path);
                            if let Some(problem) = &entry.problem {
                                ui.colored_label(Color32::LIGHT_RED, problem);
                            }
                            if ui
                                .add_enabled(!cx.library_busy, egui::Button::new("Remove listing"))
                                .on_hover_text("Remove the Library listing and keep the project file")
                                .clicked()
                            {
                                action = Some(SidebarAction::RemoveEntry(entry.id));
                            }
                        });
                    }
                });
        });
    if changed {
        editor.refresh_preview(session);
    }
    action
}

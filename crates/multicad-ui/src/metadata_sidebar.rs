//! Parameters metadata sidebar: the file's name, description and preview
//! image, plus the explicit Add to Library action. It edits only the
//! Parameters draft; Apply/Rebuild commits it with the table in one undoable
//! transaction and Discard reverts both. No modeling inspector lives here.
use crate::{
    library::{LibraryEntry, PreviewTextures},
    parameter_editor::ParameterEditor,
};
use eframe::egui::{self, Color32};
use multicad_core::{commands::Session, document::EntityId, project_file::info};
use std::path::PathBuf;

pub const WIDTH: f32 = 300.;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarAction {
    ChooseImage,
    AddToLibrary,
    ReloadLibrary,
    /// Load a library entry's symbol definitions through the explicit,
    /// pinned import/update workflow.
    UseLibraryEntry(PathBuf),
    /// Remove a listing (the file itself is untouched).
    RemoveEntry(EntityId),
}
pub struct SidebarContext<'a> {
    /// `Err` explains why Add to Library is unavailable right now.
    pub register: Result<(), String>,
    pub registering: bool,
    pub choose_image: Result<(), String>,
    /// Latest registration result: message and whether it is an error.
    pub status: Option<&'a (String, bool)>,
    /// How the current file is listed: message and whether it is listed here.
    pub listing: Option<&'a (String, bool)>,
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
            ui.heading("File");
            ui.label(
                egui::RichText::new("Describes this project file. Applied with Apply/Rebuild.")
                    .small()
                    .color(Color32::GRAY),
            );
            ui.add_space(6.);
            let locked = editor.info_locked.clone();
            if let Some(reason) = &locked {
                ui.colored_label(Color32::LIGHT_RED, reason);
            }
            ui.add_enabled_ui(locked.is_none(), |ui| {
                ui.label("Name");
                changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut editor.info.name)
                            .char_limit(info::MAX_NAME_CHARS)
                            .desired_width(f32::INFINITY),
                    )
                    .on_hover_text("Shown on the home screen and in the library")
                    .changed();
                if let Err(error) = info::validate_name(&editor.info.name) {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
                ui.label("Description");
                changed |= ui
                    .add(
                        egui::TextEdit::multiline(&mut editor.info.description)
                            .char_limit(info::MAX_DESCRIPTION_CHARS)
                            .desired_rows(4)
                            .desired_width(f32::INFINITY)
                            .hint_text("What is this part or project for?"),
                    )
                    .changed();
                ui.label("Preview image");
                textures.show(
                    ui,
                    editor.info.preview.as_ref(),
                    egui::vec2(ui.available_width().min(WIDTH - 24.), 160.),
                    "File preview image",
                );
                ui.horizontal(|ui| {
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
                    "Unapplied file changes · use Apply/Rebuild or Discard",
                );
            }
            ui.separator();
            ui.heading("Library");
            ui.label(
                egui::RichText::new(
                    "One library for this installation. Adding lists the saved file at its location; nothing is copied. Projects that already use it keep their pinned copy.",
                )
                .small()
                .color(Color32::GRAY),
            );
            let state = if cx.registering {
                Err("Adding to the library…".to_string())
            } else {
                cx.register.clone()
            };
            if enabled_button(
                ui,
                "Add to Library",
                &state,
                "List the saved file at its location in this installation's library",
            ) {
                action = Some(SidebarAction::AddToLibrary);
            }
            match cx.listing {
                Some((message, true)) => {
                    ui.colored_label(Color32::from_rgb(132, 200, 140), message);
                }
                Some((message, false)) => {
                    ui.colored_label(Color32::from_rgb(232, 190, 92), message);
                }
                None => {
                    ui.label(
                        egui::RichText::new("Not listed in the library")
                            .small()
                            .color(Color32::GRAY),
                    );
                }
            }
            if let Err(reason) = &cx.register {
                ui.label(egui::RichText::new(reason).small().color(Color32::GRAY));
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
            ui.add_space(6.);
            ui.horizontal(|ui| {
                ui.strong(format!("Entries ({})", cx.library.len()));
                if ui.small_button("Reload").clicked() {
                    action = Some(SidebarAction::ReloadLibrary);
                }
            });
            if let Some(error) = cx.library_error {
                ui.colored_label(Color32::LIGHT_RED, error);
            }
            if cx.library.is_empty() && cx.library_error.is_none() {
                ui.label(
                    egui::RichText::new("No files in the library yet.").color(Color32::GRAY),
                );
            }
            for entry in cx.library {
                ui.group(|ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        textures.show(
                            ui,
                            entry.preview.as_ref(),
                            egui::vec2(64., 48.),
                            &entry.name,
                        );
                        ui.vertical(|ui| {
                            ui.strong(&entry.name);
                            if !entry.description.is_empty() {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&entry.description).small(),
                                    )
                                    .truncate(),
                                );
                            }
                        });
                    });
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(entry.path.display().to_string())
                                .small()
                                .color(Color32::GRAY),
                        )
                        .truncate(),
                    )
                    .on_hover_text(entry.path.display().to_string());
                    if let Some(problem) = &entry.problem {
                        ui.colored_label(Color32::LIGHT_RED, problem);
                        if ui
                            .small_button("Remove from library")
                            .on_hover_text("Remove this listing; no file is deleted")
                            .clicked()
                        {
                            action = Some(SidebarAction::RemoveEntry(entry.id));
                        }
                    } else if entry.symbol_definitions > 0 {
                        if ui
                            .small_button(format!(
                                "Use symbols ({})",
                                entry.symbol_definitions
                            ))
                            .on_hover_text("Load its symbol definitions; import and updates stay explicit in Sketch › Symbols")
                            .clicked()
                        {
                            action = Some(SidebarAction::UseLibraryEntry(entry.path.clone()));
                        }
                    } else {
                        ui.label(
                            egui::RichText::new("No symbol definitions to place")
                                .small()
                                .color(Color32::GRAY),
                        );
                    }
                });
            }
        });
    if changed {
        editor.refresh_preview(session);
    }
    action
}

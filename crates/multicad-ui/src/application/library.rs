//! Installation library, Parameters metadata sidebar and home-screen cards.
use super::*;
use crate::{
    library::{
        LibraryEntry, LibraryEvent, LibraryStore, LibraryWorker, PreviewTextures, RegisterRequest,
    },
    metadata_sidebar::{self, SidebarAction, SidebarContext},
};
use multicad_core::project_file::ProjectCard;
use std::{collections::BTreeSet, path::Path};

#[derive(Default)]
pub(super) struct LibraryState {
    worker: Option<LibraryWorker>,
    pub(super) entries: Vec<LibraryEntry>,
    error: Option<String>,
    pub(super) status: Option<(String, bool)>,
    pub(super) cards: BTreeMap<PathBuf, Result<ProjectCard, String>>,
    requested: BTreeSet<PathBuf>,
    pub(super) textures: PreviewTextures,
    /// Save waiting for its preview screenshot.
    pub(super) capture: Option<super::save_preview::PendingCapture>,
    /// Outcome of the last save-time capture, appended to the save receipt.
    pub(super) capture_note: Option<String>,
    /// The 3D viewport drawn in the latest frame, if any.
    pub(super) viewport: Option<egui::Rect>,
}
impl Application {
    pub fn set_library_store(&mut self, store: Box<dyn LibraryStore>) {
        match LibraryWorker::new(store) {
            Ok(worker) => self.library.worker = Some(worker),
            Err(error) => self.library.error = Some(error),
        }
    }
    pub(super) fn poll_library(&mut self, ctx: &egui::Context) {
        let mut events = Vec::new();
        if let Some(worker) = &mut self.library.worker {
            while let Some(event) = worker.poll() {
                events.push(event);
            }
            if worker.pending() {
                ctx.request_repaint_after(std::time::Duration::from_millis(32));
            }
        }
        for event in events {
            self.accept_library_event(event);
        }
    }
    fn accept_library_event(&mut self, event: LibraryEvent) {
        let refresh = matches!(
            &event,
            LibraryEvent::Loaded(Ok(_))
                | LibraryEvent::Removed(Ok(_))
                | LibraryEvent::Registered { result: Ok(_), .. }
        );
        match event {
            LibraryEvent::Loaded(Ok(entries)) => {
                self.library.entries = entries;
                self.library.error = None;
            }
            LibraryEvent::Loaded(Err(error)) => {
                self.library.error = Some(format!("Library: {error}"));
            }
            LibraryEvent::Removed(result) => {
                let (message, error) = match result {
                    Ok(entries) => {
                        self.library.entries = entries;
                        self.library.error = None;
                        (
                            "Library listing removed. The project file is unchanged.".into(),
                            false,
                        )
                    }
                    Err(error) => (
                        format!("Could not remove the Library listing: {error}"),
                        true,
                    ),
                };
                self.library.status = Some((message.clone(), error));
                self.message = message;
                self.message_error = error;
            }
            LibraryEvent::Registered { result, .. } => match result {
                Ok((receipt, entries)) => {
                    let message = receipt.message();
                    self.library.entries = entries;
                    self.library.error = None;
                    self.library.status = Some((message.clone(), false));
                    self.message = message;
                    self.message_error = false;
                }
                Err(error) => {
                    let message = format!("Could not include the project in Library: {error}");
                    self.library.status = Some((message.clone(), true));
                    self.message = message;
                    self.message_error = true;
                }
            },
            LibraryEvent::Card { path, result } => {
                self.library.cards.insert(path, result);
            }
            LibraryEvent::Preview { local, result } => {
                if self.file_document == Some(local) {
                    self.file_action = None;
                    self.file_document = None;
                }
                let Some(project) = self.projects.iter_mut().find(|p| p.local_id == local) else {
                    return;
                };
                match result {
                    Ok(image) if project.editor.info_locked.is_none() => {
                        project.editor.info.preview = Some(image);
                        // Keep the chosen image instead of capturing on save.
                        project.editor.info.capture_on_save = false;
                        project.editor.refresh_preview(&project.session);
                        self.message =
                            "Preview image chosen · Apply/Rebuild stores it; capture on save is now off".into();
                        self.message_error = false;
                    }
                    Ok(_) => {
                        self.message = "This file's information is read-only here".into();
                        self.message_error = true;
                    }
                    Err(error) => {
                        self.message = error;
                        self.message_error = true;
                    }
                }
            }
        }
        if refresh {
            let entries = std::sync::Arc::new(self.library.entries.clone());
            for project in &mut self.projects {
                project.sketch.set_installed_symbol_library(entries.clone());
            }
        }
    }
    /// Re-read a file's home-screen card after it was opened or saved.
    pub(super) fn invalidate_card(&mut self, path: &Path) {
        self.library.requested.remove(path);
        self.library.cards.remove(path);
        // Listed files are read live: refresh the library after a save.
        let listed = self.library.entries.iter().any(|e| {
            e.path == path
                || path
                    .canonicalize()
                    .is_ok_and(|canonical| canonical == e.path)
        });
        if listed && let Some(worker) = &mut self.library.worker {
            let _ = worker.reload();
        }
    }
    /// How the current file is listed, for the sidebar status.
    fn listing_status(&self) -> Option<(String, Option<EntityId>)> {
        let p = self.current();
        let document = p.session.document().id();
        let entry = self.library.entries.iter().find(|e| {
            e.document == document && e.origin == crate::library::LibraryOrigin::ListedOriginal
        })?;
        let here = p.path.as_ref().is_some_and(|path| {
            *path == entry.path || path.canonicalize().is_ok_and(|c| c == entry.path)
        });
        Some(if here {
            (
                format!("Listed in the library · {}", entry.path.display()),
                Some(entry.id),
            )
        } else {
            (
                format!(
                    "Listed from another location: {}. Including this file replaces that listing.",
                    entry.path.display()
                ),
                None,
            )
        })
    }
    pub(super) fn request_cards(&mut self) {
        let Some(worker) = &mut self.library.worker else {
            return;
        };
        for entry in &self.recents {
            if self.library.requested.insert(entry.path.clone())
                && let Err(error) = worker.card(entry.path.clone())
            {
                self.library.cards.insert(entry.path.clone(), Err(error));
            }
        }
    }
    /// Why including the current project in Library is unavailable, if it is.
    pub(super) fn registration_blocker(&self) -> Result<RegisterRequest, String> {
        if self.library.worker.is_none() {
            return Err("The installation library is unavailable in this host".into());
        }
        let p = self.current();
        let (Some(path), Some(stamp)) = (p.path.clone(), p.file_stamp) else {
            return Err("Save the project to a file before adding it to the library".into());
        };
        if p.dirty() {
            return Err(
                "Apply and save your changes first · the library lists the saved file".into(),
            );
        }
        if self.file_busy() {
            return Err("A file operation is still running".into());
        }
        Ok(RegisterRequest {
            path,
            stamp,
            document: p.session.document().id(),
        })
    }
    pub(super) fn add_to_library(&mut self) -> Result<(), String> {
        let request = self.registration_blocker()?;
        self.library
            .worker
            .as_mut()
            .ok_or("The installation library is unavailable in this host")?
            .register(request)?;
        self.library.status = None;
        self.message = "Listing the saved file in the library…".into();
        self.message_error = false;
        Ok(())
    }
    pub(super) fn choose_preview_image(&mut self) -> Result<(), String> {
        if self.file_busy() || self.file_action.is_some() {
            return Err("Complete the current file operation first".into());
        }
        if self.library.worker.is_none() {
            return Err("Preview images are unavailable in this host".into());
        }
        self.file_action = Some(FileAction::PreviewImage);
        self.file_document = Some(self.current().local_id);
        self.file_path.clear();
        self.picker_started = false;
        self.replacement = None;
        Ok(())
    }
    pub(super) fn read_preview_image(&mut self, path: PathBuf) -> Result<(), String> {
        let local = self.current().local_id;
        if self.file_document != Some(local) {
            return Err("Return to the project that opened this image dialog".into());
        }
        self.library
            .worker
            .as_mut()
            .ok_or("Preview images are unavailable in this host")?
            .read_preview(local, path)?;
        self.message = "Reading preview image…".into();
        self.message_error = false;
        Ok(())
    }
    pub(super) fn parameters_sidebar(&mut self, ui: &mut egui::Ui) {
        let register = self.registration_blocker().map(|_| ());
        let choose_image = if self.file_busy() || self.file_action.is_some() {
            Err("Complete the current file operation first".to_string())
        } else if self.library.worker.is_none() {
            Err("Preview images are unavailable in this host".to_string())
        } else {
            Ok(())
        };
        let library_busy = self.library.worker.as_ref().is_some_and(|w| w.pending());
        let listing = self.listing_status();
        let state = &mut self.library;
        let p = &mut self.projects[self.active];
        let action = metadata_sidebar::show(
            ui,
            &mut p.editor,
            &p.session,
            &mut state.textures,
            SidebarContext {
                register,
                library_busy,
                choose_image,
                status: state.status.as_ref(),
                listing: listing.as_ref(),
                library: &state.entries,
                library_error: state.error.as_deref(),
            },
        );
        let result = match action {
            None => Ok(()),
            Some(SidebarAction::ChooseImage) => self.choose_preview_image(),
            Some(SidebarAction::AddToLibrary) => self.add_to_library(),
            Some(SidebarAction::ReloadLibrary) => self
                .library
                .worker
                .as_mut()
                .ok_or_else(|| "The installation library is unavailable in this host".into())
                .and_then(|w| w.reload()),
            Some(SidebarAction::RemoveEntry(id)) => self.remove_library_entry(id),
        };
        if let Err(error) = result {
            self.message = error;
            self.message_error = true;
        }
    }

    fn remove_library_entry(&mut self, id: EntityId) -> Result<(), String> {
        self.library
            .worker
            .as_mut()
            .ok_or("The installation library is unavailable in this host")?
            .remove(id)?;
        self.library.status = None;
        self.message = "Removing the Library listing…".into();
        self.message_error = false;
        Ok(())
    }
    /// Home-screen card: the file's own name, description and preview.
    pub(super) fn recent_card(
        &mut self,
        ui: &mut egui::Ui,
        entry: &crate::recent_projects::RecentProject,
    ) {
        let card = self.library.cards.get(&entry.path).cloned();
        ui.horizontal(|ui| {
            let preview = card
                .as_ref()
                .and_then(|c| c.as_ref().ok())
                .and_then(|c| c.info.preview.clone());
            self.library.textures.show(
                ui,
                preview.as_ref(),
                egui::vec2(96., 64.),
                "Project preview",
            );
            ui.vertical(|ui| match &card {
                Some(Ok(card)) => {
                    ui.strong(&card.info.name);
                    if !card.info.description.is_empty() {
                        ui.add(
                            egui::Label::new(egui::RichText::new(&card.info.description).small())
                                .truncate(),
                        );
                    }
                }
                Some(Err(_)) | None => {
                    ui.strong(&entry.title);
                }
            });
        });
    }
}

#[cfg(test)]
#[path = "library_tests.rs"]
mod tests;

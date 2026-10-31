//! Native library dialogs reuse bounded project I/O without opening another tab
//! or acknowledging an unrelated project save.
use super::*;
use crate::{
    storage::{FileRequest, SaveMode},
    symbols::SymbolAction,
};

impl Application {
    pub(super) fn poll_symbol_actions(&mut self) {
        let Some(action) = self.current_mut().sketch.take_symbol_action() else {
            return;
        };
        if self.file_busy() || self.file_action.is_some() {
            self.message =
                "Complete the current file operation before opening a symbol library".into();
            self.message_error = true;
            return;
        }
        self.file_action = Some(match action {
            SymbolAction::ImportLibrary => FileAction::SymbolOpen,
            SymbolAction::SaveDefinition(id) => FileAction::SymbolSave(id),
        });
        self.file_document = Some(self.current().local_id);
        self.file_path.clear();
        self.picker_started = false;
        self.replacement = None;
    }
    pub(super) fn begin_artwork_import(&mut self, path: PathBuf) -> Result<(), String> {
        self.file_action = None;
        self.file_document = None;
        self.current_mut().sketch.begin_artwork_import(path)?;
        self.mode = Mode::Sketch;
        self.message = "Reading artwork…".into();
        self.message_error = false;
        Ok(())
    }
    pub(super) fn symbol_filename(&self, id: EntityId) -> Result<String, String> {
        let definition =
            multicad_core::symbols::read_definition(self.current().session.document(), id)
                .map_err(|e| e.to_string())?;
        let raw = format!("{}-{}", definition.name, definition.revision);
        let mut name = String::new();
        for character in raw.chars() {
            let character = if character.is_control() || "<>:\"/\\|?*".contains(character) {
                '_'
            } else {
                character
            };
            if name.len() + character.len_utf8() > 180 {
                break;
            }
            name.push(character);
        }
        Ok(multicad_core::project_path::project_file_name(&name))
    }
    pub(super) fn load_symbol_library(&mut self, path: PathBuf) -> Result<(), String> {
        if self.file_busy() {
            return Err("A file operation is still running".into());
        }
        let p = self.current();
        let local = p.local_id;
        let guard = p
            .session
            .prepare(Request {
                expected_revision: p.session.revision(),
                actor: Actor::local(Channel::Ui),
                label: "Read symbol library".into(),
                edits: vec![],
            })
            .map_err(|e| e.to_string())?;
        let resources = p.resources.clone();
        let id = self.storage.submit(FileRequest::Open(path))?;
        self.pending_files.insert(
            id,
            PendingFile::SymbolOpen {
                local,
                guard,
                resources,
            },
        );
        self.message = "Reading pinned symbol library…".into();
        self.message_error = false;
        Ok(())
    }
    pub(super) fn save_symbol_library(
        &mut self,
        path: PathBuf,
        id: EntityId,
    ) -> Result<(), String> {
        if self.file_busy() {
            return Err("A file operation is still running".into());
        }
        let p = self.current();
        if self.file_document != Some(p.local_id) {
            return Err("Return to the project that opened this library dialog".into());
        }
        let definition = multicad_core::symbols::read_definition(p.session.document(), id)
            .map_err(|e| e.to_string())?;
        let document = Document::new(format!("{} · symbol library", definition.name))
            .map_err(|e| e.to_string())?;
        let mut entity = Entity::new(
            &definition.name,
            EntityData::Extension {
                definition: definition.definition().map_err(|e| e.to_string())?,
                inputs: vec![],
            },
        );
        entity.id = id;
        let document = document
            .edited(&[Mutation::Insert(entity)])
            .map_err(|e| e.to_string())?;
        let project = ProjectFile::new(document)?;
        let job = self.storage.submit(FileRequest::Save {
            path,
            project,
            mode: SaveMode::Create,
        })?;
        self.pending_files.insert(job, PendingFile::SymbolSave);
        self.message = "Saving pinned symbol library…".into();
        self.message_error = false;
        Ok(())
    }
}

//! Pluggable project storage and nonblocking file jobs. Geometry providers never
//! receive file-system handles, and file jobs never mutate the live Session.
use multicad_core::{
    document::{Document, EntityId},
    project_file::{ContentHash, ProjectFile, RecoveryInfo},
};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub manifest: ContentHash,
    pub length: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveMode {
    Create,
    Replace(FileStamp),
}
#[derive(Clone, Debug)]
pub struct SavedProject {
    pub stamp: FileStamp,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct LoadedProject {
    pub file: ProjectFile,
    pub stamp: Option<FileStamp>,
    pub diagnostics: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct RecoveryEntry {
    pub info: RecoveryInfo,
    pub document: EntityId,
    pub name: String,
}
#[derive(Clone, Debug, Default)]
pub struct RecoveryListing {
    pub entries: Vec<RecoveryEntry>,
    pub diagnostics: Vec<String>,
}
#[derive(Clone, Debug)]
pub enum StorageError {
    Message(String),
    AlreadyExists(FileStamp),
    Changed,
}
impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(message)=>f.write_str(message),
            Self::AlreadyExists(_)=>f.write_str("A project already exists at that path. Confirm replacement or choose another path."),
            Self::Changed=>f.write_str("The file changed outside this document. Use Save As to preserve both versions."),
        }
    }
}
impl From<String> for StorageError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}
impl From<std::io::Error> for StorageError {
    fn from(value: std::io::Error) -> Self {
        Self::Message(value.to_string())
    }
}
pub trait DocumentStorage: Send {
    /// Explicit legacy acceptance-fixture hooks. Production UI uses the native
    /// project methods and never silently falls back to these JSON operations.
    fn save(&mut self, path: &Path, document: &Document) -> Result<(), String>;
    fn load(&mut self, path: &Path) -> Result<Document, String>;
    fn save_project(
        &mut self,
        _path: &Path,
        _project: &ProjectFile,
        _mode: SaveMode,
    ) -> Result<SavedProject, StorageError> {
        Err(StorageError::Message(
            "Native project saving is unavailable in this storage adapter".into(),
        ))
    }
    fn open_project(&mut self, _path: &Path) -> Result<LoadedProject, StorageError> {
        Err(StorageError::Message(
            "Native project opening is unavailable in this storage adapter".into(),
        ))
    }
    fn import_source(&mut self, path: &Path) -> Result<LoadedProject, StorageError> {
        let document = self.load(path).map_err(StorageError::Message)?;
        Ok(LoadedProject {
            file: ProjectFile::new(document).map_err(StorageError::Message)?,
            stamp: None,
            diagnostics: vec![
                "Imported source snapshot. Save as a native project to preserve project resources."
                    .into(),
            ],
        })
    }
    fn write_recovery(
        &mut self,
        _project: &ProjectFile,
        _revision: u64,
        _original: Option<&Path>,
    ) -> Result<RecoveryEntry, StorageError> {
        Err(StorageError::Message(
            "Recovery snapshots are unavailable in this storage adapter".into(),
        ))
    }
    /// Update only the caller's random session/tab slot; manual checkpoints use
    /// write_recovery and retain their separate immutable identities.
    fn write_recovery_slot(
        &mut self,
        _project: &ProjectFile,
        _revision: u64,
        _original: Option<&Path>,
        _slot: EntityId,
    ) -> Result<RecoveryEntry, StorageError> {
        Err(StorageError::Message(
            "Automatic recovery slots are unavailable in this storage adapter".into(),
        ))
    }
    fn recoveries(&mut self) -> Result<RecoveryListing, StorageError> {
        Ok(RecoveryListing::default())
    }
    fn restore_recovery(&mut self, _id: EntityId) -> Result<LoadedProject, StorageError> {
        Err(StorageError::Message(
            "Recovery restoration is unavailable in this storage adapter".into(),
        ))
    }
}
pub enum FileRequest {
    Save {
        path: PathBuf,
        project: ProjectFile,
        mode: SaveMode,
    },
    Open(PathBuf),
    Import(PathBuf),
    Snapshot {
        project: ProjectFile,
        revision: u64,
        original: Option<PathBuf>,
        slot: Option<EntityId>,
    },
    Discover,
    Restore(EntityId),
}
pub enum FileResult {
    Saved(Result<SavedProject, StorageError>),
    Loaded(Result<Box<LoadedProject>, StorageError>),
    Snapshot(Result<RecoveryEntry, StorageError>),
    Discover(Result<RecoveryListing, StorageError>),
}
pub struct FileReply {
    pub id: u64,
    pub result: FileResult,
}
pub struct FileWorker {
    sender: Option<SyncSender<(u64, FileRequest)>>,
    receiver: Receiver<FileReply>,
    next: u64,
}
impl FileWorker {
    pub fn new(mut storage: Box<dyn DocumentStorage>) -> Result<Self, String> {
        let (send, receive) = mpsc::sync_channel::<(u64, FileRequest)>(4);
        let (reply, replies) = mpsc::sync_channel(4);
        std::thread::Builder::new()
            .name("multicad-files".into())
            .spawn(move || {
                while let Ok((id, request)) = receive.recv() {
                    let result = match request {
                        FileRequest::Save {
                            path,
                            project,
                            mode,
                        } => FileResult::Saved(storage.save_project(&path, &project, mode)),
                        FileRequest::Open(path) => {
                            FileResult::Loaded(storage.open_project(&path).map(Box::new))
                        }
                        FileRequest::Import(path) => {
                            FileResult::Loaded(storage.import_source(&path).map(Box::new))
                        }
                        FileRequest::Snapshot {
                            project,
                            revision,
                            original,
                            slot,
                        } => FileResult::Snapshot(match slot {
                            Some(slot) => storage.write_recovery_slot(
                                &project,
                                revision,
                                original.as_deref(),
                                slot,
                            ),
                            None => storage.write_recovery(&project, revision, original.as_deref()),
                        }),
                        FileRequest::Discover => FileResult::Discover(storage.recoveries()),
                        FileRequest::Restore(id) => {
                            FileResult::Loaded(storage.restore_recovery(id).map(Box::new))
                        }
                    };
                    if reply.send(FileReply { id, result }).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            sender: Some(send),
            receiver: replies,
            next: 0,
        })
    }
    pub fn submit(&mut self, request: FileRequest) -> Result<u64, String> {
        let id = self
            .next
            .checked_add(1)
            .ok_or("File request generation limit reached")?;
        self.sender
            .as_ref()
            .ok_or("File worker is closed")?
            .try_send((id, request))
            .map_err(|e| e.to_string())?;
        self.next = id;
        Ok(id)
    }
    pub fn poll(&self) -> Result<Option<FileReply>, String> {
        match self.receiver.try_recv() {
            Ok(reply) => Ok(Some(reply)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err("File worker stopped".into()),
        }
    }
}
impl Drop for FileWorker {
    fn drop(&mut self) {
        self.sender.take();
    }
}

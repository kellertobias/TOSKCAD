//! A private Session for one scoped project file, with no window.
use crate::host::{Host, Job, Scope, Target};
use multicad_core::{
    commands::Session,
    control::ControlLedger,
    document::Document,
    project_file::{ProjectFile, ProjectResources},
};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

/// Native project persistence. The application supplies its atomic,
/// conflict-checked store so headless saves match desktop saves.
pub trait Storage: Send {
    fn open(&mut self, path: &Path) -> Result<ProjectFile, String>;
    fn save(&mut self, path: &Path, project: &ProjectFile) -> Result<(), String>;
}

pub struct Workspace {
    session: Session,
    resources: ProjectResources,
    path: Option<PathBuf>,
    storage: Option<Box<dyn Storage>>,
    /// Headless clients are trusted: there is no reviewer, but every request
    /// is still audited.
    ledger: ControlLedger,
}
impl Workspace {
    /// An unsaved document; `save_project` is unavailable.
    pub fn new(document: Document) -> Result<Self, String> {
        Ok(Self {
            session: Session::new(document).map_err(|e| e.to_string())?,
            resources: ProjectResources::default(),
            path: None,
            storage: None,
            ledger: ControlLedger::default(),
        })
    }
    /// Open `path`, or start a new document saved there on first save.
    pub fn project(path: PathBuf, mut storage: Box<dyn Storage>) -> Result<Self, String> {
        let (document, resources) = if path.exists() {
            let file = storage.open(&path)?;
            (file.document, file.resources)
        } else {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("Untitled");
            (
                Document::new(name).map_err(|e| e.to_string())?,
                ProjectResources::default(),
            )
        };
        Ok(Self {
            session: Session::new(document).map_err(|e| e.to_string())?,
            resources,
            path: Some(path),
            storage: Some(storage),
            ledger: ControlLedger::default(),
        })
    }
}
impl Target for Workspace {
    fn session(&self) -> &Session {
        &self.session
    }
    fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }
    fn resources(&self) -> &ProjectResources {
        &self.resources
    }
    fn project_path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    fn control(&mut self) -> (&mut Session, &mut ControlLedger) {
        (&mut self.session, &mut self.ledger)
    }
    fn ledger(&self) -> &ControlLedger {
        &self.ledger
    }
    fn save(&mut self) -> Result<PathBuf, String> {
        let (Some(path), Some(storage)) = (self.path.as_ref(), self.storage.as_mut()) else {
            return Err("This server has no project file in scope".into());
        };
        let project = ProjectFile {
            document: self.session.document().clone(),
            resources: self.resources.clone(),
            recovery: None,
        };
        storage.save(path, &project)?;
        self.session
            .mark_saved(self.session.revision())
            .map_err(|e| e.to_string())?;
        Ok(path.clone())
    }
}

pub struct Headless {
    workspace: Mutex<Workspace>,
    scope: Scope,
}
impl Headless {
    pub fn new(workspace: Workspace, export_directory: Option<PathBuf>, read_only: bool) -> Self {
        let scope = Scope {
            mode: "headless",
            transport: "stdio",
            project: workspace.path.clone(),
            export_directory,
            read_only,
        };
        Self {
            workspace: Mutex::new(workspace),
            scope,
        }
    }
    /// The canonical document, for deterministic comparison in tests.
    pub fn document(&self) -> Document {
        self.workspace
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .session
            .document()
            .clone()
    }
}
impl Host for Headless {
    fn scope(&self) -> Scope {
        self.scope.clone()
    }
    fn submit(&self, job: Job) -> Result<(), String> {
        let mut workspace = self
            .workspace
            .lock()
            .map_err(|_| "The headless workspace is poisoned by an earlier failure")?;
        job(Ok(&mut *workspace));
        Ok(())
    }
}

//! File-side half of project references (CAD-185): open a referenced `.tcd`,
//! pin its native-checked content, place it, and look for the source file
//! again to report whether a reference is current, changed or missing.
//! The source contract lives in `multicad_core::symbols::project`.
use crate::Compiler;
use multicad_core::{
    commands::{Actor, CommandError, ErrorCode, Session},
    document::{Document, EntityId, Placement},
    project_file,
    symbols::{
        self, ProjectReferenceRequest, ProjectSourceCheck, ProjectSourceState, SymbolDefinition,
        SymbolPrepared,
    },
};
use std::path::{Path, PathBuf};

fn failed(message: impl Into<String>) -> CommandError {
    CommandError {
        code: ErrorCode::InvalidRequest,
        message: message.into(),
        involved: vec![],
        parameter: None,
    }
}

/// Open a native project with full validation, as the window's Open does.
pub fn open_project(path: &Path) -> Result<Document, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
    project_file::read(
        &mut file,
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .map(|opened| opened.project.document)
    .map_err(|e| format!("Cannot read {}: {e}", path.display()))
}

/// Pin a project's physical content. The project must evaluate natively;
/// tool bodies its Booleans consume are not referenced as separate parts.
pub fn reference_content(source: &Document) -> Result<SymbolDefinition, String> {
    let model = Compiler::default()
        .compile(source, 0)
        .map_err(|e| format!("{} does not evaluate: {e}", source.name()))?;
    symbols::project_definition(source, &model.snapshot().consumed_bodies)
        .map_err(|e| e.to_string())
}

/// The path to record for `source_file`: relative to the referencing file's
/// folder when there is one and both share a root, otherwise the bare file
/// name (found again next to the project or in a bundled directory).
pub fn stored_path(referencing_file: Option<&Path>, source_file: &Path) -> Result<String, String> {
    let name = source_file
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("{} has no usable file name", source_file.display()))?
        .to_string();
    let relative = referencing_file.and_then(Path::parent).and_then(|dir| {
        let dir = std::fs::canonicalize(dir).ok()?;
        let target = std::fs::canonicalize(source_file).ok()?;
        symbols::relative_path(&dir, &target)
    });
    let path = relative.unwrap_or(name);
    symbols::validate_path(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Prepare placing a reference to `source_file`, one transaction.
pub fn prepare_reference_file(
    session: &Session,
    revision: u64,
    actor: Actor,
    referencing_file: Option<&Path>,
    source_file: &Path,
    name: impl Into<String>,
    placement: Placement,
) -> Result<SymbolPrepared, CommandError> {
    let source = open_project(source_file).map_err(failed)?;
    let content = reference_content(&source).map_err(failed)?;
    let path = stored_path(referencing_file, source_file).map_err(failed)?;
    let request = ProjectReferenceRequest::new(content, &source, path, name, placement)?;
    symbols::prepare_project_reference(session, revision, actor, &request)
}

/// Prepare an explicit update or relocation of reference `record` from the
/// project at `source_file`.
pub fn prepare_reference_refresh(
    session: &Session,
    revision: u64,
    actor: Actor,
    record: EntityId,
    referencing_file: Option<&Path>,
    source_file: &Path,
) -> Result<SymbolPrepared, CommandError> {
    let source = open_project(source_file).map_err(failed)?;
    let content = reference_content(&source).map_err(failed)?;
    let path = stored_path(referencing_file, source_file).map_err(failed)?;
    symbols::prepare_project_update(session, revision, actor, record, &content, &source, &path)
}

/// Look for every reference's source file. The first candidate holding the
/// same project decides; otherwise the first candidate that exists is
/// reported as another project or unreadable; otherwise the source is
/// missing. Nothing in the document changes.
pub fn check_references(
    document: &Document,
    referencing_file: Option<&Path>,
    bundled: &[PathBuf],
) -> Result<Vec<ProjectSourceCheck>, String> {
    let dir = referencing_file.and_then(Path::parent);
    let mut checks = vec![];
    for (record, _) in symbols::project_sources(document).map_err(|e| e.to_string())? {
        let mut check =
            symbols::pending_check(document, record, dir, bundled).map_err(|e| e.to_string())?;
        let mut fallback: Option<(PathBuf, ProjectSourceState)> = None;
        for candidate in check.searched.clone() {
            if !candidate.is_file() {
                continue;
            }
            let state = match open_project(&candidate) {
                Err(message) => ProjectSourceState::Unreadable { message },
                Ok(found) => {
                    let content = reference_content(&found);
                    symbols::classify_source(
                        document,
                        record,
                        &found,
                        content.as_ref().map_err(Clone::clone),
                    )
                    .map_err(|e| e.to_string())?
                }
            };
            if matches!(
                state,
                ProjectSourceState::Current | ProjectSourceState::Updated { .. }
            ) {
                fallback = Some((candidate, state));
                break;
            }
            if fallback.is_none() {
                fallback = Some((candidate, state));
            }
        }
        if let Some((found, state)) = fallback {
            check.found = Some(found);
            check.state = state;
        }
        checks.push(check);
    }
    Ok(checks)
}

/// Environment override for the installed examples folder, for packaging
/// tests and portable layouts. Bundling itself belongs to CAD-186.
pub const EXAMPLES_DIR_ENV: &str = "MULTICAD_EXAMPLES_DIR";

/// Where an installation keeps its bundled example projects, relative to
/// the executable: `MultiCAD.app/Contents/Resources/examples` on macOS and
/// `resources/examples` next to the executable in the Windows and Linux
/// archive layout (`tools/native-package.mjs`). Never a source checkout.
pub fn installed_example_dirs(executable: &Path) -> Vec<PathBuf> {
    let Some(dir) = executable.parent() else {
        return vec![];
    };
    vec![
        dir.join("../Resources/examples"),
        dir.join("resources/examples"),
    ]
}

/// Existing bundled example folders for the running application: the
/// [`EXAMPLES_DIR_ENV`] override first, then the installed layout.
pub fn bundled_example_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![];
    if let Some(dir) = std::env::var_os(EXAMPLES_DIR_ENV).filter(|d| !d.is_empty()) {
        dirs.push(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe() {
        dirs.extend(installed_example_dirs(&exe));
    }
    dirs.retain(|d| d.is_dir());
    dirs
}

/// The projects a native project file references, as their stored paths
/// relative to that file's folder (with `/` separators). This is the hook
/// the bundled Examples use to copy an example's referenced projects next to
/// the user's copy (`multicad_ui::examples::DependencyResolver`).
pub fn referenced_files(project: &Path) -> Result<Vec<PathBuf>, String> {
    let document = open_project(project)?;
    let mut files = symbols::project_sources(&document)
        .map_err(|e| format!("{}: {e}", project.display()))?
        .into_iter()
        .map(|(_, source)| source.path.split('/').collect::<PathBuf>())
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();
    Ok(files)
}

//! `multicad --mcp`: a headless MCP server over stdio for one scoped project.
use multicad_core::project_file::ProjectFile;
use multicad_mcp::{Headless, Storage, Workspace};
use multicad_ui::{
    application::DocumentStorage,
    storage::{FileStamp, SaveMode},
};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, Default, PartialEq)]
pub struct Options {
    pub project: Option<PathBuf>,
    pub export_directory: Option<PathBuf>,
    pub read_only: bool,
    /// Bridge to the open desktop window instead of a headless document.
    pub app: bool,
    pub socket: Option<PathBuf>,
}
pub fn parse(arguments: &[OsString]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut rest = arguments.iter();
    while let Some(argument) = rest.next() {
        let mut value = |name: &str| {
            rest.next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("{name} needs a path"))
        };
        match argument.to_str() {
            Some("--project") if options.project.is_none() => {
                options.project = Some(value("--project")?)
            }
            Some("--export-dir") if options.export_directory.is_none() => {
                options.export_directory = Some(value("--export-dir")?)
            }
            Some("--read-only") if !options.read_only => options.read_only = true,
            Some("--app") if !options.app => options.app = true,
            Some("--socket") if options.socket.is_none() => {
                options.socket = Some(value("--socket")?)
            }
            _ => {
                return Err(format!(
                    "unsupported --mcp argument {:?}; see --help",
                    argument.to_string_lossy()
                ));
            }
        }
    }
    if options.app
        && (options.project.is_some() || options.export_directory.is_some() || options.read_only)
    {
        return Err("--app controls the open window; --project, --export-dir and --read-only apply to headless documents".into());
    }
    if options.socket.is_some() && !options.app {
        return Err("--socket requires --app".into());
    }
    if let Some(project) = &options.project
        && !multicad_core::project_path::is_project_path(project)
    {
        return Err("--project must name a .tcd (or legacy .multicad) file".into());
    }
    Ok(options)
}

/// The desktop's atomic, conflict-checked native store. A file changed by
/// another writer since this server opened or last saved it is not replaced.
struct NativeStorage {
    store: crate::files::ProjectStore,
    stamp: Option<FileStamp>,
}
impl Storage for NativeStorage {
    fn open(&mut self, path: &Path) -> Result<ProjectFile, String> {
        let loaded = self.store.open_project(path).map_err(|e| e.to_string())?;
        self.stamp = loaded.stamp;
        Ok(loaded.file)
    }
    fn save(&mut self, path: &Path, project: &ProjectFile) -> Result<(), String> {
        let mode = self.stamp.map_or(SaveMode::Create, SaveMode::Replace);
        let saved = self
            .store
            .save_project(path, project, mode)
            .map_err(|e| e.to_string())?;
        self.stamp = Some(saved.stamp);
        Ok(())
    }
}

pub fn run(options: Options) -> Result<(), String> {
    if options.app {
        let socket = match options.socket {
            Some(socket) => socket,
            None => crate::app_control::default_endpoint()?,
        };
        #[cfg(unix)]
        return multicad_mcp::local_socket::bridge_stdio(&socket);
        #[cfg(not(unix))]
        return Err(format!(
            "Window control is unavailable here ({})",
            socket.display()
        ));
    }
    // stdout carries JSON-RPC; native diagnostics must not interleave with it.
    multicad_kernel::route_native_output_to_stderr();
    let export_directory = options
        .export_directory
        .map(|d| {
            std::fs::canonicalize(&d)
                .ok()
                .filter(|d| d.is_dir())
                .ok_or_else(|| format!("Export folder {} does not exist", d.display()))
        })
        .transpose()?;
    let workspace = match options.project {
        Some(path) => Workspace::project(
            std::path::absolute(&path).map_err(|e| e.to_string())?,
            Box::new(NativeStorage {
                store: crate::files::ProjectStore::default(),
                stamp: None,
            }),
        )?,
        None => Workspace::new(
            multicad_core::document::Document::new("Untitled").map_err(|e| e.to_string())?,
        )?,
    };
    let host = Headless::new(workspace, export_directory, options.read_only);
    multicad_mcp::run_stdio(Arc::new(host))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }
    #[test]
    fn scope_options_are_explicit_and_unique() {
        assert_eq!(parse(&[]).unwrap(), Options::default());
        let parsed = parse(&args(&[
            "--project",
            "a.multicad",
            "--export-dir",
            "out",
            "--read-only",
        ]))
        .unwrap();
        assert_eq!(parsed.project, Some(PathBuf::from("a.multicad")));
        assert_eq!(parsed.export_directory, Some(PathBuf::from("out")));
        assert!(parsed.read_only);
        assert!(parse(&args(&["--project"])).is_err());
        assert!(parse(&args(&["--project", "a.json"])).is_err());
        assert!(
            parse(&args(&[
                "--project",
                "a.multicad",
                "--project",
                "b.multicad"
            ]))
            .is_err()
        );
        assert!(parse(&args(&["--listen", "0.0.0.0:80"])).is_err());
        let app = parse(&args(&["--app", "--socket", "/tmp/s"])).unwrap();
        assert!(app.app && app.socket == Some(PathBuf::from("/tmp/s")));
        assert!(parse(&args(&["--app", "--project", "a.multicad"])).is_err());
        assert!(parse(&args(&["--socket", "/tmp/s"])).is_err());
    }
    #[test]
    fn project_scope_accepts_tcd_in_any_case_and_legacy_multicad() {
        for name in ["a.tcd", "A.TCD", "dir/b.Tcd", "legacy.multicad"] {
            let parsed = parse(&args(&["--project", name])).unwrap();
            assert_eq!(parsed.project, Some(PathBuf::from(name)));
        }
        for name in ["a.tcd.json", "a", "a.step"] {
            let error = parse(&args(&["--project", name])).unwrap_err();
            assert!(error.contains(".tcd"), "{error}");
        }
    }
}

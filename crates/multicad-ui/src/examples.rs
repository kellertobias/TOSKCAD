//! Bundled example projects (CAD-186).
//!
//! The application ships a read-only `examples/` folder: top-level `.tcd`
//! projects, an `examples.json` manifest and any files they depend on. The
//! folder is found next to the executable (macOS bundle Resources, or the
//! `resources/` folder of the Windows/Linux archive), or in the repository
//! during debug development. Opening an example never edits the bundled file:
//! it is copied, with its dependencies, into a user-owned folder first.
use multicad_core::project_file::{self, FileLimits, ProjectCard};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    io::BufReader,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

/// File name of the manifest inside the bundled examples folder.
pub const MANIFEST: &str = "examples.json";
/// `format` value of the manifest.
pub const MANIFEST_FORMAT: &str = "toskcad.examples";
/// Folder created in the user's documents for editable example copies.
pub const COPIES_FOLDER: &str = "ToskCAD Examples";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const REINSTALL: &str = "Reinstall ToskCAD to restore its bundled examples.";

/// Returns the bundled files (relative to the examples folder) that the given
/// example file references, for example the projects an apartment places.
/// Cross-file project references install this hook; without it only the
/// manifest's `requires` lists are followed.
pub type DependencyResolver = Arc<dyn Fn(&Path) -> Result<Vec<PathBuf>, String> + Send + Sync>;

/// Where examples come from and where user copies go.
#[derive(Clone)]
pub struct ExampleLibrary {
    /// The bundled examples folder, or `None` when none was found.
    pub bundle: Option<PathBuf>,
    /// The folders that were searched, for the not-found message.
    pub searched: Vec<PathBuf>,
    /// The user-owned folder that receives editable copies.
    pub copies: PathBuf,
    pub resolver: Option<DependencyResolver>,
}

/// One gallery entry.
#[derive(Clone, Debug)]
pub struct ExampleEntry {
    /// Path relative to the examples folder, with `/` separators.
    pub file: String,
    pub path: PathBuf,
    /// Further bundled files this example needs, relative to the folder.
    pub requires: Vec<String>,
    /// Canonical name, description and preview from the file itself.
    pub card: Result<ProjectCard, String>,
}
impl ExampleEntry {
    pub fn title(&self) -> String {
        match &self.card {
            Ok(card) => card.info.name.clone(),
            Err(_) => self.file.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExampleCatalog {
    /// Canonical examples folder.
    pub root: PathBuf,
    pub entries: Vec<ExampleEntry>,
}

/// Candidate examples folders for an executable, most specific first.
pub fn candidates(executable: &Path) -> Vec<PathBuf> {
    let Some(directory) = executable.parent() else {
        return vec![];
    };
    let mut found = vec![
        // macOS: MultiCAD.app/Contents/MacOS/multicad → Contents/Resources.
        directory.join("../Resources/examples"),
        // Windows and Linux archives: resources/ beside the executable.
        directory.join("resources/examples"),
    ];
    if cfg!(debug_assertions) {
        // Development runs from the repository's target folder. Release
        // builds never embed this checkout path.
        found.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples"));
    }
    found
}

/// The first candidate that holds an examples manifest, plus every folder
/// that was searched.
pub fn locate(executable: &Path) -> (Option<PathBuf>, Vec<PathBuf>) {
    let searched = candidates(executable);
    let found = searched
        .iter()
        .find(|c| c.join(MANIFEST).is_file())
        .and_then(|c| c.canonicalize().ok());
    (found, searched)
}

/// Locate the bundled examples for the running executable.
pub fn locate_bundled() -> (Option<PathBuf>, Vec<PathBuf>) {
    match std::env::current_exe() {
        Ok(exe) => locate(&exe.canonicalize().unwrap_or(exe)),
        Err(_) => (None, vec![]),
    }
}

/// The user's documents folder, used for editable example copies.
pub fn documents_folder() -> Option<PathBuf> {
    let home = if cfg!(target_os = "windows") {
        std::env::var_os("USERPROFILE")
    } else {
        std::env::var_os("HOME")
    }
    .map(PathBuf::from)
    .filter(|p| p.is_absolute())?;
    if cfg!(target_os = "linux")
        && let Some(xdg) = std::env::var_os("XDG_DOCUMENTS_DIR").map(PathBuf::from)
        && xdg.is_absolute()
    {
        return Some(xdg);
    }
    let documents = home.join("Documents");
    Some(if documents.is_dir() { documents } else { home })
}

/// Default folder for example copies: `<Documents>/ToskCAD Examples`.
pub fn default_copies_folder() -> Option<PathBuf> {
    documents_folder().map(|d| d.join(COPIES_FOLDER))
}

fn relative(value: &str, field: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\\')
        || !path.components().all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(format!(
            "{MANIFEST}: {field} \"{value}\" must be a relative path inside the examples folder"
        ));
    }
    Ok(path.to_path_buf())
}

/// A checked manifest entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestEntry {
    pub file: String,
    pub requires: Vec<String>,
}

/// Parse and check the manifest text. Paths are not checked against disk.
pub fn parse_manifest(bytes: &[u8]) -> Result<Vec<ManifestEntry>, String> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("{MANIFEST} is not valid JSON: {e}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("{MANIFEST} must be a JSON object"))?;
    for key in object.keys() {
        if !matches!(key.as_str(), "format" | "version" | "examples") {
            return Err(format!("{MANIFEST}: unknown field \"{key}\""));
        }
    }
    if object.get("format").and_then(Value::as_str) != Some(MANIFEST_FORMAT)
        || object.get("version").and_then(Value::as_u64) != Some(1)
    {
        return Err(format!(
            "{MANIFEST}: expected format \"{MANIFEST_FORMAT}\" version 1"
        ));
    }
    let list = object
        .get("examples")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{MANIFEST}: \"examples\" must be an array"))?;
    let mut seen = BTreeSet::new();
    let mut entries = Vec::new();
    for item in list {
        let item = item
            .as_object()
            .ok_or_else(|| format!("{MANIFEST}: each example must be an object"))?;
        for key in item.keys() {
            if !matches!(key.as_str(), "file" | "requires") {
                return Err(format!("{MANIFEST}: unknown example field \"{key}\""));
            }
        }
        let file = item
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{MANIFEST}: each example needs a \"file\""))?;
        let path = relative(file, "file")?;
        if path.components().count() != 1
            || !multicad_core::project_path::is_current_project_path(&path)
        {
            return Err(format!(
                "{MANIFEST}: example \"{file}\" must be a .tcd file at the top of the examples folder"
            ));
        }
        if !seen.insert(file.to_string()) {
            return Err(format!("{MANIFEST}: example \"{file}\" is listed twice"));
        }
        let mut requires = Vec::new();
        if let Some(list) = item.get("requires") {
            for value in list
                .as_array()
                .ok_or_else(|| format!("{MANIFEST}: \"requires\" of {file} must be an array"))?
            {
                let value = value.as_str().ok_or_else(|| {
                    format!("{MANIFEST}: \"requires\" of {file} must list file names")
                })?;
                relative(value, "requires")?;
                if value == file {
                    return Err(format!("{MANIFEST}: {file} cannot require itself"));
                }
                requires.push(value.to_string());
            }
        }
        entries.push(ManifestEntry {
            file: file.to_string(),
            requires,
        });
    }
    Ok(entries)
}

fn read_card(path: &Path) -> Result<ProjectCard, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    project_file::card(
        &mut BufReader::new(file),
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )
}

/// Read the bundled catalog: manifest entries in manifest order, then any
/// unlisted top-level `.tcd` files by name. A missing or unreadable example
/// keeps its entry with an actionable error.
pub fn load(root: &Path) -> Result<ExampleCatalog, String> {
    let root = root.canonicalize().map_err(|e| {
        format!(
            "The bundled examples folder {} cannot be opened ({e}). {REINSTALL}",
            root.display()
        )
    })?;
    let manifest_path = root.join(MANIFEST);
    let metadata = fs::metadata(&manifest_path).map_err(|e| {
        format!(
            "The bundled examples manifest {} is missing ({e}). {REINSTALL}",
            manifest_path.display()
        )
    })?;
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err(format!("{MANIFEST} exceeds 64 KiB. {REINSTALL}"));
    }
    let bytes = fs::read(&manifest_path).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let listed = parse_manifest(&bytes).map_err(|e| format!("{e}. {REINSTALL}"))?;
    let mut unlisted = fs::read_dir(&root)
        .map_err(|e| format!("{}: {e}", root.display()))?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|name| multicad_core::project_path::is_current_project_path(Path::new(name)))
        .filter(|name| !listed.iter().any(|l| &l.file == name))
        .collect::<Vec<_>>();
    unlisted.sort();
    let entries = listed
        .into_iter()
        .chain(unlisted.into_iter().map(|file| ManifestEntry {
            file,
            requires: vec![],
        }))
        .map(|entry| {
            let path = root.join(&entry.file);
            let card = if !path.is_file() {
                Err(format!(
                    "The example {} is missing from {}. {REINSTALL}",
                    entry.file,
                    root.display()
                ))
            } else {
                read_card(&path).map_err(|e| {
                    format!(
                        "The example {} cannot be read: {e}. {REINSTALL}",
                        entry.file
                    )
                })
            };
            ExampleEntry {
                file: entry.file,
                path,
                requires: entry.requires,
                card,
            }
        })
        .collect();
    Ok(ExampleCatalog { root, entries })
}

impl ExampleCatalog {
    /// True when `path` names a file inside the bundled examples folder, so
    /// that it must never be written by Save or Save As.
    pub fn protects(&self, path: &Path) -> bool {
        protects(&self.root, path)
    }
    /// The gallery entry for a bundled file, if `path` is one.
    pub fn entry_for(&self, path: &Path) -> Option<usize> {
        let canonical = path.canonicalize().ok()?;
        self.entries
            .iter()
            .position(|e| e.path.canonicalize().is_ok_and(|p| p == canonical))
    }
    /// Every bundled file `entry` needs, transitively, excluding itself.
    pub fn dependencies(
        &self,
        index: usize,
        resolver: Option<&DependencyResolver>,
    ) -> Result<Vec<String>, String> {
        let entry = &self.entries[index];
        let mut pending = vec![entry.file.clone()];
        let mut seen = BTreeSet::from([entry.file.clone()]);
        let mut out = Vec::new();
        while let Some(file) = pending.pop() {
            let mut next: Vec<String> = self
                .entries
                .iter()
                .find(|e| e.file == file)
                .map(|e| e.requires.clone())
                .unwrap_or_default();
            if let Some(resolver) = resolver
                && multicad_core::project_path::is_current_project_path(Path::new(&file))
            {
                for path in resolver(&self.root.join(&file))? {
                    let text = path.to_str().ok_or_else(|| {
                        format!("{file} references a file whose name is not UTF-8")
                    })?;
                    next.push(text.replace('\\', "/"));
                }
            }
            for dependency in next {
                relative(&dependency, "reference")?;
                if !self.root.join(&dependency).is_file() {
                    return Err(format!(
                        "The example {} needs {dependency}, which is missing from the bundled examples. {REINSTALL}",
                        entry.file
                    ));
                }
                if seen.insert(dependency.clone()) {
                    out.push(dependency.clone());
                    pending.push(dependency);
                }
            }
        }
        Ok(out)
    }
}

/// True when `path` (existing or not) lies inside `root`.
pub fn protects(root: &Path, path: &Path) -> bool {
    let canonical = path.canonicalize().ok().or_else(|| {
        let parent = path.parent()?.canonicalize().ok()?;
        Some(parent.join(path.file_name()?))
    });
    canonical.is_some_and(|p| p.starts_with(root))
}

/// The result of preparing an editable copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExampleCopy {
    /// The user's copy of the chosen example, ready to open.
    pub path: PathBuf,
    /// True when an unchanged earlier copy was reused instead of written.
    pub reused: bool,
    /// Dependencies copied now, relative to the copies folder.
    pub copied_dependencies: Vec<String>,
}

fn copy_file(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
    }
    let name = to
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = to.with_file_name(format!(".{name}.{}.partial", std::process::id()));
    let result = (|| {
        fs::copy(from, &temporary)?;
        // Installed resources can be read-only; the user's copy is editable.
        let mut permissions = fs::metadata(&temporary)?.permissions();
        #[allow(
            clippy::permissions_set_readonly_false,
            reason = "the copy belongs to the user and must be editable"
        )]
        permissions.set_readonly(false);
        fs::set_permissions(&temporary, permissions)?;
        if to.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "a file appeared at the destination",
            ));
        }
        fs::rename(&temporary, to)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Cannot copy {} to {}: {error}",
            from.display(),
            to.display()
        ));
    }
    Ok(())
}

fn same_bytes(a: &Path, b: &Path) -> bool {
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(x), Ok(y)) if x.len() == y.len() => {
            matches!((fs::read(a), fs::read(b)), (Ok(x), Ok(y)) if x == y)
        }
        _ => false,
    }
}

/// Copy example `index` and the bundled files it needs into `copies`.
///
/// Existing user files are never replaced: a dependency that already exists
/// is kept as the user left it, and an edited earlier copy of the example
/// itself is kept while a new numbered copy (`name 2.tcd`) is written. An
/// unchanged earlier copy is reused.
pub fn instantiate(
    catalog: &ExampleCatalog,
    index: usize,
    copies: &Path,
    resolver: Option<&DependencyResolver>,
) -> Result<ExampleCopy, String> {
    let entry = catalog
        .entries
        .get(index)
        .ok_or("That example is no longer listed")?;
    if let Err(error) = &entry.card {
        return Err(error.clone());
    }
    if protects(&catalog.root, copies) {
        return Err("Example copies cannot be stored inside the bundled examples folder".into());
    }
    let dependencies = catalog.dependencies(index, resolver)?;
    fs::create_dir_all(copies).map_err(|e| {
        format!(
            "Cannot create the example copies folder {}: {e}",
            copies.display()
        )
    })?;
    let mut copied_dependencies = Vec::new();
    for dependency in dependencies {
        let target = copies.join(&dependency);
        if !target.exists() {
            copy_file(&catalog.root.join(&dependency), &target)?;
            copied_dependencies.push(dependency);
        }
    }
    let target = copies.join(&entry.file);
    if target.exists() && same_bytes(&target, &entry.path) {
        return Ok(ExampleCopy {
            path: target,
            reused: true,
            copied_dependencies,
        });
    }
    let path = if target.exists() {
        let stem = Path::new(&entry.file)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let extension = Path::new(&entry.file)
            .extension()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "tcd".into());
        (2..10_000)
            .map(|n| copies.join(format!("{stem} {n}.{extension}")))
            .find(|p| !p.exists())
            .ok_or("Too many copies of this example; remove some first")?
    } else {
        target
    };
    copy_file(&entry.path, &path)?;
    Ok(ExampleCopy {
        path,
        reused: false,
        copied_dependencies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository_examples() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
    }
    fn fixture() -> (tempfile_dir::Dir, PathBuf) {
        let dir = tempfile_dir::Dir::new("examples-fixture");
        let root = dir.path().join("bundle");
        fs::create_dir_all(&root).unwrap();
        for entry in fs::read_dir(repository_examples()).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
            }
        }
        (dir, root)
    }
    /// A minimal self-removing temporary folder (no extra dependency).
    mod tempfile_dir {
        use std::path::{Path, PathBuf};
        pub struct Dir(PathBuf);
        impl Dir {
            pub fn new(name: &str) -> Self {
                let path = std::env::temp_dir().join(format!(
                    "multicad-{name}-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                std::fs::create_dir_all(&path).unwrap();
                Self(path.canonicalize().unwrap())
            }
            pub fn path(&self) -> &Path {
                &self.0
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
    pub(crate) use tempfile_dir::Dir as TempDir;

    #[test]
    fn repository_catalog_lists_every_shipped_demo_with_its_own_card() {
        let catalog = load(&repository_examples()).unwrap();
        let files: Vec<_> = catalog.entries.iter().map(|e| e.file.as_str()).collect();
        let mut on_disk: Vec<_> = fs::read_dir(repository_examples())
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.ends_with(".tcd"))
            .collect();
        on_disk.sort();
        let mut sorted = files.clone();
        sorted.sort();
        assert_eq!(sorted, on_disk, "every bundled .tcd is listed once");
        assert!(files.len() >= 2);
        for entry in &catalog.entries {
            let card = entry.card.as_ref().unwrap();
            assert!(!card.info.name.is_empty(), "{}", entry.file);
            assert!(!card.info.description.is_empty(), "{}", entry.file);
            assert!(
                card.info.preview.is_some(),
                "{} has a real preview",
                entry.file
            );
            assert!(
                catalog
                    .dependencies(
                        catalog
                            .entries
                            .iter()
                            .position(|e| e.file == entry.file)
                            .unwrap(),
                        None
                    )
                    .is_ok()
            );
        }
    }

    #[test]
    fn manifest_rejects_unsafe_or_unknown_content() {
        let ok = br#"{"format":"toskcad.examples","version":1,"examples":[{"file":"a.tcd","requires":["lib/rail.tcd"]}]}"#;
        assert_eq!(
            parse_manifest(ok).unwrap(),
            vec![ManifestEntry {
                file: "a.tcd".into(),
                requires: vec!["lib/rail.tcd".into()]
            }]
        );
        for (bad, needle) in [
            (&b"{"[..], "not valid JSON"),
            (br#"{"format":"x","version":1,"examples":[]}"#, "expected format"),
            (br#"{"format":"toskcad.examples","version":1,"examples":[],"extra":1}"#, "unknown field"),
            (br#"{"format":"toskcad.examples","version":1,"examples":[{"file":"../a.tcd"}]}"#, "relative path"),
            (br#"{"format":"toskcad.examples","version":1,"examples":[{"file":"/a.tcd"}]}"#, "relative path"),
            (br#"{"format":"toskcad.examples","version":1,"examples":[{"file":"sub/a.tcd"}]}"#, "top of the examples"),
            (br#"{"format":"toskcad.examples","version":1,"examples":[{"file":"a.step"}]}"#, ".tcd file"),
            (br#"{"format":"toskcad.examples","version":1,"examples":[{"file":"a.tcd"},{"file":"a.tcd"}]}"#, "listed twice"),
            (br#"{"format":"toskcad.examples","version":1,"examples":[{"file":"a.tcd","requires":["../x"]}]}"#, "relative path"),
        ] {
            let error = parse_manifest(bad).unwrap_err();
            assert!(error.contains(needle), "{error}");
        }
    }

    #[test]
    fn missing_and_malformed_examples_give_actionable_errors() {
        let (dir, root) = fixture();
        let missing = dir.path().join("nowhere");
        assert!(load(&missing).unwrap_err().contains("Reinstall ToskCAD"));
        fs::write(root.join("front-panel.tcd"), b"MCADPRJ\0garbage").unwrap();
        fs::remove_file(root.join("welded-workbench.tcd")).unwrap();
        let catalog = load(&root).unwrap();
        let panel = catalog
            .entries
            .iter()
            .find(|e| e.file == "front-panel.tcd")
            .unwrap();
        let error = panel.card.as_ref().unwrap_err();
        assert!(
            error.contains("cannot be read") && error.contains("Reinstall"),
            "{error}"
        );
        let bench = catalog
            .entries
            .iter()
            .find(|e| e.file == "welded-workbench.tcd")
            .unwrap();
        assert!(bench.card.as_ref().unwrap_err().contains("is missing"));
        let index = catalog
            .entries
            .iter()
            .position(|e| e.file == "front-panel.tcd")
            .unwrap();
        assert!(instantiate(&catalog, index, &dir.path().join("copies"), None).is_err());
        assert!(!dir.path().join("copies/front-panel.tcd").exists());
        fs::write(root.join(MANIFEST), b"{}").unwrap();
        assert!(load(&root).unwrap_err().contains("expected format"));
    }

    #[test]
    fn copies_never_replace_user_files_and_bring_their_dependencies() {
        let (dir, root) = fixture();
        // A synthetic apartment-like example that needs the workbench and an
        // asset in a subfolder.
        fs::copy(root.join("front-panel.tcd"), root.join("apartment.tcd")).unwrap();
        fs::create_dir_all(root.join("library")).unwrap();
        fs::write(root.join("library/rails.txt"), b"rail data").unwrap();
        fs::write(
            root.join(MANIFEST),
            br#"{"format":"toskcad.examples","version":1,"examples":[
                {"file":"welded-workbench.tcd"},
                {"file":"apartment.tcd","requires":["welded-workbench.tcd","library/rails.txt"]},
                {"file":"front-panel.tcd"}]}"#,
        )
        .unwrap();
        let catalog = load(&root).unwrap();
        let originals: Vec<_> = ["welded-workbench.tcd", "apartment.tcd", "front-panel.tcd"]
            .map(|f| fs::read(root.join(f)).unwrap())
            .to_vec();
        let copies = dir.path().join("Documents").join(COPIES_FOLDER);
        let apartment = catalog
            .entries
            .iter()
            .position(|e| e.file == "apartment.tcd")
            .unwrap();
        let first = instantiate(&catalog, apartment, &copies, None).unwrap();
        assert_eq!(first.path, copies.join("apartment.tcd"));
        assert!(!first.reused);
        assert_eq!(
            first.copied_dependencies,
            vec![
                "welded-workbench.tcd".to_string(),
                "library/rails.txt".into()
            ]
        );
        assert_eq!(
            fs::read(copies.join("library/rails.txt")).unwrap(),
            b"rail data"
        );
        // Unchanged copy: reused, nothing new written.
        let again = instantiate(&catalog, apartment, &copies, None).unwrap();
        assert!(again.reused && again.path == first.path && again.copied_dependencies.is_empty());
        // The user edits both their apartment and workbench copies.
        fs::write(copies.join("apartment.tcd"), b"user apartment").unwrap();
        fs::write(copies.join("welded-workbench.tcd"), b"user bench").unwrap();
        let third = instantiate(&catalog, apartment, &copies, None).unwrap();
        assert_eq!(third.path, copies.join("apartment 2.tcd"));
        assert_eq!(
            fs::read(copies.join("apartment.tcd")).unwrap(),
            b"user apartment"
        );
        assert_eq!(
            fs::read(copies.join("welded-workbench.tcd")).unwrap(),
            b"user bench"
        );
        // A resolver hook adds references found inside the file.
        let resolver: DependencyResolver = Arc::new(|path: &Path| {
            Ok(if path.ends_with("front-panel.tcd") {
                vec![PathBuf::from("library/rails.txt")]
            } else {
                vec![]
            })
        });
        let other = dir.path().join("other");
        let panel = catalog
            .entries
            .iter()
            .position(|e| e.file == "front-panel.tcd")
            .unwrap();
        let copy = instantiate(&catalog, panel, &other, Some(&resolver)).unwrap();
        assert_eq!(
            copy.copied_dependencies,
            vec!["library/rails.txt".to_string()]
        );
        let broken: DependencyResolver = Arc::new(|_: &Path| Ok(vec![PathBuf::from("absent.tcd")]));
        let error = instantiate(&catalog, panel, &dir.path().join("x"), Some(&broken)).unwrap_err();
        assert!(
            error.contains("absent.tcd") && error.contains("Reinstall"),
            "{error}"
        );
        // Bundled originals are untouched and protected.
        for (file, bytes) in ["welded-workbench.tcd", "apartment.tcd", "front-panel.tcd"]
            .iter()
            .zip(&originals)
        {
            assert_eq!(&fs::read(root.join(file)).unwrap(), bytes);
            assert!(catalog.protects(&root.join(file)));
        }
        assert!(catalog.protects(&root.join("new-name.tcd")));
        assert!(!catalog.protects(&copies.join("apartment.tcd")));
        assert!(instantiate(&catalog, panel, &root.join("inside"), None).is_err());
    }

    #[test]
    fn located_folders_follow_each_package_layout() {
        let dir = TempDir::new("examples-locate");
        let mac = dir.path().join("MultiCAD.app/Contents");
        fs::create_dir_all(mac.join("MacOS")).unwrap();
        fs::create_dir_all(mac.join("Resources/examples")).unwrap();
        fs::write(mac.join("Resources/examples").join(MANIFEST), b"{}").unwrap();
        let (found, _) = locate(&mac.join("MacOS/multicad"));
        assert_eq!(
            found.unwrap(),
            mac.join("Resources/examples").canonicalize().unwrap()
        );
        let archive = dir.path().join("archive");
        fs::create_dir_all(archive.join("resources/examples")).unwrap();
        fs::write(archive.join("resources/examples").join(MANIFEST), b"{}").unwrap();
        let (found, _) = locate(&archive.join("multicad.exe"));
        assert_eq!(
            found.unwrap(),
            archive.join("resources/examples").canonicalize().unwrap()
        );
        // Cargo's own target/<profile>/examples folder holds compiled
        // example binaries and must never be mistaken for the bundle.
        let target = dir.path().join("target/debug");
        fs::create_dir_all(target.join("examples")).unwrap();
        let (found, searched) = locate(&target.join("multicad"));
        assert!(!searched.contains(&target.join("examples")));
        // Debug test builds fall back to the repository folder.
        assert_eq!(
            found.unwrap(),
            repository_examples().canonicalize().unwrap()
        );
    }
}

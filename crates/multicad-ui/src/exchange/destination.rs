//! The export output folder: chosen in the native folder dialog or typed,
//! and checked before Run so a missing or read-only folder is reported in
//! words the user can act on. The native publisher still owns bundle
//! identity, stale-source checks and overwrite protection.
use std::path::Path;

/// Why `directory` cannot receive an export bundle, or `None` when it can.
/// Writability is checked by creating and removing one empty probe file.
pub fn diagnose(directory: &str) -> Option<String> {
    let directory = directory.trim();
    if directory.is_empty() {
        return Some("Choose an output folder, or type its absolute path.".into());
    }
    let path = Path::new(directory);
    if !path.is_absolute() {
        return Some(format!(
            "“{directory}” is not an absolute path. Choose the folder, or type a path starting at the filesystem root."
        ));
    }
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Some(format!(
                "The folder “{directory}” does not exist. Create it, or choose another folder."
            ));
        }
        Err(error) => {
            return Some(format!(
                "The folder “{directory}” cannot be read ({error}). Choose another folder."
            ));
        }
    };
    if !metadata.is_dir() {
        return Some(format!(
            "“{directory}” is a file, not a folder. Choose a folder."
        ));
    }
    let probe = path.join(format!(
        ".toskcad-write-check-{}",
        multicad_core::document::EntityId::new()
    ));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            None
        }
        Err(error) => Some(format!(
            "ToskCAD cannot write into “{directory}” ({error}). Choose a folder you can write to."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_checks_accept_one_writable_directory_and_leave_no_probes() {
        let directory = tempfile::tempdir().unwrap();
        let barrier = std::sync::Barrier::new(16);
        std::thread::scope(|scope| {
            for _ in 0..16 {
                let directory = directory.path();
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    for _ in 0..128 {
                        assert_eq!(diagnose(directory.to_str().unwrap()), None);
                    }
                });
            }
        });
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn missing_relative_file_and_read_only_destinations_are_explained() {
        struct Dir(std::path::PathBuf);
        impl Dir {
            fn path(&self) -> &Path {
                &self.0
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let dir = Dir(std::env::temp_dir().join(format!(
            "toskcad-destination-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
        std::fs::create_dir(dir.path()).unwrap();
        let root = dir.path().to_str().unwrap().to_owned();
        assert_eq!(diagnose(&root), None);
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "the probe leaves nothing behind"
        );
        assert!(diagnose("  ").unwrap().contains("Choose an output folder"));
        assert!(
            diagnose("exports")
                .unwrap()
                .contains("not an absolute path")
        );
        let missing = dir.path().join("missing");
        assert!(
            diagnose(missing.to_str().unwrap())
                .unwrap()
                .contains("does not exist")
        );
        let file = dir.path().join("file.txt");
        std::fs::write(&file, b"x").unwrap();
        assert!(
            diagnose(file.to_str().unwrap())
                .unwrap()
                .contains("not a folder")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let locked = dir.path().join("locked");
            std::fs::create_dir(&locked).unwrap();
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
            let result = diagnose(locked.to_str().unwrap());
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
            // A superuser may write anyway; otherwise the folder is refused.
            if let Some(problem) = result {
                assert!(problem.contains("cannot write"), "{problem}");
            }
        }
    }
}

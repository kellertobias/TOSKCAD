//! Production wiring and packaged verification for bundled examples (CAD-186).
use multicad_ui::{
    application::DocumentStorage,
    examples::{self, ExampleLibrary},
};
use std::path::{Path, PathBuf};

/// Bundled examples next to the executable; copies in the user's documents.
pub fn library() -> ExampleLibrary {
    let (bundle, searched) = examples::locate_bundled();
    let copies = examples::default_copies_folder().unwrap_or_else(|| {
        let recovery = crate::files::ProjectStore::default().recovery_directory;
        recovery
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or(recovery)
            .join(examples::COPIES_FOLDER)
    });
    ExampleLibrary {
        bundle,
        searched,
        copies,
        resolver: None,
    }
}

/// Open every bundled example through the production project store and
/// check that each listed dependency is present. Returns a short receipt.
pub fn verify(root: Option<&Path>, searched: &[PathBuf]) -> Result<String, String> {
    let root = root.ok_or_else(|| {
        format!(
            "No bundled examples folder was found (looked in {})",
            searched
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    let catalog = examples::load(root)?;
    if catalog.entries.is_empty() {
        return Err(format!("{} lists no example projects", root.display()));
    }
    let mut store = crate::files::ProjectStore::default();
    let mut names = Vec::new();
    for (index, entry) in catalog.entries.iter().enumerate() {
        let card = entry.card.as_ref().map_err(Clone::clone)?;
        if card.info.preview.is_none() || card.info.description.trim().is_empty() {
            return Err(format!(
                "The example {} has no saved description or preview",
                entry.file
            ));
        }
        store
            .open_project(&entry.path)
            .map_err(|e| format!("The example {} does not open: {e}", entry.file))?;
        catalog.dependencies(index, None)?;
        names.push(card.info.name.clone());
    }
    Ok(format!(
        "Bundled examples: {} opened ({}) from {}",
        names.len(),
        names.join(", "),
        root.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn development_examples_open_through_the_production_store() {
        let (root, searched) = examples::locate(&std::env::current_exe().unwrap());
        let receipt = verify(root.as_deref(), &searched).unwrap();
        assert!(receipt.contains("C-channel front panel"), "{receipt}");
        let error = verify(None, &[PathBuf::from("/nowhere/examples")]).unwrap_err();
        assert!(error.contains("/nowhere/examples"), "{error}");
    }
}

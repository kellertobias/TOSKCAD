// SPDX-License-Identifier: GPL-3.0-or-later

//! Native project file naming. New ToskCAD projects use `.tcd`; existing
//! `.multicad` projects keep opening and saving in place without renaming.
//! Only the file name changes: both extensions name the same container.
use std::path::{Path, PathBuf};

/// Extension written for every new native project.
pub const PROJECT_EXTENSION: &str = "tcd";
/// Earlier native project extension, still accepted for open and in-place save.
pub const LEGACY_PROJECT_EXTENSION: &str = "multicad";
/// Extensions a native project dialog lists. Some platforms filter by exact
/// case, so the upper-case spelling is listed explicitly.
pub const OPEN_EXTENSIONS: [&str; 4] = ["tcd", "TCD", "multicad", "MULTICAD"];

/// True for `.tcd` and legacy `.multicad`, compared case-insensitively.
pub fn is_project_path(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        e.eq_ignore_ascii_case(PROJECT_EXTENSION)
            || e.eq_ignore_ascii_case(LEGACY_PROJECT_EXTENSION)
    })
}

/// True only for the current `.tcd` extension (any case).
pub fn is_current_project_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(PROJECT_EXTENSION))
}

/// A file name for a new copy: `name.tcd`. A name already ending in `.tcd`
/// (any case) is kept; a legacy `.multicad` suffix is replaced instead of
/// gaining a second extension. Other dots in the name are preserved.
pub fn project_file_name(name: &str) -> String {
    let trimmed = name.trim();
    let base = if trimmed.is_empty() {
        "Untitled"
    } else {
        trimmed
    };
    if is_current_project_path(Path::new(base)) {
        return base.to_string();
    }
    let stem = if is_project_path(Path::new(base)) {
        &base[..base.len() - LEGACY_PROJECT_EXTENSION.len() - 1]
    } else {
        base
    };
    let stem = if stem.trim().is_empty() {
        "Untitled"
    } else {
        stem
    };
    format!("{stem}.{PROJECT_EXTENSION}")
}

/// Completes a path chosen in a save dialog: a missing extension becomes
/// `.tcd`; any existing extension (including `.TCD` or `.multicad`) is kept.
pub fn complete_save_path(mut path: PathBuf) -> PathBuf {
    if path.extension().is_none() {
        path.set_extension(PROJECT_EXTENSION);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_current_and_legacy_extensions_case_insensitively() {
        for accepted in [
            "a.tcd",
            "a.TCD",
            "a.Tcd",
            "/x/y/b.multicad",
            "B.MULTICAD",
            "c.d.tcd",
        ] {
            assert!(is_project_path(Path::new(accepted)), "{accepted}");
        }
        for rejected in ["a", "a.tcd.json", "a.step", "tcd", "a.mcad", ".tcd"] {
            assert!(!is_project_path(Path::new(rejected)), "{rejected}");
        }
        assert!(is_current_project_path(Path::new("x.TCD")));
        assert!(!is_current_project_path(Path::new("x.multicad")));
    }
    #[test]
    fn new_copies_get_one_tcd_extension() {
        assert_eq!(project_file_name("Cabinet"), "Cabinet.tcd");
        assert_eq!(project_file_name("Cabinet.tcd"), "Cabinet.tcd");
        assert_eq!(project_file_name("Cabinet.TCD"), "Cabinet.TCD");
        assert_eq!(project_file_name("Cabinet.multicad"), "Cabinet.tcd");
        assert_eq!(project_file_name("Cabinet.MultiCAD"), "Cabinet.tcd");
        assert_eq!(project_file_name("Rev 1.2"), "Rev 1.2.tcd");
        assert_eq!(project_file_name("  "), "Untitled.tcd");
        assert_eq!(project_file_name(".multicad"), ".multicad.tcd");
    }
    #[test]
    fn save_dialog_paths_only_gain_a_missing_extension() {
        assert_eq!(
            complete_save_path("/p/Box".into()),
            PathBuf::from("/p/Box.tcd")
        );
        for kept in ["/p/Box.tcd", "/p/Box.TCD", "/p/Box.multicad"] {
            assert_eq!(complete_save_path(kept.into()), PathBuf::from(kept));
        }
    }
}

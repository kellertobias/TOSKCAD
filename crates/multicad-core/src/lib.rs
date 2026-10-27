// SPDX-License-Identifier: GPL-3.0-or-later

//! Canonical source definitions and dependency identities. No GUI, solver IDs,
//! native pointers or tessellations are part of the document.

pub mod commands;
pub mod constraint_sketch;
pub mod document;
pub mod hierarchy;
pub mod parameters;
pub mod project_file;
pub mod references;
pub mod sketch;
pub mod sketch_draw;
pub mod sketch_pattern;
pub mod sketch_regions;
pub mod sketch_transform;
pub mod sketch_trim;

/// Identity of the compiled application, independent of GUI or native adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildInfo {
    pub product: &'static str,
    pub version: &'static str,
    pub target_os: &'static str,
    pub target_arch: &'static str,
    pub commit: Option<&'static str>,
}

impl BuildInfo {
    /// Return metadata for this compiled target, not the machine running Cargo.
    pub const fn current() -> Self {
        Self {
            product: "MultiCAD",
            version: match option_env!("MULTICAD_RELEASE_VERSION") {
                Some(version) => version,
                None => env!("CARGO_PKG_VERSION"),
            },
            target_os: std::env::consts::OS,
            target_arch: std::env::consts::ARCH,
            commit: option_env!("MULTICAD_BUILD_COMMIT"),
        }
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later

//! Kernel-neutral bootstrap metadata. The parametric document and geometry APIs
//! are separate planned work; this crate does not simulate either one.

/// Identity of the compiled application, independent of GUI or native adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildInfo {
    pub product: &'static str,
    pub version: &'static str,
    pub target_os: &'static str,
    pub target_arch: &'static str,
}

impl BuildInfo {
    /// Return metadata for this compiled target, not the machine running Cargo.
    pub const fn current() -> Self {
        Self {
            product: "MultiCAD",
            version: env!("CARGO_PKG_VERSION"),
            target_os: std::env::consts::OS,
            target_arch: std::env::consts::ARCH,
        }
    }
}

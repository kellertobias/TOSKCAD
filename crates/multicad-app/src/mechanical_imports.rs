//! Native STEP and KiCad reading for the window's import Inspector. The UI
//! calls these on its own worker threads; the live Session is never touched.
use multicad_core::{
    document::Document,
    model::imports::{KicadImport, StepFile},
};
use multicad_model::{
    Compiler,
    imports::{KicadOptions, read_kicad_file, read_step_file},
};
use multicad_ui::mechanical_import::MechanicalImportBackend;
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
pub struct Backend;

impl MechanicalImportBackend for Backend {
    fn read_step(&self, path: &Path) -> Result<StepFile, String> {
        read_step_file(path).map_err(|e| e.to_string())
    }
    /// The user chose the board in the native dialog, so its footprint models
    /// are looked up wherever KiCad would find them (`${KIPRJMOD}`, the
    /// environment), not only in one folder as for MCP.
    fn read_kicad(&self, path: &Path, min_hole_mm: f64) -> Result<KicadImport, String> {
        let options = KicadOptions {
            min_hole_mm,
            ..KicadOptions::default()
        };
        read_kicad_file(path, &options).map_err(|e| e.to_string())
    }
    fn verify(
        &self,
        document: &Document,
        revision: u64,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        let interrupt = || cancel.load(Ordering::Relaxed);
        Compiler::default()
            .compile_interruptible(document, revision, Some(&interrupt))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

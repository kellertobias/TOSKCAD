//! Native check for the window's staged reference repairs. The UI runs this
//! on its own worker thread; the live Session is never touched here.
use multicad_core::{
    commands::{Actor, Channel, PreparationSnapshot},
    sketch_external::RepairSet,
};
use multicad_model::{Compiler, GeometryBackend};
use multicad_ui::reference_repair_set::RepairSetCheck;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub fn check_with_geometry_backend(
    geometry_backend: GeometryBackend,
    snapshot: PreparationSnapshot,
    set: RepairSet,
    resources: String,
    cancel: Arc<AtomicBool>,
) -> Result<RepairSetCheck, String> {
    let interrupt = || cancel.load(Ordering::Relaxed);
    let checked = Compiler::default()
        .prepare_repair_set_with_backend(
            &snapshot,
            Actor::local(Channel::Ui),
            &set,
            Some(&resources),
            geometry_backend,
            Some(&interrupt),
        )
        .map_err(|e| e.to_string())?;
    Ok(RepairSetCheck {
        rows: checked.rows,
        sketches: checked.sketches,
        blockers: checked.blockers,
        prepared: checked.prepared,
    })
}

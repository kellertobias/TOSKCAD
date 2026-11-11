//! Readback proof for a Rust ZA worm STEP shell.
use super::*;
use crate::{step::helical::Closure, worm_gear::Worm};

/// Verify the serialized worm against its certified thread shell and bore,
/// returning the volume integrated from the decoded boundary in mm³.
pub fn verify_za_worm(
    input: &str,
    worm: &Worm,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
) -> Result<f64> {
    let [origin, u, v] = crate::step::worm_thread_frame(worm, [origin, u, v]);
    helical::verify(
        input,
        worm.thread(),
        Closure::hollow(worm.bore_radius_mm()),
        [origin, u, v],
        tolerance,
    )
}

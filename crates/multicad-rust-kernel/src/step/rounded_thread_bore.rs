//! Five inward tensor bands and one independently checked outer cylinder.
use super::*;
use crate::rounded_thread::ThroughBore;
pub fn write_rounded_thread_bore(
    bore: &ThroughBore,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_rounded_thread_bore(bore, origin, u, v, tolerance))
}
pub fn supports_rounded_thread_bore(
    bore: &ThroughBore,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
) -> bool {
    write_rounded_thread_bore(bore, origin, u, v, tolerance).is_ok()
}
pub(super) fn encode(
    file: &mut StepFile,
    bore: &ThroughBore,
    placement: Placement,
    tolerance: f64,
) -> Result<Entity, StepError> {
    let mut candidate = StepFile::new();
    let solid = rounded_thread::encode_bore_shell(&mut candidate, bore, &placement, tolerance)?;
    candidate.finish(vec![solid]);
    crate::step_read::verify_rounded_thread_bore(
        &candidate.data,
        bore,
        placement.origin,
        placement.u,
        placement.v,
        tolerance,
    )
    .map_err(read_error)?;
    crate::step_read::read_rounded_thread_bore(&candidate.data, tolerance).map_err(read_error)?;
    rounded_thread::encode_bore_shell(file, bore, &placement, tolerance)
}
fn read_error(error: crate::step_read::ReadError) -> StepError {
    match error {
        crate::step_read::ReadError::Limit => StepError::Limit,
        _ => StepError::InvalidGeometry,
    }
}
#[cfg(test)]
mod tests;

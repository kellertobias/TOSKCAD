use super::*;
pub(super) use multicad_rust_kernel::rounded_thread::Properties as Metrics;
pub(super) fn compute(thread: &Thread) -> Result<Metrics, GeometryError> {
    thread.properties().map_err(source_error)
}

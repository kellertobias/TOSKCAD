//! Dedicated GUI entry point for the genuine CAD-114 multi-body fixture.
#[path = "reference_review/backend.rs"]
mod backend;
#[path = "../src/files.rs"]
mod files;
#[path = "reference_review/hierarchy.rs"]
mod hierarchy;
#[path = "reference_review/launch.rs"]
mod launch;
#[cfg(test)]
#[path = "reference_review/hierarchy_tests.rs"]
mod tests;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    launch::run(true)
}

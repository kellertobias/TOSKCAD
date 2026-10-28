//! Headless exact-geometry report for a saved native project. This verifies
//! source regeneration; it is not the production Export workspace.
#[path = "../src/files.rs"]
mod files;
use multicad_model::Compiler;
use multicad_ui::storage::DocumentStorage;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let [project, report_path] = args.as_slice() else {
        return Err("Usage: model_file_review PROJECT.multicad REPORT.json".into());
    };
    let file = files::ProjectStore::default()
        .open_project(&PathBuf::from(project))
        .map_err(|e| e.to_string())?
        .file;
    let model = Compiler::default().compile(&file.document, 0)?;
    let snapshot = model.snapshot();
    let bodies=snapshot.bodies.iter().map(|(body,producer)| {
        let shape=&snapshot.shapes[producer];
        serde_json::json!({"body":body,"producer":producer,"name":file.document.get(*body).map(|e|&e.name),"volume_mm3":shape.volume_mm3,"solid_count":shape.solid_count,"bounds":{"min":shape.bounds.min,"max":shape.bounds.max},"triangles":shape.mesh.indices.len()/3})
    }).collect::<Vec<_>>();
    let report = serde_json::json!({"document":file.document.id(),"source_hash":snapshot.stamp.source_hash,"bodies":bodies,"consumed_bodies":snapshot.consumed_bodies,"references":snapshot.references,"scope":"Saved canonical source regenerated with the production native model compiler; no display mesh used as manufacturing input"});
    std::fs::write(
        PathBuf::from(report_path),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

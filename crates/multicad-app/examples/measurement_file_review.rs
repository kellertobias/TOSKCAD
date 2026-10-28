//! Re-evaluate retained measurements from a saved native project container.
//! Reports are derived evidence; no numeric result is written into the project.
#[path = "../src/files.rs"]
mod files;
use multicad_core::{document::EntityData, inspection};
use multicad_model::{Compiler, inspection::measure_model};
use multicad_ui::storage::DocumentStorage;
use std::{io::Write, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let [project, report_path] = args.as_slice() else {
        return Err("Usage: measurement_file_review PROJECT.multicad NEW_REPORT.json".into());
    };
    let file = files::ProjectStore::default()
        .open_project(&PathBuf::from(project))
        .map_err(|e| e.to_string())?
        .file;
    let model = Compiler::default().compile(&file.document, 0)?;
    let snapshot = model.snapshot();
    let mut measurements = vec![];
    for id in inspection::pins(&file.document) {
        let pin = inspection::read(&file.document, id)?;
        measurements.push(serde_json::json!({
            "id":pin.id,"name":pin.name,"query":pin.query,
            "visible":pin.visible,"label_offset_mm":pin.label_offset_mm,
            "evaluation":measure_model(&model,&pin.query),
        }));
    }
    let bodies = snapshot
        .bodies
        .iter()
        .map(|(body, producer)| {
            let shape = &snapshot.shapes[producer];
            serde_json::json!({
                "body":body,"producer":producer,
                "name":file.document.get(*body).map(|e|&e.name),
                "volume_mm3":shape.volume_mm3,"solid_count":shape.solid_count,
                "bounds":{"min":shape.bounds.min,"max":shape.bounds.max},
            })
        })
        .collect::<Vec<_>>();
    let parameters = file.document.entities().filter_map(|entity| {
        let EntityData::Parameter { expression, unit, exposed, .. } = &entity.data else {return None};
        Some(serde_json::json!({"id":entity.id,"name":entity.name,"expression":expression,"unit":unit,"exposed":exposed}))
    }).collect::<Vec<_>>();
    let report = serde_json::json!({
        "document":file.document.id(),"source_hash":snapshot.stamp.source_hash,
        "bodies":bodies,"parameters":parameters,"measurements":measurements,
        "references":snapshot.references,
        "scope":"Saved canonical source opened through the production store and rebuilt by the native compiler; retained query results are freshly evaluated, not loaded numeric values. Read-session revision is zero.",
    });
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(PathBuf::from(report_path))?;
    output.write_all(&serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}

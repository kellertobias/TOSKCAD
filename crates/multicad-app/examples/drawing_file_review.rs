//! Regenerate saved drawing source into exact vector evidence. This reports
//! native generation; it does not replace the production drawing export UI.
#[path = "../src/files.rs"]
mod files;
use multicad_core::drawings;
use multicad_model::{Compiler, DrawingProviders};
use multicad_ui::storage::DocumentStorage;
use std::{io::Write, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let [input, output] = args.as_slice() else {
        return Err("Usage: drawing_file_review PROJECT.multicad NEW_REPORT.json".into());
    };
    let project = files::ProjectStore::default()
        .open_project(&PathBuf::from(input))
        .map_err(|e| e.to_string())?
        .file;
    let model = Compiler::default().compile(&project.document, 0)?;
    let evaluation = model.drawings(&project.document, 0, &DrawingProviders::default())?;
    let mut pages = vec![];
    for id in drawings::books(&project.document) {
        let book = drawings::read(&project.document, id)?;
        for page in &book.pages {
            evaluation.checked_page(&project.document, 0, id, page.id)?;
            pages.push(serde_json::json!({"book":id,"page":page}));
        }
    }
    let report = serde_json::json!({"scope":"Saved canonical drawing source regenerated from current native B-reps; exact curve records and separate preview paths, no viewport meshes or screenshots as input","pages":pages,"evaluation":evaluation});
    let bytes = serde_json::to_vec_pretty(&report)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(PathBuf::from(output))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

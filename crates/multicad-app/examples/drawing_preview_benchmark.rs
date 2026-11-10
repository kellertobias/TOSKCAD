//! Measure the production drawing backend against a saved canonical project.
#[path = "../src/drawings.rs"]
mod drawings;
#[path = "../src/files.rs"]
mod files;
use multicad_core::drawings::{DrawingStamp, ViewEvaluation};
use multicad_model::GeometryBackend;
use multicad_ui::{drawings::DrawingBackend, storage::DocumentStorage};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::env::args_os().nth(1).ok_or("Pass a project path")?;
    let project = files::ProjectStore::default()
        .open_project(&PathBuf::from(input))
        .map_err(|e| e.to_string())?
        .file;
    let doc = &project.document;
    let stamp = DrawingStamp {
        document: doc.id(),
        revision: 0,
        source_hash: multicad_core::project_file::ContentHash::document(doc)?,
    };
    let uncached = std::env::args_os().any(|a| a == "--uncached");
    let dump_vectors = std::env::args_os().any(|a| a == "--dump-vectors");
    let geometry = if std::env::args_os().any(|a| a == "--rust-model") {
        GeometryBackend::Rust
    } else {
        GeometryBackend::OpenCascade
    };
    let start_worker = || match (geometry, uncached) {
        (GeometryBackend::OpenCascade, false) => drawings::Backend::new(),
        (GeometryBackend::OpenCascade, true) => drawings::Backend::with_cache(None),
        (GeometryBackend::Rust, false) => drawings::Backend::with_geometry_backend(geometry),
        (GeometryBackend::Rust, true) => drawings::Backend::with_cache_and_backend(None, geometry),
    };
    let mut backend = start_worker();
    let mut first_results = std::collections::BTreeMap::new();
    for pass in 0..3 {
        if pass == 2 {
            if uncached {
                break;
            }
            // A new worker proves reuse after closing/reopening a document.
            backend = start_worker();
        }
        for book in multicad_core::drawings::books(doc) {
            for page in multicad_core::drawings::read(doc, book)?.pages {
                if page.views.is_empty() {
                    continue;
                }
                let start = Instant::now();
                backend.request_page(doc, stamp, page.id)?;
                let result = loop {
                    if let Some(reply) = backend.poll() {
                        break reply.result?;
                    }
                    if start.elapsed() > Duration::from_secs(180) {
                        return Err("Preview timeout".into());
                    }
                    std::thread::sleep(Duration::from_millis(1));
                };
                let milliseconds = start.elapsed().as_secs_f64() * 1000.;
                let vectors: usize = result
                    .views
                    .values()
                    .map(|v| match v {
                        ViewEvaluation::Ready { vectors, .. } => vectors.len(),
                        _ => 0,
                    })
                    .sum();
                if pass == 0 {
                    first_results.insert(page.id, result.clone());
                } else {
                    assert_eq!(first_results.get(&page.id), Some(&result));
                }
                let mut row = serde_json::json!({
                    "backend": format!("{geometry:?}"),
                    "pass": pass,
                    "page": page.number,
                    "views": result.views.len(),
                    "ready": result.views.values().all(|v| matches!(v, ViewEvaluation::Ready {..})),
                    "vectors": vectors,
                    "milliseconds": milliseconds,
                });
                if dump_vectors && pass == 0 {
                    row["view_results"] = serde_json::to_value(
                        page.views
                            .iter()
                            .map(|view| (view, result.views.get(&view.id)))
                            .collect::<Vec<_>>(),
                    )?;
                }
                println!("{row}");
            }
        }
    }
    Ok(())
}

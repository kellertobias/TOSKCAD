//! Measure the production drawing backend against a saved canonical project.
#[path = "../src/drawings.rs"]
mod drawings;
#[path = "../src/files.rs"]
mod files;
use multicad_core::drawings::{DrawingStamp, ViewEvaluation};
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
    let mut backend = if std::env::args_os().any(|a| a == "--uncached") {
        drawings::Backend::with_cache(None)
    } else {
        drawings::Backend::new()
    };
    let mut first_results = std::collections::BTreeMap::new();
    for pass in 0..3 {
        if pass == 2 {
            if std::env::args_os().any(|a| a == "--uncached") {
                break;
            }
            // A new worker proves reuse after closing/reopening a document.
            backend = drawings::Backend::new();
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
                println!(
                    "{}",
                    serde_json::json!({"pass":pass,"page":page.number,"views":result.views.len(),
                    "ready":result.views.values().all(|v| matches!(v, ViewEvaluation::Ready {..})),
                    "vectors":vectors,"milliseconds":start.elapsed().as_secs_f64()*1000.})
                );
            }
        }
    }
    Ok(())
}

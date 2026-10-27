//! CAD-55 native acceptance fixture, separate from production modeling commands.
#[path = "reference_review/backend.rs"]
mod backend;
#[path = "../src/files.rs"]
mod files;
#[cfg(test)]
#[path = "reference_review/tests.rs"]
mod tests;
use multicad_ui::{
    NativeApplication, NativeFrame,
    application::{Application, DocumentStorage},
    egui,
    navigation::DisplayStyle,
    registry::Mode,
};
use std::{path::PathBuf, sync::Arc};
struct Review {
    app: Application,
    fitted: bool,
}
impl NativeApplication for Review {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut NativeFrame) {
        if !self.fitted && self.app.current().scene.visible().is_some() {
            self.app.current_mut().sketch.request_fit();
            self.fitted = true;
        }
        self.app.ui(ui, frame);
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let document = backend::document()?;
    if std::env::args().any(|s| s == "--headless") {
        let evaluation = backend::evaluate(
            &document,
            multicad_ui::viewport::SceneStamp {
                document: document.id(),
                revision: 0,
                generation: 1,
            },
            || Ok(()),
        )?;
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"source_volume_mm3":evaluation.source_volume_mm3,"drilled_volume_mm3":evaluation.display_volume_mm3,"hole_center_world":evaluation.hole_center_world,"references":evaluation.report,"display_instances":evaluation.scene.instances().len(),"scope":"Native CAD-55 acceptance fixture; production modeling commands remain separate"})
            )?
        );
        return Ok(());
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("artifacts/reference-review/reference-repair.multicad.json");
    std::fs::create_dir_all(path.parent().expect("artifact parent"))?;
    let mut storage = files::SnapshotStore;
    storage.save(&path, &document)?;
    let mut app = Application::with_document(Box::new(storage), document)?;
    app.projects[0].imported_from = Some(path);
    app.mode = Mode::Model;
    app.current_mut()
        .sketch
        .set_display_style(DisplayStyle::VisibleEdges);
    app.set_scene_backends(Arc::new(|| Box::new(backend::Backend::new())));
    app.set_sketch_queries(Arc::new(|| {
        Ok(Box::new(multicad_kernel::NativeRegionQueries::default()))
    }));
    multicad_ui::launch_with(Box::new(move |_| {
        Ok(Box::new(Review { app, fitted: false }))
    }))?;
    Ok(())
}

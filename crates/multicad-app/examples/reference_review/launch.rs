//! Shared shell composition for isolated native acceptance fixtures.
use super::{backend, files, hierarchy};
use multicad_core::project_file::ProjectFile;
use multicad_ui::{
    NativeApplication, NativeFrame,
    application::{Application, DocumentStorage},
    egui,
    navigation::DisplayStyle,
    registry::Mode,
    storage::SaveMode,
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
struct Options {
    hierarchy: bool,
    headless: bool,
    project: Option<PathBuf>,
    write: Option<PathBuf>,
}
impl Options {
    fn parse(default_hierarchy: bool) -> Result<Self, String> {
        let mut options = Self {
            hierarchy: default_hierarchy,
            headless: false,
            project: None,
            write: None,
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--hierarchy" => options.hierarchy = true,
                "--headless" => options.headless = true,
                "--project" => {
                    options.project = Some(PathBuf::from(
                        args.next()
                            .ok_or("--project requires a native project path")?,
                    ))
                }
                "--write-project" => {
                    options.write = Some(PathBuf::from(
                        args.next()
                            .ok_or("--write-project requires a new native project path")?,
                    ))
                }
                _ => {
                    return Err(format!(
                        "Unknown review option {arg}; use --headless [--project FILE] [--write-project NEW_FILE]"
                    ));
                }
            }
        }
        if !options.headless && (options.project.is_some() || options.write.is_some()) {
            return Err("Project report/write flags require --headless; use native File/Open for interactive reopening".into());
        }
        Ok(options)
    }
}
pub fn run(default_hierarchy: bool) -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse(default_hierarchy)?;
    let mut storage = files::SnapshotStore;
    let mut project = if let Some(path) = &options.project {
        storage.open_project(path).map_err(|e| e.to_string())?.file
    } else {
        ProjectFile::new(if options.hierarchy {
            hierarchy::document()?
        } else {
            backend::document()?
        })?
    };
    if options.headless {
        let evaluation = backend::evaluate(
            &project.document,
            multicad_ui::viewport::SceneStamp {
                document: project.document.id(),
                revision: 0,
                generation: 1,
            },
            || Ok(()),
        )?;
        if let Some(path) = &options.write {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            project.recovery = None;
            storage
                .save_project(path, &project, SaveMode::Create)
                .map_err(|e| e.to_string())?;
        }
        let report = if let Some(evidence) = evaluation.fixture_evidence {
            serde_json::json!({"hierarchy":evidence,"references":evaluation.report})
        } else {
            serde_json::json!({"source_volume_mm3":evaluation.source_volume_mm3,"drilled_volume_mm3":evaluation.display_volume_mm3,"hole_center_world":evaluation.hole_center_world,"references":evaluation.report,"display_instances":evaluation.scene.instances().len(),"scope":"Native CAD-55 acceptance fixture; production modeling commands remain separate"})
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    let relative = if options.hierarchy {
        "artifacts/hierarchy-review/initial-hierarchy.multicad.json"
    } else {
        "artifacts/reference-review/reference-repair.multicad.json"
    };
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative);
    std::fs::create_dir_all(path.parent().expect("artifact parent"))?;
    storage.save(&path, &project.document)?;
    let mut app = Application::with_document(Box::new(storage), project.document)?;
    app.projects[0].resources = project.resources;
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

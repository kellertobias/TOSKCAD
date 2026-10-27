//! Explicit native viewport acceptance harness. This uses corpus definitions,
//! not the production solid-modeling feature provider or modeling commands.
#[path = "viewport_review/corpus.rs"]
mod corpus;
#[path = "../src/files.rs"]
mod files;
use multicad_ui::{
    NativeApplication, NativeFrame,
    application::{Application, DocumentStorage},
    camera::Camera,
    egui,
    navigation::DisplayStyle,
    registry::Mode,
    viewport::PickFilter,
};
use std::{path::PathBuf, sync::Arc, time::Instant};
struct Review {
    app: Application,
    report: PathBuf,
    mixed: bool,
    fitted: bool,
    frames: usize,
    previous: Option<Instant>,
    intervals: Vec<f64>,
    cpu: Vec<f64>,
    picks: Vec<f64>,
    input_feedback: Vec<f64>,
    click_feedback: Vec<f64>,
    feedback_deadline: Option<f64>,
}
fn p95(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    Some(sorted[(sorted.len() * 95 / 100).min(sorted.len() - 1)])
}
impl NativeApplication for Review {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut NativeFrame) {
        let start = Instant::now();
        let (pointer_input, click_input) = ui.input(|i| {
            let inside = self
                .app
                .current()
                .sketch
                .viewport_rect
                .is_some_and(|r| i.pointer.latest_pos().is_some_and(|p| r.contains(p)));
            (
                inside
                    && i.events.iter().any(|e| {
                        matches!(
                            e,
                            egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. }
                        )
                    }),
                inside
                    && i.events.iter().any(|e| {
                        matches!(
                            e,
                            egui::Event::PointerButton {
                                button: egui::PointerButton::Primary,
                                pressed: true,
                                ..
                            }
                        )
                    }),
            )
        });
        let ready = {
            let p = self.app.current();
            p.scene
                .is_current(p.session.document().id(), p.session.revision())
        };
        if ready && !self.fitted {
            self.app.current_mut().sketch.request_fit();
            self.fitted = true;
        }
        if ready && self.frames < 240 {
            let camera = &mut self.app.current_mut().sketch.camera;
            camera.yaw += 0.012;
            camera.pitch = 0.55 + (self.frames as f32 * 0.02).sin() * 0.12;
            if self.frames > 30
                && let Some(previous) = self.previous
            {
                self.intervals
                    .push(start.duration_since(previous).as_secs_f64() * 1000.);
            }
            self.previous = Some(start);
            ui.ctx().request_repaint();
        }
        self.app.ui(ui, frame);
        // This measures the real received-event path through picking and queued
        // UI painting. It intentionally excludes the OS queue, GPU and compositor.
        if ready && self.frames >= 240 && pointer_input && self.input_feedback.len() < 4096 {
            let elapsed = start.elapsed().as_secs_f64() * 1000.;
            self.input_feedback.push(elapsed);
            if click_input {
                self.click_feedback.push(elapsed);
            }
            self.feedback_deadline = Some(ui.input(|i| i.time) + 0.3);
        }
        if let Some(deadline) = self.feedback_deadline {
            let now = ui.input(|i| i.time);
            if now >= deadline {
                let p = self.app.current();
                let scene = p.scene.visible();
                let path = self.report.with_file_name(if self.mixed {
                    "mixed-100-input-feedback.json"
                } else {
                    "instances-1000-input-feedback.json"
                });
                let report = serde_json::json!({
                    "pid":std::process::id(),
                    "pointer_event_frames":self.input_feedback.len(),
                    "primary_press_frames":self.click_feedback.len(),
                    "p95_received_pointer_to_ui_paint_queue_ms":p95(&self.input_feedback),
                    "p95_received_primary_press_to_ui_paint_queue_ms":p95(&self.click_feedback),
                    "source_revision":p.session.revision(),
                    "display_generation":scene.map(|s|s.stamp().generation),
                    "display_triangles":scene.map(|s|s.geometries().values().map(|g|g.indices().len()/3).sum::<usize>()),
                    "selected_solid":p.sketch.selected_solid(),
                    "refining_display":p.scene.refining(),
                    "scope":"Actual received egui pointer event through application UI and queued highlight painting. Excludes OS input transport, GPU execution and compositor. Reports are written after 300 ms without a new measured event, outside the timed path.",
                });
                if let Err(error) = std::fs::write(
                    path,
                    serde_json::to_vec_pretty(&report).expect("feedback JSON"),
                ) {
                    eprintln!("Input feedback report: {error}");
                }
                self.feedback_deadline = None;
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f64(
                        (deadline - now).clamp(0., 0.3),
                    ));
            }
        }
        if ready && self.frames < 240 {
            self.cpu.push(start.elapsed().as_secs_f64() * 1000.);
            let p = self.app.current();
            if let (Some(scene), Some(rect)) = (p.scene.visible(), p.sketch.viewport_rect) {
                let query = Instant::now();
                let pointer = rect.center()
                    + egui::vec2(
                        ((self.frames % 17) as f32 - 8.) * 18.,
                        ((self.frames % 13) as f32 - 6.) * 15.,
                    );
                let _ = scene.pick_screen(p.sketch.camera, rect, pointer, PickFilter::Auto, false);
                self.picks.push(query.elapsed().as_secs_f64() * 1000.);
            }
            self.frames += 1;
            if self.frames == 240 {
                let scene = self.app.current().scene.visible().expect("ready");
                let report = serde_json::json!({"corpus":if self.mixed {"100 mixed parts"}else {"1000 shared instances"},"pid":std::process::id(),"frames":self.frames,"logical_window_points":[1360,860],"p95_inter_frame_ms":p95(&self.intervals),"p95_ui_cpu_ms":p95(&self.cpu),"p95_synthetic_pick_processing_ms":p95(&self.picks),"mesh_instance_payload_bytes":scene.storage_bytes(),"unique_geometry_assets":scene.geometries().len(),"instances":scene.instances().len(),"rendering":"native WGPU window; GPU presentation is asynchronous","latency_scope":"synthetic CPU selection processing only; not OS input to photon","idle_cpu_and_rss":"measure separately after animation stops","scope":"viewport acceptance fixture; production modeling commands remain separate"});
                if let Err(error) = std::fs::write(
                    &self.report,
                    serde_json::to_vec_pretty(&report).expect("report JSON"),
                ) {
                    eprintln!("Viewport report: {error}");
                }
            }
        }
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mixed = std::env::args().any(|s| s == "--mixed")
        || std::env::current_exe()?
            .file_name()
            .is_some_and(|s| s.to_string_lossy().contains("mixed"));
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("artifacts/viewport-review");
    std::fs::create_dir_all(&root)?;
    let name = if mixed { "mixed-100" } else { "instances-1000" };
    let path = root.join(format!("{name}.multicad.json"));
    let report = root.join(format!("{name}-metrics.json"));
    let document = corpus::document(mixed)?;
    let mut storage = files::SnapshotStore;
    storage.save(&path, &document)?;
    let mut app = Application::with_document(Box::new(storage), document)?;
    app.set_sketch_queries(std::sync::Arc::new(|| {
        Ok(Box::new(multicad_kernel::NativeRegionQueries::default()))
    }));
    app.projects[0].imported_from = Some(path);
    app.mode = Mode::Model;
    app.current_mut()
        .sketch
        .set_display_style(DisplayStyle::VisibleEdges);
    app.current_mut().sketch.camera = Camera::default();
    app.set_scene_backends(Arc::new(|| Box::new(corpus::Backend::new())));
    multicad_ui::launch_with(Box::new(move |_| {
        Ok(Box::new(Review {
            app,
            report,
            mixed,
            fitted: false,
            frames: 0,
            previous: None,
            intervals: vec![],
            cpu: vec![],
            picks: vec![],
            input_feedback: vec![],
            click_feedback: vec![],
            feedback_deadline: None,
        }))
    }))?;
    Ok(())
}

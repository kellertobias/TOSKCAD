//! Offline CPU profiling of real application layout on a saved canonical source.
//! No window, native worker, GPU or input-to-visible latency is measured here.
use multicad_core::{document::Document, project_file};
use multicad_ui::{
    application::{Application, DocumentStorage},
    egui,
    registry::Mode,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn preparing(shape: &egui::epaint::Shape) -> bool {
    match shape {
        egui::epaint::Shape::Text(text) => text.galley.text().contains("Preparing sketches…"),
        egui::epaint::Shape::Vec(shapes) => shapes.iter().any(preparing),
        _ => false,
    }
}
fn frame(ctx: &egui::Context, app: &mut Application) -> (f64, bool) {
    let started = Instant::now();
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1360., 860.),
            )),
            ..Default::default()
        },
        |ui| app.show(ui),
    );
    let milliseconds = started.elapsed().as_secs_f64() * 1000.;
    let pending = output.shapes.iter().any(|shape| preparing(&shape.shape));
    output.drop_without_applying_deltas();
    (milliseconds, pending)
}

struct NoIo;
impl DocumentStorage for NoIo {
    fn save(&mut self, _: &Path, _: &Document) -> Result<(), String> {
        Err("Offline profiling does not save projects".into())
    }
    fn load(&mut self, _: &Path) -> Result<Document, String> {
        Err("Use the explicit profiling input".into())
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("Pass a native project path")?;
    let repeats: usize = std::env::args().nth(2).as_deref().unwrap_or("1").parse()?;
    if !(1..=20).contains(&repeats) {
        return Err("Use between 1 and 20 cold-layout repetitions".into());
    }
    let file = project_file::read(
        &mut std::fs::File::open(&path)?,
        Default::default(),
        &project_file::supported_capabilities(),
    )?
    .project;
    let apps = (0..repeats)
        .map(|_| Application::with_document(Box::new(NoIo), file.document.clone()))
        .collect::<Result<Vec<_>, _>>()?;
    if std::env::args().any(|arg| arg == "--wait") {
        eprintln!("Ready for offline layout profiling");
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
    }
    let mut rows = vec![];
    for (repeat, mut app) in apps.into_iter().enumerate() {
        let ctx = egui::Context::default();
        multicad_ui::theme::configure(&ctx);
        for (phase, mode) in [
            ("parameters-cold", Mode::Parameters),
            ("model-cold", Mode::Model),
            ("model-warm", Mode::Model),
            ("parameters-return", Mode::Parameters),
            ("model-return", Mode::Model),
        ] {
            app.mode = mode;
            let (milliseconds, pending) = frame(&ctx, &mut app);
            let row = serde_json::json!({"repeat":repeat,"phase":phase,"layout_cpu_ms":milliseconds,"source_hash_pending":pending});
            println!("{row}");
            rows.push(row);
            if phase == "model-cold" && pending {
                let started = Instant::now();
                let mut count = 0;
                let mut max_cpu_ms: f64 = 0.;
                loop {
                    std::thread::sleep(Duration::from_millis(10));
                    let (cpu_ms, pending) = frame(&ctx, &mut app);
                    count += 1;
                    max_cpu_ms = max_cpu_ms.max(cpu_ms);
                    if !pending {
                        break;
                    }
                    if started.elapsed() > Duration::from_secs(10) {
                        return Err("Source hash preparation did not finish".into());
                    }
                }
                println!(
                    "{}",
                    serde_json::json!({"repeat":repeat,"phase":"source-hash-ready","wall_ms":started.elapsed().as_secs_f64()*1000.,"continuation_frames":count,"max_layout_cpu_ms":max_cpu_ms})
                );
            }
        }
    }
    eprintln!(
        "Offline layout only: {} phases; no presentation-latency claim",
        rows.len()
    );
    Ok(())
}

//! Native visual fixture: hold a real project's drawing request pending so its
//! loading layout can be inspected. No geometry generation result is simulated.
use multicad_core::{commands::Session, document::Document, drawings::DrawingStamp, project_file};
use multicad_ui::{
    drawings::{DrawingBackend, DrawingEditor, DrawingReply},
    egui,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

struct HeldBackend {
    fail: bool,
}
impl DrawingBackend for HeldBackend {
    fn request(&mut self, _: &Document, _: DrawingStamp) -> Result<(), String> {
        if self.fail {
            Err("The model could not be rebuilt. Inspect the drawing source and retry.".into())
        } else {
            Ok(())
        }
    }
    fn poll(&mut self) -> Option<DrawingReply> {
        None
    }
}
struct Review {
    session: Session,
    editor: DrawingEditor,
    output: PathBuf,
    frames: u32,
}
fn save_png(path: &Path, image: &egui::ColorImage) -> Result<(), String> {
    let file = std::fs::File::create_new(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(file, image.size[0] as u32, image.size[1] as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    let pixels: Vec<_> = image
        .pixels
        .iter()
        .flat_map(|p| p.to_srgba_unmultiplied())
        .collect();
    writer.write_image_data(&pixels).map_err(|e| e.to_string())
}
impl eframe::App for Review {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let screenshot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = screenshot {
            save_png(&self.output, &image).expect("save native review screenshot");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ui.heading("ToskCAD · Drawing generation review");
        self.editor.show(ui, &mut self.session);
        self.frames += 1;
        if self.frames == 5 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}
fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        return Err(
            "Usage: drawing_loading_review PROJECT NEW_SCREENSHOT.png [width height] [--error]"
                .into(),
        );
    }
    let mut file = std::fs::File::open(&args[0]).map_err(|e| e.to_string())?;
    let opened = project_file::read(
        &mut file,
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .map_err(|e| e.to_string())?;
    let session = Session::new(opened.project.document).map_err(|e| e.to_string())?;
    let mut editor = DrawingEditor::default();
    let fail = args.iter().any(|a| a == "--error");
    editor.set_backend_factory(Arc::new(move || Box::new(HeldBackend { fail })));
    let size = [
        args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1360.),
        args.get(3).and_then(|s| s.parse().ok()).unwrap_or(860.),
    ];
    let output = PathBuf::from(&args[1]);
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    eframe::run_native(
        "Drawing generation review",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size(size),
            renderer: eframe::Renderer::Wgpu,
            ..Default::default()
        },
        Box::new(move |cc| {
            multicad_ui::theme::configure(&cc.egui_ctx);
            Ok(Box::new(Review {
                session,
                editor,
                output,
                frames: 0,
            }))
        }),
    )
    .map_err(|e| e.to_string())
}

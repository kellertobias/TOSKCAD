pub mod camera;
pub mod icons;
pub mod navigation;
// Native host and reusable interaction controls. Domain and kernel ownership
// remain outside the UI.
pub use eframe::{
    App as NativeApplication, AppCreator, CreationContext, Frame as NativeFrame, egui,
};
use multicad_core::BuildInfo;

struct Host;
impl eframe::App for Host {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        egui::Panel::top("header").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("MultiCAD");
                ui.separator();
                ui.label("Untitled");
            });
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.label(format!(
                "{} · Development build",
                BuildInfo::current().version
            ));
        });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("No project open");
            });
        });
    }
}
pub fn launch() -> Result<(), String> {
    launch_with(Box::new(|_| Ok(Box::new(Host))))
}

/// The executable's composition root supplies the application implementation.
/// Document/command modules never need to initialize a window or a GPU device.
pub fn launch_with(application: AppCreator<'static>) -> Result<(), String> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([900.0, 600.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "MultiCAD",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            application(cc)
        }),
    )
    .map_err(|e| e.to_string())
}

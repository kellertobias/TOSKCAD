pub mod application;
pub mod camera;
pub mod controls;
pub mod icons;
pub mod model_tree;
pub mod navigation;
pub mod parameter_editor;
pub mod reference_repair;
pub mod registry;
pub mod storage;
// Native host and reusable interaction controls. Domain and kernel ownership
// remain outside the UI.
pub use eframe::{
    App as NativeApplication, AppCreator, CreationContext, Frame as NativeFrame, egui,
};
pub fn launch_with_storage(storage: Box<dyn application::DocumentStorage>) -> Result<(), String> {
    launch_with(Box::new(move |_| {
        application::Application::new(storage)
            .map(|app| Box::new(app) as Box<dyn eframe::App>)
            .map_err(|e| e.into())
    }))
}
/// The executable's composition root supplies the application implementation.
/// Document/command modules never need to initialize a window or a GPU device.
pub fn launch_with(application: AppCreator<'static>) -> Result<(), String> {
    let options = eframe::NativeOptions {
        depth_buffer: 32,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([1100.0, 700.0])
            .with_decorations(cfg!(target_os = "macos"))
            .with_fullsize_content_view(true)
            .with_title_shown(false)
            .with_titlebar_shown(false)
            .with_titlebar_buttons_shown(true),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "MultiCAD",
        options,
        Box::new(move |cc| {
            theme::configure(&cc.egui_ctx);
            if let Some(render_state) = &cc.wgpu_render_state {
                viewport::install(render_state);
            }
            application(cc)
        }),
    )
    .map_err(|e| e.to_string())
}

mod sketch_constraints;
pub mod sketch_context;
pub mod sketch_editor;
pub mod sketch_evaluation;
pub mod sketch_operations;
pub mod sketch_plane;
pub mod sketch_selection;
pub mod sketch_tools;

pub mod theme;

#[cfg(target_os = "macos")]
mod native_window;

pub mod shortcuts;
pub mod viewport;

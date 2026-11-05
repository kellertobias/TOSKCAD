pub mod application;
pub mod architecture_navigation;
pub mod architecture_view;
pub mod artwork_import;
pub mod assembly;
pub mod camera;
mod camera_gestures;
pub mod controls;
pub mod drawings;
pub mod examples;
pub mod external_references;
pub mod file_picker;
pub mod icons;
pub mod initial_fit;
pub mod inspection;
mod inspector_actions;
pub mod library;
pub mod materials;
pub mod mechanical_import;
pub mod metadata_sidebar;
pub mod model_tree;
pub mod modeling;
pub mod navigation;
pub mod parameter_editor;
pub mod preferences;
pub mod preview_capture;
mod quit_shortcut;
pub mod recent_projects;
pub mod reference_repair;
pub mod reference_repair_set;
pub mod registry;
pub mod render_view;
pub mod ribbon_ui;
pub mod schedules;
mod selection_input;
pub mod storage;
pub mod svg_icon;
pub mod symbols;
pub mod transfer;
pub mod wood_navigation;
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
    let icon = eframe::icon_data::from_png_bytes(include_bytes!(
        "../../../assets/branding/app/AppIcon.png"
    ))
    .map_err(|error| format!("Could not decode application icon: {error}"))?;
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions {
        depth_buffer: 32,
        viewport: egui::ViewportBuilder::default()
            .with_icon(icon)
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
    #[cfg(target_os = "windows")]
    quit_shortcut::windows::configure(&mut options);
    // Finder "open document" requests can arrive before the first frame.
    #[cfg(target_os = "macos")]
    native_window::observe_open_documents();
    eframe::run_native(
        "ToskCAD",
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
pub mod sketch_region_fill;
pub mod sketch_selection;
pub mod sketch_tools;

pub mod theme;
pub mod tool_artwork;
pub mod tool_search;

#[cfg(target_os = "macos")]
mod native_window;

pub mod shortcuts;
pub mod viewport;

pub mod exchange;
pub mod print;

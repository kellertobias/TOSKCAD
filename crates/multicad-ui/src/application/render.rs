//! A saved render view is project metadata. The preview owns its camera and
//! uses the current canonical Model scene, so framing never moves the Model.
use super::*;
use crate::{
    camera::Camera,
    navigation::DisplayStyle,
    render_view,
    viewport::{self, Scene, SceneStamp},
};
use multicad_core::model::{Projection, RenderCamera, RenderScene};
use std::sync::Arc;

const CAPTURE_HELP: &str = "Capture stores this preview's camera position, target, projection and display style in the project. Show saved view restores them.";
const BLENDER_AVAILABILITY: &str = "An external automation client connected over MCP can write the saved scene to its permitted export folder. The handoff contains a GLB with the model, camera and lights, plus a .render.json manifest. In Blender choose File, Import, then glTF 2.0 to open the GLB.";

pub(super) struct Preview {
    project: EntityId,
    camera: Camera,
    style: DisplayStyle,
    restore: Option<RenderCamera>,
    rect: Option<egui::Rect>,
    frame: Option<SceneStamp>,
    fit: bool,
    handoff: bool,
}

#[derive(Clone, Copy)]
enum Action {
    Capture,
    Restore,
    Close,
}

fn saved_scene(project: &Project) -> Result<Option<(EntityId, String, RenderScene)>, String> {
    let document = project.session.document();
    let Some(entity) = multicad_core::model::render_scenes(document).next() else {
        return Ok(None);
    };
    let scene = multicad_core::model::read_render_scene(document, entity.id)
        .map_err(|error| error.to_string())?;
    Ok(Some((entity.id, entity.name.clone(), scene)))
}

fn current_source(project: &Project) -> Option<&Arc<Scene>> {
    let scene = project.scene.visible()?;
    (project
        .scene
        .is_current(project.session.document().id(), project.session.revision())
        && scene.is_full_model()
        && scene.world_bounds().is_some())
    .then_some(scene)
}

fn projection_text(projection: Projection) -> String {
    match projection {
        Projection::Perspective { fov_deg } => format!("Perspective · {fov_deg:.1}° vertical"),
        Projection::Orthographic { height_mm } => {
            format!("Orthographic · {height_mm:.1} mm tall")
        }
    }
}

fn matches_saved(preview: &Preview, saved: &RenderScene) -> Option<bool> {
    let height = preview.rect?.height();
    let current = render_view::capture(&preview.camera, height).ok()?;
    let shown =
        render_view::capture(&render_view::restore(&saved.camera, height).ok()?, height).ok()?;
    let scale = shown
        .eye_mm
        .iter()
        .chain(&shown.target_mm)
        .fold(1f64, |m, v| m.max(v.abs()));
    let tolerance = 1e-4 * scale;
    let near = |a: [f64; 3], b: [f64; 3]| (0..3).all(|i| (a[i] - b[i]).abs() <= tolerance);
    Some(
        near(current.eye_mm, shown.eye_mm)
            && near(current.target_mm, shown.target_mm)
            && match (current.projection, shown.projection) {
                (Projection::Perspective { .. }, Projection::Perspective { .. }) => true,
                (
                    Projection::Orthographic { height_mm: a },
                    Projection::Orthographic { height_mm: b },
                ) => (a - b).abs() <= tolerance,
                _ => false,
            }
            && render_view::display_of(preview.style) == saved.display,
    )
}

impl Preview {
    fn new(project: &Project, seed_model_camera: bool) -> Self {
        let restore = if seed_model_camera && current_source(project).is_some() {
            project
                .sketch
                .viewport_rect
                .and_then(|rect| render_view::capture(&project.sketch.camera, rect.height()).ok())
        } else {
            None
        };
        Self {
            project: project.local_id,
            camera: project.sketch.camera,
            style: project.sketch.display_style(),
            restore,
            rect: None,
            frame: None,
            fit: true,
            handoff: false,
        }
    }

    fn restore_saved(&mut self, saved: &RenderScene) {
        self.restore = Some(saved.camera);
        self.style = render_view::style_of(saved.display);
        self.frame = None;
    }

    fn ready(&self, project: &Project) -> bool {
        self.project == project.local_id
            && current_source(project).is_some_and(|scene| Some(scene.stamp()) == self.frame)
            && self.rect.is_some()
            && self.restore.is_none()
            && !self.fit
    }

    fn fit_model(&mut self) {
        self.restore = None;
        self.fit = true;
    }

    fn viewport(
        &mut self,
        ui: &mut egui::Ui,
        current: Option<&Arc<Scene>>,
        retained: Option<&Arc<Scene>>,
        height: f32,
    ) -> Result<(), String> {
        let (response, painter) = ui.allocate_painter(
            egui::vec2(ui.available_width(), height),
            egui::Sense::drag(),
        );
        let rect = response.rect;
        painter.rect_filled(rect, 4., crate::theme::SURFACE);
        if let Some(scene) = current {
            if let Some(camera) = &self.restore {
                self.camera = render_view::restore(camera, rect.height())?;
                self.restore = None;
                self.fit = false;
            } else if self.fit
                && let Some(bounds) = scene.world_bounds()
            {
                self.camera
                    .fit(&bounds.map(|point| point.map(|v| v as f32)), rect);
                self.fit = false;
            } else if let Some(old) = self.rect
                && (rect.height() - old.height()).abs() > 0.5
                && let Ok(camera) = render_view::capture(&self.camera, old.height())
            {
                self.camera = render_view::restore(&camera, rect.height())?;
            }
            if response.dragged_by(egui::PointerButton::Primary) {
                self.camera.orbit(ui.input(|input| input.pointer.delta()));
            }
            crate::camera_gestures::navigate_with_plane(ui, &response, &mut self.camera, false);
            self.rect = Some(rect);
            self.frame = Some(scene.stamp());
        } else {
            self.frame = None;
        }
        if let Some(scene) = current.or(retained) {
            viewport::paint_scene(&painter, rect, scene.clone(), self.camera, None, self.style);
        }
        Ok(())
    }
}

impl Application {
    pub(super) fn render_command_enabled(&self, id: &str) -> bool {
        let project = self.current();
        if self.welcome || self.file_busy() || project.pending_edits() {
            return false;
        }
        match id {
            "appearance" => self
                .render_preview
                .as_ref()
                .is_none_or(|preview| preview.ready(project)),
            "render-view" => saved_scene(project).is_ok_and(|scene| scene.is_some()),
            "blender" => true,
            _ => false,
        }
    }

    fn start_render_preview(&mut self) -> Result<(), String> {
        if self.welcome || self.file_busy() || self.current().pending_edits() {
            return Err("Finish the current edits before opening the render preview".into());
        }
        if self.mode == Mode::Sketch {
            self.current_mut().sketch.finish()?;
        }
        if self
            .render_preview
            .as_ref()
            .is_none_or(|preview| preview.project != self.current().local_id)
        {
            self.render_preview = Some(Preview::new(self.current(), self.mode == Mode::Model));
        }
        self.mode = Mode::Parameters;
        self.tab = 0;
        Ok(())
    }

    pub(super) fn invoke_render_view(&mut self, id: &str, channel: Channel) -> Result<(), String> {
        match id {
            "appearance" => {
                let Some(preview) = self.render_preview.as_ref() else {
                    self.start_render_preview()?;
                    self.message = "Frame the model, then choose Capture render view".into();
                    return Ok(());
                };
                if !self.render_command_enabled(id) {
                    return Err(
                        "Wait for the current model to be framed before capturing a view".into(),
                    );
                }
                let height = preview.rect.expect("framed preview").height();
                let camera = render_view::capture(&preview.camera, height)?;
                let display = render_view::display_of(preview.style);
                let project = self.current_mut();
                let (id, name, mut saved) = match saved_scene(project)? {
                    Some((id, name, saved)) => (Some(id), name, saved),
                    None => (None, "Render scene".to_owned(), RenderScene::default()),
                };
                saved.camera = camera;
                saved.display = display;
                let (_, prepared) = multicad_core::model::prepare_render_scene(
                    &project.session,
                    project.session.revision(),
                    Actor::local(channel),
                    id,
                    &name,
                    &saved,
                )
                .map_err(|error| error.to_string())?;
                project
                    .session
                    .commit(prepared)
                    .map_err(|error| error.to_string())?;
                self.message = format!(
                    "{name} · camera and {} style saved",
                    render_view::style_of(display).label()
                );
                Ok(())
            }
            "render-view" => {
                let (_, name, saved) =
                    saved_scene(self.current())?.ok_or("Capture a render view first")?;
                self.start_render_preview()?;
                self.render_preview
                    .as_mut()
                    .expect("render preview opened")
                    .restore_saved(&saved);
                self.message = format!("Showing saved view · {name}");
                Ok(())
            }
            "blender" => {
                self.start_render_preview()?;
                self.render_preview
                    .as_mut()
                    .expect("render preview opened")
                    .handoff = true;
                self.message = "Blender handoff information".into();
                Ok(())
            }
            _ => Err("Choose a render view command".into()),
        }
    }

    pub(super) fn project_render_controls(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        super::project::contents_section(ui, "Render", |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        self.render_command_enabled("appearance"),
                        egui::Button::new("Open render preview").small(),
                    )
                    .on_hover_text(CAPTURE_HELP)
                    .clicked()
                {
                    action = Some("preview");
                }
            });
        });
        match saved_scene(self.current()) {
            Ok(Some((_, name, saved))) => super::project::contents_row(ui, &name, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.small(format!(
                        "{} · {} · {} light(s)",
                        projection_text(saved.camera.projection),
                        render_view::style_of(saved.display).label(),
                        saved.lights.len()
                    ));
                    if ui
                        .add_enabled(
                            self.render_command_enabled("render-view"),
                            egui::Button::new("Show saved view").frame(false),
                        )
                        .clicked()
                    {
                        action = Some("render-view");
                    }
                });
            }),
            Ok(None) => {
                ui.weak("No render view saved yet.");
            }
            Err(error) => {
                ui.colored_label(Color32::LIGHT_RED, error);
            }
        }
        ui.collapsing("Blender handoff", |ui| {
            ui.label(BLENDER_AVAILABILITY);
        });
        if let Some(action) = action {
            let result = if action == "preview" {
                self.start_render_preview()
            } else {
                self.invoke_render_view(action, Channel::Ui)
            };
            if let Err(error) = result {
                self.message = error;
                self.message_error = true;
            } else {
                self.message_error = false;
            }
        }
    }

    pub(super) fn render_window(&mut self, ctx: &egui::Context) {
        let Some(mut preview) = self.render_preview.take() else {
            return;
        };
        let project = self.current();
        if preview.project != project.local_id || self.welcome || self.mode != Mode::Parameters {
            return;
        }
        let current = current_source(project).cloned();
        let retained = project
            .scene
            .visible()
            .filter(|scene| {
                scene.stamp().document == project.session.document().id()
                    && scene.world_bounds().is_some()
            })
            .cloned();
        let source_status = if current.is_some() {
            None
        } else if let Some(error) = project.scene.error() {
            Some((error.to_owned(), true))
        } else if project.scene.pending() {
            Some(("Preparing the current Model preview…".into(), false))
        } else if project.scene.has_backend() {
            Some(("Add a Part in Model to frame a render view.".into(), false))
        } else {
            Some(("The native Model preview is unavailable.".into(), false))
        };
        let saved = saved_scene(project).ok().flatten();
        let screen = ctx.content_rect().size();
        let size = (screen.x - 60.).clamp(250., 880.);
        let height = (screen.y - 250.).clamp(160., 540.);
        let mut action = None;
        let id = egui::Id::new("project-render-preview");
        let modal = egui::Modal::new(id)
            .area(egui::Modal::default_area(id).fade_in(false))
            .backdrop_color(Color32::from_black_alpha(88))
            .show(ctx, |ui| {
                ui.set_width(size);
                crate::controls::inspector_layout(ui);
                ui.heading("Render preview");
                ui.small("Drag to orbit · middle-drag to pan · scroll to zoom");
                ui.horizontal_wrapped(|ui| {
                    ui.add_enabled_ui(current.is_some(), |ui| {
                        if ui.button("Fit model").clicked() {
                            preview.fit_model();
                        }
                        if ui.button("Orbit left").clicked() {
                            preview.camera.orbit(egui::vec2(30., 0.));
                        }
                        if ui.button("Orbit right").clicked() {
                            preview.camera.orbit(egui::vec2(-30., 0.));
                        }
                        ui.checkbox(&mut preview.camera.perspective, "Perspective");
                        egui::ComboBox::from_id_salt("render-display-style")
                            .selected_text(preview.style.label())
                            .show_ui(ui, |ui| {
                                for style in DisplayStyle::ALL {
                                    ui.selectable_value(&mut preview.style, style, style.label());
                                }
                            });
                    });
                });
                if let Err(error) =
                    preview.viewport(ui, current.as_ref(), retained.as_ref(), height)
                {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
                if let Some((message, error)) = &source_status {
                    ui.horizontal_wrapped(|ui| {
                        if project.scene.pending() {
                            ui.spinner();
                        }
                        if *error {
                            ui.colored_label(Color32::LIGHT_RED, message);
                        } else {
                            ui.small(message);
                        }
                    });
                } else if let Some((_, _, saved)) = &saved {
                    match matches_saved(&preview, saved) {
                        Some(true) => {
                            ui.small("Showing the saved render view.");
                        }
                        Some(false) => {
                            ui.small("This camera or display style differs from the saved view.");
                        }
                        None => {}
                    }
                } else {
                    ui.small("No render view saved yet.");
                }
                ui.small(CAPTURE_HELP);
                egui::CollapsingHeader::new("Blender handoff")
                    .open(preview.handoff.then_some(true))
                    .show(ui, |ui| {
                        ui.label(BLENDER_AVAILABILITY);
                    });
                preview.handoff = false;
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            preview.ready(project) && !project.pending_edits(),
                            egui::Button::new("Capture render view"),
                        )
                        .clicked()
                    {
                        action = Some(Action::Capture);
                    }
                    if ui
                        .add_enabled(saved.is_some(), egui::Button::new("Show saved view"))
                        .clicked()
                    {
                        action = Some(Action::Restore);
                    }
                    if ui.button("Close").clicked() {
                        action = Some(Action::Close);
                    }
                });
            });
        if modal.should_close() || matches!(action, Some(Action::Close)) {
            return;
        }
        self.render_preview = Some(preview);
        let result = match action {
            Some(Action::Capture) => self.invoke_render_view("appearance", Channel::Ui),
            Some(Action::Restore) => self.invoke_render_view("render-view", Channel::Ui),
            Some(Action::Close) | None => return,
        };
        if let Err(error) = result {
            self.message = error;
            self.message_error = true;
        } else {
            self.message_error = false;
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modeling::native_fixture::NativeBackend;
    use multicad_core::model::{
        self, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length,
    };
    use std::time::{Duration, Instant};

    struct MemoryStorage;
    impl DocumentStorage for MemoryStorage {
        fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
            Err("Not used".into())
        }
        fn load(&mut self, _: &std::path::Path) -> Result<Document, String> {
            Err("Not used".into())
        }
    }

    fn frame(
        app: &mut Application,
        ctx: &egui::Context,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                focused: true,
                ..Default::default()
            },
            |ui| app.show(ui),
        )
    }

    fn ready(app: &mut Application, ctx: &egui::Context, size: egui::Vec2) {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            frame(app, ctx, size, vec![]).drop_without_applying_deltas();
            if app.render_command_enabled("appearance") {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "The native render preview did not frame the Model"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn click(app: &mut Application, ctx: &egui::Context, size: egui::Vec2, label: &str) {
        let out = frame(app, ctx, size, vec![]);
        let pos = out
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    let bounds = egui::Rect::from_min_size(text.pos, text.galley.size());
                    shape
                        .clip_rect
                        .contains_rect(bounds)
                        .then_some(bounds.center())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("No visible {label} control"));
        out.drop_without_applying_deltas();
        for pressed in [true, false] {
            frame(
                app,
                ctx,
                size,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            )
            .drop_without_applying_deltas();
        }
    }

    #[test]
    fn general_render_preview_captures_and_restores_native_camera_at_both_window_sizes() {
        for size in [egui::vec2(1360., 860.), egui::vec2(1024., 600.)] {
            let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
            app.pending_files.clear();
            app.welcome = false;
            app.mode = Mode::Parameters;
            let session = &mut app.current_mut().session;
            let part = PrimitiveRequest::new(
                "Review Box",
                PrimitiveParameters::Box {
                    size: [length(60.), length(20.), length(30.)],
                    frame: PrimitiveFrame::default(),
                },
            );
            let prepared = model::prepare_primitive(
                session,
                session.revision(),
                Actor::local(Channel::Ui),
                &part,
            )
            .unwrap();
            session.commit(prepared.prepared).unwrap();
            app.set_scene_backends(Arc::new(|| Box::new(NativeBackend::new())));
            let ctx = egui::Context::default();
            let revision = app.current().session.revision();
            let model_camera = app.current().sketch.camera;
            assert!(!app.render_command_enabled("render-view"));
            app.invoke(Command::Extension("appearance"), &[], Channel::Ui)
                .unwrap();
            assert_eq!((app.mode, app.tab), (Mode::Parameters, 0));
            assert_eq!(app.current().session.revision(), revision);
            assert!(!app.render_command_enabled("appearance"));
            ready(&mut app, &ctx, size);
            let preview = app.render_preview.as_mut().unwrap();
            preview.camera.perspective = true;
            preview.style = DisplayStyle::VisibleEdges;
            click(&mut app, &ctx, size, "Capture render view");
            assert_eq!(app.current().session.revision(), revision + 1);
            let (_, _, saved) = saved_scene(app.current()).unwrap().unwrap();
            assert_eq!(saved.display, model::RenderDisplay::ShadedEdges);
            assert!(matches!(
                saved.camera.projection,
                Projection::Perspective { .. }
            ));
            assert_eq!(saved.lights.len(), 1);
            assert!(!app.render_command_enabled("appearance"));
            ready(&mut app, &ctx, size);
            click(&mut app, &ctx, size, "Orbit left");
            assert_eq!(
                matches_saved(app.render_preview.as_ref().unwrap(), &saved),
                Some(false)
            );
            // Close the preview, then reopen the saved view through the existing command.
            click(&mut app, &ctx, size, "Close");
            assert!(app.render_preview.is_none());
            app.invoke(Command::Extension("render-view"), &[], Channel::Ui)
                .unwrap();
            ready(&mut app, &ctx, size);
            assert_eq!(
                matches_saved(app.render_preview.as_ref().unwrap(), &saved),
                Some(true)
            );
            assert_eq!(app.current().sketch.camera, model_camera);
            assert_eq!(app.current().session.revision(), revision + 1);
            app.invoke(Command::Extension("blender"), &[], Channel::Ui)
                .unwrap();
            let out = frame(&mut app, &ctx, size, vec![]);
            assert!(out.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains(".render.json"))
            }));
            out.drop_without_applying_deltas();
            click(&mut app, &ctx, size, "Close");
            assert!(app.render_preview.is_none());
        }
    }
}

//! Export → Render workspace: the 3D viewport with the document's render
//! scene beside it. Capture and Show act on this viewport, so their
//! receipts always describe a view the user can see. The Blender handoff is
//! not written by the window yet; the sidebar says so in plain words and
//! names the automation route instead of an internal tool identifier.
use super::*;
use multicad_core::model::{Projection, RenderCamera};

/// What the Render sidebar shows about the stored scene and the viewport.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RenderStatus {
    pub name: String,
    pub camera: RenderCamera,
    pub display: multicad_core::model::RenderDisplay,
    pub lights: usize,
    pub parts: usize,
    /// Whether the visible viewport currently shows the stored camera and
    /// display style. `None` before the viewport has a size.
    pub viewport_matches: Option<bool>,
}

/// Plain-language availability of the Blender handoff from the window.
pub(super) const BLENDER_AVAILABILITY: &str = "Blender handoff is not written from this window yet. \
An automation client connected through ToskCAD's external control (MCP) can write the \
handoff — a GLB scene and a JSON manifest — into its export folder. Open the GLB in \
Blender with File → Import → glTF 2.0.";

fn close(a: &RenderCamera, b: &RenderCamera) -> bool {
    let scale = a
        .eye_mm
        .iter()
        .chain(a.target_mm.iter())
        .fold(1f64, |m, v| m.max(v.abs()));
    let tol = 1e-4 * scale;
    let near = |x: &[f64; 3], y: &[f64; 3]| (0..3).all(|i| (x[i] - y[i]).abs() <= tol);
    near(&a.eye_mm, &b.eye_mm)
        && near(&a.target_mm, &b.target_mm)
        && match (a.projection, b.projection) {
            (Projection::Perspective { .. }, Projection::Perspective { .. }) => true,
            (
                Projection::Orthographic { height_mm: x },
                Projection::Orthographic { height_mm: y },
            ) => (x - y).abs() <= tol,
            _ => false,
        }
}

impl Application {
    pub(super) fn render_tab(&self) -> bool {
        self.mode == Mode::Export
            && self
                .registry
                .workspace(Mode::Export)
                .tabs
                .get(self.tab)
                .is_some_and(|tab| tab.title == "Render")
    }
    /// Capture and Show always land on the Render tab, where the viewport
    /// they act on is visible.
    pub(super) fn open_render_tab(&mut self) {
        self.mode = Mode::Export;
        if let Some(index) = self
            .registry
            .workspace(Mode::Export)
            .tabs
            .iter()
            .position(|tab| tab.title == "Render")
        {
            self.tab = index;
        }
    }
    pub(super) fn render_status(&self) -> Option<RenderStatus> {
        let p = self.current();
        let document = p.session.document();
        let entity = multicad_core::model::render_scenes(document).next()?;
        let scene = multicad_core::model::read_render_scene(document, entity.id).ok()?;
        let height = p.sketch.viewport_rect.map_or(0., |r| r.height());
        let viewport_matches = crate::render_view::capture(&p.sketch.camera, height)
            .ok()
            .map(|now| {
                let restored = crate::render_view::restore(&scene.camera, height)
                    .ok()
                    .and_then(|c| crate::render_view::capture(&c, height).ok());
                restored.is_some_and(|stored| close(&now, &stored))
                    && crate::render_view::display_of(p.sketch.display_style()) == scene.display
            });
        Some(RenderStatus {
            name: entity.name.clone(),
            camera: scene.camera,
            display: scene.display,
            lights: scene.lights.len(),
            parts: scene.parts.len(),
            viewport_matches,
        })
    }
    /// The right-hand Render sidebar; the viewport fills the rest.
    pub(super) fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        let status = self.render_status();
        let mut command = None;
        egui::Panel::right("render-scene")
            .exact_size(280.)
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::PANEL)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(ui, |ui| {
                ui.heading("Render scene");
                match &status {
                    None => {
                        ui.label("No render scene yet.");
                        ui.label(
                            "Frame the model in this viewport, then Capture render view to store \
                             its camera and display style in the document.",
                        );
                    }
                    Some(s) => {
                        ui.strong(&s.name);
                        egui::Grid::new("render-scene-facts")
                            .num_columns(2)
                            .show(ui, |ui| {
                                ui.label("Projection");
                                ui.label(match s.camera.projection {
                                    Projection::Perspective { fov_deg } => {
                                        format!("Perspective · {fov_deg:.1}° vertical")
                                    }
                                    Projection::Orthographic { height_mm } => {
                                        format!("Orthographic · {height_mm:.1} mm tall")
                                    }
                                });
                                ui.end_row();
                                let fmt = |v: [f64; 3]| {
                                    format!("{:.1}, {:.1}, {:.1} mm", v[0], v[1], v[2])
                                };
                                ui.label("Eye");
                                ui.label(fmt(s.camera.eye_mm));
                                ui.end_row();
                                ui.label("Target");
                                ui.label(fmt(s.camera.target_mm));
                                ui.end_row();
                                ui.label("Display");
                                ui.label(super::render_display_label(s.display));
                                ui.end_row();
                                ui.label("Lights");
                                ui.label(s.lights.to_string());
                                ui.end_row();
                                ui.label("Scope");
                                ui.label(if s.parts == 0 {
                                    "Whole project".to_owned()
                                } else {
                                    format!("{} Part(s)", s.parts)
                                });
                                ui.end_row();
                            });
                        ui.add_space(6.);
                        match s.viewport_matches {
                            Some(true) => {
                                ui.label("The viewport shows this stored camera.");
                            }
                            Some(false) => {
                                ui.colored_label(
                                    Color32::from_rgb(232, 184, 92),
                                    "The viewport differs from the stored camera. Show render \
                                     view restores it; Capture render view replaces it.",
                                );
                            }
                            None => {
                                ui.label("The viewport is not laid out yet.");
                            }
                        }
                    }
                }
                ui.add_space(8.);
                ui.horizontal_wrapped(|ui| {
                    for (label, id) in [
                        ("Capture render view", "appearance"),
                        ("Show render view", "render-view"),
                    ] {
                        let c = Command::Extension(id);
                        if ui
                            .add_enabled(self.enabled(c), egui::Button::new(label))
                            .clicked()
                        {
                            command = Some(c);
                        }
                    }
                });
                ui.separator();
                ui.strong("Blender handoff");
                ui.label(BLENDER_AVAILABILITY);
            });
        if let Some(command) = command
            && let Err(error) = self.invoke(command, &[], Channel::Ui)
        {
            self.message = error;
            self.message_error = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::DisplayStyle;
    struct MemoryStorage;
    impl crate::storage::DocumentStorage for MemoryStorage {
        fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
            Err("Not used".into())
        }
        fn load(&mut self, _: &std::path::Path) -> Result<Document, String> {
            Err("Not used".into())
        }
    }

    fn frame(app: &mut Application, ctx: &egui::Context) {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1360.0, 860.0),
                )),
                ..Default::default()
            },
            |ui| app.show(ui),
        )
        .drop_without_applying_deltas();
    }

    #[test]
    fn render_tab_shows_the_viewport_and_capture_show_describe_it() {
        let ctx = egui::Context::default();
        let mut app = Application::new(Box::new(MemoryStorage)).unwrap();
        app.welcome = false;
        app.mode = Mode::Export;
        app.tab = 0;
        app.current_mut().sketch.viewport_rect = None;
        frame(&mut app, &ctx);
        assert!(!app.render_tab());
        assert!(
            app.current().sketch.viewport_rect.is_none(),
            "Common keeps its forms, without a viewport"
        );
        app.open_render_tab();
        assert!(app.render_tab());
        frame(&mut app, &ctx);
        let rect = app
            .current()
            .sketch
            .viewport_rect
            .expect("the Render tab lays out the 3D viewport");
        assert!(rect.width() > 300. && rect.height() > 300.);
        assert!(app.render_status().is_none());
        // Capture stores the visible viewport camera and says so.
        app.current_mut()
            .sketch
            .set_display_style(DisplayStyle::VisibleEdges);
        app.invoke(Command::Extension("appearance"), &[], Channel::Ui)
            .unwrap();
        assert!(app.message.contains("render viewport"), "{}", app.message);
        assert!(!app.message.contains("blender_handoff"), "{}", app.message);
        let status = app.render_status().unwrap();
        assert_eq!(status.viewport_matches, Some(true));
        // Moving the camera is visible as a mismatch; Show restores it.
        app.current_mut().sketch.camera.yaw += 0.7;
        app.current_mut()
            .sketch
            .set_display_style(DisplayStyle::Wire);
        assert_eq!(app.render_status().unwrap().viewport_matches, Some(false));
        // Show from the Common tab moves to the Render tab first.
        app.tab = 0;
        app.invoke(Command::Extension("render-view"), &[], Channel::Ui)
            .unwrap();
        assert!(app.render_tab());
        assert!(app.message.contains("render viewport"), "{}", app.message);
        assert_eq!(app.render_status().unwrap().viewport_matches, Some(true));
        assert_eq!(
            app.current().sketch.display_style(),
            DisplayStyle::VisibleEdges
        );
        frame(&mut app, &ctx);
        assert_eq!(app.render_status().unwrap().viewport_matches, Some(true));
    }

    #[test]
    fn blender_availability_is_plain_language() {
        assert!(!BLENDER_AVAILABILITY.contains("blender_handoff"));
        assert!(BLENDER_AVAILABILITY.contains("not written from this window"));
        let app = Application::new(Box::new(MemoryStorage)).unwrap();
        assert!(!app.enabled(Command::Extension("blender")));
    }
}

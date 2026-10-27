//! Interactive proof viewer. Geometry remains owned by the kernel worker.
use crate::{
    camera::Camera,
    geometry::Built,
    raster::{Raster, Scene, Style},
};
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
pub struct Viewport {
    pub camera: Camera,
    pub selected: Option<usize>,
    pub frames: u64,
    pub last_ms: f64,
    style: Style,
    grid: bool,
    spacing: f32,
    count: i32,
    settings: bool,
    scene: Option<Scene>,
    texture: Option<egui::TextureHandle>,
    raster: Option<Raster>,
    key: String,
}
impl Default for Viewport {
    fn default() -> Self {
        Self {
            camera: Camera::default(),
            selected: None,
            frames: 0,
            last_ms: 0.0,
            style: Style::VisibleEdges,
            grid: true,
            spacing: 20.0,
            count: 20,
            settings: false,
            scene: None,
            texture: None,
            raster: None,
            key: String::new(),
        }
    }
}
impl Viewport {
    pub fn invalidate(&mut self) {
        self.scene = None;
        self.key.clear();
    }
    fn fit(&mut self, rect: Rect) {
        if let Some(scene) = &self.scene {
            self.camera.fit(&scene.points(self.selected), rect);
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, built: Option<&Built>, pick_plane: bool) -> bool {
        let start = std::time::Instant::now();
        self.frames += 1;
        let (response, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let rect = response.rect;
        let image_slot = painter.add(egui::Shape::Noop);
        if self.scene.is_none() {
            self.scene = built.map(Scene::new);
        }
        let orientation = egui::Area::new(ui.id().with("orientation-cross"))
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(rect.right_top() + Vec2::new(-4.0, 4.0))
            .movable(false)
            .show(ui.ctx(), |ui| {
                crate::navigation::orientation(ui, &mut self.camera)
            });
        let toolbar = egui::Area::new(ui.id().with("camera-toolbar"))
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .fixed_pos(rect.right_bottom() - Vec2::splat(8.0))
            .movable(false)
            .show(ui.ctx(), |ui| {
                crate::navigation::toolbar(ui, &mut self.camera, &mut self.style)
            });
        let over_control = ui.input(|i| {
            i.pointer.hover_pos().is_some_and(|p| {
                orientation.response.rect.contains(p) || toolbar.response.rect.contains(p)
            })
        });
        if !over_control {
            if response.dragged_by(egui::PointerButton::Secondary) {
                self.camera.orbit(ui.input(|i| i.pointer.delta()));
            }
            if response.dragged_by(egui::PointerButton::Middle) {
                self.camera.pan(ui.input(|i| i.pointer.delta()));
            }
            if response.hovered() {
                self.camera.zoom(ui.input(|i| i.smooth_scroll_delta.y));
            }
        }
        let actions = toolbar.inner;
        if actions.home {
            self.camera = Camera::default();
            self.selected = None;
            self.fit(rect);
        }
        if actions.fit {
            self.fit(rect);
        }
        if actions.grid {
            self.settings = !self.settings;
        }
        // Resolution bounded for this software depth proof. Cached until a scene,
        // camera, selection, style, grid or viewport-size change occurs.
        let width = rect.width().clamp(1.0, 1200.0) as usize;
        let height =
            (rect.height() * width as f32 / rect.width().max(1.0)).clamp(1.0, 1200.0) as usize;
        let key = format!(
            "{:?}:{:?}:{:?}:{}:{}:{}:{width}:{height}",
            self.camera, self.selected, self.style, self.grid, self.spacing, self.count
        );
        if key != self.key {
            let mut raster = Raster::new(width, height);
            if let Some(scene) = &self.scene {
                let mut camera = self.camera;
                camera.scale *= width as f32 / rect.width().max(1.0);
                raster.render(
                    scene,
                    camera,
                    self.style,
                    self.selected,
                    self.grid.then_some((self.spacing, self.count)),
                );
            }
            let image = raster.image.clone();
            if let Some(texture) = &mut self.texture {
                texture.set(image, egui::TextureOptions::LINEAR);
            } else {
                self.texture = Some(ui.ctx().load_texture(
                    "cad-display",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
            self.raster = Some(raster);
            self.key = key;
        }
        // Place image behind overlay controls, which are already emitted above.
        if let Some(texture) = &self.texture {
            painter.set(
                image_slot,
                egui::Shape::image(
                    texture.id(),
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                ),
            );
        }
        let plane = [
            [-20.0, -20.0, 0.0],
            [140.0, -20.0, 0.0],
            [140.0, 100.0, 0.0],
            [-20.0, 100.0, 0.0],
        ]
        .map(|p| {
            let p = self.camera.project(p, rect);
            Pos2::new(p[0], p[1])
        });
        if pick_plane {
            painter.add(egui::Shape::convex_polygon(
                plane.to_vec(),
                Color32::from_rgba_unmultiplied(70, 160, 220, 90),
                egui::Stroke::new(2.0, Color32::LIGHT_BLUE),
            ));
        }
        if response.clicked() && !over_control && !pick_plane {
            self.selected = response.interact_pointer_pos().and_then(|p| {
                self.raster.as_ref().and_then(|r| {
                    let x = ((p.x - rect.min.x) / rect.width() * width as f32) as usize;
                    let y = ((p.y - rect.min.y) / rect.height() * height as f32) as usize;
                    r.owner.get(y * width + x).copied().flatten()
                })
            });
            self.key.clear();
            ui.ctx().request_repaint();
        }
        if self.settings {
            egui::Window::new("XY grid")
                .open(&mut self.settings)
                .show(ui.ctx(), |ui| {
                    ui.checkbox(&mut self.grid, "Show XY floor grid");
                    ui.horizontal(|ui| {
                        ui.label("Spacing (mm)");
                        ui.add(egui::DragValue::new(&mut self.spacing).range(0.1..=1000.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Lines each side");
                        ui.add(egui::DragValue::new(&mut self.count).range(2..=100));
                    });
                });
        }
        self.last_ms = start.elapsed().as_secs_f64() * 1000.0;
        pick_plane
            && response.clicked()
            && !over_control
            && response.interact_pointer_pos().is_some_and(|p| {
                inside(p, [plane[0], plane[1], plane[2]])
                    || inside(p, [plane[0], plane[2], plane[3]])
            })
    }
}
fn inside(p: Pos2, t: [Pos2; 3]) -> bool {
    let cross = |a: Pos2, b: Pos2| {
        let u = b - a;
        let v = p - a;
        u.x * v.y - u.y * v.x
    };
    let a = cross(t[0], t[1]);
    let b = cross(t[1], t[2]);
    let c = cross(t[2], t[0]);
    (a > 0.0 && b > 0.0 && c > 0.0) || (a < 0.0 && b < 0.0 && c < 0.0)
}

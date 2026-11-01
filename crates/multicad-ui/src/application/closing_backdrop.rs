//! Bounded, local screenshot blur for the quit confirmation. Capture one
//! input-blocked document frame before painting the modal itself.
use super::*;

#[derive(Debug)]
struct QuitCapture(EntityId);
pub(super) struct Backdrop {
    tag: EntityId,
    requested: bool,
    passes: u32,
    size: egui::Vec2,
    texture: Option<egui::TextureHandle>,
}
impl Default for Backdrop {
    fn default() -> Self {
        Self {
            tag: EntityId::new(),
            requested: false,
            passes: 0,
            size: egui::Vec2::ZERO,
            texture: None,
        }
    }
}
impl Backdrop {
    pub(super) fn prepare(&mut self, ctx: &egui::Context) -> bool {
        let size = ctx.content_rect().size();
        if self.requested && self.size != size {
            *self = Self::default();
        }
        if !self.requested {
            self.size = size;
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                QuitCapture(self.tag),
            )));
            ctx.request_repaint();
            return false;
        }
        if self.texture.is_none() {
            let image = ctx.input(|input| {
                input.events.iter().find_map(|event| match event {
                    egui::Event::Screenshot {
                        user_data, image, ..
                    } if user_data
                        .data
                        .as_ref()
                        .and_then(|d| d.downcast_ref::<QuitCapture>())
                        .is_some_and(|tag| tag.0 == self.tag) =>
                    {
                        Some(image.clone())
                    }
                    _ => None,
                })
            });
            if let Some(image) = image {
                self.texture = Some(ctx.load_texture(
                    "quit-confirmation-backdrop",
                    blur(&image),
                    egui::TextureOptions::LINEAR,
                ));
            } else if self.passes < 30 {
                self.passes += 1;
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
                // Wgpu captures the frame after the Screenshot request.
                // Keep capture frames free of this dialog until its reply.
                return false;
            }
        }
        true
    }
    #[cfg(test)]
    pub(super) fn testing_screenshot(&self) -> egui::Event {
        egui::Event::Screenshot {
            viewport_id: egui::ViewportId::ROOT,
            user_data: egui::UserData::new(QuitCapture(self.tag)),
            image: std::sync::Arc::new(egui::ColorImage::filled([20, 20], Color32::GRAY)),
        }
    }
    pub(super) fn paint(&self, ctx: &egui::Context, layer: egui::LayerId) {
        if let Some(texture) = &self.texture {
            ctx.layer_painter(layer).image(
                texture.id(),
                ctx.content_rect(),
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                Color32::WHITE,
            );
        }
    }
}

fn blur(image: &egui::ColorImage) -> egui::ColorImage {
    let [width, height] = image.size;
    if width == 0 || height == 0 || image.pixels.len() != width.saturating_mul(height) {
        return egui::ColorImage::filled([1, 1], Color32::BLACK);
    }
    let scale = (640. / width as f64).min(400. / height as f64).min(1.);
    let w = ((width as f64 * scale).round() as usize).max(1);
    let h = ((height as f64 * scale).round() as usize).max(1);
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            pixels.push(image.pixels[(y * height / h) * width + x * width / w]);
        }
    }
    // Three separable box passes approximate a soft Gaussian without a new
    // runtime dependency; work and storage depend only on the reduced image.
    for _ in 0..3 {
        pixels = box_pass(&pixels, w, h, true);
        pixels = box_pass(&pixels, w, h, false);
    }
    egui::ColorImage::new([w, h], pixels)
}
fn box_pass(pixels: &[Color32], w: usize, h: usize, horizontal: bool) -> Vec<Color32> {
    let (lines, length) = if horizontal { (h, w) } else { (w, h) };
    let mut out = vec![Color32::BLACK; pixels.len()];
    let index = |line: usize, p: usize| {
        if horizontal {
            line * w + p
        } else {
            p * w + line
        }
    };
    for line in 0..lines {
        let mut sum = [0u32; 4];
        for offset in -3isize..=3 {
            let color =
                pixels[index(line, offset.clamp(0, length as isize - 1) as usize)].to_array();
            for (s, c) in sum.iter_mut().zip(color) {
                *s += u32::from(c);
            }
        }
        for p in 0..length {
            let [r, g, b, a] = sum.map(|s| (s / 7) as u8);
            out[index(line, p)] = Color32::from_rgba_premultiplied(r, g, b, a);
            let remove = pixels[index(line, p.saturating_sub(3))].to_array();
            let add = pixels[index(line, (p + 4).min(length - 1))].to_array();
            for ((s, removed), added) in sum.iter_mut().zip(remove).zip(add) {
                *s = *s - u32::from(removed) + u32::from(added);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    #[test]
    fn blur_softens_edges_preserves_flat_color_and_bounds_large_screenshots() {
        let flat = egui::ColorImage::filled([100, 80], Color32::from_rgb(30, 60, 90));
        assert!(blur(&flat).pixels.iter().all(|p| *p == flat.pixels[0]));
        let image = egui::ColorImage::new(
            [100, 40],
            (0..4000)
                .map(|i| {
                    if i % 100 < 50 {
                        Color32::BLACK
                    } else {
                        Color32::WHITE
                    }
                })
                .collect(),
        );
        let soft = blur(&image);
        assert!(soft.pixels[20 * 100 + 49].r() > 0);
        assert!(soft.pixels[20 * 100 + 50].r() < 255);
        let large = blur(&egui::ColorImage::filled([2000, 1000], Color32::BLUE));
        assert_eq!(large.size, [640, 320]);
        assert!(large.pixels.iter().all(|p| *p == Color32::BLUE));
    }
    #[test]
    fn capture_precedes_overlay_and_foreign_or_stale_screenshots_are_ignored() {
        let ctx = egui::Context::default();
        let mut backdrop = Backdrop::default();
        let mut ready = false;
        let output = ctx.run_ui(Default::default(), |_| ready = backdrop.prepare(&ctx));
        assert!(!ready);
        assert!(
            output
                .viewport_output
                .values()
                .flat_map(|o| &o.commands)
                .any(|c| matches!(c, egui::ViewportCommand::Screenshot(_)))
        );
        output.drop_without_applying_deltas();
        let shot = |tag| egui::Event::Screenshot {
            viewport_id: egui::ViewportId::ROOT,
            user_data: egui::UserData::new(QuitCapture(tag)),
            image: Arc::new(egui::ColorImage::filled([20, 20], Color32::BLUE)),
        };
        for tag in [EntityId::new(), backdrop.tag] {
            ctx.run_ui(
                egui::RawInput {
                    events: vec![shot(tag)],
                    ..Default::default()
                },
                |_| ready = backdrop.prepare(&ctx),
            )
            .drop_without_applying_deltas();
            assert_eq!(ready, tag == backdrop.tag);
            assert_eq!(backdrop.texture.is_some(), tag == backdrop.tag);
        }
        let old = backdrop.tag;
        backdrop = Backdrop::default();
        assert_ne!(backdrop.tag, old);
    }
}

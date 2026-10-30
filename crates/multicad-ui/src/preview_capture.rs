//! Save-time preview capture (CAD-141): the 3D viewport as last drawn becomes
//! the file's preview image. The renderer records where it painted the scene;
//! the application asks the window for a screenshot, crops it to that rect,
//! downscales it to the preview bounds and encodes a PNG. Everything after the
//! screenshot works on a plain `ColorImage`, so tests inject images without a GPU.
use eframe::egui::{self, Color32, ColorImage, Rect};
use multicad_core::project_file::info::{MAX_PREVIEW_BYTES, PreviewImage};

/// Longest preview side written by the capture (well inside core's limits).
pub const CAPTURE_MAX: [usize; 2] = [640, 480];
const RECT_ID: &str = "multicad.preview-capture.viewport";

/// Called by the scene painter for each 3D viewport it draws.
pub fn record_viewport(ctx: &egui::Context, rect: Rect) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(RECT_ID), (rect, pass)));
}
/// The 3D viewport rect if one was drawn in this or the previous pass.
pub fn visible_viewport(ctx: &egui::Context) -> Option<Rect> {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<(Rect, u64)>(egui::Id::new(RECT_ID)))
        .filter(|(rect, drawn)| pass.saturating_sub(*drawn) <= 1 && rect.is_positive())
        .map(|(rect, _)| rect)
}
/// Crop a window screenshot (physical pixels) to a viewport rect in points.
pub fn crop(
    screenshot: &ColorImage,
    rect: Rect,
    pixels_per_point: f32,
) -> Result<ColorImage, String> {
    let [w, h] = screenshot.size;
    let scale = pixels_per_point.max(0.1);
    let clamp = |v: f32, max: usize| ((v * scale).round().max(0.) as usize).min(max);
    let (x0, x1) = (clamp(rect.min.x, w), clamp(rect.max.x, w));
    let (y0, y1) = (clamp(rect.min.y, h), clamp(rect.max.y, h));
    if x1 <= x0 + 1 || y1 <= y0 + 1 {
        return Err("The 3D view is not on screen".into());
    }
    Ok(screenshot.region_by_pixels([x0, y0], [x1 - x0, y1 - y0]))
}
/// Area-average downscale so the image fits `max`, keeping its aspect ratio.
pub fn fit(image: &ColorImage, max: [usize; 2]) -> ColorImage {
    let [w, h] = image.size;
    let scale = (max[0] as f64 / w as f64)
        .min(max[1] as f64 / h as f64)
        .min(1.);
    let (nw, nh) = (
        ((w as f64 * scale).round() as usize).max(1),
        ((h as f64 * scale).round() as usize).max(1),
    );
    if (nw, nh) == (w, h) {
        return image.clone();
    }
    let mut pixels = Vec::with_capacity(nw * nh);
    for y in 0..nh {
        let (sy0, sy1) = (y * h / nh, ((y + 1) * h / nh).max(y * h / nh + 1));
        for x in 0..nw {
            let (sx0, sx1) = (x * w / nw, ((x + 1) * w / nw).max(x * w / nw + 1));
            let mut sum = [0u32; 4];
            for sy in sy0..sy1.min(h) {
                for sx in sx0..sx1.min(w) {
                    let p = image.pixels[sy * w + sx].to_srgba_unmultiplied();
                    for (s, v) in sum.iter_mut().zip(p) {
                        *s += u32::from(v);
                    }
                }
            }
            let n = ((sy1.min(h) - sy0) * (sx1.min(w) - sx0)).max(1) as u32;
            let [r, g, b, a] = sum.map(|s| (s / n) as u8);
            pixels.push(Color32::from_rgba_unmultiplied(r, g, b, a));
        }
    }
    ColorImage::new([nw, nh], pixels)
}
fn encode_png(image: &ColorImage) -> Result<Vec<u8>, String> {
    let [w, h] = image.size;
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, w as u32, h as u32);
    // The viewport is opaque; RGB keeps the file small.
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    let data: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|p| {
            let [r, g, b, _] = p.to_srgba_unmultiplied();
            [r, g, b]
        })
        .collect();
    writer.write_image_data(&data).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(bytes)
}
/// Downscale and encode a captured view as a checked file preview.
pub fn preview_from(image: &ColorImage) -> Result<PreviewImage, String> {
    let mut max = CAPTURE_MAX;
    loop {
        let png =
            encode_png(&fit(image, max)).map_err(|e| format!("Cannot encode the preview: {e}"))?;
        if png.len() <= MAX_PREVIEW_BYTES {
            return PreviewImage::from_png(png);
        }
        if max[0] < 64 {
            return Err("The captured preview is too large to store".into());
        }
        max = [max[0] / 2, max[1] / 2];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn gradient(w: usize, h: usize) -> ColorImage {
        ColorImage::new(
            [w, h],
            (0..w * h)
                .map(|i| Color32::from_rgb((i % w) as u8, (i / w) as u8, 90))
                .collect(),
        )
    }
    #[test]
    fn crops_downscales_and_encodes_a_decodable_bounded_png() {
        let shot = gradient(1600, 1000);
        let rect = Rect::from_min_max(egui::pos2(100., 50.), egui::pos2(700., 450.));
        let cropped = crop(&shot, rect, 2.).unwrap();
        assert_eq!(cropped.size, [1200, 800]);
        assert_eq!(cropped.pixels[0], shot.pixels[100 * 1600 + 200]);
        let preview = preview_from(&cropped).unwrap();
        assert_eq!((preview.width(), preview.height()), (640, 427));
        let decoded = crate::library::decode_preview(&preview).unwrap();
        assert_eq!(decoded.size, [640, 427]);
        assert!(preview.png().len() <= MAX_PREVIEW_BYTES);
    }
    #[test]
    fn off_screen_rects_and_small_images_are_handled() {
        let shot = gradient(100, 100);
        let off = Rect::from_min_max(egui::pos2(200., 200.), egui::pos2(300., 300.));
        assert!(crop(&shot, off, 1.).is_err());
        let small = fit(&shot, CAPTURE_MAX);
        assert_eq!(small.size, [100, 100], "never upscales");
    }
    #[test]
    fn the_viewport_rect_is_visible_only_right_after_it_was_drawn() {
        let ctx = egui::Context::default();
        assert!(visible_viewport(&ctx).is_none());
        let rect = Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(10., 10.));
        record_viewport(&ctx, rect);
        assert_eq!(visible_viewport(&ctx), Some(rect));
        for _ in 0..3 {
            ctx.run_ui(Default::default(), |_| {})
                .drop_without_applying_deltas();
        }
        assert!(visible_viewport(&ctx).is_none());
    }
}
